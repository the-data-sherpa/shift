//! `delegate_task`: the shift tool a coder calls to have its change reviewed.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A tool shift serves to a session.
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

/// The shift tools one session can call. Calls may block for a long time and
/// run off the adapter's reader thread.
pub trait ToolServer: Send + Sync {
    fn tools(&self) -> Vec<ToolSpec>;

    /// Run a tool. `Err` is returned to the agent as a tool error.
    fn call(&self, name: &str, arguments: Value) -> Result<String, String>;
}

pub const DELEGATE_TASK: &str = "delegate_task";

/// What the coder passes to `delegate_task`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegateRequest {
    pub intent: String,
    pub out_of_scope: String,
    pub change_summary: String,
    pub checks: String,
    /// The coder's answer to each finding of the previous round.
    pub ledger: Option<Vec<LedgerEntry>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerEntry {
    /// The finding's id, such as `S1`.
    pub id: String,
    pub answer: Answer,
    /// The fix made, the evidence for disagreeing, or why it is declined.
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Answer {
    Fixed,
    Disagree,
    Declined,
}

pub fn delegate_task_spec() -> ToolSpec {
    let text = |description: &str| json!({"type": "string", "description": description});
    ToolSpec {
        name: DELEGATE_TASK,
        description: "Have the reviewer review your current change. Blocks until the review \
                      is done, which can take several minutes. Returns the review as JSON.",
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["intent", "out_of_scope", "change_summary", "checks"],
            "properties": {
                "intent": text("What the change is meant to do."),
                "out_of_scope": text("What the change deliberately leaves alone."),
                "change_summary": text("What you changed."),
                "checks": text("The checks you ran and their results."),
                "ledger": {
                    "type": "array",
                    "description": "From the second call on: your answer to each finding of the last review.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["id", "answer", "note"],
                        "properties": {
                            "id": text("The finding's id, such as S1."),
                            "answer": {"type": "string", "enum": ["fixed", "disagree", "declined"]},
                            "note": text("The fix, your evidence for disagreeing, or why you decline."),
                        },
                    },
                },
            },
        }),
    }
}

/// A reviewer's structured judgement of one change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub verdict: ReviewVerdict,
    pub summary: String,
    pub findings: Vec<Finding>,
    pub follow_ups: Vec<String>,
    pub commands_run: Vec<CommandRun>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewVerdict {
    Approve,
    Changes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub id: String,
    pub severity: Severity,
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub claim: String,
    pub fix: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Blocker,
    Should,
    Nit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRun {
    pub cmd: String,
    pub exit_code: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_accepts_the_documented_arguments() {
        let request: DelegateRequest = serde_json::from_value(json!({
            "intent": "i", "out_of_scope": "o", "change_summary": "c", "checks": "k",
            "ledger": [{"id": "S1", "answer": "disagree", "note": "see test"}],
        }))
        .unwrap();
        assert_eq!(request.ledger.unwrap()[0].answer, Answer::Disagree);

        let no_ledger =
            json!({"intent": "i", "out_of_scope": "o", "change_summary": "c", "checks": "k"});
        assert!(serde_json::from_value::<DelegateRequest>(no_ledger).is_ok());
        assert!(serde_json::from_value::<DelegateRequest>(json!({"intent": "i"})).is_err());
    }
}
