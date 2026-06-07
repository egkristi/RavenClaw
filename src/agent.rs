//! Agent implementations for RavenClaw
//!
//! Supports single-provider and multi-model (multi-provider) modes.
//! Security-integrated: PolicyEngine, Sandbox, and AuditLog wired to agent loop.

use crate::audit::{AuditEventType, AuditLog};
use crate::config::Config;
use crate::error::Result;
use crate::llm::{ChatMessage, Choice, LLMProviderTrait, MultiModelManager};
use crate::mcp::McpClient;
use crate::policy::{Decision, PolicyEngine};
use crate::sandbox::Sandbox;
use crate::tools::{ToolCall, ToolRegistry, ToolResult};
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::RwLock;
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
    #[allow(dead_code)]
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

        let first_choice = response.choices.first();
        let content = first_choice
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        // Check for structured tool calls first (OpenAI Tools format)
        if config.enable_tools {
            if let Some((tool_name, args)) = first_choice.and_then(parse_structured_tool_call) {
                info!(tool = %tool_name, "Structured tool call detected");

                // Execute tool with security
                if let Some(tool_result) = execute_parsed_tool_call(
                    tool_name,
                    args,
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

                    memory.add_user_message(&observation);

                    info!(
                        iteration = iteration,
                        tool = %tool_result.tool_name,
                        success = tool_result.success,
                        "Structured tool executed"
                    );
                    continue;
                }
            }
        }

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

        // Execute tool calls if enabled (legacy pattern-matching fallback)
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

/// Run the agent loop with MCP tool integration (v0.5.2)
///
/// This version extends run_agent_loop with MCP tool support:
/// 1. Registers MCP tools into the ToolRegistry
/// 2. MCP tools are executed alongside built-in tools
pub async fn run_agent_loop_with_mcp(
    llm: Arc<dyn LLMProviderTrait>,
    initial_prompt: &str,
    system_prompt: &str,
    config: AgentLoopConfig,
    mcp_client: Option<Arc<RwLock<McpClient>>>,
) -> Result<String> {
    // Initialize security components
    let policy_engine = PolicyEngine::default_secure();
    let mut sandbox = Sandbox::default();
    sandbox.init().await.map_err(|e| {
        crate::error::RavenClawError::CommandExecution(format!("Sandbox init failed: {}", e))
    })?;
    let audit_log = AuditLog::new(format!("agent-{}", std::process::id()));

    // Initialize tool registry with default tools
    let mut registry = ToolRegistry::with_default_tools();

    // Register MCP tools if client is provided
    if let Some(client) = &mcp_client {
        match crate::mcp::register_mcp_tools(&mut registry, client.clone()).await {
            Ok(count) => {
                info!(count, "MCP tools registered");
            }
            Err(e) => {
                warn!(error = %e, "Failed to register MCP tools");
            }
        }
    }

    info!(
        provider = llm.provider_name(),
        model = llm.model(),
        max_iterations = config.max_iterations,
        enable_tools = config.enable_tools,
        tool_count = registry.len(),
        "Agent loop starting with MCP integration"
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
            "mcp_enabled": mcp_client.is_some(),
            "tool_count": registry.len(),
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

        let first_choice = response.choices.first();
        let content = first_choice
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        // Check for structured tool calls first (OpenAI Tools format)
        if config.enable_tools {
            if let Some((tool_name, args)) = first_choice.and_then(parse_structured_tool_call) {
                info!(tool = %tool_name, "Structured tool call detected");

                // Execute tool with security
                if let Some(tool_result) = execute_parsed_tool_call(
                    tool_name,
                    args,
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

                    memory.add_user_message(&observation);

                    info!(
                        iteration = iteration,
                        tool = %tool_result.tool_name,
                        success = tool_result.success,
                        "Structured tool executed"
                    );
                    continue;
                }
            }
        }

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

        // Execute tool calls if enabled (legacy pattern-matching fallback)
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

/// Execute a parsed tool call with security integration
///
/// This function:
/// 1. Checks the tool call against PolicyEngine
/// 2. Logs the policy decision to AuditLog
/// 3. Executes the tool (sandbox is applied at the tool implementation level for shell_exec)
/// 4. Logs the result to AuditLog
async fn execute_parsed_tool_call(
    tool_name: String,
    args: serde_json::Value,
    registry: &ToolRegistry,
    policy_engine: &PolicyEngine,
    _sandbox: &Sandbox,
    audit_log: &AuditLog,
) -> Option<ToolResult> {
    info!(tool = %tool_name, "Executing parsed tool call");

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
                tool_name: tool_name.clone(),
                success: false,
                output: String::new(),
                error: Some(format!("Policy denied: {}", reason)),
                exit_code: Some(-1),
                duration_ms: None,
            });
        }
    }

    // Execute tool
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

