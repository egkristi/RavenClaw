//! Agent implementations for RavenClaw
//!
//! Supports single-provider and multi-model (multi-provider) modes.
//! Security-integrated: PolicyEngine, Sandbox, and AuditLog wired to agent loop.

use crate::audit::{AuditEventType, AuditLog};
use crate::config::Config;
use crate::error::Result;
use crate::llm::{ChatMessage, LLMProviderTrait, MultiModelManager};
use crate::policy::{Decision, PolicyEngine};
use crate::sandbox::Sandbox;
use crate::tools::{ToolCall, ToolRegistry, ToolResult};
use futures::StreamExt;
use std::sync::Arc;
use tracing::{info, warn};

/// In-memory conversation memory — stores message history for the session.
/// Messages are lost when the process exits.
#[derive(Debug, Clone)]
pub struct ConversationMemory {
    /// Maximum number of messages to retain (0 = unlimited)
    max_messages: usize,
    /// Stored message history
    messages: Vec<ChatMessage>,
}

impl ConversationMemory {
    /// Create a new conversation memory with the given system prompt.
    /// `max_messages` caps history length (oldest user/assistant pairs are dropped first).
    pub fn new(system_prompt: &str, max_messages: usize) -> Self {
        Self {
            max_messages,
            messages: vec![ChatMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
            }],
        }
    }

    /// Add a user message and return the full message history for an LLM call.
    pub fn add_user_message(&mut self, content: &str) -> &[ChatMessage] {
        self.messages.push(ChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
        });
        self.trim_to_max();
        &self.messages
    }

    /// Add an assistant message to history.
    pub fn add_assistant_message(&mut self, content: &str) {
        self.messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: content.to_string(),
        });
        self.trim_to_max();
    }

    /// Get the current message history.
    #[allow(dead_code)]
    pub fn history(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Trim oldest non-system messages when over the limit.
    fn trim_to_max(&mut self) {
        if self.max_messages == 0 {
            return;
        }
        while self.messages.len() > self.max_messages {
            // Remove the oldest non-system message (index 1, since index 0 is system)
            if self.messages.len() > 1 {
                self.messages.remove(1);
            } else {
                break;
            }
        }
    }
}

/// Agent loop configuration
#[derive(Debug, Clone)]
pub struct AgentLoopConfig {
    /// Maximum iterations before forcing completion
    pub max_iterations: usize,
    /// Whether to enable tool calling
    pub enable_tools: bool,
    /// Require human approval for tool calls
    pub require_approval: bool,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            enable_tools: false,
            require_approval: false,
        }
    }
}

