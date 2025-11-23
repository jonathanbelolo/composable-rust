//! Security headers middleware.
//!
//! ✅ SECURITY FIX (HIGH #9): Add security headers middleware
//!
//! Adds security headers to all HTTP responses:
//! - X-Content-Type-Options: nosniff (prevents MIME type sniffing)
//! - X-Frame-Options: DENY (prevents clickjacking)
//! - X-XSS-Protection: 1; mode=block (enables XSS filter)
//! - Strict-Transport-Security: HSTS (forces HTTPS, 1 year + subdomains)
//! - Content-Security-Policy: Restricts resource loading

use axum::{
    extract::Request,
    http::{header::{HeaderName, HeaderValue}, StatusCode},
    middleware::Next,
    response::IntoResponse,
};

/// Security headers middleware.
///
/// Adds security headers to all responses to protect against common web vulnerabilities:
///
/// - **X-Content-Type-Options: nosniff** - Prevents browsers from MIME-sniffing responses
/// - **X-Frame-Options: DENY** - Prevents page from being embedded in frames (clickjacking protection)
/// - **X-XSS-Protection: 1; mode=block** - Enables XSS filter in older browsers
/// - **Strict-Transport-Security** - Forces HTTPS connections (1 year + includeSubDomains)
/// - **Content-Security-Policy** - Restricts resource loading to same origin
pub async fn security_headers(
    request: Request,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    let mut response = next.run(request).await;

    let headers = response.headers_mut();

    // Prevent MIME type sniffing
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );

    // Prevent clickjacking by disallowing framing
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );

    // Enable XSS protection in legacy browsers
    headers.insert(
        HeaderName::from_static("x-xss-protection"),
        HeaderValue::from_static("1; mode=block"),
    );

    // Force HTTPS for 1 year including subdomains
    // Note: Only use in production with HTTPS enabled
    headers.insert(
        HeaderName::from_static("strict-transport-security"),
        HeaderValue::from_static("max-age=31536000; includeSubDomains"),
    );

    // Content Security Policy: default to same-origin only
    // Note: WebSocket requires 'connect-src' to allow ws:// and wss://
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; connect-src 'self' ws: wss:; frame-ancestors 'none'",
        ),
    );

    Ok(response)
}