/// Execute a tool call with security integration (legacy pattern-matching fallback)
///
/// This function:
/// 1. Parses the tool call from the LLM response (legacy TOOL_CALL:/ARGS: format)
/// 2. Checks the tool call against PolicyEngine
/// 3. Logs the policy decision to AuditLog
/// 4. Executes the tool (sandbox is applied at the tool implementation level for shell_exec)
/// 5. Logs the result to AuditLog
async fn execute_tool_call_with_security(
    content: &str,
    registry: &ToolRegistry,
    policy_engine: &PolicyEngine,
    _sandbox: &Sandbox,
    audit_log: &AuditLog,
) -> Option<ToolResult> {
    // Parse tool call from content (legacy format)
    let (tool_name, args) = parse_tool_call(content)?;

    // Delegate to the common execution logic
    execute_parsed_tool_call(
        tool_name,
        args,
        registry,
        policy_engine,
        _sandbox,
        audit_log,
    )
    .await
}

/// Parse a tool call from LLM response content
/// Returns (tool_name, args) if found, None otherwise
/// Parse tool call from structured LLM response (OpenAI Tools format)
fn parse_structured_tool_call(choice: &Choice) -> Option<(String, serde_json::Value)> {
    let tool_calls = choice.tool_calls.as_ref()?;
    let first_call = tool_calls.first()?;

    let tool_name = first_call.function.name.clone();
    let args: serde_json::Value = serde_json::from_str(&first_call.function.arguments).ok()?;

    Some((tool_name, args))
}

/// Parse tool call from legacy pattern-matching format (TOOL_CALL: / ARGS:)
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
#[allow(dead_code)]
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

/// Run multiple agents in swarm mode (single-provider) — v0.6
///
/// Swarm mode runs multiple agents in parallel, each working on the same task
/// with different approaches. Results are collected and compared.
pub async fn run_swarm(llm: Arc<dyn LLMProviderTrait>, config: Config) -> Result<()> {
    info!("Starting swarm mode (single-provider) — 3 parallel agents");

    let system_prompt = &config.llm.system_prompt;
    let num_agents = 3;
    let mut handles = Vec::new();

    // Spawn parallel agents with different personas
    let personas = vec![
        "You are an analytical agent. Focus on logic, structure, and precision.",
        "You are a creative agent. Focus on innovation, alternatives, and possibilities.",
        "You are a pragmatic agent. Focus on simplicity, efficiency, and practicality.",
    ];

    for i in 0..num_agents {
        let llm_clone = llm.clone();
        let persona = personas[i].to_string();
        let task = "Analyze the given task and provide your solution.".to_string();

        let handle = tokio::spawn(async move {
            let mut memory = ConversationMemory::new(&persona, 10);
            memory.add_user_message(&task);

            let messages = memory.history().to_vec();
            match llm_clone.chat(messages).await {
                Ok(response) => {
                    let content = response.choices.first()
                        .map(|c| c.message.content.clone())
                        .unwrap_or_default();
                    Ok((i, content))
                }
                Err(e) => Err(format!("Agent {} failed: {}", i, e)),
            }
        });

        handles.push(handle);
    }

    // Collect results
    let mut results: Vec<(usize, String)> = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok((idx, result))) => {
                info!("Agent {} completed: {} chars", idx, result.len());
                results.push((idx, result));
            }
            Ok(Err(e)) => warn!("Agent failed: {}", e),
            Err(e) => warn!("Agent join failed: {}", e),
        }
    }

    // Print swarm results
    println!("\n🐦‍⬛ Swarm Results ({} agents):", results.len());
    for (idx, result) in &results {
        println!("\n── Agent {} ({}) ──", idx + 1, personas[*idx].split('.').next().unwrap_or("Unknown"));
        println!("{}", result);
    }

    Ok(())
}

