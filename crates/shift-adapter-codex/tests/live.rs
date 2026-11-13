//! Live smoke test: launches the real `codex app-server` and uses a real
//! account, so it only runs with `SHIFT_LIVE=1` (ADR 0003).

use std::process::Command;

use shift_adapter_codex::{CodexReviewer, TRANSCRIPT};
use shift_core::review::{Delegate, UserRequest};
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
fn sol_reviews_a_snapshot_and_returns_a_schema_checked_review() {
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

    let review = delegate
        .call(
            "delegate_task",
            serde_json::json!({
                "intent": "Add notes.txt",
                "out_of_scope": "add.py behavior",
                "change_summary": "Added notes.txt",
                "checks": "none",
            }),
        )
        .unwrap();

    let review: shift_core::delegate::Review = serde_json::from_str(&review).unwrap();
    println!("{review:#?}");
    // The reviewer saw the snapshot: the subtraction bug is in add.py.
    assert_eq!(review.verdict, shift_core::delegate::ReviewVerdict::Changes);
    assert!(review.findings.iter().any(|f| f.file.contains("add.py")));
    // The worktree is gone and the coder's checkout is untouched.
    assert!(!dir.join("review-1").exists());
    assert!(repo.join("notes.txt").exists());
    let mut seen = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        seen.push(event);
    }
    assert!(
        seen.iter().any(|e| matches!(e, Event::ToolCall { .. })),
        "{seen:?}"
    );
    assert!(seen.contains(&Event::TurnDone { is_error: false }));
    assert!(std::fs::metadata(dir.join(TRANSCRIPT)).unwrap().len() > 0);
    println!("transcript: {}", dir.join(TRANSCRIPT).display());
}
