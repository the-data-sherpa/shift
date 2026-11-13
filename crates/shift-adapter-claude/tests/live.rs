//! Live smoke test: launches the real `claude` and uses a real account, so it
//! only runs with `SHIFT_LIVE=1` (ADR 0003).

use shift_adapter_claude::{ClaudeAdapter, TRANSCRIPT};
use shift_core::delegate::delegate_task_spec;
use shift_core::{Adapter, Event, ToolServer, ToolSpec, builtin_agent};

/// Stands in for the reviewer: blocks briefly, then answers. It records
/// the id each call arrived with.
struct SlowTool(std::sync::Mutex<Vec<String>>);

impl ToolServer for SlowTool {
    fn tools(&self) -> Vec<ToolSpec> {
        vec![delegate_task_spec()]
    }

    fn call(&self, _: &str, _: serde_json::Value, call_id: &str) -> Result<String, String> {
        self.0.lock().unwrap().push(call_id.to_owned());
        std::thread::sleep(std::time::Duration::from_secs(1));
        Ok(r#"{"verdict":"approve"}"#.into())
    }
}

#[test]
fn coder_runs_a_shell_command_through_the_gate() {
    if std::env::var_os("SHIFT_LIVE").is_none() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("shift-live-{}", std::process::id()));
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let coder = builtin_agent("coder").unwrap();
    let tools = std::sync::Arc::new(SlowTool(Default::default()));
    let (session, events) = ClaudeAdapter
        .start(&coder, &repo, &dir, tools.clone())
        .unwrap();
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
    // The id a shift tool call arrives with is the one its event carries,
    // so the Cancel review button can name the call it is shown for.
    let shown: Vec<&String> = seen
        .iter()
        .filter_map(|e| match e {
            Event::ToolCall { id, name, .. } if name == "mcp__shift__delegate_task" => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(shown, tools.0.lock().unwrap().iter().collect::<Vec<_>>());
    assert_eq!(shown.len(), 1);
    println!("transcript: {}", dir.join(TRANSCRIPT).display());
}