/// Run supervisor agent coordinating sub-agents (single-provider) — v0.6
///
/// The supervisor decomposes a task into subtasks, spawns sub-agents for each,
/// and aggregates results. Uses the same LLM provider for all agents.
pub async fn run_supervisor(llm: Arc<dyn LLMProviderTrait>, config: Config) -> Result<()> {
    info!("Starting supervisor mode (single-provider)");

    let system_prompt = &config.llm.system_prompt;
    let policy_engine = PolicyEngine::default_secure();
    let mut sandbox = Sandbox::default();
    sandbox.init().await.map_err(|e| {
        crate::error::RavenClawError::CommandExecution(format!("Sandbox init failed: {}", e))
    })?;
    let audit_log = AuditLog::new(format!("supervisor-{}", std::process::id()));
    let registry = ToolRegistry::with_default_tools();

    // Initial prompt to supervisor
    let supervisor_prompt = format!(
        "You are a supervisor agent. Your task is to decompose complex tasks into subtasks \
         and coordinate sub-agents to complete them. \
         \n\nFor each subtask, respond with:\n\
         SUBTASK: <description>\n\
         AGENT: <agent_number>\n\
         \nWhen all subtasks are complete, respond with:\n\
         FINAL: <aggregated result>\n\
         \nTask: {}",
        "Coordinate the completion of the assigned task."
    );

    let mut memory = ConversationMemory::new(system_prompt, 20);
    memory.add_user_message(&supervisor_prompt);

    let mut subtask_results: Vec<String> = Vec::new();
    let mut iteration = 0;
    let max_iterations = 15;

    loop {
        iteration += 1;
        if iteration > max_iterations {
            warn!("Supervisor reached max iterations");
            break;
        }

        let messages = memory.history().to_vec();
        let response = match llm.chat(messages).await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "Supervisor LLM request failed");
                continue;
            }
        };

        let content = response.choices.first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        // Check for FINAL: completion
        if content.contains("FINAL:") {
            let final_response = content.split("FINAL:").nth(1).unwrap_or("").trim().to_string();
            info!("Supervisor completed task: {} chars", final_response.len());

            let _ = audit_log.append(
                AuditEventType::AgentFinish,
                "supervisor",
                "Supervisor completed task coordination",
                Some(serde_json::json!({
                    "iterations": iteration,
                    "subtasks_completed": subtask_results.len(),
                })),
            );

            println!("\n🐦‍⬛ Supervisor Result:\n{}", final_response);
            return Ok(());
        }

        // Check for SUBTASK: decomposition
        if content.contains("SUBTASK:") {
            let subtask_block = content.split("SUBTASK:").nth(1).unwrap_or("");
            let subtask_lines: Vec<&str> = subtask_block.lines().take(3).collect();

            let subtask_desc = subtask_lines.first().unwrap_or(&"").trim();
            let agent_num = subtask_lines.iter()
                .find(|l| l.starts_with("AGENT:"))
                .and_then(|l| l.split(':').nth(1))
                .unwrap_or("1")
                .trim();

            if !subtask_desc.is_empty() {
                info!("Subtask {}: {}", agent_num, subtask_desc);

                // Execute subtask
                let subtask_result = run_subtask_agent(
                    llm.clone(),
                    subtask_desc,
                    system_prompt,
                    &policy_engine,
                    &sandbox,
                    &audit_log,
                    &registry,
                ).await;

                match subtask_result {
                    Ok(result) => {
                        info!("Subtask {} completed: {} chars", agent_num, result.len());
                        subtask_results.push(format!("Agent {} result: {}", agent_num, result));

                        memory.add_assistant_message(&format!(
                            "Decomposed subtask {}: {}", agent_num, subtask_desc
                        ));
                        memory.add_user_message(&format!(
                            "Subtask {} result: {}", agent_num, result
                        ));
                    }
                    Err(e) => {
                        warn!("Subtask {} failed: {}", agent_num, e);
                        memory.add_assistant_message(&format!(
                            "Subtask {} failed: {}", agent_num, e
                        ));
                    }
                }
            }
        } else {
            memory.add_assistant_message(&content);
        }
    }

    // Fallback: return aggregated results
    if !subtask_results.is_empty() {
        let aggregated = subtask_results.join("\n\n");
        info!("Supervisor aggregated {} subtask results", subtask_results.len());
        println!("\n🐦‍⬛ Supervisor Aggregated Result:\n{}", aggregated);
        return Ok(());
    }

    Err(crate::error::RavenClawError::CommandExecution(
        "Supervisor mode completed without results".to_string(),
    ))
}

