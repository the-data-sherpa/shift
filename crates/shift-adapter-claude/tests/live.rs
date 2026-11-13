//! Live smoke test: launches the real `claude` and uses a real account, so it
//! only runs with `SHIFT_LIVE=1` (ADR 0003).

use shift_adapter_claude::{ClaudeAdapter, TRANSCRIPT};
use shift_core::{Adapter, Event, builtin_agent};

#[test]
fn coder_runs_a_shell_command_through_the_gate() {
    if std::env::var_os("SHIFT_LIVE").is_none() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("shift-live-{}", std::process::id()));
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let coder = builtin_agent("coder").unwrap();
    let (session, events) = ClaudeAdapter.start(&coder, &repo, &dir).unwrap();
    session
        .send("Run `echo shift-live > out.txt` with Bash, then reply with the single word done.")
        .unwrap();

    let mut seen = Vec::new();
    loop {
        let event = events.recv_blocking().unwrap();
        let done = matches!(event, Event::TurnDone { .. } | Event::Closed);
        seen.push(event);
        if done {
            break;
        }
    }

    assert_eq!(
        seen.last(),
        Some(&Event::TurnDone { is_error: false }),
        "{seen:?}"
    );
    assert!(seen.contains(&Event::PermissionRequest {
        tool: "Bash".into(),
        allowed: true
    }));
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::ToolCall { name, .. } if name == "Bash"))
    );
    assert!(seen.iter().any(|e| matches!(e, Event::Text(_))));
    assert_eq!(
        std::fs::read_to_string(repo.join("out.txt"))
            .unwrap()
            .trim(),
        "shift-live"
    );
    assert!(std::fs::metadata(dir.join(TRANSCRIPT)).unwrap().len() > 0);
    println!("transcript: {}", dir.join(TRANSCRIPT).display());
}
