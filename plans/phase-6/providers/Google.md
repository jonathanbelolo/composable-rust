# Google OAuth 2.0 / OpenID Connect Implementation Plan

**Provider**: Google Identity Platform
**Documentation**: https://developers.google.com/identity/protocols/oauth2/web-server
**Status**: Planning
**Priority**: HIGH (First OAuth provider implementation)

---

## Overview

Implement Google OAuth 2.0 by creating a `GoogleOAuthProvider` struct that implements the existing `OAuth2Provider` trait. This follows the composable-rust architecture pattern where providers are configuration structs that implement provider traits.

### Key Features

- ✅ **OAuth 2.0 Authorization Code Flow** - Industry standard web flow
- ✅ **OpenID Connect (OIDC)** - ID tokens with user claims
- ✅ **Refresh Tokens** - Long-lived sessions with automatic renewal
- ✅ **Incremental Authorization** - Request additional scopes later
- ✅ **Granular Permissions** - Handle partial scope grants

---

## Architecture Overview

### Existing Infrastructure (Already Implemented)

Your auth system already has:

1. **`OAuth2Provider` trait** (`auth/src/providers/oauth.rs`) - Interface for all OAuth providers
2. **`OAuthReducer`** (`auth/src/reducers/oauth.rs`) - Pure business logic for OAuth flow
3. **`OAuthUserInfo`** and `OAuthTokenResponse`** - Standard response types
4. **`AuthAction::InitiateOAuth` and `AuthAction::OAuthCallback`** - Actions for OAuth flow
5. **CSRF state management** - Atomic state storage via TokenStore

### What We Need to Implement

A single file that implements the `OAuth2Provider` trait for Google:

```
auth/src/providers/google.rs
```

This will contain:
- `GoogleOAuthProvider` struct with configuration
- Implementation of the `OAuth2Provider` trait
- Google-specific endpoint URLs and parameters

---

## Google OAuth 2.0 Endpoints

### Authorization Endpoint
```
https://accounts.google.com/o/oauth2/v2/auth
```

**Required Parameters**:
- `client_id` - OAuth 2.0 client ID from Google Cloud Console
- `redirect_uri` - Where Google sends the response (must be pre-registered)
- `response_type` - Always `code` for server-side flow
- `scope` - Space-delimited list of permissions (e.g., `openid email profile`)
- `state` - CSRF protection token (passed from reducer)

**Optional Parameters**:
- `access_type=offline` - Request refresh token for offline access
- `prompt=consent` - Force consent screen even if previously authorized
- `include_granted_scopes=true` - Incremental authorization support

### Token Endpoint
```
https://oauth2.googleapis.com/token
```

**For code exchange**:
```json
POST https://oauth2.googleapis.com/token
Content-Type: application/x-www-form-urlencoded

code={authorization_code}
&client_id={client_id}
&client_secret={client_secret}
&redirect_uri={redirect_uri}
&grant_type=authorization_code
```

**Response**:
```json
{
  "access_token": "ya29.a0AfH6SMB...",
  "expires_in": 3599,
  "refresh_token": "1//0g...",
  "scope": "openid https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile",
  "token_type": "Bearer",
  "id_token": "eyJhbGciOiJSUzI1NiIsImtpZCI6IjVhNmMxM..."
}
```

**For refresh token**:
```json
POST https://oauth2.googleapis.com/token
Content-Type: application/x-www-form-urlencoded

client_id={client_id}
&client_secret={client_secret}
&refresh_token={refresh_token}
&grant_type=refresh_token
```

### UserInfo Endpoint (OpenID Connect)
```
https://openidconnect.googleapis.com/v1/userinfo
```

**Request**:
```http
GET /v1/userinfo HTTP/1.1
Host: openidconnect.googleapis.com
Authorization: Bearer {access_token}
```

**Response**:
```json
{
  "sub": "110169484474386276334",
  "name": "John Doe",
  "given_name": "John",
  "family_name": "Doe",
  "picture": "https://lh3.googleusercontent.com/a/...",
  "email": "[email protected]",
  "email_verified": true,
  "locale": "en"
}
```

---

## OpenID Connect Scopes

### Basic Scopes