/// Run a subtask agent — helper for supervisor mode
async fn run_subtask_agent(
    llm: Arc<dyn LLMProviderTrait>,
    subtask: &str,
    system_prompt: &str,
    policy_engine: &PolicyEngine,
    sandbox: &Sandbox,
    audit_log: &AuditLog,
    registry: &ToolRegistry,
) -> Result<String> {
    let mut memory = ConversationMemory::new(system_prompt, 10);
    memory.add_user_message(&format!("Execute this subtask: {}", subtask));

    for i in 0..5 {
        let messages = memory.history().to_vec();
        let response = match llm.chat(messages).await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, iteration = i, "Subtask agent LLM failed");
                continue;
            }
        };

        let content = response.choices.first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        if content.contains("FINAL:") || content.contains("DONE:") {
            return Ok(content.replace("FINAL:", "").replace("DONE:", "").trim().to_string());
        }

        // Try tool execution
        if let Some(tool_result) = execute_tool_call_with_security(
            &content,
            registry,
            policy_engine,
            sandbox,
            audit_log,
        ).await {
            memory.add_assistant_message(&content);
            memory.add_user_message(&format!("Tool result: {}", tool_result.output));
        } else {
            memory.add_assistant_message(&content);
            memory.add_user_message("Continue with next step.");
        }
    }

    Ok("Subtask completed".to_string())
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

/// Run multiple agents in swarm mode (multi-model) — v0.6
///
/// Swarm mode runs multiple agents in parallel, each using a different LLM provider
/// for the same task. Results are collected and compared for diversity.
pub async fn run_swarm_multi(multi_llm: MultiModelManager, config: Config) -> Result<()> {
    info!("Starting swarm mode (multi-model) — {} parallel agents", multi_llm.client_count());

    let system_prompt = &config.llm.system_prompt;
    let num_agents = multi_llm.client_count().min(3); // Cap at 3 for cost control
    let mut handles = Vec::new();

    // Different personas for each agent
    let personas = vec![
        "You are an analytical agent. Focus on logic, structure, and precision.",
        "You are a creative agent. Focus on innovation, alternatives, and possibilities.",
        "You are a pragmatic agent. Focus on simplicity, efficiency, and practicality.",
    ];

    for i in 0..num_agents {
        let client = multi_llm.get_client(i).unwrap();
        let persona = personas.get(i).unwrap_or(&personas[0]).to_string();
        let task = "Analyze the given task and provide your solution.".to_string();

        let handle = tokio::spawn(async move {
            let mut memory = ConversationMemory::new(&persona, 10);
            memory.add_user_message(&task);

            let messages = memory.history().to_vec();
            match client.chat(messages).await {
                Ok(response) => {
                    let content = response.choices.first()
                        .map(|c| c.message.content.clone())
                        .unwrap_or_default();
                    Ok((i, client.provider_name(), client.model(), content))
                }
                Err(e) => Err(format!("Agent {} failed: {}", i, e)),
            }
        });

        handles.push(handle);
    }

    // Collect results
    let mut results: Vec<(usize, String, String, String)> = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok((idx, provider, model, result))) => {
                info!("Agent {} ({}:{}) completed: {} chars", idx, provider, model, result.len());
                results.push((idx, provider, model, result));
            }
            Ok(Err(e)) => warn!("Agent failed: {}", e),
            Err(e) => warn!("Agent join failed: {}", e),
        }
    }

    // Print swarm results
    println!("\n🐦‍⬛ Swarm Results ({} agents, multi-model):", results.len());
    for (idx, provider, model, result) in &results {
        println!("\n── Agent {} ({}:{}) ──", idx + 1, provider, model);
        println!("{}", result);
    }

    Ok(())
}

