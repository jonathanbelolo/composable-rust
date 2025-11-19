//! Idempotency support for API endpoints.
//!
//! Provides idempotency keys for payment and reservation endpoints to prevent
//! duplicate operations from accidental retries.
//!
//! # Usage
//!
//! ```rust
//! async fn process_payment(
//!     idempotency: IdempotencyKey,
//!     // ... other extractors
//! ) -> Result<Json<Response>, AppError> {
//!     // Check for cached result
//!     if let Some(cached) = idempotency.get_cached().await? {
//!         return Ok(Json(cached));
//!     }
//!
//!     // Process payment...
//!     let response = process_payment_logic()?;
//!
//!     // Cache the result (24-hour TTL)
//!     idempotency.cache_result(&response).await?;
//!
//!     Ok(Json(response))
//! }
//! ```
//!
//! # HTTP Header
//!
//! Clients must provide the `Idempotency-Key` header:
//! ```text
//! POST /api/payments
//! Idempotency-Key: 550e8400-e29b-41d4-a716-446655440000
//! Authorization: Bearer <token>
//! ```
//!
//! # Behavior
//!
//! - **First request**: Processes operation, caches result for 24 hours
//! - **Retry with same key**: Returns cached result instantly (no duplicate processing)
//! - **Different key**: Treats as new operation
//!
//! # Security
//!
//! - Keys are scoped by user ID to prevent cross-user replay
//! - Cached responses expire after 24 hours
//! - Invalid keys return 400 Bad Request

use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use composable_rust_web::error::AppError;
use redis::AsyncCommands;
use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

/// Idempotency key extractor for Axum handlers.
///
/// Extracts the `Idempotency-Key` header and provides methods to check
/// for cached results and store new results in Redis.
pub struct IdempotencyKey {
    /// The idempotency key from the request header
    key: String,
    /// User ID (for scoping keys to prevent cross-user replay)
    user_id: Uuid,
    /// Redis connection for caching
    redis_client: redis::Client,
}

impl IdempotencyKey {
    /// Check Redis for a cached result.
    ///
    /// # Errors
    ///
    /// Returns error if Redis connection fails or deserialization fails.
    pub async fn get_cached<T: DeserializeOwned>(&self) -> Result<Option<T>, AppError> {
        let mut conn = self.redis_client.get_multiplexed_async_connection().await
            .map_err(|e| AppError::internal(format!("Redis connection error: {e}")))?;

        let cache_key = format!("idempotency:{}:{}", self.user_id, self.key);

        let cached: Option<String> = conn.get(&cache_key).await
            .map_err(|e| AppError::internal(format!("Redis GET error: {e}")))?;

        if let Some(json) = cached {
            let result = serde_json::from_str(&json)
                .map_err(|e| AppError::internal(format!("Deserialization error: {e}")))?;
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }

    /// Cache a result in Redis with 24-hour TTL.
    ///
    /// # Errors
    ///
    /// Returns error if Redis connection fails or serialization fails.
    pub async fn cache_result<T: Serialize>(&self, result: &T) -> Result<(), AppError> {
        let mut conn = self.redis_client.get_multiplexed_async_connection().await
            .map_err(|e| AppError::internal(format!("Redis connection error: {e}")))?;

        let cache_key = format!("idempotency:{}:{}", self.user_id, self.key);
        let json = serde_json::to_string(result)
            .map_err(|e| AppError::internal(format!("Serialization error: {e}")))?;

        // Cache for 24 hours (86400 seconds)
        let _: () = conn.set_ex(&cache_key, json, 86400).await
            .map_err(|e| AppError::internal(format!("Redis SET error: {e}")))?;

        Ok(())
    }
}

/// Axum extractor implementation.
///
/// Extracts the `Idempotency-Key` header and the user ID from the session.
#[async_trait]
impl<S> FromRequestParts<S> for IdempotencyKey
where
    S: Send + Sync,
    crate::auth::middleware::SessionUser: FromRequestParts<S>,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Extract the Idempotency-Key header
        let key = parts
            .headers
            .get("Idempotency-Key")
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Missing Idempotency-Key header".to_string(),
            ))?
            .to_str()
            .map_err(|_| (
                StatusCode::BAD_REQUEST,
                "Invalid Idempotency-Key header value".to_string(),
            ))?
            .to_string();

        // Validate key format (should be a UUID or similar)
        if key.len() < 16 || key.len() > 128 {
            return Err((
                StatusCode::BAD_REQUEST,
                "Idempotency-Key must be between 16 and 128 characters".to_string(),
            ));
        }

        // Extract user ID from session
        let session = crate::auth::middleware::SessionUser::from_request_parts(parts, state).await
            .map_err(|_| (StatusCode::UNAUTHORIZED, "Authentication required".to_string()))?;

        // Get Redis URL from config (loaded from environment)
        let config = crate::config::Config::from_env();
        let redis_client = redis::Client::open(config.redis.url.as_str())
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create Redis client: {e}")))?;

        Ok(IdempotencyKey {
            key,
            user_id: session.user_id.0,
            redis_client,
        })
    }
}
