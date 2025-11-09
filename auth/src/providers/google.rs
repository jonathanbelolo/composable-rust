//! Google OAuth 2.0 provider implementation.

use crate::error::{AuthError, Result};
use crate::providers::{OAuth2Provider, OAuthTokenResponse, OAuthUserInfo};
use crate::state::OAuthProvider;
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Google OAuth 2.0 provider.
///
/// Implements the `OAuth2Provider` trait for Google Identity Platform.
///
/// # Configuration
///
/// To use Google OAuth:
///
/// 1. Create OAuth 2.0 credentials in Google Cloud Console
/// 2. Configure authorized redirect URIs
/// 3. Set environment variables:
///    - `GOOGLE_CLIENT_ID`
///    - `GOOGLE_CLIENT_SECRET`
///
/// # Example
///
/// ```no_run
/// use composable_rust_auth::providers::GoogleOAuthProvider;
///
/// let google = GoogleOAuthProvider::new(
///     "your-client-id".to_string(),
///     "your-client-secret".to_string(),
/// );
/// ```
#[derive(Clone, Debug)]
pub struct GoogleOAuthProvider {
    /// OAuth 2.0 client ID from Google Cloud Console.
    client_id: String,

    /// OAuth 2.0 client secret (keep confidential).
    client_secret: String,

    /// HTTP client for making requests.
    http_client: Client,

    /// Scopes to request (default: "openid email profile").
    scopes: Vec<String>,

    /// Request refresh token for offline access.
    ///
    /// Default: true
    request_refresh_token: bool,

    /// Force consent screen even if user previously authorized.
    ///
    /// Default: false (only show consent on first authorization)
    force_consent: bool,

    /// Support incremental authorization (request additional scopes later).
    ///
    /// Default: true
    incremental_authorization: bool,
}

impl GoogleOAuthProvider {
    /// Create a new Google OAuth provider.
    ///
    /// # Arguments
    ///
    /// * `client_id` - OAuth 2.0 client ID from Google Cloud Console
    /// * `client_secret` - OAuth 2.0 client secret
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use composable_rust_auth::providers::GoogleOAuthProvider;
    /// let google = GoogleOAuthProvider::new(
    ///     std::env::var("GOOGLE_CLIENT_ID").unwrap(),
    ///     std::env::var("GOOGLE_CLIENT_SECRET").unwrap(),
    /// );
    /// ```
    #[must_use]
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client_id,
            client_secret,
            http_client: Client::new(),
            scopes: vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
            ],
            request_refresh_token: true,
            force_consent: false,
            incremental_authorization: true,
        }
    }

    /// Set custom scopes.
    ///
    /// Default scopes are: `openid email profile`
    #[must_use]
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }

    /// Request refresh token for offline access.
    ///
    /// Default: true
    #[must_use]
    pub fn with_refresh_token(mut self, request: bool) -> Self {
        self.request_refresh_token = request;
        self
    }

    /// Force consent screen on every authorization.
    ///
    /// Default: false (only show on first auth)
    #[must_use]
    pub fn with_force_consent(mut self, force: bool) -> Self {
        self.force_consent = force;
        self
    }

    /// Enable incremental authorization.
    ///
    /// Default: true
    #[must_use]
    pub fn with_incremental_authorization(mut self, enable: bool) -> Self {
        self.incremental_authorization = enable;
        self
    }
}

impl OAuth2Provider for GoogleOAuthProvider {
    async fn build_authorization_url(
        &self,
        provider: OAuthProvider,
        state: &str,
        redirect_uri: &str,
    ) -> Result<String> {
        // Only handle Google provider
        if !matches!(provider, OAuthProvider::Google) {
            return Err(AuthError::InvalidOAuthProvider);
        }

        // Build query parameters
        let scope = self.scopes.join(" ");
        let mut params = vec![
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", scope.as_str()),
            ("state", state),
        ];

        if self.request_refresh_token {
            params.push(("access_type", "offline"));
        }

        if self.force_consent {
            params.push(("prompt", "consent"));
        }

        if self.incremental_authorization {
            params.push(("include_granted_scopes", "true"));
        }

        let query = serde_urlencoded::to_string(&params)
            .map_err(|e| AuthError::InternalError(format!("Failed to build URL: {e}")))?;

        Ok(format!(
            "https://accounts.google.com/o/oauth2/v2/auth?{query}"
        ))
    }

