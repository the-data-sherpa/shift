use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::Harness;

/// An agent definition: YAML frontmatter + a Markdown system prompt body.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentDef {
    pub name: String,
    pub role: String,
    pub description: String,
    pub harness: Harness,
    pub model: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub limits: AgentLimits,
    #[serde(default)]
    pub forbidden: Vec<String>,
    /// Markdown body after the frontmatter.
    #[serde(skip)]
    pub prompt: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentLimits {
    pub max_turns: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: missing `---` frontmatter")]
    NoFrontmatter { path: PathBuf },
    #[error("{path}: {source}")]
    Yaml {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },
}

impl AgentDef {
    pub fn parse(path: &Path, text: &str) -> Result<Self, LoadError> {
        let no_fm = || LoadError::NoFrontmatter {
            path: path.to_owned(),
        };
        let rest = text.strip_prefix("---").ok_or_else(no_fm)?;
        let (yaml, body) = rest.split_once("\n---").ok_or_else(no_fm)?;
        let mut def: AgentDef =
            serde_yaml_ng::from_str(yaml).map_err(|source| LoadError::Yaml {
                path: path.to_owned(),
                source,
            })?;
        def.prompt = body.trim().to_owned();
        Ok(def)
    }

    pub fn load(path: &Path) -> Result<Self, LoadError> {
        let text = std::fs::read_to_string(path).map_err(|source| LoadError::Io {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(path, &text)
    }
}

/// Load every `*.md` agent definition in `dir`, sorted by name.
pub fn load_agents_dir(dir: &Path) -> Result<Vec<AgentDef>, LoadError> {
    let io = |source| LoadError::Io {
        path: dir.to_owned(),
        source,
    };
    let mut agents = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io)? {
        let path = entry.map_err(io)?.path();
        if path.extension().is_some_and(|e| e == "md") {
            agents.push(AgentDef::load(&path)?);
        }
    }
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(agents)
}

/// The agent definitions compiled into the binary, so an installed copy of
/// shift works from any directory.
const BUILTIN: &[(&str, &str)] = &[
    ("coder", include_str!("../../../agents/coder.md")),
    ("reviewer", include_str!("../../../agents/reviewer.md")),
];

/// A built-in agent definition by name.
pub fn builtin_agent(name: &str) -> Result<AgentDef, LoadError> {
    let path = PathBuf::from(format!("agents/{name}.md"));
    let (_, text) = BUILTIN
        .iter()
        .find(|(n, _)| *n == name)
        .ok_or_else(|| LoadError::NoFrontmatter { path: path.clone() })?;
    AgentDef::parse(&path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let text = "---\nname: coder\nrole: coder\ndescription: d\nharness: codex\nskills: [debug]\nlimits:\n  max_turns: 30\n---\n\nYou are the coder.\n";
        let def = AgentDef::parse(Path::new("coder.md"), text).unwrap();
        assert_eq!(def.harness, Harness::Codex);
        assert_eq!(def.skills, ["debug"]);
        assert_eq!(def.limits.max_turns, Some(30));
        assert_eq!(def.prompt, "You are the coder.");
    }

    #[test]
    fn builtin_coder_has_its_tools_and_no_fixed_model() {
        let coder = builtin_agent("coder").unwrap();
        assert_eq!(coder.harness, Harness::ClaudeCode);
        assert_eq!(coder.tools, ["fs.read", "fs.write", "shell.exec"]);
        assert_eq!(coder.model, None);
        assert!(coder.prompt.starts_with("You are the coder."));
        assert!(builtin_agent("reviewer").is_ok());
        assert!(builtin_agent("nobody").is_err());
    }

    #[test]
    fn loads_repo_agents() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../agents");
        let agents = load_agents_dir(&dir).unwrap();
        assert!(agents.iter().any(|a| a.name == "scout"));
    }
}
