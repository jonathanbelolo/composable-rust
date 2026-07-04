//! Magic link authentication handlers.
//!
//! Implements passwordless authentication via email magic links.
//! All business logic (token generation, validation, session creation)
//! is handled by `PostgreSQL` stored procedures.
//!
//! # Security Requirements
//!
//! ## Rate Limiting (REQUIRED)
//!
//! **This handler does NOT implement rate limiting.** You MUST add rate limiting
//! at the infrastructure or application layer to prevent abuse:
//!
//! - **Per-IP limiting**: Prevent attackers from brute-forcing tokens or
//!   spamming emails from a single source. Recommend: 10 requests/minute per IP.
//! - **Per-email limiting**: Prevent email flooding attacks against specific
//!   addresses. Recommend: 3 magic link requests per hour per email.
//! - **Global limiting**: Prevent distributed attacks. Set appropriate ceiling.
//!
//! ### Implementation Options
//!
//! 1. **Reverse proxy** (nginx, Cloudflare, etc.): Best for per-IP limiting
//! 2. **Tower middleware**: Use `tower_governor` crate
//! 3. **Database-backed**: Store request counts in `PostgreSQL` for persistent limits
//!
//! ### Example with `tower_governor`
//!
//! ```rust,ignore
//! use tower_governor::{GovernorLayer, GovernorConfigBuilder};
//!
//! let governor_config = GovernorConfigBuilder::default()
//!     .per_second(10)
//!     .burst_size(20)
//!     .finish()
//!     .unwrap();
//!
//! let app = Router::new()
//!     .route("/api/auth/magic-link", post(request_magic_link))
//!     .layer(GovernorLayer { config: governor_config });
//! ```
//!
//! ## Other Security Measures (Built-in)
//!
//! - **Timing attack mitigation**: Random delays prevent user enumeration
//! - **Email validation**: Basic format checks before hitting the database
//! - **Open redirect prevention**: Only relative URLs allowed for redirects
//! - **Short TTL**: 10-minute default token expiration
//! - **Secure cookies**: `HttpOnly`, `Secure`, `SameSite=Lax` by default

use crate::ApiError;
use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use cookie::{Cookie, SameSite};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

// ═══════════════════════════════════════════════════════════════════════════
// Request/Response Types
// ═══════════════════════════════════════════════════════════════════════════

/// Request body for magic link generation.
///
/// # Example
///
/// ```json
/// {
///     "email": "user@example.com"
/// }
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct MagicLinkRequest {
    /// Email address to send the magic link to.
    pub email: String,
}

/// Maximum allowed email length (RFC 5321).
const MAX_EMAIL_LENGTH: usize = 254;

/// Validate redirect URL to prevent open redirect attacks.
///
/// Only allows relative URLs (starting with `/`) to prevent attackers
/// from crafting magic links that redirect to phishing sites.
///
/// # Security
///
/// Open redirect vulnerabilities allow attackers to:
/// - Create phishing attacks using legitimate domain's magic links
/// - Steal session tokens via redirect to malicious sites
/// - Bypass security controls that check the referring domain
fn validate_redirect_url(url: &str) -> Result<(), &'static str> {
    if url.is_empty() {
        return Err("Redirect URL cannot be empty");
    }

    // Must start with / for relative path
    if !url.starts_with('/') {
        return Err("Redirect URL must be a relative path starting with '/'");
    }

    // Prevent protocol-relative URLs (//evil.com)
    if url.starts_with("//") {
        return Err("Redirect URL cannot be protocol-relative");
    }

    // Check for dangerous characters that could break out
    if url.contains("://") {
        return Err("Redirect URL cannot contain protocol specifier");
    }

    Ok(())
}

