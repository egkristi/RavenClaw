//! RavenClaws
//!
//! Provides a client for communicating with RavenFabric — a secure E2E-encrypted
//! RavenClaws
//! to dispatch tasks to remote agents, coordinate across a fleet, and execute
//! commands on remote hosts.
//!
//! # Protocol
//!
//! RavenFabric exposes a REST API over HTTPS:
//! - `POST /api/v1/execute` — Execute a command on a remote host
//! - `GET /api/v1/agents` — List available remote agents
//! - `GET /api/v1/health` — Health check
//!
//! All requests include the agent ID for identification. Responses are JSON.

use crate::config::RavenFabricConfig;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// RavenFabric client for remote execution and mesh coordination
///
/// Methods are currently wired for initialization and logging only.
/// Full remote execution dispatch will be integrated in a follow-up.
#[derive(Debug, Clone)]
pub struct RavenFabricClient {
    /// Configuration
    config: RavenFabricConfig,
    /// HTTP client (shared)
    http_client: reqwest::Client,
}

/// Request to execute a command on a remote host
#[derive(Debug, Serialize)]
struct ExecuteRequest {
    /// Command to execute
    command: String,
    /// Target host (optional — if None, executes on any available agent)
    #[serde(skip_serializing_if = "Option::is_none")]
    target_host: Option<String>,
    /// Timeout in seconds
    #[serde(default = "default_timeout")]
    timeout_secs: u64,
    /// Agent ID for identification
    agent_id: String,
}

#[allow(dead_code)]
fn default_timeout() -> u64 {
    30
}

/// Response from a remote execution
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ExecuteResponse {
    /// Whether the execution was successful
    pub success: bool,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Exit code
    pub exit_code: i32,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
}

/// Information about a remote agent
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct RemoteAgent {
    /// Agent ID
    pub id: String,
    /// Agent hostname
    pub hostname: String,
    /// Agent status (online/offline/busy)
    pub status: String,
    /// Last seen timestamp
    pub last_seen: String,
    /// Agent capabilities
    pub capabilities: Vec<String>,
}

impl RavenFabricClient {
    /// Create a new RavenFabric client
    ///
    /// Returns `None` if RavenFabric is not configured (no endpoint set).
    pub fn new(config: &RavenFabricConfig) -> Option<Self> {
        let endpoint = config.endpoint.as_ref()?;

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("RavenClaws/0.9.2")
            .build()
            .ok()?;

        info!(
            endpoint = %endpoint,
            agent_id = ?config.agent_id,
            "RavenFabric client initialized"
        );

        Some(Self {
            config: config.clone(),
            http_client,
        })
    }

    /// Get the configured endpoint
    pub fn endpoint(&self) -> Option<&str> {
        self.config.endpoint.as_deref()
    }

    /// Get the agent ID
    pub fn agent_id(&self) -> Option<&str> {
        self.config.agent_id.as_deref()
    }

    /// Check if RavenFabric is enabled for remote execution
    pub fn is_enabled(&self) -> bool {
        self.config.remote_exec
    }

    /// Build an HTTP URL from the configured endpoint, converting ws:// to http://
    /// and wss:// to https:// as needed for HTTP-based API calls.
    fn http_url(&self, path: &str) -> Result<String> {
        let endpoint = self.config.endpoint.as_deref().ok_or_else(|| {
            RavenFabricError::NotConfigured("No RavenFabric endpoint configured".to_string())
        })?;
        let url = format!("{}{}", endpoint.trim_end_matches('/'), path);
        // Convert WebSocket schemes to HTTP for REST API calls
        let url = url
            .replacen("ws://", "http://", 1)
            .replacen("wss://", "https://", 1);
        Ok(url)
    }

    /// Check RavenFabric health
    pub async fn health(&self) -> Result<bool> {
        let url = self.http_url("/api/v1/health")?;

        match self.http_client.get(&url).send().await {
            Ok(response) => Ok(response.status().is_success()),
            Err(e) => {
                warn!(error = %e, url = %url, "RavenFabric health check failed");
                Err(RavenFabricError::ConnectionFailed(e.to_string()))
            }
        }
    }

    /// List available remote agents
    pub async fn list_agents(&self) -> Result<Vec<RemoteAgent>> {
        let url = self.http_url("/api/v1/agents")?;

        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| RavenFabricError::ConnectionFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(RavenFabricError::RequestFailed(format!(
                "Failed to list agents: HTTP {}",
                response.status()
            )));
        }

