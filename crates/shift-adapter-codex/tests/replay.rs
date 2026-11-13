//! Recorded-session tests: replay saved Codex JSON-RPC with no account.

use shift_adapter_codex::{latest_model, translate};
use shift_core::Event;
use shift_core::delegate::ReviewVerdict;
use shift_core::review::parse_review;

#[test]
fn recorded_review_replays_into_events() {
    let events: Vec<Event> = include_str!("fixtures/reviewer-session.jsonl")
        .lines()
        .flat_map(translate)
        .collect();

    let commands: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            Event::ToolCall { name, input, .. } if name == "shell" => input["command"].as_str(),
            _ => None,
        })
        .collect();
    assert_eq!(commands.len(), 5);
    assert!(commands[0].contains("git diff aed1ab09fc7f41fe533c5c276cea27b34c1908a0"));

    // Every command got its result, matched by id.
    for event in &events {
        if let Event::ToolCall { id, .. } = event {
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, Event::ToolResult { id: done, .. } if done == id))
            );
        }
    }
    assert!(events.contains(&Event::ToolResult {
        id: "exec-d8d07391-8e8c-465c-ba54-de5a44f58273".into(),
        output: "def add(a, b):\n    return a - b\nadd now logs nothing\n".into(),
        is_error: false,
    }));
    assert_eq!(events.last(), Some(&Event::TurnDone { is_error: false }));

    // The text after the last command is the final message: a valid review.
    let last_result = events
        .iter()
        .rposition(|e| matches!(e, Event::ToolResult { .. }))
        .unwrap();
    let final_message: String = events[last_result..]
        .iter()
        .filter_map(|event| match event {
            Event::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let review = parse_review(final_message.trim()).unwrap();
    assert_eq!(review.verdict, ReviewVerdict::Changes);
    assert_eq!(review.findings[0].file, "add.py");
}

#[test]
fn failed_turn_and_errors_are_reported() {
    let error = r#"{"method":"error","params":{"error":{"message":"stream disconnected"},"willRetry":false}}"#;
    assert_eq!(
        translate(error),
        [Event::Error("stream disconnected".into())]
    );
    let failed = r#"{"method":"turn/completed","params":{"turn":{"status":"failed"}}}"#;
    assert_eq!(translate(failed), [Event::TurnDone { is_error: true }]);
}

#[test]
fn latest_model_picks_the_newest_of_the_family_or_nothing() {
    let ids = [
        "gpt-6-sol",
        "gpt-6.1-terra",
        "gpt-6.1-sol",
        "gpt-5.6-sol",
        "gpt-10-luna",
    ]
    .map(String::from);
    assert_eq!(latest_model("sol", &ids).as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(latest_model("luna", &ids).as_deref(), Some("gpt-10-luna"));
    // No Sol offered: nothing is substituted.
    assert_eq!(latest_model("sol", &ids[1..2]), None);
    assert_eq!(latest_model("sol", &["gpt-6.1-solar".to_string()]), None);
}
