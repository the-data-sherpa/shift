//! Recorded-session tests: replay saved Claude output with no account.

use serde_json::json;
use shift_adapter_claude::{Translator, launch_args};
use shift_core::{Event, builtin_agent};

fn replay(transcript: &str) -> (Vec<Event>, Vec<serde_json::Value>) {
    let mut translator = Translator::new(builtin_agent("coder").unwrap());
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
            Event::TurnDone { is_error: true }
        ]
    );
}

#[test]
fn launch_limits_built_in_tools_to_the_agents_list() {
    let mut agent = builtin_agent("coder").unwrap();
    let args = launch_args(&agent).unwrap();
    let tools = args.iter().position(|a| a == "--tools").unwrap();
    assert_eq!(args[tools + 1], "Read,Glob,Grep,Edit,Write,Bash");

    agent.tools = vec!["fs.read".into()];
    let args = launch_args(&agent).unwrap();
    assert_eq!(args[tools + 1], "Read,Glob,Grep");

    agent.tools = vec!["web.search".into()];
    assert!(launch_args(&agent).is_err());
}