        let agents: Vec<RemoteAgent> = response.json().await.map_err(|e| {
            RavenFabricError::RequestFailed(format!("Failed to parse agents: {}", e))
        })?;

        info!(count = agents.len(), "RavenFabric agents listed");
        Ok(agents)
    }

    /// Execute a command on a remote host via RavenFabric
    ///
    /// If `target_host` is `None`, RavenFabric will execute on any available agent.
    pub async fn execute(
        &self,
        command: &str,
        target_host: Option<&str>,
        timeout_secs: u64,
    ) -> Result<ExecuteResponse> {
        let url = self.http_url("/api/v1/execute")?;

        let request = ExecuteRequest {
            command: command.to_string(),
            target_host: target_host.map(|s| s.to_string()),
            timeout_secs,
            agent_id: self
                .config
                .agent_id
                .clone()
                .unwrap_or_else(|| "ravenclaws-default".to_string()),
        };

        info!(
            command = %command,
            target = ?target_host,
            timeout = timeout_secs,
            "RavenFabric execute request"
        );

        let response = self
            .http_client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| RavenFabricError::ConnectionFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(RavenFabricError::RequestFailed(format!(
                "Execute request failed: HTTP {} — {}",
                status, body
            )));
        }

        let result: ExecuteResponse = response.json().await.map_err(|e| {
            RavenFabricError::RequestFailed(format!("Failed to parse response: {}", e))
        })?;

        info!(
            success = result.success,
            exit_code = result.exit_code,
            duration_ms = result.duration_ms,
            "RavenFabric execute completed"
        );

        Ok(result)
    }

    /// Execute a command on all available remote agents (broadcast)
    pub async fn broadcast(
        &self,
        command: &str,
        timeout_secs: u64,
    ) -> Result<Vec<(String, Result<ExecuteResponse>)>> {
        let agents = self.list_agents().await?;
        let mut results = Vec::new();

        for agent in &agents {
            let result = self
                .execute(command, Some(&agent.hostname), timeout_secs)
                .await;
            results.push((agent.id.clone(), result));
        }

        info!(
            command = %command,
            agent_count = agents.len(),
            "RavenFabric broadcast completed"
        );

        Ok(results)
    }
}

/// Errors from RavenFabric operations
#[derive(Debug, thiserror::Error)]
pub enum RavenFabricError {
    #[error("RavenFabric not configured: {0}")]
    NotConfigured(String),

    #[error("RavenFabric connection failed: {0}")]
    ConnectionFailed(String),

    #[error("RavenFabric request failed: {0}")]
    RequestFailed(String),
}

