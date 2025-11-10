//! Error types for the Anthropic API client

use thiserror::Error;

/// Errors that can occur when interacting with the Anthropic API
#[derive(Debug, Error)]
pub enum ClaudeError {
    /// Missing `ANTHROPIC_API_KEY` environment variable
    #[error("Missing ANTHROPIC_API_KEY environment variable")]
    MissingApiKey,

    /// HTTP request failed
    #[error("Request failed: {0}")]
    RequestFailed(String),

    /// Response parsing failed
    #[error("Response parsing failed: {0}")]
    ResponseParseFailed(String),

    /// Rate limited - too many requests
    #[error("Rate limited - too many requests")]
    RateLimited,

    /// Unauthorized - invalid API key
    #[error("Unauthorized - invalid API key")]
    Unauthorized,

    /// API returned an error
    #[error("API error (status {status}): {message}")]
    ApiError {
        /// HTTP status code
        status: u16,
        /// Error message from API
        message: String,
    },

    /// Stream processing failed
    #[error("Stream failed: {0}")]
    StreamFailed(String),
}