    async fn exchange_code(
        &self,
        provider: OAuthProvider,
        code: &str,
        redirect_uri: &str,
    ) -> Result<OAuthTokenResponse> {
        // Only handle Google provider
        if !matches!(provider, OAuthProvider::Google) {
            return Err(AuthError::InvalidOAuthProvider);
        }

        // Build form data
        let params = [
            ("code", code),
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ];

        // Make token exchange request
        let response = self
            .http_client
            .post("https://oauth2.googleapis.com/token")
            .form(&params)
            .send()
            .await
            .map_err(|e| AuthError::OAuthTokenExchangeFailed(e.to_string()))?;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            tracing::error!("Google token exchange failed: {}", error_body);
            return Err(AuthError::OAuthTokenExchangeFailed(
                "Token exchange failed".to_string(),
            ));
        }

        // Parse Google's token response
        let google_response: GoogleTokenResponse = response
            .json()
            .await
            .map_err(|e| AuthError::OAuthTokenExchangeFailed(e.to_string()))?;

        // Convert to standard OAuthTokenResponse
        let expires_at = google_response.expires_in.map(|expires_in| {
            chrono::Utc::now() + chrono::Duration::seconds(i64::from(expires_in))
        });

        Ok(OAuthTokenResponse {
            access_token: google_response.access_token,
            refresh_token: google_response.refresh_token,
            expires_at,
        })
    }

    async fn fetch_user_info(
        &self,
        provider: OAuthProvider,
        access_token: &str,
    ) -> Result<OAuthUserInfo> {
        // Only handle Google provider
        if !matches!(provider, OAuthProvider::Google) {
            return Err(AuthError::InvalidOAuthProvider);
        }

        // Make UserInfo request
        let response = self
            .http_client
            .get("https://openidconnect.googleapis.com/v1/userinfo")
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| AuthError::OAuthUserInfoFailed(e.to_string()))?;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            tracing::error!("Google UserInfo request failed: {}", error_body);
            return Err(AuthError::OAuthUserInfoFailed(
                "UserInfo fetch failed".to_string(),
            ));
        }

        // Parse Google's UserInfo response
        let google_user: GoogleUserInfo = response
            .json()
            .await
            .map_err(|e| AuthError::OAuthUserInfoFailed(e.to_string()))?;

        // Verify email is verified
        if !google_user.email_verified {
            tracing::warn!(
                "Google user email not verified: {}",
                google_user.email
            );
            return Err(AuthError::EmailNotVerified);
        }

        // Convert to standard OAuthUserInfo
        Ok(OAuthUserInfo {
            provider_user_id: google_user.sub,
            email: google_user.email,
            email_verified: google_user.email_verified,
            name: google_user.name,
            picture: google_user.picture,
        })
    }

    async fn refresh_token(
        &self,
        provider: OAuthProvider,
        refresh_token: &str,
    ) -> Result<OAuthTokenResponse> {
        // Only handle Google provider
        if !matches!(provider, OAuthProvider::Google) {
            return Err(AuthError::InvalidOAuthProvider);
        }

        // Build form data
        let params = [
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];

        // Make token refresh request
        let response = self
            .http_client
            .post("https://oauth2.googleapis.com/token")
            .form(&params)
            .send()
            .await
            .map_err(|e| AuthError::OAuthTokenRefreshFailed(e.to_string()))?;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            tracing::error!("Google token refresh failed: {}", error_body);
            return Err(AuthError::OAuthTokenRefreshFailed(
                "Token refresh failed".to_string(),
            ));
        }

        // Parse Google's refresh response
        let google_response: GoogleTokenResponse = response
            .json()
            .await
            .map_err(|e| AuthError::OAuthTokenRefreshFailed(e.to_string()))?;

        // Convert to standard OAuthTokenResponse
        let expires_at = google_response.expires_in.map(|expires_in| {
            chrono::Utc::now() + chrono::Duration::seconds(i64::from(expires_in))
        });

        Ok(OAuthTokenResponse {
            access_token: google_response.access_token,
            refresh_token: None, // Google doesn't return new refresh token
            expires_at,
        })
    }
}

/// Google's token endpoint response format.
///
/// This is the raw response from Google's token endpoint.
/// We convert it to the standard `OAuthTokenResponse` type.
#[derive(Debug, Deserialize, Serialize)]
struct GoogleTokenResponse {
    /// Access token for API requests.
    access_token: String,

    /// Token expiration in seconds (typically 3600 = 1 hour).
    expires_in: Option<u32>,

    /// Refresh token (only on initial authorization with access_type=offline).
    refresh_token: Option<String>,

    /// Granted scopes (space-delimited string).
    #[allow(dead_code)]
    scope: Option<String>,

    /// Token type (always "Bearer").
    #[allow(dead_code)]
    token_type: String,

    /// ID token (JWT) containing user claims (only with openid scope).
    #[allow(dead_code)]
    id_token: Option<String>,
}

