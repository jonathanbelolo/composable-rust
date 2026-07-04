//! JWT token validation.
//!
//! This module provides JWT validation using RS256 (RSA) or HS256 (HMAC) algorithms.
//!
//! # Supported Algorithms
//!
//! - **RS256**: RSA signature with SHA-256 (recommended for production)
//! - **HS256**: HMAC with SHA-256 (simpler, for development/testing)
//!
//! # Example
//!
//! ```rust,ignore
//! use composable_rust_pg_gateway::identity::{JwtValidator, JwtConfig};
//!
//! // From environment (reads JWT_SECRET or JWT_PUBLIC_KEY)
//! let config = JwtConfig::from_env()?;
//! let validator = JwtValidator::new(config)?;
//!
//! // Validate a token
//! let identity = validator.validate("eyJ...")?;
//! ```

use super::types::{Claims, Identity};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};

/// JWT validation errors.
#[derive(Debug, thiserror::Error)]
pub enum JwtError {
    /// Token has expired.
    #[error("Token has expired")]
    Expired,

    /// Token signature is invalid.
    #[error("Invalid token signature")]
    InvalidSignature,

    /// Token format is invalid (malformed JWT).
    #[error("Invalid token format: {0}")]
    InvalidFormat(String),

    /// Token claims are invalid.
    #[error("Invalid token claims: {0}")]
    InvalidClaims(String),

    /// Configuration error (e.g., invalid key).
    #[error("JWT configuration error: {0}")]
    Configuration(String),
}

/// JWT validator configuration.
///
/// Supports both symmetric (HS256) and asymmetric (RS256) algorithms.
///
/// # Security
///
/// For production use, always configure:
/// - **Audience** (`aud`): Prevents tokens from other services being accepted
/// - **Issuer** (`iss`): Validates the token source
///
/// # Example
///
/// ```rust,ignore
/// // HS256 with shared secret
/// let config = JwtConfig::with_secret("my-secret-key")
///     .with_audience("my-app")
///     .with_issuer("my-auth-service");
///
/// // RS256 with public key
/// let config = JwtConfig::with_public_key(pem_bytes)?
///     .with_audience("my-app");
///
/// // From environment
/// let config = JwtConfig::from_env()?;
/// ```
#[derive(Clone)]
pub struct JwtConfig {
    /// The decoding key (derived from secret or public key).
    decoding_key: DecodingKey,

    /// Algorithm to use for validation.
    algorithm: Algorithm,

    /// Whether to validate expiration (default: true).
    validate_exp: bool,

    /// Leeway in seconds for expiration validation (default: 0).
    leeway: u64,

    /// Expected audience claim (default: None - not validated).
    audience: Option<String>,

    /// Expected issuer claim (default: None - not validated).
    issuer: Option<String>,
}

impl JwtConfig {
    /// Create configuration with an HS256 secret.
    ///
    /// # Example
    ///
    /// ```rust
    /// use composable_rust_pg_gateway::identity::JwtConfig;
    ///
    /// let config = JwtConfig::with_secret("my-secret-key");
    /// ```
    #[must_use]
    pub fn with_secret(secret: impl AsRef<[u8]>) -> Self {
        Self {
            decoding_key: DecodingKey::from_secret(secret.as_ref()),
            algorithm: Algorithm::HS256,
            validate_exp: true,
            leeway: 0,
            audience: None,
            issuer: None,
        }
    }

    /// Create configuration with an RS256 public key (PEM format).
    ///
    /// # Errors
    ///
    /// Returns an error if the PEM-encoded public key is invalid.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use composable_rust_pg_gateway::identity::JwtConfig;
    ///
    /// let pem = std::fs::read("public_key.pem")?;
    /// let config = JwtConfig::with_public_key_pem(&pem)?;
    /// ```
    pub fn with_public_key_pem(pem: &[u8]) -> Result<Self, JwtError> {
        let decoding_key = DecodingKey::from_rsa_pem(pem)
            .map_err(|e| JwtError::Configuration(format!("Invalid RSA public key: {e}")))?;

        Ok(Self {
            decoding_key,
            algorithm: Algorithm::RS256,
            validate_exp: true,
            leeway: 0,
            audience: None,
            issuer: None,
        })
    }

