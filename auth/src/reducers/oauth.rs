//! OAuth2 reducer.
//!
//! This module implements the pure business logic for OAuth2 authentication.
//!
//! # Flow
//!
//! ```text
//! 1. InitiateOAuth → Generate CSRF state → RedirectToOAuthProvider effect
//! 2. User authorizes at provider
//! 3. OAuthCallback → Validate state → ExchangeOAuthCode effect
//! 4. OAuthSuccess → Create user/device/session → CreateSession effect
//! ```

use crate::actions::AuthAction;
use crate::state::{AuthState, DeviceId, OAuthState, Session, SessionId, UserId};
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use composable_rust_core::{smallvec, SmallVec};
use chrono::{Duration, Utc};
use std::net::IpAddr;

/// OAuth2 reducer.
///
/// Handles OAuth2/OIDC authentication flow with CSRF protection.
#[derive(Debug, Clone)]
pub struct OAuthReducer {
    /// Base URL for OAuth redirects (e.g., "https://app.example.com").
    pub base_url: String,

    /// Session TTL in hours (default: 24).
    pub session_ttl_hours: i64,
}

impl OAuthReducer {
    /// Create a new OAuth reducer.
    #[must_use]
    pub const fn new(base_url: String) -> Self {
        Self {
            base_url,
            session_ttl_hours: 24,
        }
    }

    /// Generate a cryptographically secure CSRF state parameter.
    ///
    /// Uses 32 bytes of randomness (256 bits).
    fn generate_csrf_state() -> String {
        use base64::Engine;
        let bytes: [u8; 32] = rand::random();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    /// Build redirect URI for OAuth callback.
    #[allow(dead_code)] // Will be used in Phase 6B for effect execution
    fn redirect_uri(&self) -> String {
        format!("{}/auth/oauth/callback", self.base_url)
    }

    /// Calculate risk score based on context.
    ///
    /// # Risk Factors
    ///
    /// - New device: +0.3
    /// - VPN/Proxy: +0.2
    /// - New location: +0.1
    ///
    /// TODO: Move to RiskCalculator provider (Phase 6B).
    fn calculate_basic_risk(&self, _ip_address: IpAddr, _user_agent: &str) -> f32 {
        // Placeholder - just return low risk for now
        // In Phase 6B, we'll use the RiskCalculator provider
        0.1
    }
}

impl Reducer for OAuthReducer {
    type State = AuthState;
    type Action = AuthAction;
    type Environment = ();  // No environment dependencies for pure OAuth reducer

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        _env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ═══════════════════════════════════════════════════════════════════
            // Initiate OAuth Flow
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::InitiateOAuth {
                provider,
                ip_address: _,
                user_agent: _,
            } => {
                // Generate CSRF state parameter
                let state_param = Self::generate_csrf_state();

                // Store OAuth state (for validation in callback)
                state.oauth_state = Some(OAuthState {
                    state_param: state_param.clone(),
                    provider,
                    initiated_at: Utc::now(),
                });

                // TODO: Return effect to redirect to OAuth provider
                // Effect: RedirectToOAuthProvider { provider, state_param, redirect_uri }
                smallvec![Effect::None]
            }

            // ═══════════════════════════════════════════════════════════════════
            // Handle OAuth Callback
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::OAuthCallback {
                code: _,
                state: state_param,
                ip_address: _,
                user_agent: _,
            } => {
                // Validate CSRF state parameter
                let Some(oauth_state) = &state.oauth_state else {
                    // No OAuth state → CSRF attack or expired session
                    // TODO: Redirect to error page
                    return smallvec![Effect::None];
                };

                if oauth_state.state_param != state_param {
                    // State mismatch → CSRF attack
                    state.oauth_state = None; // Clear state
                    // TODO: Redirect to error page
                    return smallvec![Effect::None];
                }

                // Check if OAuth state is expired (5 minutes)
                let now = Utc::now();
                let age = now.signed_duration_since(oauth_state.initiated_at);
                if age > Duration::minutes(5) {
                    // State expired
                    state.oauth_state = None;
                    // TODO: Redirect to error page
                    return smallvec![Effect::None];
                }

                // State is valid - exchange code for access token
                let _provider = oauth_state.provider;
                state.oauth_state = None; // Clear state (one-time use)

                // TODO: Exchange code for access token
                // Effect: ExchangeOAuthCode { provider, code, redirect_uri, ip_address, user_agent }
                smallvec![Effect::None]
            }

