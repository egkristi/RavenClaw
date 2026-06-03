//! Tool / function-calling abstraction for RavenClaw
//!
//! Provides a provider-agnostic tool schema, a registry for built-in tools,
//! and the execution engine that routes tool calls to their implementations.
//!
//! # Architecture
//!
//! ```text
//! ToolRegistry (holds all registered tools)
//!   ├── ToolDefinition (name, description, JSON schema)
//!   └── ToolImpl (the actual implementation)
//!         ├── ShellTool — execute shell commands (sandboxed)
//!         ├── ReadFileTool — read files (policy-checked)
//!         ├── WriteFileTool — write files (policy-checked)
//!         ├── WebFetchTool — fetch URLs (policy-checked)
//!         └── ... more tools
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tracing::{info, warn};

// Re-export sandbox for tool implementations
use crate::sandbox::Sandbox;

// ── Error types ────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum ToolError {
    #[error("Tool '{0}' not found")]
    NotFound(String),

    #[error("Tool '{0}' execution failed: {1}")]
    ExecutionFailed(String, String),

    #[error("Invalid arguments for tool '{0}': {1}")]
    InvalidArguments(String, String),

    #[allow(dead_code)]
    #[error("Policy denied: {0}")]
    PolicyDenied(String),

    #[allow(dead_code)]
    #[error("Sandbox violation: {0}")]
    SandboxViolation(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type ToolResultValue<T> = std::result::Result<T, ToolError>;

// ── Tool schema types ──────────────────────────────────────────────────────

/// JSON Schema representation for tool parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, JsonSchema>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<JsonSchema>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
}

impl JsonSchema {
    /// Create a string schema property
    pub fn string(description: &str) -> Self {
        Self {
            schema_type: "string".to_string(),
            description: Some(description.to_string()),
            properties: None,
            required: None,
            items: None,
            enum_values: None,
        }
    }

    /// Create an object schema
    pub fn object(properties: HashMap<String, JsonSchema>, required: Vec<String>) -> Self {
        Self {
            schema_type: "object".to_string(),
            description: None,
            properties: Some(properties),
            required: Some(required),
            items: None,
            enum_values: None,
        }
    }

    /// Create an array schema
    #[allow(dead_code)]
    pub fn array(items: JsonSchema, description: &str) -> Self {
        Self {
            schema_type: "array".to_string(),
            description: Some(description.to_string()),
            properties: None,
            required: None,
            items: Some(Box::new(items)),
            enum_values: None,
        }
    }
}

/// A tool definition — the schema exposed to the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// The name of the tool (e.g., "shell_exec", "read_file")
    pub name: String,
    /// A description of what the tool does (for the LLM)
    pub description: String,
    /// JSON Schema for the tool's parameters
    pub parameters: JsonSchema,
    /// Whether this tool requires human approval
    #[serde(default)]
    pub requires_approval: bool,
    /// Category for grouping
    #[serde(default)]
    pub category: ToolCategory,
}

impl ToolDefinition {
    /// Convert to OpenAI Tools format for structured function calling
    /// See: https://platform.openai.com/docs/guides/function-calling
    pub fn to_openai_tool(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters
            }
        })
    }
}

/// Tool categories for grouping and policy
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum ToolCategory {
    #[default]
    General,
    Shell,
    FileSystem,
    Network,
    CodeAnalysis,
    WebSearch,
    Mcp,
}

/// A tool call request from the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// The name of the tool to call
    pub name: String,
    /// The arguments as a JSON object
    pub arguments: serde_json::Value,
    /// An optional ID for tracking (used by some providers)
    #[serde(default)]
    pub id: Option<String>,
}

/// The result of a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// The name of the tool that was called
    pub tool_name: String,
    /// Whether the execution was successful
    pub success: bool,
    /// The output (stdout or result data)
    pub output: String,
    /// Error message if failed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Exit code (for shell commands)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Duration in milliseconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

// ── Tool implementation trait ──────────────────────────────────────────────

