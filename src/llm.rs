//! LiteLLM client integration
//!
//! Provides a thin wrapper around LiteLLM's OpenAI-compatible API.

use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::LLMConfig;

#[derive(Error, Debug)]
pub enum LLMError {
    #[error("Request failed: {0}")]
    RequestFailed(String),
    
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    
    #[error("Rate limit exceeded")]
    RateLimited,
    
    #[error("Authentication failed")]
    AuthFailed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

pub struct LiteLLMClient {
    client: Client,
    config: LLMConfig,
}

impl LiteLLMClient {
    pub fn new(config: &LLMConfig) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("Failed to create HTTP client");
        
        Self {
            client,
            config: config.clone(),
        }
    }
    
    /// Send a chat completion request
    pub async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse, LLMError> {
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages,
            temperature: Some(0.7),
            max_tokens: Some(2048),
            stream: None,
        };
        
        let mut req = self.client
            .post(format!("{}/v1/chat/completions", self.config.endpoint.trim_end_matches('/')))
            .json(&request);
        
        if let Some(ref key) = self.config.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }
        
        let response = req
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;
        
        self.handle_response(response).await
    }
    
    async fn handle_response(&self, response: Response) -> Result<ChatResponse, LLMError> {
        let status = response.status();
        
        if status.is_success() {
            response
                .json::<ChatResponse>()
                .await
                .map_err(|e| LLMError::InvalidResponse(e.to_string()))
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            Err(LLMError::AuthFailed)
        } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            Err(LLMError::RateLimited)
        } else {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(LLMError::RequestFailed(format!("{}: {}", status, body)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_client_creation() {
        let config = LLMConfig {
            endpoint: "http://localhost:4000".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key: Some("test".to_string()),
            timeout_secs: 30,
        };
        
        let _client = LiteLLMClient::new(&config);
    }
}
