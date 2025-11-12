//! Mock OAuth2 provider for testing.

use crate::error::{AuthError, Result};
use crate::providers::{OAuth2Provider, OAuthUserInfo};
use crate::providers::oauth::OAuthTokenResponse;
use crate::state::OAuthProvider;
use std::future::Future;

/// Mock `OAuth2` provider.
///
/// Returns predefined responses for testing.
#[derive(Debug, Clone)]
pub struct MockOAuth2Provider {
    /// Whether to simulate success or failure.
    pub should_succeed: bool,
}

impl MockOAuth2Provider {
    /// Create a new mock `OAuth2` provider that succeeds.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            should_succeed: true,
        }
    }

    /// Create a mock that will fail requests.
    #[must_use]
    pub const fn failing() -> Self {
        Self {
            should_succeed: false,
        }
    }
}

impl Default for MockOAuth2Provider {
    fn default() -> Self {
        Self::new()
    }
}

impl OAuth2Provider for MockOAuth2Provider {
    fn build_authorization_url(
        &self,
        provider: OAuthProvider,
        state: &str,
        redirect_uri: &str,
    ) -> impl Future<Output = Result<String>> + Send {
        let should_succeed = self.should_succeed;
        let state = state.to_string();
        let redirect_uri = redirect_uri.to_string();

        async move {
            if !should_succeed {
                return Err(AuthError::OAuthCodeInvalid);
            }

            let provider_name = match provider {
                OAuthProvider::Google => "google",
                OAuthProvider::GitHub => "github",
                OAuthProvider::Microsoft => "microsoft",
            };

            Ok(format!(
                "https://{provider_name}.com/oauth/authorize?state={state}&redirect_uri={redirect_uri}"
            ))
        }
    }

    fn exchange_code(
        &self,
        _provider: OAuthProvider,
        _code: &str,
        _redirect_uri: &str,
    ) -> impl Future<Output = Result<OAuthTokenResponse>> + Send {
        let should_succeed = self.should_succeed;

        async move {
            if !should_succeed {
                return Err(AuthError::OAuthCodeInvalid);
            }

            Ok(OAuthTokenResponse {
                access_token: "mock_access_token_123".to_string(),
                refresh_token: Some("mock_refresh_token_456".to_string()),
                expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            })
        }
    }

    fn fetch_user_info(
        &self,
        _provider: OAuthProvider,
        _access_token: &str,
    ) -> impl Future<Output = Result<OAuthUserInfo>> + Send {
        let should_succeed = self.should_succeed;

        async move {
            if !should_succeed {
                return Err(AuthError::InvalidCredentials);
            }

            Ok(OAuthUserInfo {
                provider_user_id: "oauth_user_123".to_string(),
                email: "test@example.com".to_string(),
                email_verified: true,
                name: Some("Test User".to_string()),
                picture: Some("https://example.com/avatar.jpg".to_string()),
            })
        }
    }

    fn refresh_token(
        &self,
        _provider: OAuthProvider,
        _refresh_token: &str,
    ) -> impl Future<Output = Result<OAuthTokenResponse>> + Send {
        let should_succeed = self.should_succeed;

        async move {
            if !should_succeed {
                return Err(AuthError::InvalidRefreshToken);
            }

            Ok(OAuthTokenResponse {
                access_token: "mock_refreshed_access_token".to_string(),
                refresh_token: Some("mock_refreshed_refresh_token".to_string()),
                expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            })
        }
    }
}