| Scope | Description | Claims Returned |
|-------|-------------|-----------------|
| `openid` | **Required** for OIDC | Enables ID token, returns `sub` (user ID) |
| `email` | User's email address | `email`, `email_verified` |
| `profile` | User's basic profile | `name`, `given_name`, `family_name`, `picture`, `locale` |

**Recommended default**: `openid email profile`

---

## Implementation

### File: `auth/src/providers/google.rs`

```rust
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
        let mut params = vec![
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", &self.scopes.join(" ")),
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
```

### Update `auth/src/providers/mod.rs`

Add Google module:

```rust
pub mod google;
pub use google::GoogleOAuthProvider;
```

### Update `auth/src/error.rs`

Add missing error variants:

```rust
/// Invalid OAuth provider.
#[error("Invalid OAuth provider")]
InvalidOAuthProvider,

/// Email not verified.
#[error("Email not verified by OAuth provider")]
EmailNotVerified,
```

---

## Testing Strategy

### Unit Tests

Add to `auth/src/providers/google.rs`:

```rust
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
}
```

### Integration Tests

Add to `auth/tests/google_oauth.rs`:

```rust
//! Integration tests for Google OAuth.
//!
//! These tests require real Google OAuth credentials.
//! Set environment variables:
//! - GOOGLE_CLIENT_ID
//! - GOOGLE_CLIENT_SECRET

#![cfg(feature = "integration-tests")]

use composable_rust_auth::providers::{GoogleOAuthProvider, OAuth2Provider};
use composable_rust_auth::state::OAuthProvider;

#[tokio::test]
#[ignore] // Requires real credentials
async fn test_google_authorization_url_real() {
    let client_id = std::env::var("GOOGLE_CLIENT_ID")
        .expect("GOOGLE_CLIENT_ID not set");
    let client_secret = std::env::var("GOOGLE_CLIENT_SECRET")
        .expect("GOOGLE_CLIENT_SECRET not set");

    let google = GoogleOAuthProvider::new(client_id, client_secret);

    let url = google
        .build_authorization_url(
            OAuthProvider::Google,
            "test_state",
            "http://localhost:3000/auth/google/callback",
        )
        .await
        .unwrap();

    println!("Visit this URL to authorize: {}", url);
    println!("After authorizing, paste the authorization code:");

    let mut code = String::new();
    std::io::stdin().read_line(&mut code).unwrap();

    let tokens = google
        .exchange_code(
            OAuthProvider::Google,
            code.trim(),
            "http://localhost:3000/auth/google/callback",
        )
        .await
        .unwrap();

    assert!(!tokens.access_token.is_empty());

    let user_info = google
        .fetch_user_info(OAuthProvider::Google, &tokens.access_token)
        .await
        .unwrap();

    println!("Logged in as: {} ({})", user_info.name.unwrap_or_default(), user_info.email);
    assert!(user_info.email_verified);
}
```

---

## Implementation Checklist

### Phase 1: Core Implementation (Week 1)

- [ ] **Create Google provider file** (`auth/src/providers/google.rs`)
  - [ ] `GoogleOAuthProvider` struct
  - [ ] Builder methods (`with_scopes`, etc.)
  - [ ] `GoogleTokenResponse` and `GoogleUserInfo` structs

- [ ] **Implement `OAuth2Provider` trait**
  - [ ] `build_authorization_url()` - Build Google auth URL
  - [ ] `exchange_code()` - Exchange code for tokens
  - [ ] `fetch_user_info()` - Get user profile from Google
  - [ ] `refresh_token()` - Refresh access token

- [ ] **Add error handling**
  - [ ] Add `InvalidOAuthProvider` error
  - [ ] Add `EmailNotVerified` error
  - [ ] Proper error logging (no sensitive data)

- [ ] **Export from module**
  - [ ] Add `pub mod google;` to `providers/mod.rs`
  - [ ] Export `GoogleOAuthProvider`

### Phase 2: Testing (Week 1)

- [ ] **Unit tests**
  - [ ] Provider creation
  - [ ] Authorization URL generation
  - [ ] Wrong provider rejection
  - [ ] Builder pattern methods

- [ ] **Integration tests** (with real credentials)
  - [ ] Full OAuth flow
  - [ ] Token exchange
  - [ ] UserInfo fetch
  - [ ] Refresh token flow

### Phase 3: Documentation (Week 2)