/// Validate email format.
///
/// Performs basic validation:
/// - Not empty
/// - Not too long (max 254 chars per RFC 5321)
/// - Contains exactly one @ symbol
/// - Has non-empty local and domain parts
/// - Domain contains at least one dot
///
/// Note: This is intentionally simple. Full RFC 5322 validation is complex
/// and the `PostgreSQL` function should do final validation.
fn validate_email(email: &str) -> Result<(), &'static str> {
    // Check length
    if email.is_empty() {
        return Err("Email cannot be empty");
    }
    if email.len() > MAX_EMAIL_LENGTH {
        return Err("Email is too long");
    }

    // Basic format check
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 {
        return Err("Email must contain exactly one @ symbol");
    }

    let local = parts[0];
    let domain = parts[1];

    if local.is_empty() {
        return Err("Email local part cannot be empty");
    }
    if domain.is_empty() {
        return Err("Email domain cannot be empty");
    }
    if !domain.contains('.') {
        return Err("Email domain must contain a dot");
    }

    // Check for obvious invalid characters
    if email.contains(char::is_whitespace) {
        return Err("Email cannot contain whitespace");
    }

    Ok(())
}

/// Query parameters for magic link verification.
///
/// # Example
///
/// ```text
/// GET /auth/verify?token=abc123xyz
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct VerifyParams {
    /// The magic link token from the email.
    pub token: String,
}

/// Response from successful magic link request.
#[derive(Debug, Clone, Serialize)]
pub struct MagicLinkResponse {
    /// Confirmation message.
    pub message: String,
}

/// Result from `PostgreSQL` `auth.create_magic_link()` function.
#[derive(Debug, sqlx::FromRow)]
struct MagicLinkResult {
    /// Generated token (sent via email).
    #[allow(dead_code)]
    token: String,

    /// Token expiration time.
    #[allow(dead_code)]
    expires_at: DateTime<Utc>,
}

/// Result from `PostgreSQL` `auth.verify_magic_link()` function.
#[derive(Debug, sqlx::FromRow)]
struct SessionResult {
    /// Session ID to store in cookie.
    session_id: String,

    /// User ID.
    #[allow(dead_code)]
    user_id: String,

    /// Tenant ID.
    #[allow(dead_code)]
    tenant_id: String,

    /// User roles.
    #[allow(dead_code)]
    roles: Vec<String>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Configuration
// ═══════════════════════════════════════════════════════════════════════════

/// Configuration for magic link authentication.
///
/// # Example
///
/// ```rust
/// use composable_rust_pg_gateway::auth::MagicLinkConfig;
///
/// let config = MagicLinkConfig::new()
///     .with_redirect_url("/app/dashboard")
///     .with_cookie_secure(true)
///     .with_ttl_seconds(1800); // 30 minutes
/// ```
#[derive(Debug, Clone)]
pub struct MagicLinkConfig {
    /// Session cookie name (without any prefix).
    pub cookie_name: String,

    /// Whether the cookie requires HTTPS.
    pub cookie_secure: bool,

    /// Whether the cookie is inaccessible to JavaScript.
    pub cookie_http_only: bool,

    /// Cookie `SameSite` policy.
    pub cookie_same_site: SameSite,

    /// Cookie max age in days.
    pub cookie_max_age_days: i64,

    /// URL to redirect to after successful verification.
    pub redirect_url: String,

    /// Magic link token TTL in seconds.
    pub ttl_seconds: i64,

    /// Whether to use the `__Host-` cookie prefix.
    ///
    /// When enabled, the cookie name will be prefixed with `__Host-` which
    /// provides enhanced security by requiring:
    /// - The `Secure` flag (HTTPS only)
    /// - `Path=/` (root path)
    /// - No `Domain` attribute (prevents subdomain attacks)
    ///
    /// This prevents:
    /// - Subdomain hijacking attacks
    /// - Cookie injection via insecure channels
    /// - Session fixation via sibling domains
    ///
    /// **Default**: `false` (opt-in, as it may break some deployments)
    pub use_host_prefix: bool,
}

impl Default for MagicLinkConfig {
    fn default() -> Self {
        Self {
            cookie_name: "session".to_string(),
            cookie_secure: true,
            cookie_http_only: true,
            cookie_same_site: SameSite::Lax,
            cookie_max_age_days: 30,
            redirect_url: "/dashboard".to_string(),
            ttl_seconds: 600, // 10 minutes - short TTL for security
            use_host_prefix: false, // Opt-in for compatibility
        }
    }
}

impl MagicLinkConfig {
    /// Create a new configuration with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for custom configuration.
    #[must_use]
    pub const fn builder() -> MagicLinkConfigBuilder {
        MagicLinkConfigBuilder::new()
    }

