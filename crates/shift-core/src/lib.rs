//! Domain types for shift. See `CONTEXT.md` for vocabulary.

pub mod agent;
pub mod cancel;
pub mod delegate;
pub mod event;
pub mod git;
pub mod harness;
pub mod history;
pub mod policy;
pub mod review;
pub mod session;

pub use agent::{AgentDef, AgentLimits, LoadError, builtin_agent, load_agents_dir};
pub use cancel::{Cancel, Cancellable};
pub use delegate::{ToolServer, ToolSpec};
pub use event::Event;
pub use git::{Snapshot, WorkBranch, snapshot, start_work_branch};
pub use harness::Harness;
pub use session::{Adapter, Session, new_session_dir, state_dir};
