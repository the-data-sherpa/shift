//! Recorded-session tests: replay saved Claude output with no account.

use serde_json::json;
use shift_adapter_claude::{Translator, launch_args};
use shift_core::{Event, builtin_agent};

fn replay(transcript: &str) -> (Vec<Event>, Vec<serde_json::Value>) {
    let tools = [shift_core::delegate::delegate_task_spec()];
    let mut translator = Translator::new(builtin_agent("coder").unwrap(), &tools);
    let (mut events, mut replies) = (Vec::new(), Vec::new());
    for line in transcript.lines() {
        let output = translator.handle_line(line);
        events.extend(output.events);
        replies.extend(output.replies);
    }
    (events, replies)
}

#[test]
fn recorded_session_replays_into_events() {
    let (events, replies) = replay(include_str!("fixtures/coder-session.ndjson"));

    let command = json!({
        "command": "echo shift-live > out.txt",
        "description": "Write shift-live to out.txt",
    });
    assert_eq!(
        events,
        [
            Event::ToolCall {
                id: "toolu_01Hti6j7GYgG1sf8DMA5Mnt4".into(),
                name: "Bash".into(),
                input: command.clone(),
            },
            Event::PermissionRequest {
                tool: "Bash".into(),
                allowed: true
            },
            Event::ToolResult {
                id: "toolu_01Hti6j7GYgG1sf8DMA5Mnt4".into(),
                output: "(Bash completed with no output)".into(),
                is_error: false,
            },
            Event::Text("done".into()),
            Event::TurnDone { is_error: false },
        ]
    );
    assert_eq!(
        replies,
        [json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": "82ede63d-1cae-417d-97ad-15faf72d280b",
                "response": {"behavior": "allow", "updatedInput": command},
            },
        })]
    );
}

#[test]
fn gate_denies_a_tool_outside_the_agents_list() {
    let prompt = json!({
        "type": "control_request",
        "request_id": "r1",
        "request": {"subtype": "can_use_tool", "tool_name": "WebFetch", "input": {"url": "x"}},
    });
    let (events, replies) = replay(&prompt.to_string());

    assert_eq!(
        events,
        [Event::PermissionRequest {
            tool: "WebFetch".into(),
            allowed: false
        }]
    );
    assert_eq!(replies[0]["response"]["request_id"], "r1");
    assert_eq!(replies[0]["response"]["response"]["behavior"], "deny");
}

#[test]
fn failed_turn_reports_the_error() {
    let result = json!({"type": "result", "subtype": "success", "is_error": true, "result": "Not logged in"});
    let (events, _) = replay(&result.to_string());
    assert_eq!(
        events,
        [
            Event::Error("Not logged in".into()),
            Event::TurnDone { is_error: true },
        ]
    );

    let result = json!({
        "type": "result",
        "subtype": "error_during_execution",
        "is_error": true,
        "errors": ["first", "second"],
    });
    let (events, _) = replay(&result.to_string());
    assert_eq!(events[0], Event::Error("first\nsecond".into()));
}

#[test]
fn launch_limits_built_in_tools_to_the_agents_list() {
    let mut agent = builtin_agent("coder").unwrap();
    let args = launch_args(&agent).unwrap();
    let tools = args.iter().position(|a| a == "--tools").unwrap();
    assert_eq!(args[tools + 1], "Read,Glob,Grep,Edit,Write,Bash");
    assert!(args.contains(&"--strict-mcp-config".to_string()));

    agent.tools = vec!["fs.read".into()];
    let args = launch_args(&agent).unwrap();
    assert_eq!(args[tools + 1], "Read,Glob,Grep");

    agent.tools = vec!["web.search".into()];
    assert!(launch_args(&agent).is_err());
}

/// Frames as Claude Code 2.1.325 sends them for the in-process shift server.
#[test]
fn shift_tools_are_served_over_the_control_channel() {
    let mcp = |request_id: &str, message: serde_json::Value| {
        json!({
            "type": "control_request",
            "request_id": request_id,
            "request": {"subtype": "mcp_message", "server_name": "shift", "message": message},
        })
        .to_string()
    };
    let arguments =
        json!({"intent": "i", "out_of_scope": "o", "change_summary": "c", "checks": "k"});
    let transcript = [
        mcp("a", json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-11-25"}})),
        mcp("b", json!({"jsonrpc": "2.0", "method": "notifications/initialized"})),
        mcp("c", json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})),
        json!({
            "type": "control_request",
            "request_id": "d",
            "request": {"subtype": "can_use_tool", "tool_name": "mcp__shift__delegate_task", "input": arguments},
        })
        .to_string(),
        mcp("e", json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "delegate_task", "arguments": arguments, "_meta": {"claudecode/toolUseId": "toolu_1"}}})),
    ]
    .join("\n");

    let mut translator = Translator::new(
        builtin_agent("coder").unwrap(),
        &[shift_core::delegate::delegate_task_spec()],
    );
    let (mut events, mut replies, mut calls) = (Vec::new(), Vec::new(), Vec::new());
    for line in transcript.lines() {
        let output = translator.handle_line(line);
        events.extend(output.events);
        replies.extend(output.replies);
        calls.extend(output.calls);
    }

    let reply = |i: usize| &replies[i]["response"]["response"];
    assert_eq!(reply(0)["mcp_response"]["id"], 0);
    assert_eq!(
        reply(0)["mcp_response"]["result"]["protocolVersion"],
        "2025-11-25"
    );
    assert_eq!(
        reply(1)["mcp_response"],
        json!({"jsonrpc": "2.0", "result": {}})
    );
    let listed = &reply(2)["mcp_response"]["result"]["tools"];
    assert_eq!(listed[0]["name"], "delegate_task");
    assert_eq!(
        listed[0]["inputSchema"]["required"],
        json!(["intent", "out_of_scope", "change_summary", "checks"])
    );
    assert!(listed[0]["inputSchema"]["properties"]["ledger"].is_object());
    assert_eq!(reply(3)["behavior"], "allow");
    assert_eq!(
        events,
        [Event::PermissionRequest {
            tool: "mcp__shift__delegate_task".into(),
            allowed: true
        }]
    );

    // The call itself is not answered inline: it blocks, so it runs elsewhere.
    assert_eq!(replies.len(), 4);
    assert_eq!(calls.len(), 1);
    // The call carries Claude's tool use id, the one the pane shows it under.
    assert_eq!(calls[0].call_id, "toolu_1");
    assert_eq!(
        (calls[0].name.as_str(), &calls[0].arguments),
        ("delegate_task", &arguments)
    );

    let done = shift_adapter_claude::tool_reply(&calls[0], Ok("review".into()));
    assert_eq!(done["response"]["request_id"], "e");
    assert_eq!(
        done["response"]["response"]["mcp_response"],
        json!({"jsonrpc": "2.0", "id": 2, "result": {"content": [{"type": "text", "text": "review"}], "isError": false}})
    );
    let failed = shift_adapter_claude::tool_reply(&calls[0], Err("boom".into()));
    assert_eq!(
        failed["response"]["response"]["mcp_response"]["result"]["isError"],
        true
    );
}

#[test]
fn gate_denies_an_mcp_tool_shift_does_not_serve() {
    let prompt = json!({
        "type": "control_request",
        "request_id": "r1",
        "request": {"subtype": "can_use_tool", "tool_name": "mcp__shift__other", "input": {}},
    });
    let (events, _) = replay(&prompt.to_string());
    assert_eq!(
        events,
        [Event::PermissionRequest {
            tool: "mcp__shift__other".into(),
            allowed: false
        }]
    );
}
