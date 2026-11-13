//! The minimal policy gate: one check for every harness approval request.

use crate::AgentDef;

/// Allow `tool` (a shift tool name such as `fs.write`) only if the agent
/// definition lists it. `None` means the adapter could not map the harness
/// tool to a shift tool, which is always a deny.
pub fn allows(agent: &AgentDef, tool: Option<&str>) -> bool {
    tool.is_some_and(|tool| agent.tools.iter().any(|t| t == tool))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_only_listed_tools() {
        let coder = crate::builtin_agent("coder").unwrap();
        assert!(allows(&coder, Some("shell.exec")));
        assert!(!allows(&coder, Some("web.search")));
        assert!(!allows(&coder, None));
    }
}
