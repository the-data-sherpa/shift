//! The Claude Code adapter: launches `claude` from an agent definition and
//! speaks its stream-json protocol over stdio (ADR 0003).

mod translate;

use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

use async_channel::{Receiver, Sender};
use serde_json::{Value, json};
use shift_core::{Adapter, AgentDef, Event, Harness, Session};

pub use translate::{Output, Translator};

/// File name of the raw Claude NDJSON inside a session's transcript directory.
pub const TRANSCRIPT: &str = "coder.claude.ndjson";

/// The Claude built-in tools behind each shift tool name.
const TOOL_MAP: &[(&str, &[&str])] = &[
    ("fs.read", &["Read", "Glob", "Grep"]),
    ("fs.write", &["Edit", "Write"]),
    ("shell.exec", &["Bash"]),
];

/// The shift tool a Claude built-in tool belongs to.
pub fn shift_tool(claude_tool: &str) -> Option<&'static str> {
    TOOL_MAP
        .iter()
        .find(|(_, claude)| claude.contains(&claude_tool))
        .map(|(shift, _)| *shift)
}

/// The command line for `agent`. Fails if the agent lists a tool Claude Code
/// cannot be limited to, instead of quietly widening the boundary.
pub fn launch_args(agent: &AgentDef) -> io::Result<Vec<String>> {
    let mut tools = Vec::new();
    for tool in &agent.tools {
        let (_, claude) = TOOL_MAP
            .iter()
            .find(|(shift, _)| shift == tool)
            .ok_or_else(|| {
                io::Error::other(format!(
                    "agent `{}` lists tool `{tool}`, which the Claude Code adapter cannot map",
                    agent.name
                ))
            })?;
        tools.extend_from_slice(claude);
    }
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        // shift is the host: it answers every permission prompt over stdio.
        "--permission-prompts",
        "host",
        "--permission-prompt-tool",
        "stdio",
        // User settings carry the user's usual model. Project and local
        // settings stay out; `--tools` is the boundary either way.
        "--setting-sources",
        "user",
        "--strict-mcp-config",
        "--tools",
    ]
    .map(String::from)
    .into();
    args.push(tools.join(","));
    args.push("--append-system-prompt".into());
    args.push(agent.prompt.clone());
    if let Some(model) = &agent.model {
        args.push("--model".into());
        args.push(model.clone());
    }
    Ok(args)
}

pub struct ClaudeAdapter;

impl Adapter for ClaudeAdapter {
    fn harness(&self) -> Harness {
        Harness::ClaudeCode
    }

    fn start(
        &self,
        agent: &AgentDef,
        cwd: &Path,
        transcript_dir: &Path,
    ) -> io::Result<(Box<dyn Session>, Receiver<Event>)> {
        let mut transcript = File::create(transcript_dir.join(TRANSCRIPT))?;
        let mut child = Command::new(Harness::ClaudeCode.binary())
            .args(launch_args(agent)?)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("could not start `claude`: {e}")))?;
        let stdin = Arc::new(Mutex::new(child.stdin.take().expect("piped stdin")));
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        let (events, receiver) = async_channel::unbounded();

        let writer = stdin.clone();
        let mut translator = Translator::new(agent.clone());
        std::thread::spawn(move || {
            for line in stdout.lines() {
                let Ok(line) = line else { break };
                if let Err(e) = writeln!(transcript, "{line}") {
                    send(
                        &events,
                        Event::Error(format!("could not save transcript: {e}")),
                    );
                }
                let output = translator.handle_line(&line);
                for reply in output.replies {
                    if let Err(e) = write_frame(&writer, &reply) {
                        send(
                            &events,
                            Event::Error(format!("could not answer Claude: {e}")),
                        );
                    }
                }
                for event in output.events {
                    send(&events, event);
                }
            }
            send(&events, Event::Closed);
        });

        Ok((Box::new(ClaudeSession { stdin, child }), receiver))
    }
}

fn send(events: &Sender<Event>, event: Event) {
    // The receiver only goes away when the app is closing.
    let _ = events.send_blocking(event);
}

fn write_frame(stdin: &Mutex<ChildStdin>, frame: &Value) -> io::Result<()> {
    let mut stdin = stdin.lock().unwrap();
    writeln!(stdin, "{frame}")?;
    stdin.flush()
}

struct ClaudeSession {
    stdin: Arc<Mutex<ChildStdin>>,
    child: Child,
}

impl Session for ClaudeSession {
    fn send(&self, text: &str) -> io::Result<()> {
        write_frame(
            &self.stdin,
            &json!({
                "type": "user",
                "message": {"role": "user", "content": text},
                "parent_tool_use_id": null,
                "session_id": "",
            }),
        )
    }
}

impl Drop for ClaudeSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
