use serde::Deserialize;

/// A backend harness shift can launch an instance of. See ADR 0001 / 0002.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Codex,
    Antigravity,
}

impl Harness {
    pub fn label(self) -> &'static str {
        match self {
            Harness::ClaudeCode => "Claude Code",
            Harness::Codex => "Codex",
            Harness::Antigravity => "Antigravity",
        }
    }

    /// Executable the adapter launches.
    pub fn binary(self) -> &'static str {
        match self {
            Harness::ClaudeCode => "claude",
            Harness::Codex => "codex",
            Harness::Antigravity => "agy",
        }
    }
}
