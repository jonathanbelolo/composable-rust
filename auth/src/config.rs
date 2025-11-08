//! Authentication configuration.
//!
//! This module provides configuration structures for all authentication reducers.
//! Configuration values should be provided by the application, not hardcoded.

use chrono::Duration;

/// Magic Link authentication configuration.
#[derive(Debug, Clone)]
pub struct MagicLinkConfig {
    /// Base URL for magic link generation (e.g., "https://app.example.com").
    ///
    /// Magic links will be formatted as: `{base_url}/auth/verify?token={token}`
    pub base_url: String,

    /// Token time-to-live in minutes.
    ///
    /// Default: 10 minutes
    pub token_ttl_minutes: i64,

    /// Session duration after successful authentication.
    ///
    /// Default: 24 hours
    pub session_duration: Duration,
}

impl MagicLinkConfig {
    /// Create new Magic Link configuration.
    ///
    /// # Arguments
    ///
    /// * `base_url` - Base URL for your application (e.g., "https://app.example.com")
    #[must_use]
    pub const fn new(base_url: String) -> Self {
        Self {
            base_url,
            token_ttl_minutes: 10,
            session_duration: Duration::hours(24),
        }
    }

    /// Set token time-to-live.
    #[must_use]
    pub const fn with_token_ttl(mut self, minutes: i64) -> Self {
        self.token_ttl_minutes = minutes;
        self
    }

    /// Set session duration.
    #[must_use]
    pub const fn with_session_duration(mut self, duration: Duration) -> Self {
        self.session_duration = duration;
        self
    }
}

impl Default for MagicLinkConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:3000".to_string(),
            token_ttl_minutes: 10,
            session_duration: Duration::hours(24),
        }
    }
}

/// OAuth2/OIDC authentication configuration.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// Base URL for OAuth redirect URI (e.g., "https://app.example.com").
    ///
    /// Redirect URI will be: `{base_url}/auth/oauth/callback`
    pub base_url: String,

    /// CSRF state time-to-live in minutes.
    ///
    /// Default: 5 minutes
    pub state_ttl_minutes: i64,

    /// Session duration after successful authentication.
    ///
    /// Default: 24 hours
    pub session_duration: Duration,
}

impl OAuthConfig {
    /// Create new OAuth configuration.
    ///
    /// # Arguments
    ///
    /// * `base_url` - Base URL for your application (e.g., "https://app.example.com")
    #[must_use]
    pub const fn new(base_url: String) -> Self {
        Self {
            base_url,
            state_ttl_minutes: 5,
            session_duration: Duration::hours(24),
        }
    }

    /// Set CSRF state time-to-live.
    #[must_use]
    pub const fn with_state_ttl(mut self, minutes: i64) -> Self {
        self.state_ttl_minutes = minutes;
        self
    }

    /// Set session duration.
    #[must_use]
    pub const fn with_session_duration(mut self, duration: Duration) -> Self {
        self.session_duration = duration;
        self
    }
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:3000".to_string(),
            state_ttl_minutes: 5,
            session_duration: Duration::hours(24),
        }
    }
}

/// WebAuthn/Passkey authentication configuration.
#[derive(Debug, Clone)]
pub struct PasskeyConfig {
    /// Expected origin for WebAuthn (e.g., "https://app.example.com").
    ///
    /// Must match the origin in the client-side WebAuthn call.
    pub origin: String,

    /// Relying Party ID (e.g., "app.example.com").
    ///
    /// Must be a valid domain. Usually the domain portion of the origin.
    pub rp_id: String,

    /// Challenge time-to-live in minutes.
    ///
    /// Default: 5 minutes
    pub challenge_ttl_minutes: i64,

    /// Session duration after successful authentication.
    ///
    /// Default: 24 hours
    pub session_duration: Duration,
}

impl PasskeyConfig {
    /// Create new Passkey configuration.
    ///
    /// # Arguments
    ///
    /// * `origin` - Expected origin (e.g., "https://app.example.com")
    /// * `rp_id` - Relying Party ID (e.g., "app.example.com")
    #[must_use]
    pub const fn new(origin: String, rp_id: String) -> Self {
        Self {
            origin,
            rp_id,
            challenge_ttl_minutes: 5,
            session_duration: Duration::hours(24),
        }
    }

    /// Set challenge time-to-live.
    #[must_use]
    pub const fn with_challenge_ttl(mut self, minutes: i64) -> Self {
        self.challenge_ttl_minutes = minutes;
        self
    }

    /// Set session duration.
    #[must_use]
    pub const fn with_session_duration(mut self, duration: Duration) -> Self {
        self.session_duration = duration;
        self
    }
}

impl Default for PasskeyConfig {
    fn default() -> Self {
        Self {
            origin: "http://localhost:3000".to_string(),
            rp_id: "localhost".to_string(),
            challenge_ttl_minutes: 5,
            session_duration: Duration::hours(24),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_magic_link_config_builder() {
        let config = MagicLinkConfig::new("https://example.com".to_string())
            .with_token_ttl(15)
            .with_session_duration(Duration::hours(48));

        assert_eq!(config.base_url, "https://example.com");
        assert_eq!(config.token_ttl_minutes, 15);
        assert_eq!(config.session_duration, Duration::hours(48));
    }

    #[test]
    fn test_oauth_config_builder() {
        let config = OAuthConfig::new("https://example.com".to_string())
            .with_state_ttl(10)
            .with_session_duration(Duration::hours(12));

        assert_eq!(config.base_url, "https://example.com");
        assert_eq!(config.state_ttl_minutes, 10);
        assert_eq!(config.session_duration, Duration::hours(12));
    }

    #[test]
    fn test_passkey_config_builder() {
        let config = PasskeyConfig::new(
            "https://example.com".to_string(),
            "example.com".to_string(),
        )
        .with_challenge_ttl(3)
        .with_session_duration(Duration::hours(6));

        assert_eq!(config.origin, "https://example.com");
        assert_eq!(config.rp_id, "example.com");
        assert_eq!(config.challenge_ttl_minutes, 3);
        assert_eq!(config.session_duration, Duration::hours(6));
    }

    #[test]
    fn test_default_configs() {
        let magic_link = MagicLinkConfig::default();
        assert_eq!(magic_link.base_url, "http://localhost:3000");
        assert_eq!(magic_link.token_ttl_minutes, 10);

        let oauth = OAuthConfig::default();
        assert_eq!(oauth.base_url, "http://localhost:3000");
        assert_eq!(oauth.state_ttl_minutes, 5);

        let passkey = PasskeyConfig::default();
        assert_eq!(passkey.origin, "http://localhost:3000");
        assert_eq!(passkey.rp_id, "localhost");
        assert_eq!(passkey.challenge_ttl_minutes, 5);
    }
}
