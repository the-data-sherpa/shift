//! Turns Codex app-server notifications into shift `Event`s. It does no
//! I/O, so a saved transcript replays through it.

use serde_json::{Value, json};
use shift_core::Event;

/// The events one line of Codex JSON-RPC leads to.
pub fn translate(line: &str) -> Vec<Event> {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    let params = &message["params"];
    let item = &params["item"];
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    let event = match message["method"].as_str() {
        Some("item/agentMessage/delta") => Event::Text(text(&params["delta"])),
        // Deltas run one message into the next; end each with a break.
        Some("item/completed") if item["type"] == "agentMessage" => Event::Text("\n\n".into()),
        Some("item/started") if item["type"] == "commandExecution" => Event::ToolCall {
            id: text(&item["id"]),
            name: "shell".into(),
            input: json!({"command": item["command"]}),
        },
        Some("item/completed") if item["type"] == "commandExecution" => Event::ToolResult {
            id: text(&item["id"]),
            output: text(&item["aggregatedOutput"]),
            is_error: item["exitCode"] != 0,
        },
        Some("error") => Event::Error(text(&params["error"]["message"])),
        Some("turn/completed") => {
            let is_error = params["turn"]["status"] != "completed";
            Event::TurnDone { is_error }
        }
        _ => return Vec::new(),
    };
    vec![event]
}