    /// Create configuration from environment variables.
    ///
    /// Checks the following environment variables in order:
    ///
    /// 1. `JWT_SECRET`: HS256 shared secret
    /// 2. `JWT_PUBLIC_KEY`: RS256 public key (PEM format)
    /// 3. `JWT_PUBLIC_KEY_FILE`: Path to RS256 public key file
    ///
    /// Additionally reads (optional but recommended):
    /// - `JWT_AUDIENCE`: Expected audience claim
    /// - `JWT_ISSUER`: Expected issuer claim
    ///
    /// # Errors
    ///
    /// Returns an error if no valid configuration is found.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// std::env::set_var("JWT_SECRET", "my-secret-key");
    /// std::env::set_var("JWT_AUDIENCE", "my-app");
    /// std::env::set_var("JWT_ISSUER", "my-auth-service");
    /// let config = JwtConfig::from_env()?;
    /// ```
    pub fn from_env() -> Result<Self, JwtError> {
        // Try HS256 secret first
        let mut config = if let Ok(secret) = std::env::var("JWT_SECRET") {
            Self::with_secret(secret)
        } else if let Ok(pem) = std::env::var("JWT_PUBLIC_KEY") {
            // Try inline public key
            Self::with_public_key_pem(pem.as_bytes())?
        } else if let Ok(path) = std::env::var("JWT_PUBLIC_KEY_FILE") {
            // Try public key file
            let pem = std::fs::read(&path).map_err(|e| {
                JwtError::Configuration(format!("Failed to read JWT_PUBLIC_KEY_FILE '{path}': {e}"))
            })?;
            Self::with_public_key_pem(&pem)?
        } else {
            return Err(JwtError::Configuration(
                "No JWT configuration found. Set JWT_SECRET, JWT_PUBLIC_KEY, or JWT_PUBLIC_KEY_FILE"
                    .to_string(),
            ));
        };

        // Read optional audience and issuer
        if let Ok(audience) = std::env::var("JWT_AUDIENCE") {
            config.audience = Some(audience);
        }
        if let Ok(issuer) = std::env::var("JWT_ISSUER") {
            config.issuer = Some(issuer);
        }

        Ok(config)
    }

    /// Set expiration validation leeway in seconds.
    ///
    /// Allows tokens to be valid for this many seconds after expiration.
    /// Useful for handling clock skew between servers.
    #[must_use]
    pub const fn with_leeway(mut self, seconds: u64) -> Self {
        self.leeway = seconds;
        self
    }

    /// Disable expiration validation.
    ///
    /// **Warning**: Only use this for testing or when expiration is
    /// validated elsewhere.
    #[must_use]
    pub const fn without_exp_validation(mut self) -> Self {
        self.validate_exp = false;
        self
    }

    /// Set the expected audience claim.
    ///
    /// Tokens without a matching `aud` claim will be rejected.
    /// **Recommended for production** to prevent cross-service token misuse.
    ///
    /// # Example
    ///
    /// ```rust
    /// use composable_rust_pg_gateway::identity::JwtConfig;
    ///
    /// let config = JwtConfig::with_secret("secret")
    ///     .with_audience("my-api");
    /// ```
    #[must_use]
    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    /// Set the expected issuer claim.
    ///
    /// Tokens without a matching `iss` claim will be rejected.
    /// **Recommended for production** to validate token source.
    ///
    /// # Example
    ///
    /// ```rust
    /// use composable_rust_pg_gateway::identity::JwtConfig;
    ///
    /// let config = JwtConfig::with_secret("secret")
    ///     .with_issuer("my-auth-service");
    /// ```
    #[must_use]
    pub fn with_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.issuer = Some(issuer.into());
        self
    }
}

impl std::fmt::Debug for JwtConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtConfig")
            .field("algorithm", &self.algorithm)
            .field("validate_exp", &self.validate_exp)
            .field("leeway", &self.leeway)
            .field("audience", &self.audience)
            .field("issuer", &self.issuer)
            .field("decoding_key", &"[REDACTED]")
            .finish()
    }
}