    /// Set the session cookie name.
    #[must_use]
    pub fn with_cookie_name(mut self, name: impl Into<String>) -> Self {
        self.cookie_name = name.into();
        self
    }

    /// Set whether the cookie requires HTTPS.
    #[must_use]
    pub const fn with_cookie_secure(mut self, secure: bool) -> Self {
        self.cookie_secure = secure;
        self
    }

    /// Set whether the cookie is HTTP-only.
    #[must_use]
    pub const fn with_cookie_http_only(mut self, http_only: bool) -> Self {
        self.cookie_http_only = http_only;
        self
    }

    /// Set the cookie `SameSite` policy.
    #[must_use]
    pub const fn with_cookie_same_site(mut self, same_site: SameSite) -> Self {
        self.cookie_same_site = same_site;
        self
    }

    /// Set the cookie max age in days.
    #[must_use]
    pub const fn with_cookie_max_age_days(mut self, days: i64) -> Self {
        self.cookie_max_age_days = days;
        self
    }

    /// Set the redirect URL after successful verification.
    ///
    /// # Security
    ///
    /// **Only relative URLs are allowed** to prevent open redirect attacks.
    /// The URL should start with `/` and not contain protocol specifiers.
    ///
    /// # Validation
    ///
    /// The URL is validated at runtime in [`verify_magic_link`] to prevent open
    /// redirect attacks. Invalid URLs will result in a 400 Bad Request response.
    /// Use `validate_redirect_url` (crate-internal) to validate URLs before configuration if needed.
    ///
    /// # Example
    ///
    /// ```rust
    /// use composable_rust_pg_gateway::auth::MagicLinkConfig;
    ///
    /// let config = MagicLinkConfig::new()
    ///     .with_redirect_url("/app/dashboard");  // OK - relative path
    /// ```
    #[must_use]
    pub fn with_redirect_url(mut self, url: impl Into<String>) -> Self {
        self.redirect_url = url.into();
        self
    }

    /// Set the magic link TTL in seconds.
    #[must_use]
    pub const fn with_ttl_seconds(mut self, seconds: i64) -> Self {
        self.ttl_seconds = seconds;
        self
    }

