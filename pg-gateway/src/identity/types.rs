//! Identity and JWT claims types.
//!
//! This module provides core identity types used throughout the gateway:
//!
//! - [`Identity`]: Validated user identity extracted from JWT or session
//! - [`Claims`]: JWT token claims for validation
//!
//! # Identity Flow
//!
//! ```text
//! JWT Token → Claims (validated) → Identity (used in handlers)
//!                                      │
//!                                      ▼
//!                             PostgreSQL session vars
//!                             (app.user_id, app.tenant_id, app.roles)
//! ```

use serde::{Deserialize, Serialize};

/// Validated user identity.
///
/// This struct represents an authenticated user's identity, extracted from
/// either a JWT token or a server-side session. It is used to:
///
/// 1. Pass identity to `PostgreSQL` via session variables
/// 2. Make authorization decisions in handlers
/// 3. Audit logging
///
/// # Example
///
/// ```rust
/// use composable_rust_pg_gateway::identity::Identity;
///
/// let identity = Identity {
///     user_id: "user-123".to_string(),
///     tenant_id: "tenant-456".to_string(),
///     roles: vec!["admin".to_string(), "user".to_string()],
///     session_id: None,
/// };
///
/// assert!(identity.has_role("admin"));
/// assert!(!identity.has_role("superuser"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// Unique user identifier.
    pub user_id: String,

    /// Tenant identifier for multi-tenancy.
    pub tenant_id: String,

    /// User roles for authorization.
    pub roles: Vec<String>,

    /// Session ID if authenticated via session cookie (not JWT).
    pub session_id: Option<String>,
}

impl Identity {
    /// Create a new identity.
    ///
    /// # Example
    ///
    /// ```rust
    /// use composable_rust_pg_gateway::identity::Identity;
    ///
    /// let identity = Identity::new("user-123", "tenant-456", vec!["user".to_string()]);
    /// ```
    #[must_use]
    pub fn new(
        user_id: impl Into<String>,
        tenant_id: impl Into<String>,
        roles: Vec<String>,
    ) -> Self {
        Self {
            user_id: user_id.into(),
            tenant_id: tenant_id.into(),
            roles,
            session_id: None,
        }
    }

    /// Create identity with a session ID.
    ///
    /// Use this when identity comes from a server-side session rather than JWT.
    #[must_use]
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Check if the user has a specific role.
    ///
    /// # Example
    ///
    /// ```rust
    /// use composable_rust_pg_gateway::identity::Identity;
    ///
    /// let identity = Identity::new("user-123", "tenant-456", vec!["admin".to_string()]);
    /// assert!(identity.has_role("admin"));
    /// assert!(!identity.has_role("superuser"));
    /// ```
    #[must_use]
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// Check if the user has any of the specified roles.
    ///
    /// # Example
    ///
    /// ```rust
    /// use composable_rust_pg_gateway::identity::Identity;
    ///
    /// let identity = Identity::new("user-123", "tenant-456", vec!["editor".to_string()]);
    /// assert!(identity.has_any_role(&["admin", "editor"]));
    /// assert!(!identity.has_any_role(&["admin", "superuser"]));
    /// ```
    #[must_use]
    pub fn has_any_role(&self, roles: &[&str]) -> bool {
        roles.iter().any(|role| self.has_role(role))
    }

    /// Check if the user has all of the specified roles.
    ///
    /// # Example
    ///
    /// ```rust
    /// use composable_rust_pg_gateway::identity::Identity;
    ///
    /// let identity = Identity::new(
    ///     "user-123",
    ///     "tenant-456",
    ///     vec!["admin".to_string(), "editor".to_string()],
    /// );
    /// assert!(identity.has_all_roles(&["admin", "editor"]));
    /// assert!(!identity.has_all_roles(&["admin", "superuser"]));
    /// ```
    #[must_use]
    pub fn has_all_roles(&self, roles: &[&str]) -> bool {
        roles.iter().all(|role| self.has_role(role))
    }
}

/// JWT token claims.
///
/// This struct represents the claims embedded in a JWT token.
/// It follows standard JWT claim names where applicable.
///
/// # Standard Claims
///
/// - `sub`: Subject (user ID)
/// - `iat`: Issued at (Unix timestamp)
/// - `exp`: Expiration time (Unix timestamp)
/// - `jti`: JWT ID (for revocation)
///
/// # Custom Claims
///
/// - `tenant_id`: Multi-tenancy support
/// - `roles`: Authorization roles
///
/// # Example
///
/// ```rust
/// use composable_rust_pg_gateway::identity::Claims;
///
/// let claims = Claims {
///     sub: "user-123".to_string(),
///     tenant_id: "tenant-456".to_string(),
///     roles: vec!["user".to_string()],
///     iat: 1700000000,
///     exp: 1700003600,
///     jti: "token-789".to_string(),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// Subject - the user ID.
    pub sub: String,

    /// Tenant ID for multi-tenancy.
    pub tenant_id: String,

    /// User roles.
    #[serde(default)]
    pub roles: Vec<String>,

    /// Issued at (Unix timestamp).
    pub iat: i64,

    /// Expiration time (Unix timestamp).
    pub exp: i64,

    /// JWT ID - unique identifier for this token.
    ///
    /// Used for token revocation.
    #[serde(default)]
    pub jti: String,
}

