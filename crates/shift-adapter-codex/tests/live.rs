//! Live smoke test: launches the real `codex app-server` and uses a real
//! account, so it only runs with `SHIFT_LIVE=1` (ADR 0003).

use std::process::Command;

use shift_adapter_codex::{CodexReviewer, TRANSCRIPT};
use shift_core::delegate::ReviewVerdict;
use shift_core::review::{Delegate, RoundResult, UserRequest};
use shift_core::{Event, ToolServer, builtin_agent};

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn sol_reviews_two_rounds_on_one_session() {
    if std::env::var_os("SHIFT_LIVE").is_none() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("shift-codex-live-{}", std::process::id()));
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet"]);
    std::fs::write(repo.join("add.py"), "def add(a, b):\n    return a + b\n").unwrap();
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
    let base = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    // The coder's uncommitted work: one tracked change with a bug, one new file.
    std::fs::write(repo.join("add.py"), "def add(a, b):\n    return a - b\n").unwrap();
    std::fs::write(repo.join("notes.txt"), "add now logs nothing\n").unwrap();

    let (events, receiver) = async_channel::unbounded();
    let reviewer = CodexReviewer::new(
        builtin_agent("reviewer").unwrap(),
        dir.join("target"),
        &dir,
        events,
    )
    .unwrap();
    let request = UserRequest::default();
    *request.lock().unwrap() = Some("Add a notes.txt file. Do not change behavior.".into());
    let delegate = Delegate::new(
        repo.clone(),
        base.trim().into(),
        dir.clone(),
        request,
        Box::new(reviewer),
    );

    let arguments = serde_json::json!({
        "intent": "Add notes.txt",
        "out_of_scope": "add.py behavior",
        "change_summary": "Added notes.txt",
        "checks": "none",
    });
    let first = delegate.call("delegate_task", arguments.clone()).unwrap();
    let first: RoundResult = serde_json::from_str(&first).unwrap();
    println!("{first:#?}");
    // The reviewer saw the snapshot: the subtraction bug is in add.py.
    assert_eq!(first.review.verdict, ReviewVerdict::Changes);
    assert!(
        first
            .review
            .findings
            .iter()
            .any(|f| f.file.contains("add.py"))
    );
    assert_eq!((first.round, first.ended), (1, false));
    // The worktree is gone and the coder's checkout is untouched.
    assert!(!dir.join("review-1").exists());
    assert!(repo.join("notes.txt").exists());

    // Round 2: the coder fixes the bug and answers every finding.
    std::fs::write(repo.join("add.py"), "def add(a, b):\n    return a + b\n").unwrap();
    let ledger: Vec<_> = first
        .review
        .findings
        .iter()
        .map(|f| serde_json::json!({"id": f.id, "answer": "fixed", "note": "Restored add.py."}))
        .collect();
    let mut arguments = arguments;
    arguments["ledger"] = ledger.into();
    let second = delegate.call("delegate_task", arguments).unwrap();
    let second: RoundResult = serde_json::from_str(&second).unwrap();
    println!("{second:#?}");
    // The reviewer saw the new snapshot and resolved its finding.
    assert_eq!((second.round, second.ended), (2, true));
    assert!(
        !second
            .review
            .findings
            .iter()
            .any(|f| f.file.contains("add.py"))
    );
    assert!(!dir.join("review-2").exists());

    // Both rounds ran on one reviewer session: one thread, two turns.
    let transcript = std::fs::read_to_string(dir.join(TRANSCRIPT)).unwrap();
    assert_eq!(transcript.matches(r#""method":"thread/start""#).count(), 1);
    assert_eq!(transcript.matches(r#""method":"turn/start""#).count(), 2);

    let mut seen = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        seen.push(event);
    }
    assert!(
        seen.iter().any(|e| matches!(e, Event::ToolCall { .. })),
        "{seen:?}"
    );
    assert!(seen.contains(&Event::TurnDone { is_error: false }));
    println!("transcript: {}", dir.join(TRANSCRIPT).display());
}
