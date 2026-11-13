//! The Codex adapter: runs the reviewer through `codex app-server`, speaking
//! its JSON-RPC protocol over stdio (ADR 0003).

mod translate;

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_channel::Sender;
use serde_json::{Value, json};
use shift_core::review::{Reviewer, review_schema};
use shift_core::{AgentDef, Cancel, Event, Harness};

pub use translate::translate;

/// File name of the raw Codex JSON-RPC inside a session's transcript
/// directory. Lines are in wire order, in both directions.
pub const TRANSCRIPT: &str = "reviewer.codex.jsonl";

/// The newest model of `family` (such as `sol`) among `ids`, which look like
/// `gpt-6.1-sol`. `None` if the family is not offered: the caller must fail
/// rather than use another model.
pub fn latest_model(family: &str, ids: &[String]) -> Option<String> {
    let version = |id: &str| -> Option<Vec<u32>> {
        let version = id
            .strip_prefix("gpt-")?
            .strip_suffix(family)?
            .strip_suffix('-')?;
        version.split('.').map(|part| part.parse().ok()).collect()
    };
    ids.iter()
        .filter_map(|id| Some((version(id)?, id)))
        .max()
        .map(|(_, id)| id.clone())
}

/// The wall-clock limit on one review. It also stands in for `max_turns`.
pub const REVIEW_LIMIT: Duration = Duration::from_secs(15 * 60);

/// Runs the reviewer agent on Codex.
pub struct CodexReviewer {
    /// The program to launch; `codex` unless a test swaps it.
    pub program: PathBuf,
    /// How long one review may run; [`REVIEW_LIMIT`] unless a test swaps it.
    pub limit: Duration,
    cancel: Cancel,
    agent: AgentDef,
    /// The reviewer's own persistent `CARGO_TARGET_DIR`.
    target_dir: PathBuf,
    transcript: Arc<Mutex<File>>,
    /// The reviewer session, for the read-only reviewer pane.
    events: Sender<Event>,
    /// The running app-server, kept between rounds.
    session: Mutex<Option<Connection>>,
}

impl CodexReviewer {
    pub fn new(
        agent: AgentDef,
        target_dir: PathBuf,
        transcript_dir: &Path,
        events: Sender<Event>,
        cancel: Cancel,
    ) -> std::io::Result<Self> {
        std::fs::create_dir_all(&target_dir)?;
        let transcript = File::create(transcript_dir.join(TRANSCRIPT))?;
        Ok(Self {
            program: Harness::Codex.binary().into(),
            limit: REVIEW_LIMIT,
            cancel,
            agent,
            target_dir,
            transcript: Arc::new(Mutex::new(transcript)),
            events,
            session: Mutex::new(None),
        })
    }
}

impl Reviewer for CodexReviewer {
    fn review(&self, worktree: &Path, brief: &str) -> Result<String, String> {
        let mut session = self.session.lock().unwrap();
        // Later rounds are new turns on the same thread. A session that
        // failed is not reused: the next round starts a fresh one.
        let mut codex = match session.take() {
            Some(mut codex) => {
                // The limit is per review, not per session.
                codex.start_clock(self.limit);
                codex
            }
            None => self.start(worktree)?,
        };
        let reply = codex.turn(worktree, brief, &self.target_dir)?;
        *session = Some(codex);
        Ok(reply)
    }
}

