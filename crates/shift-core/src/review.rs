//! One review round: snapshot the coder's work, brief the reviewer, and
//! return a schema-checked review.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::delegate::{DELEGATE_TASK, DelegateRequest, Review, delegate_task_spec};
use crate::history::{Entry, History, Stuck};
use crate::{ToolServer, ToolSpec, snapshot};

/// Runs the reviewer agent on its harness.
pub trait Reviewer: Send + Sync {
    /// Review the snapshot checked out at `worktree`, given only `brief`.
    /// Returns the reviewer's final message, which should be a review.
    /// Later rounds of one review continue the same reviewer session.
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
    /// Rounds and the ledger. In memory only: nothing resumes after a restart.
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    history: History,
    /// The snapshot the last round reviewed.
    last_snapshot: Option<String>,
    /// Review attempts, which name the worktrees. Failed ones count here
    /// but use no round.
    attempts: u32,
    /// Once Stuck, always Stuck.
    stuck: Option<Stuck>,
}

/// What `delegate_task` returns for a completed round: the review, plus
/// where the review stands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoundResult {
    #[serde(flatten)]
    pub review: Review,
    pub round: u32,
    pub rounds_left: u32,
    /// True when no blocker or should finding is open: report the review.
    pub ended: bool,
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
            state: Mutex::default(),
        }
    }

    /// Run one round. A failed round returns `Err` and changes nothing, so
    /// it uses no round and the same ledger can be sent again.
    fn round(&self, request: &DelegateRequest) -> Result<String, String> {
        let mut state = self.state.lock().unwrap();
        if let Some(stuck) = &state.stuck {
            return Ok(to_json(stuck));
        }
        let user_request = self.request.lock().unwrap().clone();
        let user_request = user_request.ok_or("no user request has been recorded")?;

        let mut history = state.history.clone();
        history.answer(request.ledger.as_deref().unwrap_or_default())?;
        if history.rounds_left() == 0 {
            state.history = history;
            let stuck = state.history.out_of_rounds();
            state.stuck = Some(stuck.clone());
            return Ok(to_json(&stuck));
        }

        state.attempts += 1;
        let worktree = self.session_dir.join(format!("review-{}", state.attempts));
        // Dropping the snapshot removes the worktree, whatever the outcome.
        let snapshot = snapshot(&self.repo, &worktree)
            .map_err(|e| format!("could not snapshot the working tree: {e}"))?;
        let mut brief = brief(&user_request, request, &self.base, &snapshot.commit);
        if let Some(previous) = &state.last_snapshot {
            brief.push_str(&later_round(&history, previous));
        }
        let review = parse_review(&self.reviewer.review(&snapshot.worktree, &brief)?)?;

        history.review(&review);
        state.history = history;
        state.last_snapshot = Some(snapshot.commit.clone());
        if let Some(stuck) = state.history.disputed(&review) {
            state.stuck = Some(stuck.clone());
            return Ok(to_json(&stuck));
        }
        Ok(to_json(&RoundResult {
            review,
            round: state.history.rounds,
            rounds_left: state.history.rounds_left(),
            ended: state.history.ended(),
        }))
    }
}

fn to_json(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("a tool result serializes")
}

impl ToolServer for Delegate {
    fn tools(&self) -> Vec<ToolSpec> {
        vec![delegate_task_spec()]
    }

