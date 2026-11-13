//! Turns Claude's NDJSON output into shift `Event`s and the replies shift
//! owes Claude. It does no I/O, so a saved transcript replays through it.

use serde_json::{Value, json};
use shift_core::{AgentDef, Event, ToolSpec, policy};

use crate::shift_tool;

/// What one line of Claude output leads to.
#[derive(Debug, Default, PartialEq)]
pub struct Output {
    pub events: Vec<Event>,
    /// Frames to write back to Claude's stdin.
    pub replies: Vec<Value>,
    /// Shift tool calls to run. They may block, so the caller runs them off
    /// the reader thread and answers each with [`tool_reply`].
    pub calls: Vec<ToolCall>,
}

/// A shift tool call Claude is waiting on.
#[derive(Debug, PartialEq)]
pub struct ToolCall {
    request_id: Value,
    mcp_id: Value,
    pub name: String,
    pub arguments: Value,
}

/// The name Claude gives the in-process shift MCP server.
pub const SERVER: &str = "shift";

pub struct Translator {
    agent: AgentDef,
    /// The shift tools served to this session, as MCP tool definitions.
    tools: Vec<Value>,
}

impl Translator {
    pub fn new(agent: AgentDef, tools: &[ToolSpec]) -> Self {
        let tools = tools
            .iter()
            .map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.input_schema}))
            .collect();
        Self { agent, tools }
    }

    fn serves(&self, claude_tool: &str) -> bool {
        claude_tool
            .strip_prefix("mcp__shift__")
            .is_some_and(|name| self.tools.iter().any(|t| t["name"] == name))
    }

    pub fn handle_line(&mut self, line: &str) -> Output {
        let mut out = Output::default();
        let Ok(frame) = serde_json::from_str::<Value>(line) else {
            return out;
        };
        match frame["type"].as_str() {
            Some("stream_event") => {
                let delta = &frame["event"]["delta"];
                if delta["type"] == "text_delta"
                    && let Some(text) = delta["text"].as_str()
                {
                    out.events.push(Event::Text(text.into()));
                }
            }
            // Text already arrived as deltas; the full message adds the
            // complete tool calls.
            Some("assistant") => {
                for item in content(&frame) {
                    if item["type"] == "tool_use" {
                        out.events.push(Event::ToolCall {
                            id: string(&item["id"]),
                            name: string(&item["name"]),
                            input: item["input"].clone(),
                        });
                    }
                }
            }
            Some("user") => {
                for item in content(&frame) {
                    if item["type"] == "tool_result" {
                        out.events.push(Event::ToolResult {
                            id: string(&item["tool_use_id"]),
                            output: text_of(&item["content"]),
                            is_error: item["is_error"] == true,
                        });
                    }
                }
            }
            Some("result") => {
                let is_error = frame["is_error"] == true;
                if is_error {
                    // A failed execution has `errors` instead of `result`.
                    let text = match frame["result"].as_str() {
                        Some(text) => text.to_owned(),
                        None => text_of(&frame["errors"]),
                    };
                    out.events.push(Event::Error(text));
                }
                out.events.push(Event::TurnDone { is_error });
            }
            Some("control_request") => self.control(&frame, &mut out),
            _ => {}
        }
        out
    }

    fn control(&mut self, frame: &Value, out: &mut Output) {
        let request = &frame["request"];
        let response = match request["subtype"].as_str() {
            Some("can_use_tool") => {
                let tool = string(&request["tool_name"]);
                // shift's own tools are always the agent's to call.
                let allowed = self.serves(&tool) || policy::allows(&self.agent, shift_tool(&tool));
                let response = if allowed {
                    json!({"behavior": "allow", "updatedInput": request["input"]})
                } else {
                    json!({
                        "behavior": "deny",
                        "message": format!("shift: `{tool}` is not in this agent's tools."),
                    })
                };
                out.events.push(Event::PermissionRequest { tool, allowed });
                response
            }
            Some("mcp_message") => {
                let message = &request["message"];
                let result = match message["method"].as_str() {
                    Some("initialize") => json!({
                        "protocolVersion": message["params"]["protocolVersion"],
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": SERVER, "version": env!("CARGO_PKG_VERSION")},
                    }),
                    Some("tools/list") => json!({"tools": self.tools}),
                    Some("tools/call") => {
                        out.calls.push(ToolCall {
                            request_id: frame["request_id"].clone(),
                            mcp_id: message["id"].clone(),
                            name: string(&message["params"]["name"]),
                            arguments: message["params"]["arguments"].clone(),
                        });
                        return;
                    }
                    // Notifications and anything else get an empty result.
                    _ => json!({}),
                };
                mcp_response(&message["id"], result)
            }
            _ => return,
        };
        out.replies
            .push(control_response(&frame["request_id"], response));
    }
}

/// The frame that answers a shift tool call with its result.
pub fn tool_reply(call: &ToolCall, result: Result<String, String>) -> Value {
    let (text, is_error) = match result {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    let result = json!({"content": [{"type": "text", "text": text}], "isError": is_error});
    control_response(&call.request_id, mcp_response(&call.mcp_id, result))
}

fn mcp_response(id: &Value, result: Value) -> Value {
    let mut response = json!({"jsonrpc": "2.0", "result": result});
    if !id.is_null() {
        response["id"] = id.clone();
    }
    json!({"mcp_response": response})
}

fn control_response(request_id: &Value, response: Value) -> Value {
    json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": request_id, "response": response},
    })
}

fn content(frame: &Value) -> impl Iterator<Item = &Value> {
    frame["message"]["content"].as_array().into_iter().flatten()
}

fn string(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// A string, or a list of strings or text content blocks.
fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.as_str().or(b["text"].as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
