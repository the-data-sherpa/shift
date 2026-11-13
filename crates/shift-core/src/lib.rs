//! Domain types for shift. See `CONTEXT.md` for vocabulary.

pub mod agent;
pub mod event;
pub mod git;
pub mod harness;
pub mod policy;
pub mod session;

pub use agent::{AgentDef, AgentLimits, LoadError, builtin_agent, load_agents_dir};
pub use event::Event;
pub use git::{WorkBranch, start_work_branch};
pub use harness::Harness;
pub use session::{Adapter, Session, new_session_dir};