    /// Enable or disable the `__Host-` cookie prefix.
    ///
    /// When enabled, the session cookie will be prefixed with `__Host-`,
    /// which provides enhanced security against subdomain attacks.
    ///
    /// **Requirements when enabled:**
    /// - HTTPS must be used (`cookie_secure` should be true)
    /// - The cookie path will be set to `/`
    /// - No Domain attribute is set
    ///
    /// # Example
    ///
    /// ```rust
    /// use composable_rust_pg_gateway::auth::MagicLinkConfig;
    ///
    /// // Enable __Host- prefix for maximum security
    /// let config = MagicLinkConfig::new()
    ///     .with_host_prefix(true);
    /// // Cookie name becomes "__Host-session"
    /// ```
    #[must_use]
    pub const fn with_host_prefix(mut self, use_prefix: bool) -> Self {
        self.use_host_prefix = use_prefix;
        self
    }
}

/// Builder for [`MagicLinkConfig`].
#[derive(Debug, Default)]
pub struct MagicLinkConfigBuilder {
    cookie_name: Option<String>,
    cookie_secure: Option<bool>,
    cookie_http_only: Option<bool>,
    cookie_same_site: Option<SameSite>,
    cookie_max_age_days: Option<i64>,
    redirect_url: Option<String>,
    ttl_seconds: Option<i64>,
    use_host_prefix: Option<bool>,
}

impl MagicLinkConfigBuilder {
    /// Create a new builder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cookie_name: None,
            cookie_secure: None,
            cookie_http_only: None,
            cookie_same_site: None,
            cookie_max_age_days: None,
            redirect_url: None,
            ttl_seconds: None,
            use_host_prefix: None,
        }
    }

    /// Set the session cookie name.
    #[must_use]
    pub fn cookie_name(mut self, name: impl Into<String>) -> Self {
        self.cookie_name = Some(name.into());
        self
    }

    /// Set whether the cookie requires HTTPS.
    #[must_use]
    pub const fn cookie_secure(mut self, secure: bool) -> Self {
        self.cookie_secure = Some(secure);
        self
    }

    /// Set whether the cookie is HTTP-only.
    #[must_use]
    pub const fn cookie_http_only(mut self, http_only: bool) -> Self {
        self.cookie_http_only = Some(http_only);
        self
    }

    /// Set the cookie `SameSite` policy.
    #[must_use]
    pub const fn cookie_same_site(mut self, same_site: SameSite) -> Self {
        self.cookie_same_site = Some(same_site);
        self
    }

    /// Set the cookie max age in days.
    #[must_use]
    pub const fn cookie_max_age_days(mut self, days: i64) -> Self {
        self.cookie_max_age_days = Some(days);
        self
    }

    /// Set the redirect URL after successful verification.
    ///
    /// The URL is validated at runtime in [`verify_magic_link`] to prevent open
    /// redirect attacks.
    #[must_use]
    pub fn redirect_url(mut self, url: impl Into<String>) -> Self {
        self.redirect_url = Some(url.into());
        self
    }

    /// Set the magic link TTL in seconds.
    #[must_use]
    pub const fn ttl_seconds(mut self, seconds: i64) -> Self {
        self.ttl_seconds = Some(seconds);
        self
    }

    /// Enable or disable the `__Host-` cookie prefix.
    #[must_use]
    pub const fn use_host_prefix(mut self, use_prefix: bool) -> Self {
        self.use_host_prefix = Some(use_prefix);
        self
    }

    /// Build the configuration.
    #[must_use]
    pub fn build(self) -> MagicLinkConfig {
        let default = MagicLinkConfig::default();
        MagicLinkConfig {
            cookie_name: self.cookie_name.unwrap_or(default.cookie_name),
            cookie_secure: self.cookie_secure.unwrap_or(default.cookie_secure),
            cookie_http_only: self.cookie_http_only.unwrap_or(default.cookie_http_only),
            cookie_same_site: self.cookie_same_site.unwrap_or(default.cookie_same_site),
            cookie_max_age_days: self.cookie_max_age_days.unwrap_or(default.cookie_max_age_days),
            redirect_url: self.redirect_url.unwrap_or(default.redirect_url),
            ttl_seconds: self.ttl_seconds.unwrap_or(default.ttl_seconds),
            use_host_prefix: self.use_host_prefix.unwrap_or(default.use_host_prefix),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Request a magic link email.
///
/// This handler calls `PostgreSQL`'s `auth.create_magic_link()` function which:
/// 1. Validates the email and checks if user exists
/// 2. Generates a secure token
/// 3. Stores the token with expiration
/// 4. Queues an email via the outbox pattern
///
/// # Route
///
/// `POST /api/auth/magic-link`
///
/// # Request Body
///
/// ```json
/// {
///     "email": "user@example.com"
/// }
/// ```
///
/// # Response
///
/// - `200 OK` - Always returned to prevent user enumeration
///
/// # Security
///
/// This handler always returns `200 OK` with the same message regardless of
/// whether the email is registered. This prevents attackers from discovering
/// which email addresses have accounts (user enumeration).
///
/// ## Rate Limiting Warning ⚠️
///
/// **This handler does NOT implement rate limiting.** Without rate limiting,
/// attackers can:
/// - Spam thousands of emails to arbitrary addresses
/// - Brute-force magic link tokens
/// - Cause reputation damage to your email sending domain
///
/// You MUST add rate limiting at the infrastructure or middleware layer.
/// See the module documentation for implementation options.
///
/// # Errors
///
/// Returns [`ApiError`] only for:
/// - Database connection failures (500 Internal Server Error)
/// - Invalid email format (`P0001` → 400 Bad Request)
///
/// # Example
///
/// ```rust,ignore
/// use axum::{Router, routing::post};
/// use composable_rust_pg_gateway::auth::request_magic_link;
///
/// let app = Router::new()
///     .route("/auth/magic-link", post(request_magic_link))
///     .with_state(app_state);
/// ```
pub async fn request_magic_link(
    State(pool): State<PgPool>,
    State(config): State<MagicLinkConfig>,
    Json(req): Json<MagicLinkRequest>,
) -> Result<(StatusCode, Json<MagicLinkResponse>), ApiError> {
    // Validate email format before database call
    if let Err(msg) = validate_email(&req.email) {
        return Err(ApiError::BadRequest(msg.to_string()));
    }

    // Call PostgreSQL - it handles user lookup, token generation, email queuing
    let result: Result<MagicLinkResult, _> = sqlx::query_as(
        "SELECT token, expires_at FROM auth.create_magic_link($1, $2)",
    )
    .bind(&req.email)
    .bind(config.ttl_seconds)
    .fetch_one(&pool)
    .await;

    // Security: Always return success to prevent user enumeration.
    // Only propagate errors that don't reveal user existence.
    match result {
        Ok(_) => {
            tracing::debug!(email = %req.email, "Magic link created");
        }
        Err(sqlx::Error::RowNotFound | sqlx::Error::Database(_)) => {
            // User not found or PostgreSQL returned P0404 - don't reveal this.
            // Add random delay to mitigate timing attacks.
            // The delay simulates the time it would take to:
            // 1. Generate a token
            // 2. Store it in the database
            // 3. Queue an email
            //
            // Using 50-150ms range based on typical production latencies.
            let delay_ms = 50 + (rand::random::<u64>() % 100);
            tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            tracing::debug!(email = %req.email, "Magic link requested for unknown email");
        }
        Err(e) => {
            // Other errors (connection issues) should be propagated
            return Err(e.into());
        }
    }

    Ok((
        StatusCode::OK,
        Json(MagicLinkResponse {
            message: "If an account exists for this email, a magic link has been sent.".to_string(),
        }),
    ))
}

/// Verify a magic link token and create a session.
///
/// This handler calls `PostgreSQL`'s `auth.verify_magic_link()` function which:
/// 1. Validates the token exists and is not expired
/// 2. Marks the token as used (single-use)
/// 3. Creates a session
/// 4. Returns session details
///
/// After successful verification, this handler:
/// 1. Sets a session cookie
/// 2. Redirects to the configured URL
///
/// # Route
///
/// `GET /api/auth/verify?token=xxx`
///
/// # Response
///
/// - `303 See Other` - Success, redirects with session cookie
/// - `401 Unauthorized` - Invalid or expired token (`P0401`)
///
/// # Errors
///
/// Returns [`ApiError`] if:
/// - The token is invalid or expired (`P0401` → 401 Unauthorized)
/// - Database connection fails (500 Internal Server Error)
///
/// # Example
///
/// ```rust,ignore
/// use axum::{Router, routing::get};
/// use composable_rust_pg_gateway::auth::verify_magic_link;
///
/// let app = Router::new()
///     .route("/auth/verify", get(verify_magic_link))
///     .with_state(app_state);
/// ```
pub async fn verify_magic_link(
    State(pool): State<PgPool>,
    State(config): State<MagicLinkConfig>,
    Query(params): Query<VerifyParams>,
) -> Result<Response, ApiError> {
    // Defense-in-depth: validate redirect URL even though config should have validated it
    if let Err(msg) = validate_redirect_url(&config.redirect_url) {
        tracing::error!(url = %config.redirect_url, error = %msg, "Invalid redirect URL in config");
        return Err(ApiError::Internal);
    }

    // Call PostgreSQL - it validates token, creates session
    let session: SessionResult = sqlx::query_as(
        "SELECT session_id, user_id, tenant_id, roles FROM auth.verify_magic_link($1)",
    )
    .bind(&params.token)
    .fetch_one(&pool)
    .await?;

    // Build session cookie
    // Apply __Host- prefix if enabled for enhanced security
    let cookie_name = if config.use_host_prefix {
        format!("__Host-{}", config.cookie_name)
    } else {
        config.cookie_name.clone()
    };

    let cookie = Cookie::build((cookie_name, session.session_id))
        .http_only(config.cookie_http_only)
        .secure(config.cookie_secure)
        .same_site(config.cookie_same_site)
        .max_age(cookie::time::Duration::days(config.cookie_max_age_days))
        .path("/")
        .build();

    tracing::info!(
        user_id = %session.user_id,
        tenant_id = %session.tenant_id,
        "Magic link verified, session created"
    );

    // Redirect to app with session cookie
    Ok((
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, cookie.to_string()),
            (header::LOCATION, config.redirect_url.clone()),
        ],
    )
        .into_response())
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let config = MagicLinkConfig::default();

        assert_eq!(config.cookie_name, "session");
        assert!(config.cookie_secure);
        assert!(config.cookie_http_only);
        assert_eq!(config.cookie_same_site, SameSite::Lax);
        assert_eq!(config.cookie_max_age_days, 30);
        assert_eq!(config.redirect_url, "/dashboard");
        assert_eq!(config.ttl_seconds, 600); // 10 minutes for security
    }

    #[test]
    fn config_builder() {
        let config = MagicLinkConfig::builder()
            .cookie_name("my_session")
            .cookie_secure(false)
            .cookie_http_only(false)
            .cookie_same_site(SameSite::Strict)
            .cookie_max_age_days(7)
            .redirect_url("/app")
            .ttl_seconds(600)
            .build();

        assert_eq!(config.cookie_name, "my_session");
        assert!(!config.cookie_secure);
        assert!(!config.cookie_http_only);
        assert_eq!(config.cookie_same_site, SameSite::Strict);
        assert_eq!(config.cookie_max_age_days, 7);
        assert_eq!(config.redirect_url, "/app");
        assert_eq!(config.ttl_seconds, 600);
    }

    #[test]
    fn config_fluent_api() {
        let config = MagicLinkConfig::new()
            .with_cookie_name("custom")
            .with_cookie_secure(false)
            .with_redirect_url("/home");

        assert_eq!(config.cookie_name, "custom");
        assert!(!config.cookie_secure);
        assert_eq!(config.redirect_url, "/home");
    }

    #[test]
    fn builder_partial_override() {
        let config = MagicLinkConfig::builder()
            .cookie_name("partial")
            .build();

        // Overridden
        assert_eq!(config.cookie_name, "partial");
        // Defaults preserved
        assert!(config.cookie_secure);
        assert_eq!(config.ttl_seconds, 600);
    }

    #[test]
    fn magic_link_request_deserialize() {
        let json = r#"{"email": "test@example.com"}"#;
        let req: MagicLinkRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.email, "test@example.com");
    }

    #[test]
    fn verify_params_deserialize() {
        let json = r#"{"token": "abc123"}"#;
        let params: VerifyParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.token, "abc123");
    }

    #[test]
    fn magic_link_response_serialize() {
        let response = MagicLinkResponse {
            message: "Check your email".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("Check your email"));
    }

    // Email validation tests
    #[test]
    fn email_validation_valid() {
        assert!(validate_email("user@example.com").is_ok());
        assert!(validate_email("user.name@example.com").is_ok());
        assert!(validate_email("user+tag@example.com").is_ok());
        assert!(validate_email("user@sub.example.com").is_ok());
    }

    #[test]
    fn email_validation_empty() {
        assert!(validate_email("").is_err());
    }

    #[test]
    fn email_validation_no_at_symbol() {
        assert!(validate_email("userexample.com").is_err());
    }

    #[test]
    fn email_validation_multiple_at_symbols() {
        assert!(validate_email("user@@example.com").is_err());
        assert!(validate_email("user@middle@example.com").is_err());
    }

    #[test]
    fn email_validation_empty_local_part() {
        assert!(validate_email("@example.com").is_err());
    }

    #[test]
    fn email_validation_empty_domain() {
        assert!(validate_email("user@").is_err());
    }

    #[test]
    fn email_validation_domain_no_dot() {
        assert!(validate_email("user@localhost").is_err());
    }

    #[test]
    fn email_validation_whitespace() {
        assert!(validate_email("user @example.com").is_err());
        assert!(validate_email("user@ example.com").is_err());
        assert!(validate_email(" user@example.com").is_err());
        assert!(validate_email("user@example.com ").is_err());
    }

    #[test]
    fn email_validation_too_long() {
        let long_email = format!("{}@example.com", "a".repeat(300));
        assert!(validate_email(&long_email).is_err());
    }

    // Redirect URL validation tests
    #[test]
    fn redirect_url_validation_valid() {
        assert!(validate_redirect_url("/").is_ok());
        assert!(validate_redirect_url("/dashboard").is_ok());
        assert!(validate_redirect_url("/app/dashboard").is_ok());
        assert!(validate_redirect_url("/path?query=value").is_ok());
        assert!(validate_redirect_url("/path#anchor").is_ok());
    }

    #[test]
    fn redirect_url_validation_empty() {
        assert!(validate_redirect_url("").is_err());
    }

    #[test]
    fn redirect_url_validation_absolute_url() {
        assert!(validate_redirect_url("https://evil.com").is_err());
        assert!(validate_redirect_url("http://evil.com").is_err());
        assert!(validate_redirect_url("ftp://evil.com").is_err());
    }

    #[test]
    fn redirect_url_validation_protocol_relative() {
        assert!(validate_redirect_url("//evil.com").is_err());
        assert!(validate_redirect_url("//evil.com/path").is_err());
    }

    #[test]
    fn redirect_url_validation_no_leading_slash() {
        assert!(validate_redirect_url("dashboard").is_err());
        assert!(validate_redirect_url("path/to/page").is_err());
    }

    #[test]
    fn redirect_url_validation_protocol_in_path() {
        // Even if it starts with /, reject if it contains protocol
        assert!(validate_redirect_url("/redirect?url=https://evil.com").is_err());
    }

    #[test]
    fn invalid_redirect_url_accepted_by_builder() {
        // Builder accepts invalid URLs - validation happens at runtime in handler
        // This is intentional: handler performs defense-in-depth validation
        let config = MagicLinkConfig::new().with_redirect_url("https://evil.com");
        assert_eq!(config.redirect_url, "https://evil.com");

        let config2 = MagicLinkConfig::builder()
            .redirect_url("//evil.com")
            .build();
        assert_eq!(config2.redirect_url, "//evil.com");
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // __Host- Prefix Tests
    // ═══════════════════════════════════════════════════════════════════════════

    #[test]
    fn host_prefix_disabled_by_default() {
        let config = MagicLinkConfig::default();
        assert!(!config.use_host_prefix);
    }

    #[test]
    fn host_prefix_enabled_via_method() {
        let config = MagicLinkConfig::new().with_host_prefix(true);
        assert!(config.use_host_prefix);
    }

    #[test]
    fn host_prefix_enabled_via_builder() {
        let config = MagicLinkConfig::builder()
            .use_host_prefix(true)
            .build();
        assert!(config.use_host_prefix);
    }

    #[test]
    fn host_prefix_can_be_disabled() {
        let config = MagicLinkConfig::new()
            .with_host_prefix(true)
            .with_host_prefix(false);
        assert!(!config.use_host_prefix);
    }
}
