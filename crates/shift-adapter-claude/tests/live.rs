//! Live smoke test: launches the real `claude` and uses a real account, so it
//! only runs with `SHIFT_LIVE=1` (ADR 0003).

use shift_adapter_claude::{ClaudeAdapter, TRANSCRIPT};
use shift_core::delegate::StubReviewer;
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
    let tools = std::sync::Arc::new(StubReviewer {
        wait: std::time::Duration::from_secs(1),
    });
    let (session, events) = ClaudeAdapter.start(&coder, &repo, &dir, tools).unwrap();
    session
        .send(
            "Run `echo shift-live > out.txt` with Bash, then call delegate_task with short \
             made-up arguments, then reply with the verdict you received.",
        )
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
    assert!(seen.iter().any(|e| matches!(
        e,
        Event::ToolResult { output, is_error: false, .. } if output.contains("\"verdict\":\"approve\"")
    )));
    assert_eq!(
        std::fs::read_to_string(repo.join("out.txt"))
            .unwrap()
            .trim(),
        "shift-live"
    );
    assert!(std::fs::metadata(dir.join(TRANSCRIPT)).unwrap().len() > 0);
    println!("transcript: {}", dir.join(TRANSCRIPT).display());
}
