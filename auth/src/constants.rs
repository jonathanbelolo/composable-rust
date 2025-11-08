//! Authentication constants.
//!
//! This module contains constant values used throughout the authentication system.

/// Login method identifiers used in `UserLoggedIn` events.
pub mod login_methods {
    /// Magic link (passwordless email) authentication.
    pub const MAGIC_LINK: &str = "magic_link";

    /// OAuth prefix for OAuth-based authentication.
    ///
    /// Full method format: `oauth_{provider}` (e.g., "oauth_google", "oauth_github").
    pub const OAUTH_PREFIX: &str = "oauth_";

    /// WebAuthn/FIDO2 passkey authentication.
    pub const PASSKEY: &str = "passkey";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_login_method_constants() {
        assert_eq!(login_methods::MAGIC_LINK, "magic_link");
        assert_eq!(login_methods::OAUTH_PREFIX, "oauth_");
        assert_eq!(login_methods::PASSKEY, "passkey");
    }

    #[test]
    fn test_oauth_method_format() {
        // Verify OAuth method format
        let provider = "google";
        let method = format!("{}{}", login_methods::OAUTH_PREFIX, provider);
        assert_eq!(method, "oauth_google");
    }
}
