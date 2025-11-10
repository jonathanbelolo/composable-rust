//! Anthropic API client implementation

use crate::{
    error::ClaudeError,
    messages::{MessagesRequest, MessagesResponse, StreamEvent},
};
use async_stream::stream;
use futures::stream::Stream;
use reqwest::{Client, StatusCode};
use std::pin::Pin;

/// Anthropic API client
#[derive(Clone)]
pub struct AnthropicClient {
    client: Client,
    api_key: String,
    api_url: String,
}

impl AnthropicClient {
    /// Create a new client with API key from environment
    ///
    /// # Errors
    ///
    /// Returns `ClaudeError::MissingApiKey` if `ANTHROPIC_API_KEY` is not set
    pub fn from_env() -> Result<Self, ClaudeError> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| ClaudeError::MissingApiKey)?;

        Ok(Self::new(api_key))
    }

    /// Create a new client with explicit API key
    #[must_use]
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            api_url: "https://api.anthropic.com/v1".to_string(),
        }
    }

    /// Create messages (non-streaming)
    ///
    /// # Errors
    ///
    /// Returns errors for network failures, API errors, or parsing failures
    pub async fn messages(&self, request: MessagesRequest) -> Result<MessagesResponse, ClaudeError> {
        let response = self.client
            .post(format!("{}/messages", self.api_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| ClaudeError::RequestFailed(e.to_string()))?;

        match response.status() {
            StatusCode::OK => {
                response.json::<MessagesResponse>().await
                    .map_err(|e| ClaudeError::ResponseParseFailed(e.to_string()))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                Err(ClaudeError::RateLimited)
            }
            StatusCode::UNAUTHORIZED => {
                Err(ClaudeError::Unauthorized)
            }
            status => {
                let body = response.text().await.unwrap_or_default();
                Err(ClaudeError::ApiError {
                    status: status.as_u16(),
                    message: body,
                })
            }
        }
    }

    /// Create messages (streaming)
    ///
    /// Returns a stream of `StreamEvent` items. The stream yields events as they
    /// arrive from the API.
    ///
    /// # Errors
    ///
    /// Returns errors for network failures or API errors. Individual stream items
    /// may also contain errors if event parsing fails.
    pub async fn messages_stream(
        &self,
        request: MessagesRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ClaudeError>> + Send>>, ClaudeError> {
        let mut streaming_request = request;
        streaming_request.stream = true;

        let response = self.client
            .post(format!("{}/messages", self.api_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&streaming_request)
            .send()
            .await
            .map_err(|e| ClaudeError::RequestFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ClaudeError::ApiError {
                status: status.as_u16(),
                message: body,
            });
        }

        let byte_stream = response.bytes_stream();

        Ok(Box::pin(stream! {
            let mut buffer = String::new();

            for await chunk in byte_stream {
                match chunk {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        // Parse SSE events (lines starting with "data: ")
                        while let Some(pos) = buffer.find('\n') {
                            let line = buffer[..pos].trim().to_string();
                            buffer.drain(..=pos);

                            if let Some(json_data) = line.strip_prefix("data: ") {
                                if json_data == "[DONE]" {
                                    break;
                                }

                                match serde_json::from_str::<StreamEvent>(json_data) {
                                    Ok(event) => yield Ok(event),
                                    Err(e) => yield Err(ClaudeError::ResponseParseFailed(e.to_string())),
                                }
                            }
                        }
                    }
                    Err(e) => {
                        yield Err(ClaudeError::StreamFailed(e.to_string()));
                        break;
                    }
                }
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;

    #[test]
    fn test_client_creation() {
        let client = AnthropicClient::new("test-key".to_string());
        assert_eq!(client.api_key, "test-key");
        assert_eq!(client.api_url, "https://api.anthropic.com/v1");
    }

    #[test]
    fn test_messages_request_creation() {
        let request = MessagesRequest::new(vec![Message::user("Hello")]);
        assert_eq!(request.messages.len(), 1);
        assert!(!request.stream);
    }
}