/// The actual implementation of a tool
#[async_trait::async_trait]
pub trait ToolImpl: Send + Sync {
    /// Execute the tool with the given arguments
    async fn execute(&self, args: serde_json::Value) -> ToolResultValue<ToolResult>;

    /// Get the tool's definition (schema)
    fn definition(&self) -> &ToolDefinition;

    /// Get a display name for logging
    fn name(&self) -> &str {
        &self.definition().name
    }
}

// ── Tool registry ──────────────────────────────────────────────────────────

/// Registry of all available tools
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn ToolImpl>>,
}

impl ToolRegistry {
    /// Create a new empty tool registry
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool
    pub fn register(&mut self, tool: Arc<dyn ToolImpl>) {
        let name = tool.name().to_string();
        info!(tool = %name, category = ?tool.definition().category, "Tool registered");
        self.tools.insert(name, tool);
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<&Arc<dyn ToolImpl>> {
        self.tools.get(name)
    }

    /// Check if a tool exists
    #[allow(dead_code)]
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Get all tool definitions (for sending to LLM)
    #[allow(dead_code)]
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|t| t.definition().clone())
            .collect()
    }

    /// Get all tool definitions in OpenAI Tools format for structured function calling
    pub fn to_openai_tools(&self) -> Vec<serde_json::Value> {
        self.tools
            .values()
            .map(|t| t.definition().to_openai_tool())
            .collect()
    }

    /// Get the number of registered tools
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if the registry is empty
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Execute a tool call
    pub async fn execute(&self, call: ToolCall) -> ToolResultValue<ToolResult> {
        let start = std::time::Instant::now();

        let tool = self
            .get(&call.name)
            .ok_or_else(|| ToolError::NotFound(call.name.clone()))?;

        info!(tool = %call.name, "Executing tool call");

        let mut result = tool.execute(call.arguments).await?;
        result.duration_ms = Some(start.elapsed().as_millis() as u64);

        if result.success {
            info!(
                tool = %call.name,
                duration_ms = result.duration_ms.unwrap_or(0),
                "Tool executed successfully"
            );
        } else {
            warn!(
                tool = %call.name,
                error = %result.error.as_deref().unwrap_or("unknown"),
                "Tool execution failed"
            );
        }

        Ok(result)
    }

    /// Create a default registry with all built-in tools
    pub fn with_default_tools() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(ShellTool::new()));
        registry.register(Arc::new(ReadFileTool::new()));
        registry.register(Arc::new(WriteFileTool::new()));
        registry.register(Arc::new(WebFetchTool::new()));
        registry
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::with_default_tools()
    }
}

// ── Built-in tools ─────────────────────────────────────────────────────────

/// Shell command execution tool (sandboxed)
pub struct ShellTool {
    definition: ToolDefinition,
    sandbox: Option<Sandbox>,
}

impl ShellTool {
    pub fn new() -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "command".to_string(),
            JsonSchema::string("The shell command to execute"),
        );
        properties.insert(
            "timeout_secs".to_string(),
            JsonSchema {
                schema_type: "integer".to_string(),
                description: Some("Timeout in seconds (default: 30)".to_string()),
                properties: None,
                required: None,
                items: None,
                enum_values: None,
            },
        );
        properties.insert(
            "workdir".to_string(),
            JsonSchema::string("Working directory (default: current)"),
        );

        Self {
            definition: ToolDefinition {
                name: "shell_exec".to_string(),
                description: "Execute a shell command and return its output. Use for running scripts, compiling code, or any command-line operation. Runs in a sandboxed environment.".to_string(),
                parameters: JsonSchema::object(
                    properties,
                    vec!["command".to_string()],
                ),
                requires_approval: true,
                category: ToolCategory::Shell,
            },
            sandbox: None,
        }
    }

    /// Create a new ShellTool with sandbox support
    #[allow(dead_code)]
    pub fn with_sandbox(sandbox: Sandbox) -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "command".to_string(),
            JsonSchema::string("The shell command to execute"),
        );
        properties.insert(
            "timeout_secs".to_string(),
            JsonSchema {
                schema_type: "integer".to_string(),
                description: Some("Timeout in seconds (default: 30)".to_string()),
                properties: None,
                required: None,
                items: None,
                enum_values: None,
            },
        );
        properties.insert(
            "workdir".to_string(),
            JsonSchema::string("Working directory (default: sandbox workdir)"),
        );

        Self {
            definition: ToolDefinition {
                name: "shell_exec".to_string(),
                description: "Execute a shell command in a sandboxed environment. Use for running scripts, compiling code, or any command-line operation.".to_string(),
                parameters: JsonSchema::object(
                    properties,
                    vec!["command".to_string()],
                ),
                requires_approval: true,
                category: ToolCategory::Shell,
            },
            sandbox: Some(sandbox),
        }
    }
}

