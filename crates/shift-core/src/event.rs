use serde_json::Value;

/// What a session reports, in the same shape for every harness (ADR 0003).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The next piece of the agent's reply.
    Text(String),
    ToolCall {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
    },
    /// The harness asked for approval and the policy gate answered.
    PermissionRequest {
        tool: String,
        allowed: bool,
    },
    TurnDone {
        is_error: bool,
    },
    Error(String),
    /// The harness process ended; no more events follow.
    Closed,
}
