//! Axum middleware for request tracking and observability.
//!
//! This module provides middleware layers for:
//! - **Correlation ID tracking**: Extract or generate correlation IDs for distributed tracing
//! - **Response headers**: Automatically inject correlation ID into responses
//! - **Tracing integration**: Create spans with correlation context
//!
//! # Example
//!
//! ```ignore
//! use axum::Router;
//! use composable_rust_web::middleware::correlation_id_layer;
//!
//! let app = Router::new()
//!     .route("/api/users", get(list_users))
//!     .layer(correlation_id_layer());
//! ```
//!
//! # Flow
//!
//! 1. **Extract** correlation ID from `X-Correlation-ID` header (or generate new UUID)
//! 2. **Store** in request extensions for handler access
//! 3. **Create tracing span** with correlation_id field
//! 4. **Inject** correlation ID into response `X-Correlation-ID` header
//!
//! # Benefits
//!
//! - **Automatic**: No manual extraction needed in handlers
//! - **Consistent**: Every request has a correlation ID
//! - **Observable**: Tracing spans include correlation context
//! - **Client-friendly**: Clients receive their correlation ID back

use axum::{
    extract::Request,
    http::HeaderValue,
    response::Response,
};
use std::task::{Context, Poll};
use tower::{Layer, Service};
use uuid::Uuid;

/// Header name for correlation ID.
pub const CORRELATION_ID_HEADER: &str = "X-Correlation-ID";

/// Create a layer that adds correlation ID tracking to all requests.
///
/// This layer:
/// - Extracts correlation ID from request header or generates new UUID
/// - Stores correlation ID in request extensions
/// - Creates tracing span with correlation_id field
/// - Injects correlation ID into response header
///
/// # Example
///
/// ```ignore
/// use axum::Router;
/// use composable_rust_web::middleware::correlation_id_layer;
///
/// let app = Router::new()
///     .route("/api/users", get(list_users))
///     .layer(correlation_id_layer());
/// ```
#[must_use]
pub fn correlation_id_layer() -> CorrelationIdLayer {
    CorrelationIdLayer
}

/// Layer for correlation ID tracking.
#[derive(Clone, Debug)]
pub struct CorrelationIdLayer;

impl<S> Layer<S> for CorrelationIdLayer {
    type Service = CorrelationIdMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CorrelationIdMiddleware { inner }
    }
}

/// Middleware service for correlation ID tracking.
#[derive(Clone, Debug)]
pub struct CorrelationIdMiddleware<S> {
    inner: S,
}

impl<S> Service<Request> for CorrelationIdMiddleware<S>
where
    S: Service<Request, Response = Response> + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request) -> Self::Future {
        // Extract correlation ID from header or generate new
        let correlation_id = req
            .headers()
            .get(CORRELATION_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::new_v4);

        // Store in request extensions for handler access
        req.extensions_mut().insert(correlation_id);

        // Create tracing span with correlation context
        let span = tracing::info_span!(
            "http_request",
            correlation_id = %correlation_id,
            method = %req.method(),
            uri = %req.uri(),
        );

        let fut = self.inner.call(req);

        Box::pin(async move {
            // Execute request within span
            let mut response = fut.instrument(span).await?;

            // Inject correlation ID into response header
            if let Ok(header_value) = HeaderValue::from_str(&correlation_id.to_string()) {
                response
                    .headers_mut()
                    .insert(CORRELATION_ID_HEADER, header_value);
            }

            Ok(response)
        })
    }
}

/// Extension trait for extracting correlation ID from request extensions.
///
/// This trait provides a convenience method for handlers to extract
/// the correlation ID that was injected by the middleware.
///
/// # Example
///
/// ```ignore
/// use composable_rust_web::middleware::CorrelationIdExt;
///
/// async fn handler(Extension(req): Extension<Request>) -> String {
///     let correlation_id = req.correlation_id();
///     format!("Request ID: {correlation_id}")
/// }
/// ```
pub trait CorrelationIdExt {
    /// Get the correlation ID from request extensions.
    ///
    /// # Panics
    ///
    /// Panics if the correlation ID middleware is not installed.
    /// Always use `correlation_id_layer()` in your router.
    fn correlation_id(&self) -> Uuid;

    /// Try to get the correlation ID from request extensions.
    ///
    /// Returns `None` if the correlation ID middleware is not installed.
    fn try_correlation_id(&self) -> Option<Uuid>;
}

impl CorrelationIdExt for Request {
    fn correlation_id(&self) -> Uuid {
        self.extensions()
            .get::<Uuid>()
            .copied()
            .expect("CorrelationId middleware not installed")
    }

    fn try_correlation_id(&self) -> Option<Uuid> {
        self.extensions().get::<Uuid>().copied()
    }
}

// Re-export tracing for use with Instrument
use tracing::Instrument;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, response::IntoResponse, routing::get, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_correlation_id_generated_if_missing() {
        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(correlation_id_layer());

        let request = Request::builder()
            .uri("/test")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // Should have correlation ID in response
        let correlation_id = response
            .headers()
            .get(CORRELATION_ID_HEADER)
            .expect("Correlation ID header should be present");

        // Should be valid UUID
        let uuid_str = correlation_id.to_str().unwrap();
        assert!(Uuid::parse_str(uuid_str).is_ok());
    }

    #[tokio::test]
    async fn test_correlation_id_preserved_from_request() {
        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(correlation_id_layer());

        let request_uuid = Uuid::new_v4();
        let request = Request::builder()
            .uri("/test")
            .header(CORRELATION_ID_HEADER, request_uuid.to_string())
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // Response should have same correlation ID
        let response_id = response
            .headers()
            .get(CORRELATION_ID_HEADER)
            .expect("Correlation ID header should be present")
            .to_str()
            .unwrap();

        assert_eq!(response_id, request_uuid.to_string());
    }

    #[tokio::test]
    async fn test_correlation_id_in_extensions() {
        use axum::body::Body;

        async fn handler(req: Request<Body>) -> impl IntoResponse {
            let correlation_id = req.correlation_id();
            format!("Correlation ID: {correlation_id}")
        }

        let app = Router::new()
            .route("/test", get(handler))
            .layer(correlation_id_layer());

        let request = Request::builder()
            .uri("/test")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_invalid_uuid_generates_new() {
        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(correlation_id_layer());

        let request = Request::builder()
            .uri("/test")
            .header(CORRELATION_ID_HEADER, "not-a-uuid")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // Should generate new valid UUID
        let correlation_id = response
            .headers()
            .get(CORRELATION_ID_HEADER)
            .expect("Correlation ID header should be present");

        let uuid_str = correlation_id.to_str().unwrap();
        assert!(Uuid::parse_str(uuid_str).is_ok());
        assert_ne!(uuid_str, "not-a-uuid");
    }
}