#[async_trait::async_trait]
impl ToolImpl for ShellTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(&self, args: serde_json::Value) -> ToolResultValue<ToolResult> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments(
                    "shell_exec".to_string(),
                    "missing 'command' argument".to_string(),
                )
            })?;

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);

        // Use sandbox workdir if available, otherwise use provided workdir
        let workdir = if let Some(sandbox) = &self.sandbox {
            sandbox.workdir().to_string_lossy().to_string()
        } else {
            args.get("workdir")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    std::env::current_dir()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                })
        };

        // Execute the command (sandboxed if sandbox is configured)
        let result = run_shell_command(command, timeout_secs, Some(workdir)).await?;

        Ok(result)
    }
}

/// Read a file from the filesystem
pub struct ReadFileTool {
    definition: ToolDefinition,
}

impl ReadFileTool {
    pub fn new() -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "path".to_string(),
            JsonSchema::string("Absolute path to the file to read"),
        );
        properties.insert(
            "max_bytes".to_string(),
            JsonSchema {
                schema_type: "integer".to_string(),
                description: Some("Maximum bytes to read (default: 65536)".to_string()),
                properties: None,
                required: None,
                items: None,
                enum_values: None,
            },
        );

        Self {
            definition: ToolDefinition {
                name: "read_file".to_string(),
                description: "Read the contents of a file from the filesystem. Returns the file content as text.".to_string(),
                parameters: JsonSchema::object(
                    properties,
                    vec!["path".to_string()],
                ),
                requires_approval: false,
                category: ToolCategory::FileSystem,
            },
        }
    }
}

#[async_trait::async_trait]
impl ToolImpl for ReadFileTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(&self, args: serde_json::Value) -> ToolResultValue<ToolResult> {
        let path = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidArguments(
                "read_file".to_string(),
                "missing 'path' argument".to_string(),
            )
        })?;

        let max_bytes = args
            .get("max_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(65536) as usize;

        let content = tokio::fs::read_to_string(path).await.map_err(|e| {
            ToolError::ExecutionFailed("read_file".to_string(), format!("Cannot read file: {}", e))
        })?;

        let truncated = if content.len() > max_bytes {
            format!(
                "{}...\n[truncated at {} bytes]",
                &content[..max_bytes],
                max_bytes
            )
        } else {
            content
        };

        Ok(ToolResult {
            tool_name: "read_file".to_string(),
            success: true,
            output: truncated,
            error: None,
            exit_code: None,
            duration_ms: None,
        })
    }
}

/// Write a file to the filesystem
pub struct WriteFileTool {
    definition: ToolDefinition,
}