- [ ] **Setup guide**
  - [ ] Google Cloud Console setup
  - [ ] Environment variables
  - [ ] Redirect URI configuration

- [ ] **Code examples**
  - [ ] Basic usage
  - [ ] Custom scopes
  - [ ] Error handling

- [ ] **Security notes**
  - [ ] Client secret protection
  - [ ] State validation (handled by reducer)
  - [ ] Email verification requirement

---

## Google Cloud Console Setup

### Prerequisites

1. **Create Google Cloud Project**
   - Visit: https://console.cloud.google.com/
   - Create new project or select existing

2. **Enable Google+ API**
   - Go to: APIs & Services → Library
   - Search for "Google+ API"
   - Click "Enable"

3. **Create OAuth 2.0 Credentials**
   - Go to: APIs & Services → Credentials
   - Click "Create Credentials" → "OAuth client ID"
   - Application type: "Web application"
   - Name: "Your App Name"

4. **Configure Authorized Redirect URIs**
   - Production: `https://app.example.com/auth/oauth/callback`
   - Development: `http://localhost:3000/auth/oauth/callback`

5. **Save Credentials**
   - Copy `Client ID` → `GOOGLE_CLIENT_ID`
   - Copy `Client Secret` → `GOOGLE_CLIENT_SECRET`

### Environment Variables

```bash
# .env
GOOGLE_CLIENT_ID="123456789-abcdefg.apps.googleusercontent.com"
GOOGLE_CLIENT_SECRET="GOCSPX-..."
```

---

## Example Usage

### Creating the Provider

```rust
use composable_rust_auth::providers::GoogleOAuthProvider;

let google = GoogleOAuthProvider::new(
    std::env::var("GOOGLE_CLIENT_ID").unwrap(),
    std::env::var("GOOGLE_CLIENT_SECRET").unwrap(),
);
```

### Configuring the Environment

```rust
use composable_rust_auth::environment::AuthEnvironment;

let env = AuthEnvironment {
    oauth: Arc::new(google), // GoogleOAuthProvider implements OAuth2Provider
    // ... other providers
};
```

### The Flow (Already Implemented in Reducer)

The OAuth reducer already handles the complete flow:

1. **User clicks "Sign in with Google"**
   - Dispatch: `AuthAction::InitiateOAuth { provider: OAuthProvider::Google, ... }`
   - Reducer calls: `env.oauth.build_authorization_url()`
   - User redirected to Google consent screen

2. **User authorizes and returns**
   - Dispatch: `AuthAction::OAuthCallback { code, state, ... }`
   - Reducer validates state (CSRF protection)
   - Reducer calls: `env.oauth.exchange_code()` → gets tokens
   - Reducer calls: `env.oauth.fetch_user_info()` → gets profile
   - Dispatch: `AuthAction::OAuthSuccess { email, name, ... }`

3. **Session created**
   - Reducer emits events (UserLoggedIn, etc.)
   - Session stored in Redis
   - User logged in

---

## Success Metrics

- [ ] **Authorization URL** contains correct parameters
- [ ] **Token exchange** succeeds with valid code
- [ ] **UserInfo fetch** returns verified email
- [ ] **Email verification** is enforced
- [ ] **Refresh token** flow works
- [ ] **Error handling** provides clear messages
- [ ] **Tests pass** (unit + integration)

---

## Future Enhancements (Post Phase 6)

- **Google One Tap** - Streamlined sign-in widget
- **Automatic Profile Sync** - Keep user data updated
- **Additional Scopes** - Calendar, Drive, etc.
- **Service Account Support** - Server-to-server auth
- **Token Revocation** - Explicit logout from Google

---

## References

- **Web Server Flow**: https://developers.google.com/identity/protocols/oauth2/web-server
- **OpenID Connect**: https://developers.google.com/identity/openid-connect/openid-connect
- **Scopes**: https://developers.google.com/identity/protocols/oauth2/scopes
- **UserInfo Endpoint**: https://openidconnect.googleapis.com/v1/userinfo
- **Error Codes**: https://developers.google.com/identity/protocols/oauth2/web-server#error-codes-for-the-token-endpoint

---

**Status**: Ready for implementation
**Next Steps**: Create `auth/src/providers/google.rs`
**Estimated Time**: 1 week for core implementation + tests