impl Claims {
    /// Convert claims to an [`Identity`].
    ///
    /// This is used after JWT validation to create the identity
    /// that will be used throughout the request lifecycle.
    #[must_use]
    pub fn into_identity(self) -> Identity {
        Identity {
            user_id: self.sub,
            tenant_id: self.tenant_id,
            roles: self.roles,
            session_id: None,
        }
    }

    /// Check if the token has expired.
    ///
    /// Compares the expiration time against the provided current timestamp.
    #[must_use]
    pub const fn is_expired(&self, now: i64) -> bool {
        self.exp < now
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn identity_new() {
        let identity = Identity::new("user-123", "tenant-456", vec!["admin".to_string()]);

        assert_eq!(identity.user_id, "user-123");
        assert_eq!(identity.tenant_id, "tenant-456");
        assert_eq!(identity.roles, vec!["admin"]);
        assert!(identity.session_id.is_none());
    }

    #[test]
    fn identity_with_session() {
        let identity =
            Identity::new("user-123", "tenant-456", vec![]).with_session("session-789");

        assert_eq!(identity.session_id, Some("session-789".to_string()));
    }

    #[test]
    fn identity_has_role() {
        let identity = Identity::new(
            "user-123",
            "tenant-456",
            vec!["admin".to_string(), "editor".to_string()],
        );

        assert!(identity.has_role("admin"));
        assert!(identity.has_role("editor"));
        assert!(!identity.has_role("superuser"));
    }

    #[test]
    fn identity_has_any_role() {
        let identity = Identity::new("user-123", "tenant-456", vec!["editor".to_string()]);

        assert!(identity.has_any_role(&["admin", "editor"]));
        assert!(!identity.has_any_role(&["admin", "superuser"]));
    }

    #[test]
    fn identity_has_all_roles() {
        let identity = Identity::new(
            "user-123",
            "tenant-456",
            vec!["admin".to_string(), "editor".to_string()],
        );

        assert!(identity.has_all_roles(&["admin", "editor"]));
        assert!(!identity.has_all_roles(&["admin", "superuser"]));
    }

    #[test]
    fn claims_into_identity() {
        let claims = Claims {
            sub: "user-123".to_string(),
            tenant_id: "tenant-456".to_string(),
            roles: vec!["admin".to_string()],
            iat: 1_700_000_000,
            exp: 1_700_003_600,
            jti: "token-789".to_string(),
        };

        let identity = claims.into_identity();

        assert_eq!(identity.user_id, "user-123");
        assert_eq!(identity.tenant_id, "tenant-456");
        assert_eq!(identity.roles, vec!["admin"]);
        assert!(identity.session_id.is_none());
    }

    #[test]
    fn claims_is_expired() {
        let claims = Claims {
            sub: "user-123".to_string(),
            tenant_id: "tenant-456".to_string(),
            roles: vec![],
            iat: 1_700_000_000,
            exp: 1_700_003_600,
            jti: String::new(),
        };

        // Not expired (exp >= now means valid)
        assert!(!claims.is_expired(1_700_000_000));
        assert!(!claims.is_expired(1_700_003_599));
        assert!(!claims.is_expired(1_700_003_600)); // Boundary: exp == now is not expired

        // Expired (exp < now)
        assert!(claims.is_expired(1_700_003_601));
    }

    #[test]
    fn identity_serialize_deserialize() {
        let identity = Identity::new("user-123", "tenant-456", vec!["admin".to_string()])
            .with_session("session-789");

        let json = serde_json::to_string(&identity).unwrap();
        let deserialized: Identity = serde_json::from_str(&json).unwrap();

        assert_eq!(identity, deserialized);
    }

    #[test]
    fn claims_serialize_deserialize() {
        let claims = Claims {
            sub: "user-123".to_string(),
            tenant_id: "tenant-456".to_string(),
            roles: vec!["admin".to_string()],
            iat: 1_700_000_000,
            exp: 1_700_003_600,
            jti: "token-789".to_string(),
        };

        let json = serde_json::to_string(&claims).unwrap();
        let deserialized: Claims = serde_json::from_str(&json).unwrap();

        assert_eq!(claims, deserialized);
    }
}
