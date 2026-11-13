use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_channel::Receiver;

use crate::{AgentDef, Event, Harness, ToolServer};

/// One agent's conversation inside one harness.
pub trait Session: Send {
    /// Send a user message to the agent.
    fn send(&self, text: &str) -> io::Result<()>;
}

/// shift's integration with one harness.
pub trait Adapter {
    fn harness(&self) -> Harness;

    /// Start `agent` in `cwd`. Raw harness output is saved under
    /// `transcript_dir`; translated events arrive on the receiver. The agent
    /// can call the shift tools in `tools`.
    fn start(
        &self,
        agent: &AgentDef,
        cwd: &Path,
        transcript_dir: &Path,
        tools: Arc<dyn ToolServer>,
    ) -> io::Result<(Box<dyn Session>, Receiver<Event>)>;
}

/// shift's state directory: `$XDG_STATE_HOME/shift` or `~/.local/state/shift`.
pub fn state_dir() -> io::Result<PathBuf> {
    let state = match std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home =
                std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is not set"))?;
            PathBuf::from(home).join(".local/state")
        }
    };
    Ok(state.join("shift"))
}

/// Create `<state>/sessions/<id>/` for a new session and return the id and
/// the directory.
pub fn new_session_dir() -> io::Result<(String, PathBuf)> {
    let id = uuid::Uuid::new_v4().to_string();
    let dir = state_dir()?.join("sessions").join(&id);
    std::fs::create_dir_all(&dir)?;
    Ok((id, dir))
}
