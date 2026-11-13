//! Turns Claude's NDJSON output into shift `Event`s and the replies shift
//! owes Claude. It does no I/O, so a saved transcript replays through it.

use serde_json::{Value, json};
use shift_core::{AgentDef, Event, policy};

use crate::shift_tool;

/// What one line of Claude output leads to.
#[derive(Debug, Default, PartialEq)]
pub struct Output {
    pub events: Vec<Event>,
    /// Frames to write back to Claude's stdin.
    pub replies: Vec<Value>,
}

pub struct Translator {
    agent: AgentDef,
}

impl Translator {
    pub fn new(agent: AgentDef) -> Self {
        Self { agent }
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
                    out.events.push(Event::Error(text_of(&frame["result"])));
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
                let allowed = policy::allows(&self.agent, shift_tool(&tool));
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
            _ => return,
        };
        out.replies.push(json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": frame["request_id"],
                "response": response,
            },
        }));
    }
}

fn content(frame: &Value) -> impl Iterator<Item = &Value> {
    frame["message"]["content"].as_array().into_iter().flatten()
}

fn string(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// Tool output is either a string or a list of content blocks.
fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
