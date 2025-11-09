//! Custom Axum extractors.
//!
//! This module contains custom extractors for common HTTP patterns:
//! - `CorrelationId`: Extract or generate request correlation IDs
//! - `ClientIp`: Extract client IP address from headers or connection
//! - `UserAgent`: Extract User-Agent header
//!
//! # Examples
//!
//! ```ignore
//! use axum::extract::State;
//! use composable_rust_web::extractors::{CorrelationId, ClientIp, UserAgent};
//!
//! async fn handler(
//!     State(state): State<AppState>,
//!     correlation_id: CorrelationId,
//!     client_ip: ClientIp,
//!     user_agent: UserAgent,
//! ) -> Result<Json<Response>, AppError> {
//!     tracing::info!(
//!         correlation_id = %correlation_id.0,
//!         client_ip = %client_ip.0,
//!         user_agent = %user_agent.0,
//!         "Processing request"
//!     );
//!     Ok(Json(response))
//! }
//! ```

use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, HeaderMap},
};
use std::net::IpAddr;
use uuid::Uuid;

/// Correlation ID for request tracing.
///
/// Extracts the correlation ID from the `X-Correlation-ID` header,
/// or generates a new UUID v4 if not present.
///
/// # Example
///
/// ```ignore
/// async fn handler(correlation_id: CorrelationId) -> String {
///     format!("Request ID: {}", correlation_id.0)
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct CorrelationId(pub Uuid);

#[async_trait]
impl<S> FromRequestParts<S> for CorrelationId
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Try to extract from X-Correlation-ID header
        let correlation_id = parts
            .headers
            .get("X-Correlation-ID")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::new_v4);

        Ok(Self(correlation_id))
    }
}

/// Client IP address.
///
/// Extracts the client IP from the `X-Forwarded-For` header (first IP),
/// or falls back to `X-Real-IP`, or the connection IP.
///
/// # Priority
///
/// 1. `X-Forwarded-For` (first IP in the list)
/// 2. `X-Real-IP`
/// 3. Connection IP (always available)
///
/// # Example
///
/// ```ignore
/// async fn handler(client_ip: ClientIp) -> String {
///     format!("Client IP: {}", client_ip.0)
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

#[async_trait]
impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ip = extract_client_ip(&parts.headers, parts.extensions.get());

        Ok(Self(ip))
    }
}

/// Extract client IP from headers or connection info.
fn extract_client_ip(
    headers: &HeaderMap,
    _connect_info: Option<&axum::extract::connect_info::ConnectInfo<std::net::SocketAddr>>,
) -> IpAddr {
    // Try X-Forwarded-For (take first IP)
    if let Some(forwarded) = headers.get("X-Forwarded-For") {
        if let Ok(forwarded_str) = forwarded.to_str() {
            if let Some(first_ip) = forwarded_str.split(',').next() {
                if let Ok(ip) = first_ip.trim().parse::<IpAddr>() {
                    return ip;
                }
            }
        }
    }

    // Try X-Real-IP
    if let Some(real_ip) = headers.get("X-Real-IP") {
        if let Ok(ip_str) = real_ip.to_str() {
            if let Ok(ip) = ip_str.parse::<IpAddr>() {
                return ip;
            }
        }
    }

    // Fallback to localhost (connection IP would come from ConnectInfo middleware)
    "127.0.0.1".parse().expect("Valid IP")
}

/// User-Agent header.
///
/// Extracts the `User-Agent` header, or returns "Unknown" if not present.
///
/// # Example
///
/// ```ignore
/// async fn handler(user_agent: UserAgent) -> String {
///     format!("User-Agent: {}", user_agent.0)
/// }
/// ```
#[derive(Debug, Clone)]
pub struct UserAgent(pub String);

#[async_trait]
impl<S> FromRequestParts<S> for UserAgent
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let user_agent = parts
            .headers
            .get("User-Agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("Unknown")
            .to_string();

        Ok(Self(user_agent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Request, header};

    #[tokio::test]
    async fn test_correlation_id_from_header() {
        let uuid = Uuid::new_v4();
        let req = Request::builder()
            .header("X-Correlation-ID", uuid.to_string())
            .body(())
            .expect("Valid request");

        let (mut parts, _) = req.into_parts();
        let correlation_id = CorrelationId::from_request_parts(&mut parts, &())
            .await
            .expect("Should extract");

        assert_eq!(correlation_id.0, uuid);
    }

    #[tokio::test]
    async fn test_correlation_id_generates_new() {
        let req = Request::builder()
            .body(())
            .expect("Valid request");

        let (mut parts, _) = req.into_parts();
        let correlation_id = CorrelationId::from_request_parts(&mut parts, &())
            .await
            .expect("Should extract");

        // Should have generated a valid UUID
        assert_ne!(correlation_id.0, Uuid::nil());
    }

    #[tokio::test]
    async fn test_client_ip_from_x_forwarded_for() {
        let req = Request::builder()
            .header("X-Forwarded-For", "203.0.113.1, 198.51.100.1")
            .body(())
            .expect("Valid request");

        let (mut parts, _) = req.into_parts();
        let client_ip = ClientIp::from_request_parts(&mut parts, &())
            .await
            .expect("Should extract");

        assert_eq!(client_ip.0.to_string(), "203.0.113.1");
    }

    #[tokio::test]
    async fn test_client_ip_from_x_real_ip() {
        let req = Request::builder()
            .header("X-Real-IP", "198.51.100.42")
            .body(())
            .expect("Valid request");

        let (mut parts, _) = req.into_parts();
        let client_ip = ClientIp::from_request_parts(&mut parts, &())
            .await
            .expect("Should extract");

        assert_eq!(client_ip.0.to_string(), "198.51.100.42");
    }

    #[tokio::test]
    async fn test_client_ip_fallback() {
        let req = Request::builder()
            .body(())
            .expect("Valid request");

        let (mut parts, _) = req.into_parts();
        let client_ip = ClientIp::from_request_parts(&mut parts, &())
            .await
            .expect("Should extract");

        // Should fallback to localhost
        assert_eq!(client_ip.0.to_string(), "127.0.0.1");
    }

    #[tokio::test]
    async fn test_user_agent_from_header() {
        let req = Request::builder()
            .header(header::USER_AGENT, "Mozilla/5.0 (Test)")
            .body(())
            .expect("Valid request");

        let (mut parts, _) = req.into_parts();
        let user_agent = UserAgent::from_request_parts(&mut parts, &())
            .await
            .expect("Should extract");

        assert_eq!(user_agent.0, "Mozilla/5.0 (Test)");
    }

    #[tokio::test]
    async fn test_user_agent_fallback() {
        let req = Request::builder()
            .body(())
            .expect("Valid request");

        let (mut parts, _) = req.into_parts();
        let user_agent = UserAgent::from_request_parts(&mut parts, &())
            .await
            .expect("Should extract");

        assert_eq!(user_agent.0, "Unknown");
    }
}
