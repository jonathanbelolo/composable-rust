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
use crate::environment::AuthEnvironment;
use crate::providers::{OAuth2Provider, UserRepository, DeviceRepository, SessionStore, RiskCalculator, EmailProvider, WebAuthnProvider};
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
pub struct OAuthReducer<O, E, W, S, U, D, R>
where
    O: OAuth2Provider + Clone + 'static,
    E: EmailProvider + Clone + 'static,
    W: WebAuthnProvider + Clone + 'static,
    S: SessionStore + Clone + 'static,
    U: UserRepository + Clone + 'static,
    D: DeviceRepository + Clone + 'static,
    R: RiskCalculator + Clone + 'static,
{
    /// Base URL for OAuth redirects (e.g., "https://app.example.com").
    pub base_url: String,

    /// Session TTL in hours (default: 24).
    pub session_ttl_hours: i64,

    /// Phantom data to hold type parameters.
    _phantom: std::marker::PhantomData<(O, E, W, S, U, D, R)>,
}

impl<O, E, W, S, U, D, R> OAuthReducer<O, E, W, S, U, D, R>
where
    O: OAuth2Provider + Clone + 'static,
    E: EmailProvider + Clone + 'static,
    W: WebAuthnProvider + Clone + 'static,
    S: SessionStore + Clone + 'static,
    U: UserRepository + Clone + 'static,
    D: DeviceRepository + Clone + 'static,
    R: RiskCalculator + Clone + 'static,
{
    /// Create a new OAuth reducer.
    #[must_use]
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            session_ttl_hours: 24,
            _phantom: std::marker::PhantomData,
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

impl<O, E, W, S, U, D, R> Reducer for OAuthReducer<O, E, W, S, U, D, R>
where
    O: OAuth2Provider + Clone + 'static,
    E: EmailProvider + Clone + 'static,
    W: WebAuthnProvider + Clone + 'static,
    S: SessionStore + Clone + 'static,
    U: UserRepository + Clone + 'static,
    D: DeviceRepository + Clone + 'static,
    R: RiskCalculator + Clone + 'static,
{
    type State = AuthState;
    type Action = AuthAction;
    type Environment = AuthEnvironment<O, E, W, S, U, D, R>;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
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

                // Build redirect URL and redirect user to OAuth provider
                let redirect_uri = self.redirect_uri();
                let oauth_provider = env.oauth.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // Build authorization URL
                    match oauth_provider.build_authorization_url(provider, &state_param, &redirect_uri).await {
                        Ok(_auth_url) => {
                            // In a real implementation, this would trigger an HTTP redirect
                            // For now, we'll emit a "redirect ready" action
                            // The web framework integration will handle the actual redirect
                            None // TODO: Return action with redirect URL when we have HTTP effects
                        }
                        Err(_) => {
                            Some(AuthAction::OAuthFailed {
                                error: "url_generation_failed".to_string(),
                                error_description: Some("Failed to generate OAuth authorization URL".to_string()),
                            })
                        }
                    }
                }))]
            }

            // ═══════════════════════════════════════════════════════════════════
            // Handle OAuth Callback
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::OAuthCallback {
                code,
                state: state_param,
                ip_address,
                user_agent,
            } => {
                // Validate CSRF state parameter
                let Some(oauth_state) = &state.oauth_state else {
                    // No OAuth state → CSRF attack or expired session
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(AuthAction::OAuthFailed {
                            error: "no_state".to_string(),
                            error_description: Some("No OAuth state found".to_string()),
                        })
                    }))];
                };

                if oauth_state.state_param != state_param {
                    // State mismatch → CSRF attack
                    state.oauth_state = None; // Clear state
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(AuthAction::OAuthFailed {
                            error: "invalid_state".to_string(),
                            error_description: Some("CSRF state validation failed".to_string()),
                        })
                    }))];
                }

                // Check if OAuth state is expired (5 minutes)
                let now = Utc::now();
                let age = now.signed_duration_since(oauth_state.initiated_at);
                if age > Duration::minutes(5) {
                    // State expired
                    state.oauth_state = None;
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(AuthAction::OAuthFailed {
                            error: "state_expired".to_string(),
                            error_description: Some("OAuth state has expired".to_string()),
                        })
                    }))];
                }

                // State is valid - exchange code for access token
                let provider = oauth_state.provider;
                state.oauth_state = None; // Clear state (one-time use)

                // Exchange authorization code for access token
                let redirect_uri = self.redirect_uri();
                let oauth_provider = env.oauth.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // Exchange code for token
                    match oauth_provider.exchange_code(provider, &code, &redirect_uri).await {
                        Ok(token_response) => {
                            // Fetch user info with access token
                            match oauth_provider.fetch_user_info(provider, &token_response.access_token).await {
                                Ok(user_info) => {
                                    Some(AuthAction::OAuthSuccess {
                                        email: user_info.email,
                                        name: user_info.name,
                                        provider,
                                        access_token: token_response.access_token,
                                        refresh_token: token_response.refresh_token,
                                        ip_address,
                                        user_agent,
                                    })
                                }
                                Err(e) => {
                                    Some(AuthAction::OAuthFailed {
                                        error: "user_info_failed".to_string(),
                                        error_description: Some(format!("Failed to fetch user info: {e}")),
                                    })
                                }
                            }
                        }
                        Err(e) => {
                            Some(AuthAction::OAuthFailed {
                                error: "token_exchange_failed".to_string(),
                                error_description: Some(format!("Failed to exchange code for token: {e}")),
                            })
                        }
                    }
                }))]
            }

            // ═══════════════════════════════════════════════════════════════════
            // OAuth Success (Token Exchange Complete)
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::OAuthSuccess {
                email,
                name,
                provider,
                access_token: _,  // TODO: Store OAuth access token
                refresh_token: _,  // TODO: Store OAuth refresh token
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

                // Execute effects to persist user, device, and session
                let users = env.users.clone();
                let devices = env.devices.clone();
                let sessions = env.sessions.clone();
                let session_clone = session.clone();
                let session_ttl = Duration::hours(self.session_ttl_hours);

                smallvec![Effect::Future(Box::pin(async move {
                    use crate::providers::{User as ProviderUser, Device as ProviderDevice};
                    use crate::actions::DeviceTrustLevel;

                    // 1. Create or get user
                    let final_user = match users.get_user_by_email(&email).await {
                        Ok(existing_user) => {
                            // User exists - use their ID
                            existing_user
                        }
                        Err(_) => {
                            // User doesn't exist - create new user
                            let new_user = ProviderUser {
                                user_id,
                                email: email.clone(),
                                name: name.clone(),
                                email_verified: true,  // OAuth emails are verified
                                created_at: Utc::now(),
                                updated_at: Utc::now(),
                            };

                            match users.create_user(&new_user).await {
                                Ok(created_user) => created_user,
                                Err(_) => {
                                    return Some(AuthAction::OAuthFailed {
                                        error: "user_creation_failed".to_string(),
                                        error_description: Some("Failed to create user".to_string()),
                                    });
                                }
                            }
                        }
                    };

                    // 2. Create device
                    let new_device = ProviderDevice {
                        device_id,
                        user_id: final_user.user_id,
                        name: "Web Browser".to_string(),  // TODO: Parse from user agent
                        device_type: crate::providers::DeviceType::Desktop,
                        platform: user_agent.clone(),
                        first_seen: Utc::now(),
                        last_seen: Utc::now(),
                        trust_level: DeviceTrustLevel::Unknown,
                        passkey_credential_id: None,
                        public_key: None,
                    };

                    if let Err(_) = devices.create_device(&new_device).await {
                        return Some(AuthAction::OAuthFailed {
                            error: "device_creation_failed".to_string(),
                            error_description: Some("Failed to create device".to_string()),
                        });
                    }

                    // 3. Create session in Redis
                    if let Err(_) = sessions.create_session(&session_clone, session_ttl).await {
                        return Some(AuthAction::OAuthFailed {
                            error: "session_creation_failed".to_string(),
                            error_description: Some("Failed to create session".to_string()),
                        });
                    }

                    // 4. Emit session created event
                    Some(AuthAction::SessionCreated {
                        session: session_clone,
                    })
                }))]
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
            // Session Created
            // ═══════════════════════════════════════════════════════════════════
            AuthAction::SessionCreated { session: _ } => {
                // Session is already in state from OAuthSuccess
                // This is the final event - nothing more to do
                // In a real app, this would trigger analytics, webhooks, etc.
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
    // Tests temporarily disabled - will be replaced with proper tests using mock providers
    // See TODO item: "Implement mock OAuth provider for testing"

    // use super::*;
    // use std::net::Ipv4Addr;

    // TODO: Implement mock providers and re-enable tests
}