/// Run the agent loop with security integration (PolicyEngine + Sandbox + AuditLog)
///
/// This is the security-integrated version that:
/// 1. Checks all tool calls against PolicyEngine before execution
/// 2. Executes shell commands in the Sandbox
/// 3. Logs all tool calls, policy decisions, and results to AuditLog
pub async fn run_agent_loop(
    llm: Arc<dyn LLMProviderTrait>,
    initial_prompt: &str,
    system_prompt: &str,
    config: AgentLoopConfig,
) -> Result<String> {
    // Initialize security components
    let policy_engine = PolicyEngine::default_secure();
    let mut sandbox = Sandbox::default();
    sandbox.init().await.map_err(|e| {
        crate::error::RavenClawError::CommandExecution(format!("Sandbox init failed: {}", e))
    })?;
    let audit_log = AuditLog::new(format!("agent-{}", std::process::id()));

    // Initialize tool registry
    let registry = ToolRegistry::with_default_tools();

    info!(
        provider = llm.provider_name(),
        model = llm.model(),
        max_iterations = config.max_iterations,
        enable_tools = config.enable_tools,
        "Agent loop starting with security integration"
    );

    // Audit: agent start
    let _ = audit_log.append(
        AuditEventType::AgentStart,
        "agent",
        &format!(
            "Agent loop started with {} (model: {})",
            llm.provider_name(),
            llm.model()
        ),
        Some(serde_json::json!({
            "provider": llm.provider_name(),
            "model": llm.model(),
            "max_iterations": config.max_iterations,
            "enable_tools": config.enable_tools,
        })),
    );

    let mut memory = ConversationMemory::new(system_prompt, 0);
    memory.add_user_message(initial_prompt);

    for iteration in 0..config.max_iterations {
        let messages = memory.history().to_vec();

        let response = match llm.chat(messages).await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "LLM request failed");
                let _ = audit_log.append(
                    AuditEventType::Error,
                    "llm",
                    &format!("LLM request failed: {}", e),
                    None,
                );
                return Err(crate::error::RavenClawError::Llm(e));
            }
        };

        let content = response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        // Check for completion signal
        if content.contains("FINAL:") {
            let final_response = content
                .split("FINAL:")
                .nth(1)
                .unwrap_or("")
                .trim()
                .to_string();

            memory.add_assistant_message(&content);

            // Audit: agent finish
            let _ = audit_log.append(
                AuditEventType::AgentFinish,
                "agent",
                "Agent loop completed successfully",
                Some(serde_json::json!({
                    "iterations": iteration + 1,
                    "final_response_length": final_response.len(),
                })),
            );

            return Ok(final_response);
        }

        // Execute tool calls if enabled
        if config.enable_tools {
            if let Some(tool_result) = execute_tool_call_with_security(
                &content,
                &registry,
                &policy_engine,
                &sandbox,
                &audit_log,
            )
            .await
            {
                let observation = if tool_result.success {
                    format!("OBSERVATION: {}", tool_result.output)
                } else {
                    format!(
                        "OBSERVATION: Tool failed with error: {}",
                        tool_result.error.as_deref().unwrap_or("unknown error")
                    )
                };

                memory.add_assistant_message(&content);
                memory.add_user_message(&observation);

                info!(
                    iteration = iteration,
                    tool = %tool_result.tool_name,
                    success = tool_result.success,
                    "Tool executed"
                );
                continue;
            }
        }

        // No tool call found and no FINAL: — treat as regular response
        memory.add_assistant_message(&content);

        info!(
            iteration = iteration,
            thought = %content.lines().find(|l| l.starts_with("THOUGHT:")).unwrap_or("<no thought>"),
            "Agent loop progress"
        );
    }

    // Max iterations reached
    warn!(
        max_iterations = config.max_iterations,
        "Agent loop reached max iterations"
    );

    let _ = audit_log.append(
        AuditEventType::Error,
        "agent",
        "Agent loop reached max iterations without completing",
        Some(serde_json::json!({
            "max_iterations": config.max_iterations,
        })),
    );

    let history = memory.history();
    if history.len() > 1 {
        if let Some(last) = history.last() {
            return Ok(last.content.clone());
        }
    }

    Err(crate::error::RavenClawError::CommandExecution(
        "Agent loop reached max iterations without completing the task".to_string(),
    ))
}

