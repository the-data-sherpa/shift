//! A review that breaks, runs out of time, or is cancelled ends as a tool
//! error, never as a review. A tiny fake app-server stands in for Codex.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use shift_adapter_codex::CodexReviewer;
use shift_core::review::{Delegate, UserRequest};
use shift_core::{Cancel, Cancellable, ToolServer, builtin_agent};

/// Answers the four requests a review makes, then runs `$1`'s tail.
const FAKE: &str = r#"#!/bin/sh
while read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/^{"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) echo "{\"id\":$id,\"result\":{}}" ;;
    *'"method":"model/list"'*) echo "{\"id\":$id,\"result\":{\"data\":[{\"id\":\"gpt-6.1-sol\"}]}}" ;;
    *'"method":"thread/start"'*) echo "{\"id\":$id,\"result\":{\"thread\":{\"id\":\"t1\"},\"model\":\"gpt-6.1-sol\"}}" ;;
    *'"method":"turn/start"'*) echo "{\"id\":$id,\"result\":{}}"
TAIL
    ;;
  esac
done
"#;

/// After `turn/start`: say something that is not a review, and finish.
const NOT_A_REVIEW: &str = r#"echo '{"method":"item/completed","params":{"threadId":"t1","item":{"type":"agentMessage","text":"Looks fine to me."}}}'
echo '{"method":"turn/completed","params":{"threadId":"t1","turn":{"status":"completed"}}}'"#;

