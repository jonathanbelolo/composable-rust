//! OAuth2/OIDC provider trait.

use crate::error::Result;
use crate::state::OAuthProvider;
use super::OAuthUserInfo;

/// OAuth2/OIDC provider.
///
/// This trait abstracts over OAuth2 provider implementations
/// (Google, GitHub, Microsoft, etc.).
///
/// # Implementation Notes
///
/// - Use the `oauth2` crate for OAuth2 flows
/// - Use the `openidconnect` crate for OIDC flows
/// - Handle provider-specific quirks (scopes, endpoints, etc.)
pub trait OAuth2Provider: Send + Sync {
    /// Build authorization URL.
    ///
    /// # Returns
    ///
    /// The URL to redirect the user to for authorization.
    ///
    /// # Errors
    ///
    /// Returns error if URL construction fails.
    fn build_authorization_url(
        &self,
        provider: OAuthProvider,
        state: &str,
        redirect_uri: &str,
    ) -> impl std::future::Future<Output = Result<String>> + Send;

    /// Exchange authorization code for access token.
    ///
    /// # Returns
    ///
    /// Access token, optional refresh token, and optional expiration.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails
    /// - Provider rejects the code
    /// - Response is malformed
    fn exchange_code(
        &self,
        provider: OAuthProvider,
        code: &str,
        redirect_uri: &str,
    ) -> impl std::future::Future<Output = Result<OAuthTokenResponse>> + Send;

    /// Fetch user info from provider.
    ///
    /// # Returns
    ///
    /// User information from the provider.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails
    /// - Token is invalid
    /// - Response is malformed
    fn fetch_user_info(
        &self,
        provider: OAuthProvider,
        access_token: &str,
    ) -> impl std::future::Future<Output = Result<OAuthUserInfo>> + Send;

    /// Refresh access token.
    ///
    /// # Returns
    ///
    /// New access token and optional expiration.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails
    /// - Refresh token is invalid
    /// - Response is malformed
    fn refresh_token(
        &self,
        provider: OAuthProvider,
        refresh_token: &str,
    ) -> impl std::future::Future<Output = Result<OAuthTokenResponse>> + Send;
}

/// OAuth token response.
#[derive(Debug, Clone, PartialEq)]
pub struct OAuthTokenResponse {
    /// Access token.
    pub access_token: String,

    /// Refresh token (if available).
    pub refresh_token: Option<String>,

    /// Expiration timestamp (if provided).
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}