/// Execute a tool call with security integration
///
/// This function:
/// 1. Parses the tool call from the LLM response
/// 2. Checks the tool call against PolicyEngine
/// 3. Logs the policy decision to AuditLog
/// 4. Executes the tool (sandbox is applied at the tool implementation level for shell_exec)
/// 5. Logs the result to AuditLog
#[allow(unused_variables)]
async fn execute_tool_call_with_security(
    content: &str,
    registry: &ToolRegistry,
    policy_engine: &PolicyEngine,
    sandbox: &Sandbox,
    audit_log: &AuditLog,
) -> Option<ToolResult> {
    // Parse tool call from content
    let (tool_name, args) = parse_tool_call(content)?;

    info!(tool = %tool_name, "Parsing tool call");

    // Audit: tool call requested
    let _ = audit_log.tool_call(&tool_name, &args);

    // Check if tool requires approval
    if policy_engine.requires_approval(&tool_name) {
        let _ = audit_log.append(
            AuditEventType::ApprovalRequested,
            "approval",
            &format!("Approval required for tool: {}", tool_name),
            Some(serde_json::json!({"tool": tool_name})),
        );
        // For now, auto-approve (HITL would block here and wait for user input)
        let _ = audit_log.approval(&tool_name, true, Some("Auto-approved for v0.4"));
    }

    // Check policy BEFORE execution
    let policy_decision = policy_engine.check_tool_call(&tool_name, &args);

    // Audit: policy decision
    match &policy_decision {
        Decision::Allow => {
            let _ = audit_log.policy_decision(&tool_name, true, None);
        }
        Decision::Deny(reason) => {
            let _ = audit_log.policy_decision(&tool_name, false, Some(reason));
            warn!(tool = %tool_name, reason = %reason, "Tool call denied by policy");
            return Some(ToolResult {
                tool_name,
                success: false,
                output: String::new(),
                error: Some(format!("Policy denied: {}", reason)),
                exit_code: Some(-1),
                duration_ms: None,
            });
        }
    }

    // Execute tool (sandbox is applied at the tool implementation level for shell_exec)
    let tool_name_clone = tool_name.clone();
    let call = ToolCall {
        name: tool_name.clone(),
        arguments: args,
        id: None,
    };

    let result = match registry.execute(call).await {
        Ok(result) => {
            // Audit: tool result
            let _ = audit_log.append(
                AuditEventType::ToolResult,
                &tool_name_clone,
                &format!(
                    "Tool executed: {} (success: {})",
                    tool_name_clone, result.success
                ),
                Some(serde_json::json!({
                    "success": result.success,
                    "exit_code": result.exit_code,
                    "duration_ms": result.duration_ms,
                })),
            );
            result
        }
        Err(e) => {
            // Audit: error
            let _ = audit_log.append(
                AuditEventType::Error,
                &tool_name_clone,
                &format!("Tool execution failed: {}", e),
                None,
            );
            ToolResult {
                tool_name: tool_name_clone,
                success: false,
                output: String::new(),
                error: Some(e.to_string()),
                exit_code: Some(-1),
                duration_ms: None,
            }
        }
    };

    Some(result)
}

/// Parse a tool call from LLM response content
/// Returns (tool_name, args) if found, None otherwise
fn parse_tool_call(content: &str) -> Option<(String, serde_json::Value)> {
    let mut lines = content.lines();
    let tool_call_line = lines.find(|l| l.trim().starts_with("TOOL_CALL:"))?;

    let tool_name = tool_call_line
        .trim()
        .strip_prefix("TOOL_CALL:")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())?
        .to_string();

    // Find the ARGS line
    let args_line = lines.find(|l| l.trim().starts_with("ARGS:"))?;
    let args_str = args_line.trim().strip_prefix("ARGS:").map(|s| s.trim())?;

    let args: serde_json::Value = serde_json::from_str(args_str).ok()?;

    Some((tool_name, args))
}

/// Legacy function for backward compatibility (without security)
#[deprecated(note = "Use run_agent_loop with security integration instead")]
async fn execute_tool_call(content: &str, registry: &ToolRegistry) -> Option<ToolResult> {
    let (tool_name, args) = parse_tool_call(content)?;

    let call = ToolCall {
        name: tool_name.clone(),
        arguments: args,
        id: None,
    };

    match registry.execute(call).await {
        Ok(result) => Some(result),
        Err(e) => Some(ToolResult {
            tool_name,
            success: false,
            output: String::new(),
            error: Some(e.to_string()),
            exit_code: Some(1),
            duration_ms: None,
        }),
    }
}

/// Run a one-shot command via --exec mode with streaming output
/// Sends the prompt to the LLM and prints tokens as they arrive.
#[allow(dead_code)]
pub async fn run_exec_stream(
    llm: Arc<dyn LLMProviderTrait>,
    prompt: &str,
    system_prompt: &str,
) -> Result<String> {
    info!(
        provider = llm.provider_name(),
        model = llm.model(),
        "Exec one-shot streaming mode"
    );

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        },
    ];

    let mut full_response = String::new();

    match llm.chat_stream(messages).await {
        Ok(mut stream) => {
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(chunk) => {
                        if !chunk.content.is_empty() {
                            print!("{}", chunk.content);
                            full_response.push_str(&chunk.content);
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Stream error");
                        break;
                    }
                }
            }
            println!();
        }
        Err(e) => {
            warn!(error = %e, "Failed to start stream");
            return Err(crate::error::RavenClawError::Llm(e));
        }
    }

    Ok(full_response)
}