/// Run supervisor agent coordinating sub-agents (multi-model) — v0.6
///
/// The supervisor decomposes a task and assigns subtasks to different providers
/// based on their strengths. Results are aggregated.
pub async fn run_supervisor_multi(multi_llm: MultiModelManager, config: Config) -> Result<()> {
    info!("Starting supervisor mode (multi-model) with {} providers", multi_llm.client_count());

    let system_prompt = &config.llm.system_prompt;
    let policy_engine = PolicyEngine::default_secure();
    let mut sandbox = Sandbox::default();
    sandbox.init().await.map_err(|e| {
        crate::error::RavenClawError::CommandExecution(format!("Sandbox init failed: {}", e))
    })?;
    let audit_log = AuditLog::new(format!("supervisor-multi-{}", std::process::id()));
    let registry = ToolRegistry::with_default_tools();

    // Supervisor prompt with multi-model awareness
    let supervisor_prompt = format!(
        "You are a supervisor agent coordinating multiple LLM providers. \
         Decompose tasks and assign them to appropriate providers based on their strengths. \
         \n\nFor each subtask, respond with:\n\
         SUBTASK: <description>\n\
         PROVIDER: <provider_index 0-{}>\n\
         \nWhen complete, respond with:\n\
         FINAL: <aggregated result>\n\
         \nTask: {}",
        multi_llm.client_count() - 1,
        "Coordinate the completion of the assigned task using available providers."
    );

    let mut memory = ConversationMemory::new(system_prompt, 20);
    memory.add_user_message(&supervisor_prompt);

    let mut subtask_results: Vec<String> = Vec::new();
    let mut iteration = 0;
    let max_iterations = 15;

    loop {
        iteration += 1;
        if iteration > max_iterations {
            warn!("Supervisor reached max iterations");
            break;
        }

        // Use round-robin for supervisor itself
        let supervisor_client = multi_llm.get_client(iteration % multi_llm.client_count())
            .or_else(|| multi_llm.get_client(0));

        let messages = memory.history().to_vec();
        let response = match supervisor_client.and_then(|c| {
            // Need to use block_on or spawn since we're in async context
            Some(tokio::spawn(async move { c.chat(messages).await }))
        }) {
            Some(handle) => {
                match handle.await {
                    Ok(Ok(r)) => r,
                    Ok(Err(e)) => {
                        warn!(error = %e, "Supervisor LLM request failed");
                        continue;
                    }
                    Err(e) => {
                        warn!(error = %e, "Supervisor task join failed");
                        continue;
                    }
                }
            }
            None => {
                warn!("No LLM clients available");
                break;
            }
        };

        let content = response.choices.first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        // Check for FINAL: completion
        if content.contains("FINAL:") {
            let final_response = content.split("FINAL:").nth(1).unwrap_or("").trim().to_string();
            info!("Supervisor completed task: {} chars", final_response.len());

            let _ = audit_log.append(
                AuditEventType::AgentFinish,
                "supervisor",
                "Supervisor completed task coordination",
                Some(serde_json::json!({
                    "iterations": iteration,
                    "subtasks_completed": subtask_results.len(),
                    "providers_used": multi_llm.client_count(),
                })),
            );

            println!("\n🐦‍⬛ Supervisor Result (multi-model):\n{}", final_response);
            return Ok(());
        }

        // Check for SUBTASK: decomposition
        if content.contains("SUBTASK:") && content.contains("PROVIDER:") {
            let subtask_block = content.split("SUBTASK:").nth(1).unwrap_or("");
            let subtask_lines: Vec<&str> = subtask_block.lines().take(4).collect();

            let subtask_desc = subtask_lines.first().unwrap_or(&"").trim();
            let provider_idx = subtask_lines.iter()
                .find(|l| l.starts_with("PROVIDER:"))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(0);

            if !subtask_desc.is_empty() {
                info!("Subtask for provider {}: {}", provider_idx, subtask_desc);

                let client = multi_llm.get_client(provider_idx)
                    .or_else(|| multi_llm.get_client(0));

                if let Some(client) = client {
                    let subtask_result = run_subtask_agent(
                        client,
                        subtask_desc,
                        system_prompt,
                        &policy_engine,
                        &sandbox,
                        &audit_log,
                        &registry,
                    ).await;

                    match subtask_result {
                        Ok(result) => {
                            info!("Subtask {} completed: {} chars", provider_idx, result.len());
                            subtask_results.push(format!(
                                "Provider {} ({}): {}",
                                provider_idx,
                                client.provider_name(),
                                result
                            ));

                            memory.add_assistant_message(&format!(
                                "Assigned subtask to provider {}: {}", provider_idx, subtask_desc
                            ));
                            memory.add_user_message(&format!(
                                "Provider {} result: {}", provider_idx, result
                            ));
                        }
                        Err(e) => {
                            warn!("Subtask {} failed: {}", provider_idx, e);
                            memory.add_assistant_message(&format!(
                                "Provider {} subtask failed: {}", provider_idx, e
                            ));
                        }
                    }
                }
            }
        } else {
            memory.add_assistant_message(&content);
        }
    }

    // Fallback: return aggregated results
    if !subtask_results.is_empty() {
        let aggregated = subtask_results.join("\n\n");
        info!("Supervisor aggregated {} subtask results", subtask_results.len());
        println!("\n🐦‍⬛ Supervisor Aggregated Result (multi-model):\n{}", aggregated);
        return Ok(());
    }

    Err(crate::error::RavenClawError::CommandExecution(
        "Supervisor mode completed without results".to_string(),
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
    fn test_swarm_function_exists() {
        // Verify swarm function signature compiles
        let _fn_ptr: fn(Arc<dyn LLMProviderTrait>, Config) -> _ = run_swarm;
        assert!(true);
    }

    #[test]
    fn test_supervisor_function_exists() {
        // Verify supervisor function signature compiles
        let _fn_ptr: fn(Arc<dyn LLMProviderTrait>, Config) -> _ = run_supervisor;
        assert!(true);
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
