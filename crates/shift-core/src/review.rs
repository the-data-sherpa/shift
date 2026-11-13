//! One review round: snapshot the coder's work, brief the reviewer, and
//! return a schema-checked review.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::delegate::{DELEGATE_TASK, DelegateRequest, Review, delegate_task_spec};
use crate::{ToolServer, ToolSpec, snapshot};

/// Runs the reviewer agent on its harness.
pub trait Reviewer: Send + Sync {
    /// Review the snapshot checked out at `worktree`, given only `brief`.
    /// Returns the reviewer's final message, which should be a review.
    fn review(&self, worktree: &Path, brief: &str) -> Result<String, String>;
}

/// The user's original request: the first message they sent the coder.
pub type UserRequest = Arc<Mutex<Option<String>>>;

/// Serves `delegate_task` for one coder session.
pub struct Delegate {
    /// The coder's repo.
    pub repo: PathBuf,
    /// `HEAD` when the coder session started.
    pub base: String,
    /// Where review worktrees are checked out.
    pub session_dir: PathBuf,
    pub request: UserRequest,
    pub reviewer: Box<dyn Reviewer>,
    reviews: AtomicU32,
}

impl Delegate {
    pub fn new(
        repo: PathBuf,
        base: String,
        session_dir: PathBuf,
        request: UserRequest,
        reviewer: Box<dyn Reviewer>,
    ) -> Self {
        Self {
            repo,
            base,
            session_dir,
            request,
            reviewer,
            reviews: AtomicU32::new(0),
        }
    }

    fn review(&self, request: &DelegateRequest) -> Result<Review, String> {
        let user_request = self.request.lock().unwrap().clone();
        let user_request = user_request.ok_or("no user request has been recorded")?;
        let n = self.reviews.fetch_add(1, Ordering::Relaxed) + 1;
        let worktree = self.session_dir.join(format!("review-{n}"));
        // Dropping the snapshot removes the worktree, whatever the outcome.
        let snapshot = snapshot(&self.repo, &worktree)
            .map_err(|e| format!("could not snapshot the working tree: {e}"))?;
        let brief = brief(&user_request, request, &self.base, &snapshot.commit);
        let reply = self.reviewer.review(&snapshot.worktree, &brief)?;
        parse_review(&reply)
    }
}

impl ToolServer for Delegate {
    fn tools(&self) -> Vec<ToolSpec> {
        vec![delegate_task_spec()]
    }

    fn call(&self, name: &str, arguments: Value) -> Result<String, String> {
        if name != DELEGATE_TASK {
            return Err(format!("shift has no tool `{name}`"));
        }
        let request: DelegateRequest = serde_json::from_value(arguments)
            .map_err(|e| format!("invalid delegate_task arguments: {e}"))?;
        let review = self.review(&request)?;
        Ok(serde_json::to_string(&review).expect("a review serializes"))
    }
}

/// Everything the reviewer is told. It holds the user's request word for
/// word and the coder's statement, and none of the coder's history.
pub fn brief(user_request: &str, request: &DelegateRequest, base: &str, snapshot: &str) -> String {
    format!(
        "Review this change.\n\n\
         ## The user's request, word for word\n\n{user_request}\n\n\
         ## The coder's statement\n\n\
         Intent: {}\n\n\
         Out of scope: {}\n\n\
         Change summary: {}\n\n\
         Checks: {}\n\n\
         ## The change\n\n\
         Your working directory is a snapshot of the coder's work (commit {snapshot}). \
         The session base is {base}: read the change with `git diff {base}`.\n\n\
         Flag any mismatch between the user's request and the coder's intent.",
        request.intent, request.out_of_scope, request.change_summary, request.checks,
    )
}

/// The review shape as a JSON Schema, for harnesses that can enforce one.
pub fn review_schema() -> Value {
    let string = json!({"type": "string"});
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["verdict", "summary", "findings", "follow_ups", "commands_run"],
        "properties": {
            "verdict": {"type": "string", "enum": ["approve", "changes"]},
            "summary": string,
            "findings": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    // Strict schemas need every key; a finding with no line sends null.
                    "required": ["id", "severity", "file", "line", "claim", "fix"],
                    "properties": {
                        "id": string,
                        "severity": {"type": "string", "enum": ["blocker", "should", "nit"]},
                        "file": string,
                        "line": {"type": ["integer", "null"]},
                        "claim": string,
                        "fix": string,
                    },
                },
            },
            "follow_ups": {"type": "array", "items": string},
            "commands_run": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["cmd", "exit_code"],
                    "properties": {"cmd": string, "exit_code": {"type": "integer"}},
                },
            },
        },
    })
}