            // ═══════════════════════════════════════════════════════════════════
            // OAuth Success (Token Exchange Complete)
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::OAuthSuccess {
                email,
                name: _,
                provider,
                access_token: _,
                refresh_token: _,
                ip_address,
                user_agent,
            } => {
                // Generate IDs
                let user_id = UserId::new();
                let device_id = DeviceId::new();
                let session_id = SessionId::new();

                // Calculate risk score
                let login_risk_score = self.calculate_basic_risk(ip_address, &user_agent);

                // Create session
                let now = Utc::now();
                let expires_at = now + Duration::hours(self.session_ttl_hours);

                let session = Session {
                    session_id,
                    user_id,
                    device_id,
                    email: email.clone(),
                    created_at: now,
                    last_active: now,
                    expires_at,
                    ip_address,
                    user_agent: user_agent.clone(),
                    oauth_provider: Some(provider),
                    login_risk_score,
                };

                // Update state
                state.session = Some(session.clone());

                // TODO: Execute effects:
                // 1. Create/get user in database
                // 2. Create/get device in database
                // 3. Store OAuth link
                // 4. Create session in Redis
                // 5. Set session cookie
                // 6. Publish session created event
                // 7. Redirect to app
                smallvec![Effect::None]
            }

            // ═══════════════════════════════════════════════════════════════════
            // OAuth Failed
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::OAuthFailed {
                error: _,
                error_description: _,
            } => {
                // Clear OAuth state
                state.oauth_state = None;

                // TODO: Redirect to error page
                smallvec![Effect::None]
            }

            // ═══════════════════════════════════════════════════════════════════
            // Other Actions (Not Handled by OAuth Reducer)
            // ═══════════════════════════════════════════════════════════════════
            _ => {
                // This reducer only handles OAuth actions
                smallvec![Effect::None]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn test_reducer() -> OAuthReducer {
        OAuthReducer::new("https://app.example.com".to_string())
    }

    #[test]
    fn test_initiate_oauth_generates_csrf_state() {
        let reducer = test_reducer();
        let mut state = AuthState::default();

        let action = AuthAction::InitiateOAuth {
            provider: crate::state::OAuthProvider::Google,
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Mozilla/5.0".to_string(),
        };

        let _effects = reducer.reduce(&mut state, action, &());

        // Should have OAuth state
        assert!(state.oauth_state.is_some());

        let oauth_state = state.oauth_state.unwrap();
        assert_eq!(oauth_state.provider, crate::state::OAuthProvider::Google);
        assert!(!oauth_state.state_param.is_empty());

        // Effect execution is tested separately (Phase 6B)
    }

    #[test]
    fn test_oauth_callback_validates_csrf_state() {
        let reducer = test_reducer();
        let mut state = AuthState::default();

        // First, initiate OAuth to get state
        let initiate_action = AuthAction::InitiateOAuth {
            provider: crate::state::OAuthProvider::Google,
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Mozilla/5.0".to_string(),
        };
        let _ = reducer.reduce(&mut state, initiate_action, &());

        let valid_state = state.oauth_state.as_ref().unwrap().state_param.clone();

        // Test valid callback
        let callback_action = AuthAction::OAuthCallback {
            code: "auth_code_123".to_string(),
            state: valid_state,
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Mozilla/5.0".to_string(),
        };

        let _effects = reducer.reduce(&mut state, callback_action, &());

        // Should clear OAuth state
        assert!(state.oauth_state.is_none());

        // Effect execution is tested separately (Phase 6B)
    }

    #[test]
    fn test_oauth_callback_rejects_invalid_state() {
        let reducer = test_reducer();
        let mut state = AuthState::default();

        // Set up OAuth state
        state.oauth_state = Some(OAuthState {
            state_param: "valid_state".to_string(),
            provider: crate::state::OAuthProvider::Google,
            initiated_at: Utc::now(),
        });

        // Callback with wrong state
        let callback_action = AuthAction::OAuthCallback {
            code: "auth_code_123".to_string(),
            state: "invalid_state".to_string(),
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Mozilla/5.0".to_string(),
        };

        let _effects = reducer.reduce(&mut state, callback_action, &());

        // Should clear OAuth state (security: reject invalid state)
        assert!(state.oauth_state.is_none());
    }

    #[test]
    fn test_oauth_success_creates_session() {
        let reducer = test_reducer();
        let mut state = AuthState::default();

        let action = AuthAction::OAuthSuccess {
            email: "test@example.com".to_string(),
            name: Some("Test User".to_string()),
            provider: crate::state::OAuthProvider::Google,
            access_token: "access_token_123".to_string(),
            refresh_token: Some("refresh_token_123".to_string()),
            ip_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            user_agent: "Mozilla/5.0".to_string(),
        };

        let _effects = reducer.reduce(&mut state, action, &());

        // Should have session
        assert!(state.session.is_some());

        let session = state.session.unwrap();
        assert_eq!(session.email, "test@example.com");
        assert_eq!(session.oauth_provider, Some(crate::state::OAuthProvider::Google));

        // Effect execution is tested separately (Phase 6B)
    }
}