/// JWT token validator.
///
/// Validates JWT tokens and extracts [`Identity`] from valid tokens.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::identity::{JwtValidator, JwtConfig};
///
/// let config = JwtConfig::with_secret("my-secret-key");
/// let validator = JwtValidator::new(config);
///
/// match validator.validate("eyJ...") {
///     Ok(identity) => println!("User: {}", identity.user_id),
///     Err(e) => println!("Invalid token: {}", e),
/// }
/// ```
#[derive(Debug, Clone)]
pub struct JwtValidator {
    config: JwtConfig,
}

impl JwtValidator {
    /// Create a new JWT validator with the given configuration.
    #[must_use]
    pub const fn new(config: JwtConfig) -> Self {
        Self { config }
    }

    /// Validate a JWT token and extract the identity.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Token format is invalid
    /// - Token signature is invalid
    /// - Token has expired (if validation enabled)
    /// - Required claims are missing
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let identity = validator.validate("eyJ...")?;
    /// println!("User ID: {}", identity.user_id);
    /// ```
    pub fn validate(&self, token: &str) -> Result<Identity, JwtError> {
        let mut validation = Validation::new(self.config.algorithm);
        validation.validate_exp = self.config.validate_exp;
        validation.leeway = self.config.leeway;

        // Start with no required spec claims
        validation.required_spec_claims.clear();

        // Configure audience validation if set
        if let Some(ref aud) = self.config.audience {
            validation.set_audience(&[aud]);
            // Require the aud claim to be present
            validation.required_spec_claims.insert("aud".to_string());
        }

        // Configure issuer validation if set
        if let Some(ref iss) = self.config.issuer {
            validation.set_issuer(&[iss]);
            // Require the iss claim to be present
            validation.required_spec_claims.insert("iss".to_string());
        }

        let token_data =
            decode::<Claims>(token, &self.config.decoding_key, &validation).map_err(|e| match e
                .kind()
            {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => JwtError::Expired,
                jsonwebtoken::errors::ErrorKind::InvalidSignature => JwtError::InvalidSignature,
                jsonwebtoken::errors::ErrorKind::InvalidAudience => {
                    JwtError::InvalidClaims("Invalid audience".to_string())
                },
                jsonwebtoken::errors::ErrorKind::InvalidIssuer => {
                    JwtError::InvalidClaims("Invalid issuer".to_string())
                },
                jsonwebtoken::errors::ErrorKind::MissingRequiredClaim(claim) => {
                    // Map missing aud/iss to more specific messages
                    if claim == "aud" {
                        JwtError::InvalidClaims("Missing required audience claim".to_string())
                    } else if claim == "iss" {
                        JwtError::InvalidClaims("Missing required issuer claim".to_string())
                    } else {
                        JwtError::InvalidClaims(format!("Missing required claim: {claim}"))
                    }
                },
                jsonwebtoken::errors::ErrorKind::InvalidToken
                | jsonwebtoken::errors::ErrorKind::InvalidAlgorithm
                | jsonwebtoken::errors::ErrorKind::Base64(_)
                | jsonwebtoken::errors::ErrorKind::Json(_)
                | jsonwebtoken::errors::ErrorKind::Utf8(_) => {
                    JwtError::InvalidFormat(e.to_string())
                },
                _ => JwtError::InvalidClaims(e.to_string()),
            })?;

        // Validate required fields
        let claims = token_data.claims;
        if claims.sub.is_empty() {
            return Err(JwtError::InvalidClaims(
                "Missing required claim: sub".to_string(),
            ));
        }
        if claims.tenant_id.is_empty() {
            return Err(JwtError::InvalidClaims(
                "Missing required claim: tenant_id".to_string(),
            ));
        }

        Ok(claims.into_identity())
    }