/// Check the reviewer's final message against the review shape.
pub fn parse_review(reply: &str) -> Result<Review, String> {
    serde_json::from_str(reply)
        .map_err(|e| format!("the reviewer's output does not match the review schema: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegate::{ReviewVerdict, Severity};

    fn request() -> DelegateRequest {
        DelegateRequest {
            intent: "INTENT".into(),
            out_of_scope: "SCOPE".into(),
            change_summary: "SUMMARY".into(),
            checks: "CHECKS".into(),
            ledger: None,
        }
    }

    /// Records what the reviewer was given, and replies with `reply`.
    struct FakeReviewer {
        reply: &'static str,
        seen: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl Reviewer for FakeReviewer {
        fn review(&self, worktree: &Path, brief: &str) -> Result<String, String> {
            let file = std::fs::read_to_string(worktree.join("new.txt")).unwrap();
            self.seen.lock().unwrap().push((file, brief.to_owned()));
            Ok(self.reply.to_owned())
        }
    }

    #[test]
    fn delegate_task_reviews_a_snapshot_and_returns_the_checked_review() {
        const APPROVE: &str = r#"{"verdict":"approve","summary":"ok","findings":[],"follow_ups":[],"commands_run":[]}"#;
        let arguments =
            json!({"intent": "INTENT", "out_of_scope": "o", "change_summary": "c", "checks": "k"});
        for (name, reply, ok) in [("good", APPROVE, true), ("bad", "looks fine to me", false)] {
            let repo = crate::git::tests::repo(&format!("delegate-{name}"));
            let session = crate::git::tests::Temp::new(&format!("delegate-{name}-session"));
            std::fs::write(repo.0.join("new.txt"), "the coder's work\n").unwrap();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let request = UserRequest::default();
            let delegate = Delegate::new(
                repo.0.clone(),
                "BASE".into(),
                session.0.clone(),
                request.clone(),
                Box::new(FakeReviewer {
                    reply,
                    seen: seen.clone(),
                }),
            );

            // Without a user request there is nothing to review against.
            assert!(delegate.call(DELEGATE_TASK, arguments.clone()).is_err());
            *request.lock().unwrap() = Some("USER REQUEST".into());
            let result = delegate.call(DELEGATE_TASK, arguments.clone());

            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].0, "the coder's work\n");
            assert!(seen[0].1.contains("USER REQUEST") && seen[0].1.contains("Intent: INTENT"));
            assert!(seen[0].1.contains("git diff BASE"));
            match result {
                Ok(review) => {
                    assert!(ok);
                    assert_eq!(parse_review(&review).unwrap().summary, "ok");
                }
                // Output that fails the schema is a tool error, never a review.
                Err(error) => assert!(!ok && error.contains("review schema"), "{error}"),
            }
            // The worktree is removed either way.
            assert_eq!(std::fs::read_dir(&session.0).unwrap().count(), 0);
        }
    }

    #[test]
    fn brief_has_the_request_the_statement_and_the_base() {
        let brief = brief("Fix the\n  login bug, please.", &request(), "BASE", "SNAP");
        for part in [
            "Fix the\n  login bug, please.",
            "Intent: INTENT",
            "Out of scope: SCOPE",
            "Change summary: SUMMARY",
            "Checks: CHECKS",
            "git diff BASE",
            "commit SNAP",
            "mismatch between the user's request and the coder's intent",
        ] {
            assert!(brief.contains(part), "missing {part:?} in:\n{brief}");
        }
    }

    #[test]
    fn review_is_checked_against_the_shape() {
        let review = parse_review(
            r#"{"verdict":"changes","summary":"s","findings":[
                {"id":"S1","severity":"should","file":"a.rs","line":null,"claim":"c","fix":"f"},
                {"id":"S2","severity":"nit","file":"b.rs","line":7,"claim":"c","fix":"f"}],
              "follow_ups":["old bug"],"commands_run":[{"cmd":"cargo test","exit_code":0}]}"#,
        )
        .unwrap();
        assert_eq!(review.verdict, ReviewVerdict::Changes);
        assert_eq!(review.findings[0].severity, Severity::Should);
        assert_eq!(
            (review.findings[0].line, review.findings[1].line),
            (None, Some(7))
        );

        for bad in [
            "I approve.",
            r#"{"verdict":"approve"}"#,
            r#"{"verdict":"pass","summary":"s","findings":[],"follow_ups":[],"commands_run":[]}"#,
        ] {
            let error = parse_review(bad).unwrap_err();
            assert!(error.contains("review schema"), "{error}");
        }
    }

    #[test]
    fn schema_requires_every_key_the_review_type_reads() {
        let schema = review_schema();
        assert_eq!(schema["required"].as_array().unwrap().len(), 5);
        assert_eq!(
            schema["properties"]["findings"]["items"]["required"]
                .as_array()
                .unwrap()
                .len(),
            6
        );
    }
}
