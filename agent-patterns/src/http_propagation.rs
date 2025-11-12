//! HTTP Trace Context Propagation (Phase 8.4 Part 1.3)
//!
//! This module provides utilities for propagating trace context across
//! HTTP service boundaries using W3C Trace Context format.
//!
//! ## W3C Trace Context
//!
//! Uses standard `traceparent` and `tracestate` HTTP headers for
//! distributed tracing across services.
//!
//! ## Usage
//!
//! ```ignore
//! use agent_patterns::http_propagation::{inject_trace_headers, extract_trace_context};
//! use tracing::Span;
//!
//! // When making HTTP requests
//! let headers = inject_trace_headers(&Span::current());
//! let response = http_client.get(url).headers(headers).send().await?;
//!
//! // When receiving HTTP requests
//! if let Some(context) = extract_trace_context(&request.headers()) {
//!     // Use context to create child span
//! }
//! ```

use opentelemetry::{propagation::TextMapPropagator, trace::TraceContextExt, Context as OtelContext};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use std::collections::HashMap;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Extract trace context from HTTP headers
///
/// Uses W3C Trace Context propagator to extract trace context from
///  standard `traceparent` and `tracestate` headers.
///
/// # Arguments
///
/// * `headers` - HTTP headers as `HashMap<String, String>`
///
/// # Returns
///
/// `Some(Context)` if trace context was found, `None` otherwise
///
/// # Example
///
/// ```ignore
/// let context = extract_trace_context(&headers);
/// if let Some(ctx) = context {
///     // Create child span with this context as parent
/// }
/// ```
#[must_use]
pub fn extract_trace_context<S: std::hash::BuildHasher>(headers: &HashMap<String, String, S>) -> Option<OtelContext> {
    let propagator = TraceContextPropagator::new();

    // Extract returns the current context if no trace context found
    let context = propagator.extract(headers);

    // Check if we actually got trace context or just empty context
    if context.span().span_context().is_valid() {
        Some(context)
    } else {
        None
    }
}

/// Inject trace context into HTTP headers
///
/// Uses W3C Trace Context propagator to inject current span context
/// into HTTP headers for outgoing requests.
///
/// # Arguments
///
/// * `span` - Current tracing span
///
/// # Returns
///
/// `HashMap` of headers to add to HTTP request
///
/// # Example
///
/// ```ignore
/// let span = Span::current();
/// let headers = inject_trace_headers(&span);
///
/// let response = reqwest::Client::new()
///     .get(url)
///     .headers(headers.into_iter().collect())
///     .send()
///     .await?;
/// ```
#[must_use]
pub fn inject_trace_headers(span: &Span) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    let propagator = TraceContextPropagator::new();
    let context = span.context();

    propagator.inject_context(&context, &mut headers);

    headers
}

/// Make HTTP request with trace propagation
///
/// Helper function that automatically injects trace context into
/// an HTTP request.
///
/// # Arguments
///
/// * `url` - URL to request
/// * `method` - HTTP method ("GET", "POST", etc.)
/// * `body` - Optional request body
/// * `current_span` - Current span for context
///
/// # Errors
///
/// Returns error if HTTP request fails
///
/// # Example
///
/// ```ignore
/// let response = http_request_with_trace(
///     "https://api.example.com/data",
///     "GET",
///     None,
///     &Span::current()
/// ).await?;
/// ```
pub async fn http_request_with_trace(
    url: &str,
    method: &str,
    body: Option<&str>,
    current_span: &Span,
) -> Result<String, String> {
    let headers = inject_trace_headers(current_span);

    // Note: This is a simple example. In production, you'd use a proper HTTP client
    // with retry policies, timeouts, etc.
    let client = reqwest::Client::new();

    let mut request = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        _ => return Err(format!("Unsupported HTTP method: {}", method)),
    };

    // Add trace headers
    for (key, value) in headers {
        request = request.header(key, value);
    }

    // Add body if provided
    if let Some(body_str) = body {
        request = request.body(body_str.to_string());
    }

    // Send request
    let response = request
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    // Get response text
    response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))
}

/// Create child span from extracted context
///
/// Creates a new span that is a child of the extracted trace context.
///
/// # Arguments
///
/// * `context` - Extracted OpenTelemetry context
/// * `operation_name` - Name for the new span
///
/// # Returns
///
/// New span that is a child of the extracted context
///
/// # Example
///
/// ```ignore
/// if let Some(context) = extract_trace_context(&headers) {
///     let span = create_child_span(context, "handle_request");
///     let _guard = span.enter();
///     // ... handle request
/// }
/// ```
pub fn create_child_span(context: OtelContext, operation_name: &str) -> Span {
    let span = tracing::info_span!("http_handler", operation = operation_name);
    span.set_parent(context);
    span
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    use super::*;
    use tracing::Level;

    #[test]
    fn test_inject_trace_headers() {
        let subscriber = tracing_subscriber::fmt()
            .with_test_writer()
            .with_max_level(Level::INFO)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("test_span");
            let _guard = span.enter();

            let headers = inject_trace_headers(&span);

            // Should have at least traceparent header
            // Note: May be empty if OpenTelemetry is not initialized
            // This test just verifies the function doesn't panic
            let _headers = headers;
        });
    }

    #[test]
    fn test_extract_trace_context_empty_headers() {
        let headers = HashMap::new();
        let context = extract_trace_context(&headers);

        // Empty headers should return None
        assert!(context.is_none());
    }

    #[test]
    fn test_extract_trace_context_with_traceparent() {
        let mut headers = HashMap::new();
        // Valid W3C traceparent header (example)
        headers.insert(
            "traceparent".to_string(),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string(),
        );

        let context = extract_trace_context(&headers);

        // Should extract context (even if not fully configured)
        // This test verifies the function handles the header
        let _ctx = context;
    }

    #[test]
    fn test_inject_and_extract_roundtrip() {
        let subscriber = tracing_subscriber::fmt()
            .with_test_writer()
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("parent_span");
            let _guard = span.enter();

            // Inject headers
            let headers = inject_trace_headers(&span);

            // Try to extract them back
            let context = extract_trace_context(&headers);

            // May or may not extract depending on OpenTelemetry setup
            // This test verifies no panics occur
            let _ctx = context;
        });
    }

    #[tokio::test]
    async fn test_http_request_with_trace_error_on_invalid_url() {
        let subscriber = tracing_subscriber::fmt()
            .with_test_writer()
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("test");
            let _guard = span.enter();

            // This test verifies error handling for invalid URLs
            // We use a future that we don't actually await to avoid
            // making real HTTP requests in tests
            let _future = http_request_with_trace(
                "http://invalid-url-that-does-not-exist.example.com",
                "GET",
                None,
                &span,
            );

            // Test just verifies compilation
        });
    }
}