/// Google's UserInfo endpoint response format.
///
/// This is the raw response from Google's UserInfo endpoint.
/// We convert it to the standard `OAuthUserInfo` type.
#[derive(Debug, Deserialize, Serialize)]
struct GoogleUserInfo {
    /// Google user ID (stable, unique identifier).
    ///
    /// Example: "110169484474386276334"
    sub: String,

    /// Full name.
    name: Option<String>,

    /// First name.
    #[allow(dead_code)]
    given_name: Option<String>,

    /// Last name.
    #[allow(dead_code)]
    family_name: Option<String>,

    /// Profile picture URL.
    picture: Option<String>,

    /// Email address.
    email: String,

    /// Whether email is verified by Google.
    email_verified: bool,

    /// User's locale (language preference).
    #[allow(dead_code)]
    locale: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_google_provider_creation() {
        let google = GoogleOAuthProvider::new(
            "test_client_id".to_string(),
            "test_secret".to_string(),
        );

        assert_eq!(google.scopes, vec!["openid", "email", "profile"]);
        assert!(google.request_refresh_token);
        assert!(!google.force_consent);
        assert!(google.incremental_authorization);
    }

    #[test]
    fn test_custom_scopes() {
        let google = GoogleOAuthProvider::new(
            "test_client_id".to_string(),
            "test_secret".to_string(),
        )
        .with_scopes(vec!["openid".to_string(), "email".to_string()]);

        assert_eq!(google.scopes, vec!["openid", "email"]);
    }

    #[test]
    fn test_builder_methods() {
        let google = GoogleOAuthProvider::new(
            "test_client_id".to_string(),
            "test_secret".to_string(),
        )
        .with_refresh_token(false)
        .with_force_consent(true)
        .with_incremental_authorization(false);

        assert!(!google.request_refresh_token);
        assert!(google.force_consent);
        assert!(!google.incremental_authorization);
    }

    #[tokio::test]
    async fn test_authorization_url() {
        let google = GoogleOAuthProvider::new(
            "test_client_id".to_string(),
            "test_secret".to_string(),
        );

        let url = google
            .build_authorization_url(
                OAuthProvider::Google,
                "test_state_123",
                "http://localhost:3000/callback",
            )
            .await
            .unwrap();

        assert!(url.contains("client_id=test_client_id"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A3000%2Fcallback"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("scope=openid+email+profile"));
        assert!(url.contains("state=test_state_123"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("include_granted_scopes=true"));
    }

    #[tokio::test]
    async fn test_authorization_url_without_optional_params() {
        let google = GoogleOAuthProvider::new(
            "test_client_id".to_string(),
            "test_secret".to_string(),
        )
        .with_refresh_token(false)
        .with_incremental_authorization(false);

        let url = google
            .build_authorization_url(
                OAuthProvider::Google,
                "test_state",
                "http://localhost:3000/callback",
            )
            .await
            .unwrap();

        assert!(!url.contains("access_type=offline"));
        assert!(!url.contains("include_granted_scopes=true"));
    }

    #[tokio::test]
    async fn test_wrong_provider_returns_error() {
        let google = GoogleOAuthProvider::new(
            "test_client_id".to_string(),
            "test_secret".to_string(),
        );

        let result = google
            .build_authorization_url(
                OAuthProvider::GitHub, // Wrong provider!
                "state",
                "http://localhost/callback",
            )
            .await;

        assert!(matches!(result, Err(AuthError::InvalidOAuthProvider)));
    }

    #[tokio::test]
    async fn test_exchange_code_wrong_provider() {
        let google = GoogleOAuthProvider::new(
            "test_client_id".to_string(),
            "test_secret".to_string(),
        );

        let result = google
            .exchange_code(
                OAuthProvider::GitHub, // Wrong provider!
                "code",
                "http://localhost/callback",
            )
            .await;

        assert!(matches!(result, Err(AuthError::InvalidOAuthProvider)));
    }

    #[tokio::test]
    async fn test_fetch_user_info_wrong_provider() {
        let google = GoogleOAuthProvider::new(
            "test_client_id".to_string(),
            "test_secret".to_string(),
        );

        let result = google
            .fetch_user_info(OAuthProvider::GitHub, "token")
            .await;

        assert!(matches!(result, Err(AuthError::InvalidOAuthProvider)));
    }

    #[tokio::test]
    async fn test_refresh_token_wrong_provider() {
        let google = GoogleOAuthProvider::new(
            "test_client_id".to_string(),
            "test_secret".to_string(),
        );

        let result = google
            .refresh_token(OAuthProvider::GitHub, "refresh_token")
            .await;

        assert!(matches!(result, Err(AuthError::InvalidOAuthProvider)));
    }
}