    /// Validate a JWT token and return the raw claims.
    ///
    /// Use this when you need access to all claims, not just identity.
    ///
    /// # Errors
    ///
    /// Same as [`validate`](Self::validate).
    pub fn validate_claims(&self, token: &str) -> Result<Claims, JwtError> {
        let mut validation = Validation::new(self.config.algorithm);
        validation.validate_exp = self.config.validate_exp;
        validation.leeway = self.config.leeway;

        // Start with no required spec claims
        validation.required_spec_claims.clear();

        // Configure audience validation if set
        if let Some(ref aud) = self.config.audience {
            validation.set_audience(&[aud]);
            // Require the aud claim to be present
            validation.required_spec_claims.insert("aud".to_string());
        }

        // Configure issuer validation if set
        if let Some(ref iss) = self.config.issuer {
            validation.set_issuer(&[iss]);
            // Require the iss claim to be present
            validation.required_spec_claims.insert("iss".to_string());
        }

        let token_data =
            decode::<Claims>(token, &self.config.decoding_key, &validation).map_err(|e| match e
                .kind()
            {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => JwtError::Expired,
                jsonwebtoken::errors::ErrorKind::InvalidSignature => JwtError::InvalidSignature,
                jsonwebtoken::errors::ErrorKind::InvalidAudience => {
                    JwtError::InvalidClaims("Invalid audience".to_string())
                },
                jsonwebtoken::errors::ErrorKind::InvalidIssuer => {
                    JwtError::InvalidClaims("Invalid issuer".to_string())
                },
                jsonwebtoken::errors::ErrorKind::MissingRequiredClaim(claim) => {
                    // Map missing aud/iss to more specific messages
                    if claim == "aud" {
                        JwtError::InvalidClaims("Missing required audience claim".to_string())
                    } else if claim == "iss" {
                        JwtError::InvalidClaims("Missing required issuer claim".to_string())
                    } else {
                        JwtError::InvalidClaims(format!("Missing required claim: {claim}"))
                    }
                },
                jsonwebtoken::errors::ErrorKind::InvalidToken
                | jsonwebtoken::errors::ErrorKind::InvalidAlgorithm
                | jsonwebtoken::errors::ErrorKind::Base64(_)
                | jsonwebtoken::errors::ErrorKind::Json(_)
                | jsonwebtoken::errors::ErrorKind::Utf8(_) => {
                    JwtError::InvalidFormat(e.to_string())
                },
                _ => JwtError::InvalidClaims(e.to_string()),
            })?;

        Ok(token_data.claims)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::{Deserialize, Serialize};

    fn create_test_token(claims: &Claims, secret: &str) -> String {
        encode(
            &Header::default(),
            claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    fn valid_claims() -> Claims {
        let now = chrono::Utc::now().timestamp();
        Claims {
            sub: "user-123".to_string(),
            tenant_id: "tenant-456".to_string(),
            roles: vec!["admin".to_string()],
            iat: now,
            exp: now + 3600, // 1 hour from now
            jti: "token-789".to_string(),
        }
    }

    #[test]
    fn validate_valid_token() {
        let secret = "test-secret-key";
        let claims = valid_claims();
        let token = create_test_token(&claims, secret);

        let config = JwtConfig::with_secret(secret);
        let validator = JwtValidator::new(config);

        let identity = validator.validate(&token).unwrap();
        assert_eq!(identity.user_id, "user-123");
        assert_eq!(identity.tenant_id, "tenant-456");
        assert_eq!(identity.roles, vec!["admin"]);
    }

    #[test]
    fn validate_expired_token() {
        let secret = "test-secret-key";
        let mut claims = valid_claims();
        claims.exp = chrono::Utc::now().timestamp() - 3600; // 1 hour ago

        let token = create_test_token(&claims, secret);

        let config = JwtConfig::with_secret(secret);
        let validator = JwtValidator::new(config);

        let result = validator.validate(&token);
        assert!(matches!(result, Err(JwtError::Expired)));
    }

    #[test]
    fn validate_wrong_secret() {
        let claims = valid_claims();
        let token = create_test_token(&claims, "correct-secret");

        let config = JwtConfig::with_secret("wrong-secret");
        let validator = JwtValidator::new(config);

        let result = validator.validate(&token);
        assert!(matches!(result, Err(JwtError::InvalidSignature)));
    }

    #[test]
    fn validate_malformed_token() {
        let config = JwtConfig::with_secret("test-secret");
        let validator = JwtValidator::new(config);

        let result = validator.validate("not-a-valid-jwt");
        assert!(matches!(result, Err(JwtError::InvalidFormat(_))));
    }

    #[test]
    fn validate_missing_sub() {
        let secret = "test-secret-key";
        let mut claims = valid_claims();
        claims.sub = String::new();

        let token = create_test_token(&claims, secret);

        let config = JwtConfig::with_secret(secret);
        let validator = JwtValidator::new(config);

        let result = validator.validate(&token);
        assert!(matches!(result, Err(JwtError::InvalidClaims(_))));
    }

    #[test]
    fn validate_missing_tenant_id() {
        let secret = "test-secret-key";
        let mut claims = valid_claims();
        claims.tenant_id = String::new();

        let token = create_test_token(&claims, secret);

        let config = JwtConfig::with_secret(secret);
        let validator = JwtValidator::new(config);

        let result = validator.validate(&token);
        assert!(matches!(result, Err(JwtError::InvalidClaims(_))));
    }

    #[test]
    fn validate_with_leeway() {
        let secret = "test-secret-key";
        let mut claims = valid_claims();
        claims.exp = chrono::Utc::now().timestamp() - 10; // 10 seconds ago

        let token = create_test_token(&claims, secret);

        // Without leeway - should fail
        let config = JwtConfig::with_secret(secret);
        let validator = JwtValidator::new(config);
        assert!(matches!(validator.validate(&token), Err(JwtError::Expired)));

        // With 60 second leeway - should succeed
        let config = JwtConfig::with_secret(secret).with_leeway(60);
        let validator = JwtValidator::new(config);
        assert!(validator.validate(&token).is_ok());
    }

    #[test]
    fn validate_without_exp_validation() {
        let secret = "test-secret-key";
        let mut claims = valid_claims();
        claims.exp = chrono::Utc::now().timestamp() - 3600; // 1 hour ago

        let token = create_test_token(&claims, secret);

        let config = JwtConfig::with_secret(secret).without_exp_validation();
        let validator = JwtValidator::new(config);

        // Should succeed even though expired
        let identity = validator.validate(&token).unwrap();
        assert_eq!(identity.user_id, "user-123");
    }

    #[test]
    fn validate_claims_returns_full_claims() {
        let secret = "test-secret-key";
        let original_claims = valid_claims();
        let token = create_test_token(&original_claims, secret);

        let config = JwtConfig::with_secret(secret);
        let validator = JwtValidator::new(config);

        let claims = validator.validate_claims(&token).unwrap();
        assert_eq!(claims.sub, original_claims.sub);
        assert_eq!(claims.tenant_id, original_claims.tenant_id);
        assert_eq!(claims.roles, original_claims.roles);
        assert_eq!(claims.jti, original_claims.jti);
    }

    #[test]
    fn config_debug_redacts_key() {
        let config = JwtConfig::with_secret("super-secret-key");
        let debug = format!("{config:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret-key"));
    }

    // Claims with audience and issuer for testing
    #[derive(Debug, Serialize, Deserialize)]
    struct ClaimsWithAudIss {
        sub: String,
        tenant_id: String,
        roles: Vec<String>,
        iat: i64,
        exp: i64,
        jti: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        aud: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        iss: Option<String>,
    }

    fn create_token_with_aud_iss(claims: &ClaimsWithAudIss, secret: &str) -> String {
        encode(
            &Header::default(),
            claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    fn valid_claims_with_aud_iss() -> ClaimsWithAudIss {
        let now = chrono::Utc::now().timestamp();
        ClaimsWithAudIss {
            sub: "user-123".to_string(),
            tenant_id: "tenant-456".to_string(),
            roles: vec!["admin".to_string()],
            iat: now,
            exp: now + 3600,
            jti: "token-789".to_string(),
            aud: None,
            iss: None,
        }
    }

    #[test]
    fn validate_with_correct_audience() {
        let secret = "test-secret-key";
        let mut claims = valid_claims_with_aud_iss();
        claims.aud = Some("my-app".to_string());
        let token = create_token_with_aud_iss(&claims, secret);

        let config = JwtConfig::with_secret(secret).with_audience("my-app");
        let validator = JwtValidator::new(config);

        let identity = validator.validate(&token).unwrap();
        assert_eq!(identity.user_id, "user-123");
    }

    #[test]
    fn validate_with_wrong_audience() {
        let secret = "test-secret-key";
        let mut claims = valid_claims_with_aud_iss();
        claims.aud = Some("other-app".to_string());
        let token = create_token_with_aud_iss(&claims, secret);

        let config = JwtConfig::with_secret(secret).with_audience("my-app");
        let validator = JwtValidator::new(config);

        let result = validator.validate(&token);
        assert!(matches!(result, Err(JwtError::InvalidClaims(msg)) if msg.contains("audience")));
    }

    #[test]
    fn validate_with_missing_audience_when_required() {
        let secret = "test-secret-key";
        let claims = valid_claims_with_aud_iss(); // No aud set
        let token = create_token_with_aud_iss(&claims, secret);

        let config = JwtConfig::with_secret(secret).with_audience("my-app");
        let validator = JwtValidator::new(config);

        let result = validator.validate(&token);
        // Token without audience should be rejected when audience is required
        assert!(
            matches!(result, Err(JwtError::InvalidClaims(ref msg)) if msg.to_lowercase().contains("audience"))
        );
    }

    #[test]
    fn validate_with_correct_issuer() {
        let secret = "test-secret-key";
        let mut claims = valid_claims_with_aud_iss();
        claims.iss = Some("my-auth-service".to_string());
        let token = create_token_with_aud_iss(&claims, secret);

        let config = JwtConfig::with_secret(secret).with_issuer("my-auth-service");
        let validator = JwtValidator::new(config);

        let identity = validator.validate(&token).unwrap();
        assert_eq!(identity.user_id, "user-123");
    }

    #[test]
    fn validate_with_wrong_issuer() {
        let secret = "test-secret-key";
        let mut claims = valid_claims_with_aud_iss();
        claims.iss = Some("other-service".to_string());
        let token = create_token_with_aud_iss(&claims, secret);

        let config = JwtConfig::with_secret(secret).with_issuer("my-auth-service");
        let validator = JwtValidator::new(config);

        let result = validator.validate(&token);
        assert!(matches!(result, Err(JwtError::InvalidClaims(msg)) if msg.contains("issuer")));
    }

    #[test]
    fn validate_with_missing_issuer_when_required() {
        let secret = "test-secret-key";
        let claims = valid_claims_with_aud_iss(); // No iss set
        let token = create_token_with_aud_iss(&claims, secret);

        let config = JwtConfig::with_secret(secret).with_issuer("my-auth-service");
        let validator = JwtValidator::new(config);

        let result = validator.validate(&token);
        // Token without issuer should be rejected when issuer is required
        assert!(
            matches!(result, Err(JwtError::InvalidClaims(ref msg)) if msg.to_lowercase().contains("issuer"))
        );
    }

    #[test]
    fn validate_with_both_audience_and_issuer() {
        let secret = "test-secret-key";
        let mut claims = valid_claims_with_aud_iss();
        claims.aud = Some("my-app".to_string());
        claims.iss = Some("my-auth-service".to_string());
        let token = create_token_with_aud_iss(&claims, secret);

        let config = JwtConfig::with_secret(secret)
            .with_audience("my-app")
            .with_issuer("my-auth-service");
        let validator = JwtValidator::new(config);

        let identity = validator.validate(&token).unwrap();
        assert_eq!(identity.user_id, "user-123");
    }

    #[test]
    fn validate_claims_with_audience_validation() {
        let secret = "test-secret-key";
        let mut claims = valid_claims_with_aud_iss();
        claims.aud = Some("my-app".to_string());
        let token = create_token_with_aud_iss(&claims, secret);

        let config = JwtConfig::with_secret(secret).with_audience("my-app");
        let validator = JwtValidator::new(config);

        // Should work
        let result = validator.validate_claims(&token);
        assert!(result.is_ok());

        // Wrong audience should fail
        let config_wrong = JwtConfig::with_secret(secret).with_audience("other-app");
        let validator_wrong = JwtValidator::new(config_wrong);

        let result_wrong = validator_wrong.validate_claims(&token);
        assert!(
            matches!(result_wrong, Err(JwtError::InvalidClaims(msg)) if msg.contains("audience"))
        );
    }

    #[test]
    fn config_builder_sets_audience_and_issuer() {
        let config = JwtConfig::with_secret("secret")
            .with_audience("my-audience")
            .with_issuer("my-issuer");

        let debug = format!("{config:?}");
        assert!(debug.contains("my-audience"));
        assert!(debug.contains("my-issuer"));
    }
}