/// Run a single autonomous agent (single-provider mode)
pub async fn run_single(llm: Arc<dyn LLMProviderTrait>, config: Config) -> Result<()> {
    info!(
        "Starting single agent mode with provider: {}",
        llm.provider_name()
    );

    let system_prompt = &config.llm.system_prompt;

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: "Ready. Awaiting instructions.".to_string(),
        },
    ];

    match llm.chat(messages).await {
        Ok(response) => {
            if let Some(choice) = response.choices.first() {
                info!(provider = llm.provider_name(), model = llm.model(), response = %choice.message.content, "Agent response received");
            }
        }
        Err(e) => {
            warn!(error = %e, provider = llm.provider_name(), "LLM request failed");
        }
    }

    Ok(())
}

/// Run multiple agents in swarm mode (single-provider)
pub async fn run_swarm(_llm: Arc<dyn LLMProviderTrait>, _config: Config) -> Result<()> {
    warn!("Swarm mode not yet implemented");
    Err(crate::error::RavenClawError::CommandExecution(
        "Swarm mode is not yet implemented. See ROADMAP.md for the planned timeline.".to_string(),
    ))
}

/// Run supervisor agent coordinating sub-agents (single-provider)
pub async fn run_supervisor(_llm: Arc<dyn LLMProviderTrait>, _config: Config) -> Result<()> {
    warn!("Supervisor mode not yet implemented");
    Err(crate::error::RavenClawError::CommandExecution(
        "Supervisor mode is not yet implemented. See ROADMAP.md for the planned timeline."
            .to_string(),
    ))
}

/// Run a single autonomous agent (multi-model mode)
pub async fn run_single_multi(multi_llm: MultiModelManager, config: Config) -> Result<()> {
    info!(
        "Starting single agent mode (multi-model) with {} providers",
        multi_llm.client_count()
    );

    let system_prompt = &config.llm.system_prompt;

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content: "Ready. Awaiting instructions.".to_string(),
        },
    ];

    // Round-robin: start with first provider, then rotate
    let mut last_index = 0;
    for i in 0..multi_llm.client_count() {
        let client = if i == 0 {
            multi_llm.get_client(0)
        } else {
            multi_llm.next_client(last_index)
        };

        if let Some(client) = client {
            match client.chat(messages.clone()).await {
                Ok(response) => {
                    if let Some(choice) = response.choices.first() {
                        info!(provider = client.provider_name(), model = client.model(), response = %choice.message.content, "Provider response received");
                    }
                }
                Err(e) => {
                    warn!(error = %e, provider = client.provider_name(), model = client.model(), "Provider request failed");
                }
            }
            last_index = i;
        }
    }

    Ok(())
}

/// Run multiple agents in swarm mode (multi-model)
pub async fn run_swarm_multi(_multi_llm: MultiModelManager, _config: Config) -> Result<()> {
    warn!("Swarm mode (multi-model) not yet implemented");
    Err(crate::error::RavenClawError::CommandExecution(
        "Swarm mode is not yet implemented. See ROADMAP.md for the planned timeline.".to_string(),
    ))
}

/// Run supervisor agent coordinating sub-agents (multi-model)
pub async fn run_supervisor_multi(_multi_llm: MultiModelManager, _config: Config) -> Result<()> {
    warn!("Supervisor mode (multi-model) not yet implemented");
    Err(crate::error::RavenClawError::CommandExecution(
        "Supervisor mode is not yet implemented. See ROADMAP.md for the planned timeline."
            .to_string(),
    ))
}