    fn call(&self, name: &str, arguments: Value, _: &str) -> Result<String, String> {
        if name != DELEGATE_TASK {
            return Err(format!("shift has no tool `{name}`"));
        }
        let request: DelegateRequest = serde_json::from_value(arguments)
            .map_err(|e| format!("invalid delegate_task arguments: {e}"))?;
        self.round(&request)
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

/// What a later round adds to the brief: the coder's ledger and what to
/// check. Like the rest of the brief, it has none of the coder's history.
fn later_round(history: &History, previous_snapshot: &str) -> String {
    let mut text = format!(
        "\n\n## Round {}\n\n\
         This is a later round of the same review. The last round reviewed commit \
         {previous_snapshot}: read what changed since with `git diff {previous_snapshot}`.\n\n\
         The open findings, each with the coder's answer:\n",
        history.rounds + 1
    );
    for open in history.findings.iter().filter(|f| f.open) {
        let finding = &open.finding;
        let line = finding
            .line
            .map(|line| format!(":{line}"))
            .unwrap_or_default();
        // The claim is restated so the round stands on its own, even if the
        // reviewer session had to be restarted.
        text.push_str(&format!(
            "\n- {} ({:?}, {}{line}): {}",
            finding.id, finding.severity, finding.file, finding.claim
        ));
        if let Some(Entry::Answered { answer, note }) = open.log.last() {
            text.push_str(&format!("\n  Coder: {}: {note}", answer.label()));
        }
    }
    text.push_str(
        "\n\nCheck only these findings and the code changed since the last round. Check each \
         answer yourself; do not take it on trust. List every earlier finding that is still \
         open under its original id, with the reason in `claim`. Leave out a finding to mark \
         it resolved. Give new findings new ids.",
    );
    text
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
            assert!(
                delegate
                    .call(DELEGATE_TASK, arguments.clone(), "t")
                    .is_err()
            );
            *request.lock().unwrap() = Some("USER REQUEST".into());
            let result = delegate.call(DELEGATE_TASK, arguments.clone(), "t");

            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].0, "the coder's work\n");
            assert!(seen[0].1.contains("USER REQUEST") && seen[0].1.contains("Intent: INTENT"));
            assert!(seen[0].1.contains("git diff BASE"));
            match result {
                Ok(review) => {
                    assert!(ok);
                    let result: RoundResult = serde_json::from_str(&review).unwrap();
                    assert_eq!(result.review.summary, "ok");
                    assert_eq!(
                        (result.round, result.rounds_left, result.ended),
                        (1, 2, true)
                    );
                }
                // Output that fails the schema is a tool error, never a review.
                Err(error) => assert!(!ok && error.contains("review schema"), "{error}"),
            }
            // The worktree is removed either way.
            assert_eq!(std::fs::read_dir(&session.0).unwrap().count(), 0);
        }
    }

    /// Replies with each of `replies` in turn and records the briefs.
    struct Scripted {
        replies: Mutex<Vec<Result<String, String>>>,
        briefs: Arc<Mutex<Vec<String>>>,
    }

    impl Reviewer for Scripted {
        fn review(&self, _: &Path, brief: &str) -> Result<String, String> {
            self.briefs.lock().unwrap().push(brief.to_owned());
            self.replies.lock().unwrap().remove(0)
        }
    }

    fn changes(ids: &[&str]) -> Result<String, String> {
        let findings: Vec<Value> = ids
            .iter()
            .map(|id| json!({"id": id, "severity": "should", "file": "a.txt", "line": 1, "claim": format!("{id} is wrong"), "fix": "f"}))
            .collect();
        Ok(json!({"verdict": "changes", "summary": "s", "findings": findings, "follow_ups": [], "commands_run": []}).to_string())
    }

    /// A delegate over a fresh repo, its recorded briefs, and the temp dirs.
    fn scripted(
        name: &str,
        replies: Vec<Result<String, String>>,
    ) -> (Delegate, Arc<Mutex<Vec<String>>>, impl Sized) {
        let repo = crate::git::tests::repo(&format!("rounds-{name}"));
        let session = crate::git::tests::Temp::new(&format!("rounds-{name}-session"));
        let briefs = Arc::new(Mutex::new(Vec::new()));
        let request = UserRequest::default();
        *request.lock().unwrap() = Some("USER REQUEST".into());
        let delegate = Delegate::new(
            repo.0.clone(),
            "BASE".into(),
            session.0.clone(),
            request,
            Box::new(Scripted {
                replies: Mutex::new(replies),
                briefs: briefs.clone(),
            }),
        );
        (delegate, briefs, (repo, session))
    }

    fn call(delegate: &Delegate, ledger: &[(&str, &str)]) -> Result<Value, String> {
        let mut arguments =
            json!({"intent": "i", "out_of_scope": "o", "change_summary": "c", "checks": "k"});
        if !ledger.is_empty() {
            arguments["ledger"] = ledger
                .iter()
                .map(|(id, answer)| json!({"id": id, "answer": answer, "note": format!("{id} {answer} note")}))
                .collect();
        }
        let result = delegate.call(DELEGATE_TASK, arguments, "t")?;
        Ok(serde_json::from_str(&result).unwrap())
    }

    #[test]
    fn later_rounds_carry_the_ledger_and_a_fourth_call_is_stuck() {
        let (delegate, briefs, _dirs) = scripted(
            "budget",
            vec![changes(&["S1", "S2"]), changes(&["S2"]), changes(&["S3"])],
        );

        let first = call(&delegate, &[]).unwrap();
        assert_eq!(
            (&first["round"], &first["rounds_left"], &first["ended"]),
            (&json!(1), &json!(2), &json!(false))
        );
        // The next call must answer every open finding.
        let error = call(&delegate, &[("S1", "fixed")]).unwrap_err();
        assert!(error.contains("missing: S2"), "{error}");

        let second = call(&delegate, &[("S1", "fixed"), ("S2", "disagree")]).unwrap();
        assert_eq!(second["round"], 2);
        let third = call(&delegate, &[("S2", "fixed")]).unwrap();
        assert_eq!(
            (&third["round"], &third["rounds_left"]),
            (&json!(3), &json!(0))
        );

        // The rejected call used no round and reached no reviewer.
        let briefs = briefs.lock().unwrap();
        assert_eq!(briefs.len(), 3);
        assert!(!briefs[0].contains("## Round"));
        // Round 2's brief has the ledger shift stored, and what to check.
        for part in [
            "## Round 2",
            "USER REQUEST",
            "- S1 (Should, a.txt:1): S1 is wrong",
            "Coder: fixed: S1 fixed note",
            "- S2 (Should, a.txt:1): S2 is wrong",
            "Coder: disagree: S2 disagree note",
            "still open under its original id",
        ] {
            assert!(
                briefs[1].contains(part),
                "missing {part:?} in:\n{}",
                briefs[1]
            );
        }
        assert!(
            briefs[2].contains("## Round 3") && briefs[2].contains("Coder: fixed: S2 fixed note")
        );
        assert!(
            !briefs[2].contains("S1 fixed note"),
            "resolved findings are not re-sent"
        );
        drop(briefs);

        // The budget is spent: a fourth call is Stuck, with both sides.
        let fourth = call(&delegate, &[("S3", "declined")]).unwrap();
        assert!(fourth["stuck"].as_str().unwrap().contains("round budget"));
        assert_eq!(fourth["disputes"][0]["id"], "S3");
        assert_eq!(fourth["disputes"][0]["reviewer"], "S3 is wrong");
        assert_eq!(
            fourth["disputes"][0]["coder"],
            json!(["declined: S3 declined note"])
        );
        assert!(fourth.get("verdict").is_none() && fourth.get("review").is_none());
        // Stuck stays Stuck.
        assert_eq!(call(&delegate, &[]).unwrap(), fourth);
    }

    #[test]
    fn a_finding_disputed_twice_ends_in_stuck_after_the_third_round() {
        let (delegate, _, _dirs) = scripted(
            "dispute",
            vec![changes(&["S1"]), changes(&["S1"]), changes(&["S1"])],
        );
        call(&delegate, &[]).unwrap();
        let second = call(&delegate, &[("S1", "disagree")]).unwrap();
        assert_eq!(second["verdict"], "changes");

        let third = call(&delegate, &[("S1", "disagree")]).unwrap();

        assert!(
            third["stuck"]
                .as_str()
                .unwrap()
                .contains("disputed after two replies")
        );
        assert_eq!(third["disputes"][0]["coder"].as_array().unwrap().len(), 2);
        assert!(third.get("verdict").is_none());
        // The round that ended in Stuck comes with it.
        assert_eq!(third["review"]["findings"][0]["id"], "S1");
    }

    #[test]
    fn a_failed_review_uses_no_round_and_keeps_the_ledger_unanswered() {
        let (delegate, _, _dirs) = scripted(
            "failure",
            vec![
                changes(&["S1"]),
                Err("Codex crashed".into()),
                Ok("not a review".into()),
                changes(&[]),
            ],
        );
        call(&delegate, &[]).unwrap();
        assert_eq!(
            call(&delegate, &[("S1", "fixed")]).unwrap_err(),
            "Codex crashed"
        );
        assert!(
            call(&delegate, &[("S1", "fixed")])
                .unwrap_err()
                .contains("review schema")
        );

        // The same ledger is accepted again, and this is round 2, not 4.
        let done = call(&delegate, &[("S1", "fixed")]).unwrap();
        assert_eq!((&done["round"], &done["ended"]), (&json!(2), &json!(true)));
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