/// Type alias for RavenFabric results
pub type Result<T> = std::result::Result<T, RavenFabricError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(endpoint: Option<String>) -> RavenFabricConfig {
        RavenFabricConfig {
            endpoint,
            agent_id: Some("agent-1".to_string()),
            remote_exec: true,
            allowed_hosts: vec![],
        }
    }

    #[test]
    fn test_ravenfabric_client_new_no_endpoint() {
        let config = RavenFabricConfig {
            endpoint: None,
            agent_id: None,
            remote_exec: true,
            allowed_hosts: vec![],
        };
        let client = RavenFabricClient::new(&config);
        assert!(client.is_none(), "Client should be None when no endpoint");
    }

    #[test]
    fn test_ravenfabric_client_new_with_endpoint() {
        let config = RavenFabricConfig {
            endpoint: Some("http://localhost:8080".to_string()),
            agent_id: Some("test-agent".to_string()),
            remote_exec: true,
            allowed_hosts: vec![],
        };
        let client = RavenFabricClient::new(&config);
        assert!(
            client.is_some(),
            "Client should be Some when endpoint is set"
        );
        let client = client.unwrap();
        assert_eq!(client.endpoint(), Some("http://localhost:8080"));
        assert_eq!(client.agent_id(), Some("test-agent"));
        assert!(client.is_enabled());
    }

    #[test]
    fn test_ravenfabric_client_disabled() {
        let config = RavenFabricConfig {
            endpoint: Some("http://localhost:8080".to_string()),
            agent_id: None,
            remote_exec: false,
            allowed_hosts: vec![],
        };
        let client = RavenFabricClient::new(&config);
        assert!(client.is_some());
        assert!(!client.unwrap().is_enabled());
    }

    #[test]
    fn test_ravenfabric_error_display() {
        let err = RavenFabricError::NotConfigured("no endpoint".to_string());
        assert_eq!(
            format!("{}", err),
            "RavenFabric not configured: no endpoint"
        );

        let err = RavenFabricError::ConnectionFailed("timeout".to_string());
        assert_eq!(format!("{}", err), "RavenFabric connection failed: timeout");

        let err = RavenFabricError::RequestFailed("bad request".to_string());
        assert_eq!(
            format!("{}", err),
            "RavenFabric request failed: bad request"
        );
    }

    #[tokio::test]
    async fn test_ravenfabric_health_no_endpoint() {
        let config = RavenFabricConfig {
            endpoint: None,
            agent_id: None,
            remote_exec: true,
            allowed_hosts: vec![],
        };
        let client = RavenFabricClient::new(&config);
        assert!(client.is_none());
    }

    #[tokio::test]
    async fn test_ravenfabric_execute_no_endpoint() {
        let config = RavenFabricConfig {
            endpoint: None,
            agent_id: None,
            remote_exec: true,
            allowed_hosts: vec![],
        };
        // Can't create client without endpoint
        assert!(RavenFabricClient::new(&config).is_none());
    }

    #[tokio::test]
    async fn test_ravenfabric_health_connection_refused() {
        let config = RavenFabricConfig {
            endpoint: Some("http://127.0.0.1:1".to_string()), // Port 1 will refuse
            agent_id: Some("test".to_string()),
            remote_exec: true,
            allowed_hosts: vec![],
        };
        let client = RavenFabricClient::new(&config).unwrap();
        let result = client.health().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            RavenFabricError::ConnectionFailed(_) => {} // Expected
            other => panic!("Expected ConnectionFailed, got: {}", other),
        }
    }

    #[tokio::test]
    async fn test_ravenfabric_list_agents_connection_refused() {
        let config = RavenFabricConfig {
            endpoint: Some("http://127.0.0.1:1".to_string()),
            agent_id: Some("test".to_string()),
            remote_exec: true,
            allowed_hosts: vec![],
        };
        let client = RavenFabricClient::new(&config).unwrap();
        let result = client.list_agents().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ravenfabric_execute_connection_refused() {
        let config = RavenFabricConfig {
            endpoint: Some("http://127.0.0.1:1".to_string()),
            agent_id: Some("test".to_string()),
            remote_exec: true,
            allowed_hosts: vec![],
        };
        let client = RavenFabricClient::new(&config).unwrap();
        let result = client.execute("echo hello", None, 10).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_request_serialization() {
        let request = ExecuteRequest {
            command: "echo hello".to_string(),
            target_host: Some("agent-1".to_string()),
            timeout_secs: 30,
            agent_id: "ravenclaws-test".to_string(),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("echo hello"));
        assert!(json.contains("agent-1"));
        assert!(json.contains("ravenclaws-test"));
        assert!(json.contains("30"));
    }

    #[test]
    fn test_execute_request_no_target() {
        let request = ExecuteRequest {
            command: "uptime".to_string(),
            target_host: None,
            timeout_secs: 10,
            agent_id: "test".to_string(),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("uptime"));
        assert!(
            !json.contains("target_host"),
            "target_host should be skipped when None"
        );
    }

    #[test]
    fn test_remote_agent_deserialization() {
        let json = r#"{
            "id": "agent-1",
            "hostname": "worker-01.example.com",
            "status": "online",
            "last_seen": "2026-06-18T12:00:00Z",
            "capabilities": ["shell", "file", "docker"]
        }"#;
        let agent: RemoteAgent = serde_json::from_str(json).unwrap();
        assert_eq!(agent.id, "agent-1");
        assert_eq!(agent.hostname, "worker-01.example.com");
        assert_eq!(agent.status, "online");
        assert_eq!(agent.capabilities.len(), 3);
    }

    #[test]
    fn test_execute_response_deserialization() {
        let json = r#"{
            "success": true,
            "stdout": "hello world\n",
            "stderr": "",
            "exit_code": 0,
            "duration_ms": 42
        }"#;
        let response: ExecuteResponse = serde_json::from_str(json).unwrap();
        assert!(response.success);
        assert_eq!(response.stdout, "hello world\n");
        assert_eq!(response.exit_code, 0);
        assert_eq!(response.duration_ms, 42);
    }

    #[test]
    fn test_execute_response_failure() {
        let json = r#"{
            "success": false,
            "stdout": "",
            "stderr": "command not found",
            "exit_code": 127,
            "duration_ms": 5
        }"#;
        let response: ExecuteResponse = serde_json::from_str(json).unwrap();
        assert!(!response.success);
        assert_eq!(response.stderr, "command not found");
        assert_eq!(response.exit_code, 127);
    }

    // ── Integration tests (mockito-backed HTTP round-trips) ──────────────

    #[tokio::test]
    async fn test_integration_health_round_trip() {
        let mut server = mockito::Server::new_async().await;
        let health_mock = server
            .mock("GET", "/api/v1/health")
            .with_status(200)
            .with_body(r#"{"status":"ok"}"#)
            .expect(1)
            .create_async()
            .await;

        let config = test_config(Some(server.url()));
        let client = RavenFabricClient::new(&config).expect("client should be created");
        let healthy = client.health().await.expect("health should succeed");
        assert!(healthy, "relay should report healthy");
        health_mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_integration_list_agents_round_trip() {
        let mut server = mockito::Server::new_async().await;
        let agents_mock = server
            .mock("GET", "/api/v1/agents")
            .with_status(200)
            .with_body(
                r#"[{"id":"a1","hostname":"worker-1","status":"online","last_seen":"2026-06-18T12:00:00Z","capabilities":["shell"]}]"#,
            )
            .expect(1)
            .create_async()
            .await;

        let config = test_config(Some(server.url()));
        let client = RavenFabricClient::new(&config).expect("client should be created");
        let agents = client.list_agents().await.expect("list should succeed");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "a1");
        assert_eq!(agents[0].hostname, "worker-1");
        agents_mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_integration_execute_round_trip() {
        let mut server = mockito::Server::new_async().await;
        let execute_mock = server
            .mock("POST", "/api/v1/execute")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "command": "echo hello"
            })))
            .with_status(200)
            .with_body(
                r#"{"success":true,"stdout":"hello\n","stderr":"","exit_code":0,"duration_ms":7}"#,
            )
            .expect(1)
            .create_async()
            .await;

        let config = test_config(Some(server.url()));
        let client = RavenFabricClient::new(&config).expect("client should be created");
        let result = client
            .execute("echo hello", Some("worker-1"), 10)
            .await
            .expect("execute should succeed");
        assert!(result.success);
        assert_eq!(result.stdout, "hello\n");
        assert_eq!(result.exit_code, 0);
        execute_mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_integration_execute_policy_deny() {
        let mut server = mockito::Server::new_async().await;
        // A 403 simulates a policy-denied command rejected by the relay.
        let execute_mock = server
            .mock("POST", "/api/v1/execute")
            .with_status(403)
            .with_body(r#"{"error":"policy denied: command not allowed"}"#)
            .expect(1)
            .create_async()
            .await;

        let config = test_config(Some(server.url()));
        let client = RavenFabricClient::new(&config).expect("client should be created");
        let result = client.execute("rm -rf /", None, 10).await;
        assert!(result.is_err(), "policy-denied execute should error");
        match result.unwrap_err() {
            RavenFabricError::RequestFailed(msg) => {
                assert!(msg.contains("403"), "error should report HTTP 403: {}", msg);
            }
            other => panic!("Expected RequestFailed, got: {}", other),
        }
        execute_mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_integration_broadcast_round_trip() {
        let mut server = mockito::Server::new_async().await;
        let agents_mock = server
            .mock("GET", "/api/v1/agents")
            .with_status(200)
            .with_body(
                r#"[
                    {"id":"a1","hostname":"worker-1","status":"online","last_seen":"2026-06-18T12:00:00Z","capabilities":["shell"]},
                    {"id":"a2","hostname":"worker-2","status":"online","last_seen":"2026-06-18T12:00:00Z","capabilities":["shell"]}
                ]"#,
            )
            .expect(1)
            .create_async()
            .await;
        let execute_mock = server
            .mock("POST", "/api/v1/execute")
            .with_status(200)
            .with_body(
                r#"{"success":true,"stdout":"ok\n","stderr":"","exit_code":0,"duration_ms":3}"#,
            )
            .expect(2)
            .create_async()
            .await;

        let config = test_config(Some(server.url()));
        let client = RavenFabricClient::new(&config).expect("client should be created");
        let results = client
            .broadcast("uptime", 10)
            .await
            .expect("broadcast should succeed");
        assert_eq!(results.len(), 2, "broadcast fans out to both agents");
        assert!(results.iter().all(|(_, r)| r.is_ok()));
        agents_mock.assert_async().await;
        execute_mock.assert_async().await;
    }
}