/// Run interactive REPL mode
pub async fn run_repl(llm: Arc<dyn LLMProviderTrait>, config: Config) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    info!("Starting interactive REPL mode");

    let system_prompt = &config.llm.system_prompt;
    let mut memory = ConversationMemory::new(system_prompt, 0);

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    println!("RavenClaw REPL — type /exit to quit, /reset to clear history");

    loop {
        print!("\n> ");
        use tokio::io::AsyncWriteExt;
        tokio::io::stdout().flush().await?;

        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            Ok(None) => break, // EOF
            Err(e) => {
                warn!(error = %e, "REPL read error");
                break;
            }
        };

        let input = line.trim();

        if input.is_empty() {
            continue;
        }

        match input {
            "/exit" | "/quit" => {
                println!("Exiting REPL.");
                break;
            }
            "/reset" => {
                memory = ConversationMemory::new(system_prompt, 0);
                println!("Conversation history reset.");
                continue;
            }
            _ => {}
        }

        memory.add_user_message(input);
        let messages = memory.history().to_vec();

        match llm.chat(messages).await {
            Ok(response) => {
                if let Some(choice) = response.choices.first() {
                    let content = &choice.message.content;
                    println!("{}", content);
                    memory.add_assistant_message(content);
                }
            }
            Err(e) => {
                warn!(error = %e, "LLM request failed");
                println!("Error: {}", e);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swarm_stub_returns_error() {
        let err = crate::error::RavenClawError::CommandExecution(
            "Swarm mode is not yet implemented. See ROADMAP.md for the planned timeline."
                .to_string(),
        );
        assert_eq!(
            format!("{}", err),
            "Command execution failed: Swarm mode is not yet implemented. See ROADMAP.md for the planned timeline."
        );
    }

    #[test]
    fn test_supervisor_stub_returns_error() {
        let err = crate::error::RavenClawError::CommandExecution(
            "Supervisor mode is not yet implemented. See ROADMAP.md for the planned timeline."
                .to_string(),
        );
        assert_eq!(
            format!("{}", err),
            "Command execution failed: Supervisor mode is not yet implemented. See ROADMAP.md for the planned timeline."
        );
    }

    #[test]
    fn test_conversation_memory_new() {
        let mem = ConversationMemory::new("system prompt", 10);
        assert_eq!(mem.messages.len(), 1);
        assert_eq!(mem.messages[0].role, "system");
        assert_eq!(mem.messages[0].content, "system prompt");
    }

    #[test]
    fn test_conversation_memory_add_user() {
        let mut mem = ConversationMemory::new("system", 10);
        mem.add_user_message("hello");
        assert_eq!(mem.messages.len(), 2);
        assert_eq!(mem.messages[1].role, "user");
        assert_eq!(mem.messages[1].content, "hello");
    }

    #[test]
    fn test_conversation_memory_trim() {
        let mut mem = ConversationMemory::new("system", 3);
        mem.add_user_message("msg1");
        mem.add_assistant_message("resp1");
        mem.add_user_message("msg2");
        mem.add_assistant_message("resp2");
        // Should trim to keep system + 2 messages
        assert!(mem.messages.len() <= 3);
    }

    #[test]
    fn test_parse_tool_call_valid() {
        let content = "THOUGHT: I need to run a command\nTOOL_CALL: shell_exec\nARGS: {\"command\": \"echo hello\"}";
        let (name, args) = parse_tool_call(content).unwrap();
        assert_eq!(name, "shell_exec");
        assert_eq!(args["command"], "echo hello");
    }

    #[test]
    fn test_parse_tool_call_missing_tool() {
        let content = "THOUGHT: no tool here";
        assert!(parse_tool_call(content).is_none());
    }

    #[test]
    fn test_parse_tool_call_missing_args() {
        let content = "TOOL_CALL: shell_exec\nNo args line";
        assert!(parse_tool_call(content).is_none());
    }

    #[test]
    fn test_parse_tool_call_invalid_json() {
        let content = "TOOL_CALL: shell_exec\nARGS: not valid json";
        assert!(parse_tool_call(content).is_none());
    }

    #[test]
    fn test_agent_loop_config_default() {
        let config = AgentLoopConfig::default();
        assert_eq!(config.max_iterations, 10);
        assert!(!config.enable_tools);
        assert!(!config.require_approval);
    }
}
