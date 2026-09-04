//! # Skill Bundles (`skill.yaml`)
//!
//! A **skill** is a higher-level, human-authored unit of reusable capability that
//! layers *over* the low-level tool and WASM plugin primitives. Where a plugin is
//! a compiled `.wasm` binary and a tool is a single built-in operation, a skill is
//! a declarative `skill.yaml` manifest bundling a script (or command) with its
//! metadata, parameters, environment, timeout, and sandbox policy.
//!
//! ## Manifest format (`skill.yaml`)
//!
//! ```yaml
//! name: summarize
//! version: 1.0.0
//! description: Summarize a text file.
//!
//! entrypoint:
//!   command: ./scripts/summarize.sh
//!   args: ["--max-words", "50"]
//!
//! env:
//!   LOG_LEVEL: info
//!
//! timeout_secs: 30
//!
//! sandbox:
//!   allow_network: false
//!   allow_workdir_write: true
//! ```
//!
//! ## Security model
//!
//! Skills never bypass the policy engine or sandbox. A skill's `entrypoint.command`
//! is executed through the same shell/sandbox path as the `shell_exec` tool, so it
//! is subject to the deny-by-default shell policy, path jail, resource limits, and
//! audit logging. The manifest only *declares* preferences (timeout, network,
//! write) which are intersected with the ambient sandbox config.
//!
//! ## Stability
//! `SkillManifest` is `#[non_exhaustive]` — new fields may be added in minor releases.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A single declarative skill bundle, deserialized from `skill.yaml`.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SkillManifest {
    /// Unique, kebab-case skill name (e.g. "summarize", "deploy-backend").
    pub name: String,

    /// Optional semantic version of the skill.
    #[serde(default)]
    pub version: Option<String>,

    /// One-line description of what the skill does.
    #[serde(default)]
    pub description: Option<String>,

    /// The command/script to run, plus optional fixed arguments.
    #[serde(default)]
    pub entrypoint: SkillEntrypoint,

    /// Environment variables to inject (intersected with the sandbox allow-list).
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,

    /// Maximum execution time in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// Optional sandbox preferences for this skill.
    #[serde(default)]
    pub sandbox: Option<SkillSandbox>,
}

fn default_timeout() -> u64 {
    30
}

/// The entrypoint of a skill — a command plus optional fixed arguments.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct SkillEntrypoint {
    /// The command or script path (e.g. `./scripts/run.sh`, `python3`, `node`).
    pub command: String,

    /// Optional fixed arguments appended before any runtime arguments.
    #[serde(default)]
    pub args: Vec<String>,
}

/// Sandbox preferences declared by a skill manifest.
///
/// These are **preferences**, not grants: they are intersected with the ambient
/// [`crate::sandbox::SandboxConfig`] so a skill can only ever *narrow* (never
/// widen) the security posture.
///
/// # Stability
/// This struct is `#[non_exhaustive]` — new fields may be added in minor releases.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct SkillSandbox {
    /// Whether the skill requests network access. `false` by default.
    #[serde(default)]
    pub allow_network: bool,

    /// Whether the skill requests write access within the workdir. `true` by default.
    #[serde(default = "default_true")]
    pub allow_workdir_write: bool,
}

fn default_true() -> bool {
    true
}

/// Errors that can occur when parsing, validating, or executing a skill.
#[derive(Debug, Error)]
pub enum SkillError {
    #[error("Failed to parse skill manifest: {0}")]
    Parse(String),

    #[error("Invalid skill manifest: {0}")]
    Validation(String),
}

impl SkillManifest {
    /// Parse a skill manifest from a YAML string.
    pub fn from_yaml(yaml_str: &str) -> Result<Self, SkillError> {
        serde_yaml::from_str(yaml_str).map_err(|e| SkillError::Parse(e.to_string()))
    }

    /// Parse a skill manifest from a JSON string (JSON is a YAML subset).
    pub fn from_json(json_str: &str) -> Result<Self, SkillError> {
        serde_json::from_str(json_str).map_err(|e| SkillError::Parse(e.to_string()))
    }

    /// Validate the manifest's internal consistency.
    ///
    /// Returns an error if the name is empty, the entrypoint command is empty,
    /// or the timeout is zero.
    pub fn validate(&self) -> Result<(), SkillError> {
        if self.name.trim().is_empty() {
            return Err(SkillError::Validation(
                "skill name must not be empty".to_string(),
            ));
        }
        if self.entrypoint.command.trim().is_empty() {
            return Err(SkillError::Validation(
                "skill entrypoint.command must not be empty".to_string(),
            ));
        }
        if self.timeout_secs == 0 {
            return Err(SkillError::Validation(
                "timeout_secs must be >= 1".to_string(),
            ));
        }
        Ok(())
    }

