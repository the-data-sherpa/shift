//! Domain types for shift. See `CONTEXT.md` for vocabulary.

pub mod agent;
pub mod harness;

pub use agent::{AgentDef, AgentLimits, LoadError, load_agents_dir};
pub use harness::Harness;
