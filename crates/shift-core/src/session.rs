use std::io;
use std::path::{Path, PathBuf};

use async_channel::Receiver;

use crate::{AgentDef, Event, Harness};

/// One agent's conversation inside one harness.
pub trait Session: Send {
    /// Send a user message to the agent.
    fn send(&self, text: &str) -> io::Result<()>;
}

/// shift's integration with one harness.
pub trait Adapter {
    fn harness(&self) -> Harness;

    /// Start `agent` in `cwd`. Raw harness output is saved under
    /// `transcript_dir`; translated events arrive on the receiver.
    fn start(
        &self,
        agent: &AgentDef,
        cwd: &Path,
        transcript_dir: &Path,
    ) -> io::Result<(Box<dyn Session>, Receiver<Event>)>;
}

/// Create `<state>/shift/sessions/<id>/` for a new session and return the id
/// and the directory. `<state>` is `$XDG_STATE_HOME` or `~/.local/state`.
pub fn new_session_dir() -> io::Result<(String, PathBuf)> {
    let state = match std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home =
                std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is not set"))?;
            PathBuf::from(home).join(".local/state")
        }
    };
    let id = uuid::Uuid::new_v4().to_string();
    let dir = state.join("shift/sessions").join(&id);
    std::fs::create_dir_all(&dir)?;
    Ok((id, dir))
}