    /// Render the skill's full command string, combining the entrypoint command,
    /// fixed arguments, and any runtime arguments into a single shell command.
    ///
    /// Arguments are shell-quoted so the result can be passed directly to the
    /// sandboxed `shell_exec` tool.
    pub fn to_command(&self, runtime_args: &[String]) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(self.entrypoint.command.clone());
        parts.extend(self.entrypoint.args.iter().cloned());
        parts.extend(runtime_args.iter().cloned());
        parts
            .into_iter()
            .map(|p| shell_quote(&p))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The skill's effective timeout (seconds).
    pub fn timeout_secs(&self) -> u64 {
        self.timeout_secs
    }

    /// Whether the skill requests network access.
    pub fn requests_network(&self) -> bool {
        self.sandbox
            .as_ref()
            .map(|s| s.allow_network)
            .unwrap_or(false)
    }
}

/// Shell-quote a single argument for safe inclusion in a `sh -c` command string.
fn shell_quote(arg: &str) -> String {
    // If the argument is a safe bare word, leave it as-is; otherwise single-quote
    // it, escaping any embedded single quotes.
    let safe = !arg.is_empty()
        && arg.chars().all(|c| {
            c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '=' | '@' | '%' | '+')
        });
    if safe {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_yaml() -> &'static str {
        r#"
name: summarize
version: 1.0.0
description: Summarize a text file.
entrypoint:
  command: ./scripts/summarize.sh
  args: ["--max-words", "50"]
env:
  LOG_LEVEL: info
timeout_secs: 30
sandbox:
  allow_network: false
  allow_workdir_write: true
"#
    }

    #[test]
    fn test_parse_yaml() {
        let skill = SkillManifest::from_yaml(sample_yaml()).unwrap();
        assert_eq!(skill.name, "summarize");
        assert_eq!(skill.version.as_deref(), Some("1.0.0"));
        assert_eq!(skill.description.as_deref(), Some("Summarize a text file."));
        assert_eq!(skill.entrypoint.command, "./scripts/summarize.sh");
        assert_eq!(skill.entrypoint.args, vec!["--max-words", "50"]);
        assert_eq!(skill.timeout_secs, 30);
        assert!(skill.validate().is_ok());
    }

    #[test]
    fn test_parse_json() {
        let json = r#"{
            "name": "deploy",
            "entrypoint": { "command": "kubectl", "args": ["apply", "-f", "k8s/"] },
            "timeout_secs": 120
        }"#;
        let skill = SkillManifest::from_json(json).unwrap();
        assert_eq!(skill.name, "deploy");
        assert_eq!(skill.entrypoint.command, "kubectl");
        assert_eq!(skill.timeout_secs, 120);
        assert!(skill.validate().is_ok());
    }

    #[test]
    fn test_defaults() {
        let yaml = "name: hello\nentrypoint:\n  command: echo\n";
        let skill = SkillManifest::from_yaml(yaml).unwrap();
        assert_eq!(skill.version, None);
        assert_eq!(skill.description, None);
        assert_eq!(skill.timeout_secs, 30);
        assert!(skill.entrypoint.args.is_empty());
        assert!(skill.env.is_empty());
        assert!(skill.sandbox.is_none());
    }

    #[test]
    fn test_validate_empty_name() {
        let skill = SkillManifest::from_yaml("name: ''\nentrypoint:\n  command: echo\n").unwrap();
        assert!(skill.validate().is_err());
    }

    #[test]
    fn test_validate_empty_command() {
        let skill = SkillManifest::from_yaml("name: hello\nentrypoint:\n  command: ''\n").unwrap();
        assert!(skill.validate().is_err());
    }

    #[test]
    fn test_validate_zero_timeout() {
        let skill = SkillManifest::from_yaml(
            "name: hello\nentrypoint:\n  command: echo\ntimeout_secs: 0\n",
        )
        .unwrap();
        assert!(skill.validate().is_err());
    }

    #[test]
    fn test_to_command_basic() {
        let skill = SkillManifest::from_yaml(sample_yaml()).unwrap();
        let cmd = skill.to_command(&[]);
        assert_eq!(cmd, "./scripts/summarize.sh --max-words 50");
    }

    #[test]
    fn test_to_command_with_runtime_args_and_quoting() {
        let skill = SkillManifest::from_yaml("name: x\nentrypoint:\n  command: echo\n").unwrap();
        let cmd = skill.to_command(&["hello world".to_string(), "a'b".to_string()]);
        assert_eq!(cmd, "echo 'hello world' 'a'\\''b'");
    }

    #[test]
    fn test_shell_quote() {
        assert_eq!(shell_quote("simple"), "simple");
        assert_eq!(shell_quote("with space"), "'with space'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn test_requests_network() {
        let skill = SkillManifest::from_yaml(sample_yaml()).unwrap();
        assert!(!skill.requests_network());

        let skill = SkillManifest::from_yaml(
            "name: x\nentrypoint:\n  command: curl\nsandbox:\n  allow_network: true\n",
        )
        .unwrap();
        assert!(skill.requests_network());
    }
}