struct Fixture {
    dir: PathBuf,
    /// `delegate_task`, wired as the app wires it.
    delegate: Cancellable<Delegate>,
    cancel: Cancel,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// A repo with uncommitted work, and `delegate_task` backed by `script`.
fn fixture(name: &str, script: &str, limit: Duration) -> Fixture {
    let dir = std::env::temp_dir().join(format!("shift-fail-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-qm",
            "init",
        ],
    );
    std::fs::write(repo.join("a.txt"), "two\n").unwrap();

    let program = dir.join("fake-codex");
    std::fs::write(&program, script).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

    let cancel = Cancel::default();
    let (events, _) = async_channel::unbounded();
    let mut reviewer = CodexReviewer::new(
        builtin_agent("reviewer").unwrap(),
        dir.join("target"),
        &dir,
        events,
        cancel.clone(),
    )
    .unwrap();
    reviewer.program = program;
    reviewer.limit = limit;
    let request = UserRequest::default();
    *request.lock().unwrap() = Some("Change a.txt.".into());
    let delegate = Delegate::new(
        repo,
        "HEAD".into(),
        dir.clone(),
        request,
        Box::new(reviewer),
    );
    let delegate = Cancellable {
        tools: delegate,
        cancel: cancel.clone(),
    };
    Fixture {
        dir,
        delegate,
        cancel,
    }
}

/// The id of the review call the tests make.
const CALL: &str = "toolu_review";

/// Call `delegate_task` and return its tool error. The review worktree must
/// be gone afterwards.
fn tool_error(fixture: &Fixture) -> String {
    let arguments = serde_json::json!({
        "intent": "i", "out_of_scope": "o", "change_summary": "c", "checks": "k",
    });
    let error = fixture
        .delegate
        .call("delegate_task", arguments, CALL)
        .unwrap_err();
    let worktrees = git(&fixture.dir.join("repo"), &["worktree", "list"]);
    assert_eq!(worktrees.lines().count(), 1, "{worktrees}");
    error
}

#[test]
fn a_crash_is_a_tool_error_that_says_so() {
    let fixture = fixture("crash", "#!/bin/sh\nexit 3\n", Duration::from_secs(30));
    let error = tool_error(&fixture);
    assert!(
        error.contains("Codex exited before the review finished"),
        "{error}"
    );
    assert!(error.contains("exit status: 3"), "{error}");
}

#[test]
fn output_that_is_not_a_review_is_a_tool_error() {
    let script = FAKE.replace("TAIL", NOT_A_REVIEW);
    let fixture = fixture("schema", &script, Duration::from_secs(30));
    let error = tool_error(&fixture);
    assert!(
        error.contains("does not match the review schema"),
        "{error}"
    );
}

#[test]
fn a_review_past_its_limit_is_a_tool_error() {
    // The fake never finishes its turn.
    let script = FAKE.replace("TAIL", "");
    let fixture = fixture("timeout", &script, Duration::from_secs(1));
    let started = Instant::now();
    let error = tool_error(&fixture);
    assert_eq!(error, "the review exceeded its 1-second limit");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn each_round_on_a_kept_session_gets_the_full_limit() {
    // The fake approves the first turn and never finishes the second.
    let approve = r#"n=$((n+1))
if [ "$n" = 1 ]; then
echo '{"method":"item/completed","params":{"threadId":"t1","item":{"type":"agentMessage","text":"{\"verdict\":\"approve\",\"summary\":\"ok\",\"findings\":[],\"follow_ups\":[],\"commands_run\":[]}"}}}'
echo '{"method":"turn/completed","params":{"threadId":"t1","turn":{"status":"completed"}}}'
fi"#;
    let fixture = fixture(
        "rounds",
        &FAKE.replace("TAIL", approve),
        Duration::from_secs(2),
    );
    let arguments = serde_json::json!({
        "intent": "i", "out_of_scope": "o", "change_summary": "c", "checks": "k",
    });
    fixture
        .delegate
        .call("delegate_task", arguments, CALL)
        .unwrap();
    // The first round's clock has run out by the time the second starts.
    std::thread::sleep(Duration::from_millis(2500));

    let started = Instant::now();
    let error = tool_error(&fixture);

    assert_eq!(error, "the review exceeded its 2-second limit");
    assert!(
        started.elapsed() >= Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_session_that_died_between_rounds_is_a_tool_error_then_replaced() {
    // The fake approves one turn and exits; each new process does the same.
    let approve_once = r#"echo '{"method":"item/completed","params":{"threadId":"t1","item":{"type":"agentMessage","text":"{\"verdict\":\"approve\",\"summary\":\"ok\",\"findings\":[],\"follow_ups\":[],\"commands_run\":[]}"}}}'
echo '{"method":"turn/completed","params":{"threadId":"t1","turn":{"status":"completed"}}}'
exit 7"#;
    let script = FAKE.replace("TAIL", approve_once);
    let fixture = fixture("died", &script, Duration::from_secs(30));
    let arguments = serde_json::json!({
        "intent": "i", "out_of_scope": "o", "change_summary": "c", "checks": "k",
    });
    let call = || {
        fixture
            .delegate
            .call("delegate_task", arguments.clone(), CALL)
    };
    assert!(call().unwrap().contains(r#""round":1"#));
    std::thread::sleep(Duration::from_millis(300));

    // The kept session is gone: an error that says so, and no round used.
    let error = tool_error(&fixture);
    assert!(
        error.contains("Codex exited before the review finished"),
        "{error}"
    );
    assert!(error.contains("exit status: 7"), "{error}");

    // No retry happened by itself; the next call starts a fresh session.
    assert!(call().unwrap().contains(r#""round":2"#));
}

#[test]
fn a_codex_that_stops_reading_cannot_outlast_the_limit() {
    // The fake starts the thread, then never reads again. The brief is far
    // larger than a pipe holds, so writing it would block forever.
    let script = r#"#!/bin/sh
while read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/^{"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) echo "{\"id\":$id,\"result\":{}}" ;;
    *'"method":"model/list"'*) echo "{\"id\":$id,\"result\":{\"data\":[{\"id\":\"gpt-6.1-sol\"}]}}" ;;
    *'"method":"thread/start"'*) echo "{\"id\":$id,\"result\":{\"thread\":{\"id\":\"t1\"},\"model\":\"gpt-6.1-sol\"}}"
      exec sleep 60 ;;
  esac
done
"#;
    let fixture = fixture("deaf", script, Duration::from_secs(2));
    let arguments = serde_json::json!({
        "intent": "x".repeat(1 << 20), "out_of_scope": "o", "change_summary": "c", "checks": "k",
    });
    let started = Instant::now();
    let error = fixture
        .delegate
        .call("delegate_task", arguments, CALL)
        .unwrap_err();
    assert_eq!(error, "the review exceeded its 2-second limit");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn cancelling_stops_the_review_with_a_cancelled_by_user_error() {
    let script = FAKE.replace("TAIL", "");
    let fixture = fixture("cancel", &script, Duration::from_secs(60));
    // A cancel left over from an earlier call must not stop this one.
    fixture.cancel.cancel("an-earlier-call");
    let cancel = fixture.cancel.clone();
    let button = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(1));
        cancel.cancel(CALL);
    });
    let started = Instant::now();
    let error = tool_error(&fixture);
    button.join().unwrap();
    assert_eq!(error, "cancelled by user");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(900) && elapsed < Duration::from_secs(10),
        "{elapsed:?}"
    );
}

#[test]
fn a_cancel_pressed_before_the_reviewer_starts_still_counts() {
    // The button shows as soon as the coder calls delegate_task, which is
    // before the snapshot is taken and Codex is started.
    let script = FAKE.replace("TAIL", "");
    let fixture = fixture("early-cancel", &script, Duration::from_secs(60));
    // The call has begun and the user cancels at once; the rest of the call
    // (snapshot, then the reviewer) runs after that.
    fixture.cancel.begin(CALL);
    fixture.cancel.cancel(CALL);
    let arguments = serde_json::json!({
        "intent": "i", "out_of_scope": "o", "change_summary": "c", "checks": "k",
    });
    let started = Instant::now();
    let error = fixture
        .delegate
        .tools
        .call("delegate_task", arguments, CALL)
        .unwrap_err();
    assert_eq!(error, "cancelled by user");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn closed_output_from_a_live_process_is_a_tool_error_not_a_hang() {
    // The fake closes its stdout and stays alive.
    let script = "#!/bin/sh\nexec 1>&-\nexec sleep 60\n";
    let fixture = fixture("closed", script, Duration::from_secs(60));
    let started = Instant::now();
    let error = tool_error(&fixture);
    assert_eq!(error, "Codex closed its output before the review finished");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn the_default_limit_is_fifteen_minutes() {
    assert_eq!(shift_adapter_codex::REVIEW_LIMIT, Duration::from_secs(900));
    let (events, _) = async_channel::unbounded();
    let dir = std::env::temp_dir().join(format!("shift-fail-{}-limit", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let reviewer = CodexReviewer::new(
        builtin_agent("reviewer").unwrap(),
        dir.join("target"),
        &dir,
        events,
        Cancel::default(),
    )
    .unwrap();
    assert_eq!(reviewer.limit, Duration::from_secs(900));
    assert_eq!(reviewer.program, PathBuf::from("codex"));
    let _ = std::fs::remove_dir_all(&dir);
}