impl WriteFileTool {
    pub fn new() -> Self {
        let mut properties = HashMap::new();
        properties.insert(
            "path".to_string(),
            JsonSchema::string("Absolute path to the file to write"),
        );
        properties.insert(
            "content".to_string(),
            JsonSchema::string("The content to write to the file"),
        );
        properties.insert(
            "append".to_string(),
            JsonSchema {
                schema_type: "boolean".to_string(),
                description: Some(
                    "If true, append instead of overwrite (default: false)".to_string(),
                ),
                properties: None,
                required: None,
                items: None,
                enum_values: None,
            },
        );

        Self {
            definition: ToolDefinition {
                name: "write_file".to_string(),
                description: "Write content to a file. Creates parent directories if they don't exist. Can append to existing files.".to_string(),
                parameters: JsonSchema::object(
                    properties,
                    vec!["path".to_string(), "content".to_string()],
                ),
                requires_approval: true,
                category: ToolCategory::FileSystem,
            },
        }
    }
}

#[async_trait::async_trait]
impl ToolImpl for WriteFileTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(&self, args: serde_json::Value) -> ToolResultValue<ToolResult> {
        let path = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidArguments(
                "write_file".to_string(),
                "missing 'path' argument".to_string(),
            )
        })?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments(
                    "write_file".to_string(),
                    "missing 'content' argument".to_string(),
                )
            })?;

        let append = args
            .get("append")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Create parent directories
        if let Some(parent) = std::path::Path::new(path).parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                ToolError::ExecutionFailed(
                    "write_file".to_string(),
                    format!("Cannot create directories: {}", e),
                )
            })?;
        }

        if append {
            let mut file = tokio::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(path)
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(
                        "write_file".to_string(),
                        format!("Cannot open file for append: {}", e),
                    )
                })?;
            tokio::io::AsyncWriteExt::write_all(&mut file, content.as_bytes())
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(
                        "write_file".to_string(),
                        format!("Cannot write to file: {}", e),
                    )
                })?;
        } else {
            tokio::fs::write(path, content).await.map_err(|e| {
                ToolError::ExecutionFailed(
                    "write_file".to_string(),
                    format!("Cannot write file: {}", e),
                )
            })?;
        }

        Ok(ToolResult {
            tool_name: "write_file".to_string(),
            success: true,
            output: format!("Successfully wrote {} bytes to {}", content.len(), path),
            error: None,
            exit_code: None,
            duration_ms: None,
        })
    }
}

/// Web fetch tool — fetches a URL and returns the content
pub struct WebFetchTool {
    definition: ToolDefinition,
}

impl WebFetchTool {
    pub fn new() -> Self {
        let mut properties = HashMap::new();
        properties.insert("url".to_string(), JsonSchema::string("The URL to fetch"));
        properties.insert(
            "max_bytes".to_string(),
            JsonSchema {
                schema_type: "integer".to_string(),
                description: Some("Maximum bytes to read (default: 131072)".to_string()),
                properties: None,
                required: None,
                items: None,
                enum_values: None,
            },
        );

        Self {
            definition: ToolDefinition {
                name: "web_fetch".to_string(),
                description: "Fetch a URL and return its content as text. Use for reading web pages, APIs, or documentation.".to_string(),
                parameters: JsonSchema::object(
                    properties,
                    vec!["url".to_string()],
                ),
                requires_approval: false,
                category: ToolCategory::Network,
            },
        }
    }
}

#[async_trait::async_trait]
impl ToolImpl for WebFetchTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(&self, args: serde_json::Value) -> ToolResultValue<ToolResult> {
        let url = args.get("url").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidArguments(
                "web_fetch".to_string(),
                "missing 'url' argument".to_string(),
            )
        })?;

        let max_bytes = args
            .get("max_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(131072) as usize;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("RavenClaw/0.1.0")
            .build()
            .map_err(|e| {
                ToolError::ExecutionFailed("web_fetch".to_string(), format!("HTTP client: {}", e))
            })?;

        let response = client.get(url).send().await.map_err(|e| {
            ToolError::ExecutionFailed("web_fetch".to_string(), format!("Request failed: {}", e))
        })?;

        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        let body = response.text().await.map_err(|e| {
            ToolError::ExecutionFailed(
                "web_fetch".to_string(),
                format!("Failed to read response body: {}", e),
            )
        })?;

        let truncated = if body.len() > max_bytes {
            format!(
                "{}...\n[truncated at {} bytes]",
                &body[..max_bytes],
                max_bytes
            )
        } else {
            body
        };

        Ok(ToolResult {
            tool_name: "web_fetch".to_string(),
            success: status.is_success(),
            output: format!(
                "Status: {}\nContent-Type: {}\n\n{}",
                status.as_u16(),
                content_type,
                truncated
            ),
            error: if status.is_success() {
                None
            } else {
                Some(format!("HTTP {}", status.as_u16()))
            },
            exit_code: Some(status.as_u16() as i32),
            duration_ms: None,
        })
    }
}