impl CodexReviewer {
    /// Start `codex app-server` and the reviewer's thread on the latest Sol.
    fn start(&self, worktree: &Path) -> Result<Connection, String> {
        let mut codex = Connection::open(self)?;
        codex.start_clock(self.limit);
        let family = self
            .agent
            .model
            .as_deref()
            .ok_or("the reviewer has no model")?;

        codex.request(
            "initialize",
            json!({"clientInfo": {"name": "shift", "title": "shift", "version": env!("CARGO_PKG_VERSION")}}),
        )?;
        codex.send(&json!({"method": "initialized"}))?;

        let models = codex.request("model/list", json!({}))?;
        let ids: Vec<String> = models["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|model| model["id"].as_str().map(String::from))
            .collect();
        let model = latest_model(family, &ids).ok_or_else(|| {
            format!("{family} is unavailable on Codex; not substituting another model")
        })?;

        let thread = codex.request(
            "thread/start",
            json!({
                "model": model,
                "cwd": worktree,
                "sandbox": "workspace-write",
                // Nothing outside the sandbox is ever approved.
                "approvalPolicy": "never",
                "developerInstructions": self.agent.prompt,
            }),
        )?;
        codex.thread = thread["thread"]["id"].as_str().map(String::from);
        if thread["model"] != model.as_str() {
            return Err(format!(
                "Codex started the reviewer on {} instead of {model}",
                thread["model"]
            ));
        }
        Ok(codex)
    }
}

/// One `codex app-server` process.
struct Connection {
    child: Child,
    /// Lines for Codex's stdin. A thread writes them, so a Codex that
    /// stops reading cannot block the review past its limit or a cancel.
    outgoing: mpsc::Sender<String>,
    /// Every line Codex sends.
    incoming: Receiver<String>,
    transcript: Arc<Mutex<File>>,
    events: Sender<Event>,
    /// The reviewer's thread, once started. Codex also reports on threads of
    /// its own; those are not the review.
    thread: Option<String>,
    next_id: u64,
    cancel: Cancel,
    /// The running review's limit and when it runs out.
    clock: Option<(Duration, Instant)>,
}

impl Connection {
    fn open(reviewer: &CodexReviewer) -> Result<Self, String> {
        let mut child = Command::new(&reviewer.program)
            .arg("app-server")
            .env("CARGO_TARGET_DIR", &reviewer.target_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                let program = reviewer.program.display();
                format!("could not start `{program} app-server`: {e}")
            })?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let (outgoing, lines) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in lines {
                if writeln!(stdin, "{line}")
                    .and_then(|()| stdin.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        let (sender, incoming) = mpsc::channel();
        let transcript = reviewer.transcript.clone();
        std::thread::spawn(move || {
            for line in stdout.lines() {
                let Ok(line) = line else { break };
                let _ = writeln!(transcript.lock().unwrap(), "{line}");
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            outgoing,
            incoming,
            transcript: reviewer.transcript.clone(),
            events: reviewer.events.clone(),
            thread: None,
            next_id: 0,
            cancel: reviewer.cancel.clone(),
            clock: None,
        })
    }

    /// Start one review's wall clock.
    fn start_clock(&mut self, limit: Duration) {
        self.clock = Some((limit, Instant::now() + limit));
    }

    /// The next line from Codex, unless the user cancels, the review runs
    /// out of time, or Codex exits first.
    fn next_line(&mut self) -> Result<String, String> {
        loop {
            if self.cancel.is_cancelled() {
                return Err("cancelled by user".into());
            }
            if let Some((limit, deadline)) = self.clock
                && Instant::now() >= deadline
            {
                let limit = match limit.as_secs() {
                    secs if secs >= 60 => format!("{}-minute", secs / 60),
                    secs => format!("{secs}-second"),
                };
                return Err(format!("the review exceeded its {limit} limit"));
            }
            match self.incoming.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => return Ok(line),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(self.closed()),
            }
        }
    }

    /// Why Codex's output ended early. An exiting process takes a moment to
    /// be reapable; one that only closed its output is not waited for.
    fn closed(&mut self) -> String {
        for _ in 0..10 {
            if let Ok(Some(status)) = self.child.try_wait() {
                return format!("Codex exited before the review finished ({status})");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        "Codex closed its output before the review finished".into()
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        let _ = writeln!(self.transcript.lock().unwrap(), "{message}");
        // The writer only stops when Codex is gone, as when a session
        // kept between rounds has died; say that.
        self.outgoing
            .send(message.to_string())
            .map_err(|_| self.closed())
    }

    /// The next message about the reviewer's thread, or about no thread.
    /// Its events go to the reviewer pane.
    fn next(&mut self) -> Result<Value, String> {
        loop {
            let line = self.next_line()?;
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let thread = &message["params"]["threadId"];
            if !thread.is_null() && self.thread.as_deref() != thread.as_str() {
                continue;
            }
            for event in translate(&line) {
                let _ = self.events.send_blocking(event);
            }
            return Ok(message);
        }
    }

    /// Send a request and wait for its result.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"id": id, "method": method, "params": params}))?;
        loop {
            let message = self.next()?;
            if message["id"] == id && message.get("method").is_none() {
                return match message.get("error") {
                    Some(error) => Err(format!("Codex refused {method}: {}", error["message"])),
                    None => Ok(message["result"].clone()),
                };
            }
            self.refuse_request(&message)?;
        }
    }

    /// Run one review turn in `worktree`, this round's snapshot, and return
    /// the reviewer's final message.
    fn turn(&mut self, worktree: &Path, brief: &str, target_dir: &Path) -> Result<String, String> {
        self.request(
            "turn/start",
            json!({
                "threadId": self.thread,
                "input": [{"type": "text", "text": brief}],
                "outputSchema": review_schema(),
                "cwd": worktree,
                "sandboxPolicy": {
                    "type": "workspaceWrite",
                    "writableRoots": [target_dir],
                    "networkAccess": false,
                },
            }),
        )?;
        self.finish_turn()
    }

    /// Wait for the running turn to end and return its final message.
    fn finish_turn(&mut self) -> Result<String, String> {
        let mut last_message = None;
        loop {
            let message = self.next()?;
            let params = &message["params"];
            match message["method"].as_str() {
                Some("item/completed") if params["item"]["type"] == "agentMessage" => {
                    last_message = params["item"]["text"].as_str().map(String::from);
                }
                Some("model/rerouted") => {
                    return Err("Codex rerouted the reviewer to another model".into());
                }
                Some("turn/completed") => {
                    let turn = &params["turn"];
                    if turn["status"] != "completed" {
                        return Err(format!(
                            "the reviewer's turn ended as {}: {}",
                            turn["status"], turn["error"]["message"]
                        ));
                    }
                    return last_message.ok_or_else(|| "the reviewer returned no message".into());
                }
                _ => self.refuse_request(&message)?,
            }
        }
    }

    /// Codex asks the client for approvals and other input through requests.
    /// The reviewer gets none: its sandbox is its boundary.
    fn refuse_request(&mut self, message: &Value) -> Result<(), String> {
        if message.get("method").is_some() && message.get("id").is_some() {
            self.send(&json!({
                "id": message["id"],
                "error": {"code": -32601, "message": "shift does not answer reviewer requests"},
            }))?;
        }
        Ok(())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