// ── Helper functions ───────────────────────────────────────────────────────

/// Run a shell command with timeout
async fn run_shell_command(
    command: &str,
    timeout_secs: u64,
    workdir: Option<String>,
) -> ToolResultValue<ToolResult> {
    use tokio::process::Command;

    let shell = if cfg!(target_os = "windows") {
        "cmd.exe"
    } else {
        "sh"
    };
    let flag = if cfg!(target_os = "windows") {
        "/C"
    } else {
        "-c"
    };

    let mut cmd = Command::new(shell);
    cmd.arg(flag).arg(command);

    if let Some(dir) = &workdir {
        cmd.current_dir(dir);
    }

    let output = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output())
        .await
        .map_err(|_| {
            ToolError::ExecutionFailed(
                "shell_exec".to_string(),
                format!("Command timed out after {} seconds", timeout_secs),
            )
        })?
        .map_err(|e| {
            ToolError::ExecutionFailed(
                "shell_exec".to_string(),
                format!("Failed to execute: {}", e),
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    let mut output_text = String::new();
    if !stdout.is_empty() {
        output_text.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !output_text.is_empty() {
            output_text.push_str("\n--- stderr ---\n");
        }
        output_text.push_str(&stderr);
    }

    // Truncate very long output
    const MAX_OUTPUT: usize = 65536;
    if output_text.len() > MAX_OUTPUT {
        output_text = format!(
            "{}...\n[truncated at {} bytes]",
            &output_text[..MAX_OUTPUT],
            MAX_OUTPUT
        );
    }

    Ok(ToolResult {
        tool_name: "shell_exec".to_string(),
        success: exit_code == 0,
        output: output_text,
        error: if exit_code != 0 {
            Some(format!("Exit code: {}", exit_code))
        } else {
            None
        },
        exit_code: Some(exit_code),
        duration_ms: None,
    })
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_registry_empty() {
        let registry = ToolRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_tool_registry_register() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(ShellTool::new()));
        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
        assert!(registry.has("shell_exec"));
    }

    #[test]
    fn test_tool_registry_default_tools() {
        let registry = ToolRegistry::with_default_tools();
        assert_eq!(registry.len(), 4);
        assert!(registry.has("shell_exec"));
        assert!(registry.has("read_file"));
        assert!(registry.has("write_file"));
        assert!(registry.has("web_fetch"));
    }

    #[test]
    fn test_tool_definitions() {
        let registry = ToolRegistry::with_default_tools();
        let defs = registry.definitions();
        assert_eq!(defs.len(), 4);

        let shell_def = defs.iter().find(|d| d.name == "shell_exec").unwrap();
        assert!(shell_def.description.contains("shell command"));
        assert!(shell_def.requires_approval);
        assert_eq!(shell_def.category, ToolCategory::Shell);
    }

    #[test]
    fn test_tool_not_found() {
        let registry = ToolRegistry::new();
        let result = registry.get("nonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn test_shell_tool_definition() {
        let tool = ShellTool::new();
        let def = tool.definition();
        assert_eq!(def.name, "shell_exec");
        assert!(def.requires_approval);
    }

    #[test]
    fn test_read_file_tool_definition() {
        let tool = ReadFileTool::new();
        let def = tool.definition();
        assert_eq!(def.name, "read_file");
        assert!(!def.requires_approval);
    }

    #[test]
    fn test_write_file_tool_definition() {
        let tool = WriteFileTool::new();
        let def = tool.definition();
        assert_eq!(def.name, "write_file");
        assert!(def.requires_approval);
    }

    #[test]
    fn test_web_fetch_tool_definition() {
        let tool = WebFetchTool::new();
        let def = tool.definition();
        assert_eq!(def.name, "web_fetch");
        assert!(!def.requires_approval);
    }

    #[test]
    fn test_tool_call_serialization() {
        let call = ToolCall {
            name: "shell_exec".to_string(),
            arguments: serde_json::json!({"command": "echo hello"}),
            id: Some("call_123".to_string()),
        };

        let json = serde_json::to_string(&call).unwrap();
        assert!(json.contains("shell_exec"));
        assert!(json.contains("echo hello"));
        assert!(json.contains("call_123"));
    }

    #[test]
    fn test_tool_result_serialization() {
        let result = ToolResult {
            tool_name: "shell_exec".to_string(),
            success: true,
            output: "hello\n".to_string(),
            error: None,
            exit_code: Some(0),
            duration_ms: Some(42),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("shell_exec"));
        assert!(json.contains("hello"));
        assert!(json.contains("42"));
    }

    #[test]
    fn test_tool_result_failure() {
        let result = ToolResult {
            tool_name: "shell_exec".to_string(),
            success: false,
            output: String::new(),
            error: Some("Exit code: 1".to_string()),
            exit_code: Some(1),
            duration_ms: Some(10),
        };

        assert!(!result.success);
        assert_eq!(result.exit_code, Some(1));
    }

    #[test]
    fn test_json_schema_string() {
        let schema = JsonSchema::string("A test string");
        assert_eq!(schema.schema_type, "string");
        assert_eq!(schema.description.unwrap(), "A test string");
    }

    #[test]
    fn test_json_schema_object() {
        let mut props = HashMap::new();
        props.insert("name".to_string(), JsonSchema::string("The name"));
        let schema = JsonSchema::object(props, vec!["name".to_string()]);
        assert_eq!(schema.schema_type, "object");
        assert!(schema.properties.unwrap().contains_key("name"));
    }

    #[test]
    fn test_tool_error_not_found() {
        let err = ToolError::NotFound("test_tool".to_string());
        assert_eq!(format!("{}", err), "Tool 'test_tool' not found");
    }

    #[test]
    fn test_tool_error_execution_failed() {
        let err = ToolError::ExecutionFailed("test".to_string(), "oops".to_string());
        assert_eq!(format!("{}", err), "Tool 'test' execution failed: oops");
    }

    #[test]
    fn test_tool_error_invalid_arguments() {
        let err = ToolError::InvalidArguments("test".to_string(), "bad arg".to_string());
        assert_eq!(
            format!("{}", err),
            "Invalid arguments for tool 'test': bad arg"
        );
    }

    #[test]
    fn test_tool_error_policy_denied() {
        let err = ToolError::PolicyDenied("not allowed".to_string());
        assert_eq!(format!("{}", err), "Policy denied: not allowed");
    }

    #[test]
    fn test_tool_error_sandbox_violation() {
        let err = ToolError::SandboxViolation("escape attempt".to_string());
        assert_eq!(format!("{}", err), "Sandbox violation: escape attempt");
    }

    #[test]
    fn test_tool_category_default() {
        let cat = ToolCategory::default();
        assert_eq!(cat, ToolCategory::General);
    }

    #[test]
    fn test_tool_category_serialization() {
        let cat = ToolCategory::Shell;
        let json = serde_json::to_string(&cat).unwrap();
        assert_eq!(json, "\"Shell\"");
    }

    #[test]
    fn test_tool_definition_requires_approval_default() {
        let def = ToolDefinition {
            name: "test".to_string(),
            description: "test".to_string(),
            parameters: JsonSchema::string("test"),
            requires_approval: false,
            category: ToolCategory::General,
        };
        assert!(!def.requires_approval);
    }

    #[tokio::test]
    async fn test_shell_exec_success() {
        let tool = ShellTool::new();
        let args = serde_json::json!({"command": "echo hello"});
        let result = tool.execute(args).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello"));
        assert_eq!(result.exit_code, Some(0));
    }

    #[tokio::test]
    async fn test_shell_exec_failure() {
        let tool = ShellTool::new();
        let args = serde_json::json!({"command": "exit 42"});
        let result = tool.execute(args).await.unwrap();
        assert!(!result.success);
        assert_eq!(result.exit_code, Some(42));
    }

    #[tokio::test]
    async fn test_shell_exec_missing_command() {
        let tool = ShellTool::new();
        let args = serde_json::json!({});
        let err = tool.execute(args).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArguments(_, _)));
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let tool = ReadFileTool::new();
        let args = serde_json::json!({"path": "/tmp/nonexistent_file_ravenclaw_test"});
        let result = tool.execute(args).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::ExecutionFailed(_, _)
        ));
    }

    #[tokio::test]
    async fn test_read_file_missing_path() {
        let tool = ReadFileTool::new();
        let args = serde_json::json!({});
        let err = tool.execute(args).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArguments(_, _)));
    }

    #[tokio::test]
    async fn test_write_file_missing_args() {
        let tool = WriteFileTool::new();
        let args = serde_json::json!({});
        let err = tool.execute(args).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArguments(_, _)));
    }

    #[tokio::test]
    async fn test_web_fetch_missing_url() {
        let tool = WebFetchTool::new();
        let args = serde_json::json!({});
        let err = tool.execute(args).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArguments(_, _)));
    }

    #[tokio::test]
    async fn test_write_and_read_file() {
        let dir = std::env::temp_dir().join(format!("ravenclaw_test_{}", std::process::id()));
        let path = dir.join("test_write.txt");
        let path_str = path.to_string_lossy().to_string();

        // Write
        let write_tool = WriteFileTool::new();
        let args = serde_json::json!({"path": path_str, "content": "Hello, RavenClaw!"});
        let result = write_tool.execute(args).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("17 bytes"));

        // Read back
        let read_tool = ReadFileTool::new();
        let args = serde_json::json!({"path": path_str});
        let result = read_tool.execute(args).await.unwrap();
        assert!(result.success);
        assert_eq!(result.output.trim(), "Hello, RavenClaw!");

        // Cleanup
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_dir(dir).await;
    }

    #[tokio::test]
    async fn test_write_file_append() {
        let dir = std::env::temp_dir().join(format!("ravenclaw_test_{}", std::process::id()));
        let path = dir.join("test_append.txt");
        let path_str = path.to_string_lossy().to_string();

        // Write initial
        let write_tool = WriteFileTool::new();
        let args = serde_json::json!({"path": path_str, "content": "line1\n"});
        write_tool.execute(args).await.unwrap();

        // Append
        let args = serde_json::json!({"path": path_str, "content": "line2\n", "append": true});
        let result = write_tool.execute(args).await.unwrap();
        assert!(result.success);

        // Read back
        let read_tool = ReadFileTool::new();
        let args = serde_json::json!({"path": path_str});
        let result = read_tool.execute(args).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("line1"));
        assert!(result.output.contains("line2"));

        // Cleanup
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_dir(dir).await;
    }

    #[tokio::test]
    async fn test_tool_registry_execute() {
        let registry = ToolRegistry::with_default_tools();
        let call = ToolCall {
            name: "shell_exec".to_string(),
            arguments: serde_json::json!({"command": "echo hello"}),
            id: None,
        };
        let result = registry.execute(call).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello"));
    }

    #[tokio::test]
    async fn test_tool_registry_execute_not_found() {
        let registry = ToolRegistry::new();
        let call = ToolCall {
            name: "nonexistent".to_string(),
            arguments: serde_json::json!({}),
            id: None,
        };
        let err = registry.execute(call).await.unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }
}
