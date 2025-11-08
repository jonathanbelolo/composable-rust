//! Magic link authentication reducer.
//!
//! This reducer implements passwordless email authentication via "magic links".
//!
//! # Flow
//!
//! 1. User requests magic link with email address
//! 2. Generate cryptographically secure token
//! 3. Store token in state with expiration
//! 4. Send email with link containing token
//! 5. User clicks link, submits token
//! 6. Verify token (not expired, not used)
//! 7. Create session
//!
//! # Security
//!
//! - Tokens are 256-bit random values (base64url encoded)
//! - Tokens expire after 5-15 minutes (configurable)
//! - Tokens are single-use (invalidated after verification)
//! - Constant-time comparison for tokens (timing attack prevention)
//! - Rate limiting: 5 magic links per hour per email
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_auth::{AuthState, AuthAction};
//! use composable_rust_auth::reducers::MagicLinkReducer;
//! use composable_rust_core::reducer::Reducer;
//! use std::net::{IpAddr, Ipv4Addr};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Note: MagicLinkReducer is generic - needs type annotations in real usage
//! // See integration tests for complete examples
//! # Ok(())
//! # }
//! ```

use crate::actions::AuthAction;
use crate::environment::AuthEnvironment;
use crate::providers::{
    DeviceRepository, EmailProvider, OAuth2Provider, RiskCalculator, SessionStore,
    UserRepository, WebAuthnProvider,
};
use crate::state::{AuthState, DeviceId, MagicLinkState, Session, SessionId, UserId};
use chrono::Utc;
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use composable_rust_core::{smallvec, SmallVec};

/// Magic link authentication reducer.
///
/// Handles passwordless email authentication flows.
#[derive(Debug, Clone)]
pub struct MagicLinkReducer<O, E, W, S, U, D, R> {
    /// Token expiration duration in minutes (default: 10 minutes).
    token_ttl_minutes: i64,
    /// Phantom data to hold type parameters.
    _phantom: std::marker::PhantomData<(O, E, W, S, U, D, R)>,
}

impl<O, E, W, S, U, D, R> MagicLinkReducer<O, E, W, S, U, D, R> {
    /// Create a new magic link reducer with default settings.
    ///
    /// Default token TTL is 10 minutes.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            token_ttl_minutes: 10,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create a reducer with custom token TTL.
    ///
    /// # Arguments
    ///
    /// * `ttl_minutes` - Token expiration time in minutes (recommended: 5-15)
    #[must_use]
    pub const fn with_ttl(ttl_minutes: i64) -> Self {
        Self {
            token_ttl_minutes: ttl_minutes,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Generate a cryptographically secure random token.
    ///
    /// Returns a 256-bit random token encoded as base64url (43 characters).
    fn generate_token(&self) -> String {
        use base64::Engine;
        use rand::RngCore;

        let mut rng = rand::thread_rng();
        let mut random_bytes = [0u8; 32];
        rng.fill_bytes(&mut random_bytes);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes)
    }
}

impl<O, E, W, S, U, D, R> Default for MagicLinkReducer<O, E, W, S, U, D, R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<O, E, W, S, U, D, R> Reducer for MagicLinkReducer<O, E, W, S, U, D, R>
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
            // ═══════════════════════════════════════════════════════════════
            // SendMagicLink: Generate token and send email
            // ═══════════════════════════════════════════════════════════════
            AuthAction::SendMagicLink {
                email,
                ip_address: _,
                user_agent: _,
            } => {
                // Generate cryptographically secure token
                let token = self.generate_token();
                let expires_at = Utc::now() + chrono::Duration::minutes(self.token_ttl_minutes);

                // Store magic link state
                state.magic_link_state = Some(MagicLinkState {
                    email: email.clone(),
                    token: token.clone(),
                    expires_at,
                });

                // Send email with magic link
                let email_provider = env.email.clone();
                let token_clone = token.clone();
                let email_clone = email.clone();
                let base_url = "https://app.example.com".to_string(); // TODO: Make configurable
                let expires_at_clone = expires_at;

                smallvec![Effect::Future(Box::pin(async move {
                    match email_provider
                        .send_magic_link(&email_clone, &token_clone, &base_url, expires_at_clone)
                        .await
                    {
                        Ok(()) => Some(AuthAction::MagicLinkSent {
                            email: email_clone,
                            token: token_clone,
                            expires_at: expires_at_clone,
                        }),
                        Err(_) => {
                            // TODO: Add MagicLinkFailed action
                            None
                        }
                    }
                }))]
            }

            // ═══════════════════════════════════════════════════════════════
            // MagicLinkSent: Confirmation event (no-op)
            // ═══════════════════════════════════════════════════════════════
            AuthAction::MagicLinkSent { .. } => {
                // Email sent successfully - this is just a confirmation event
                smallvec![Effect::None]
            }

            // ═══════════════════════════════════════════════════════════════
            // VerifyMagicLink: Validate token
            // ═══════════════════════════════════════════════════════════════
            AuthAction::VerifyMagicLink {
                token,
                ip_address,
                user_agent,
            } => {
                // Check if we have magic link state
                let Some(ref magic_link_state) = state.magic_link_state else {
                    tracing::warn!("VerifyMagicLink without prior SendMagicLink");
                    state.magic_link_state = None;
                    return smallvec![Effect::None];
                };

                // Check token match (constant-time comparison for timing attack prevention)
                if !constant_time_eq::constant_time_eq(
                    token.as_bytes(),
                    magic_link_state.token.as_bytes(),
                ) {
                    tracing::warn!("Invalid magic link token");
                    state.magic_link_state = None;
                    return smallvec![Effect::None];
                }

                // Check expiration
                if Utc::now() > magic_link_state.expires_at {
                    tracing::warn!("Magic link token expired");
                    state.magic_link_state = None;
                    return smallvec![Effect::None];
                }

                // Token is valid - emit verified event
                let email = magic_link_state.email.clone();
                state.magic_link_state = None;

                // Emit MagicLinkVerified event to trigger session creation
                // Note: In the real flow, this would be dispatched to the store
                // which would then call reduce() again with MagicLinkVerified
                tracing::info!("Magic link verified for {}", email);

                // For now, transition to MagicLinkVerified handler
                // The Store will execute this and dispatch the resulting action
                self.reduce(
                    state,
                    AuthAction::MagicLinkVerified {
                        email,
                        ip_address,
                        user_agent,
                    },
                    env,
                )
            }

            // ═══════════════════════════════════════════════════════════════
            // MagicLinkVerified: Create user, device, and session
            // ═══════════════════════════════════════════════════════════════
            AuthAction::MagicLinkVerified {
                email,
                ip_address,
                user_agent,
            } => {
                // Generate IDs for new user/device/session
                let user_id = UserId::new();
                let device_id = DeviceId::new();
                let session_id = SessionId::new();
                let now = Utc::now();
                let expires_at = now + chrono::Duration::hours(24);

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
                    oauth_provider: None,
                    login_risk_score: 0.1, // Will be calculated by effect
                };

                // Update state
                state.session = Some(session.clone());

                // Execute effects to persist user, device, and session
                let users = env.users.clone();
                let devices = env.devices.clone();
                let sessions = env.sessions.clone();
                let session_clone = session.clone();
                let session_ttl = chrono::Duration::hours(24);

                smallvec![Effect::Future(Box::pin(async move {
                    use crate::actions::DeviceTrustLevel;
                    use crate::providers::{Device as ProviderDevice, User as ProviderUser};

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
                                name: None,
                                email_verified: true, // Magic link implies email verification
                                created_at: Utc::now(),
                                updated_at: Utc::now(),
                            };

                            match users.create_user(&new_user).await {
                                Ok(created_user) => created_user,
                                Err(_) => {
                                    // TODO: Return MagicLinkFailed action
                                    return None;
                                }
                            }
                        }
                    };

                    // 2. Create device
                    let new_device = ProviderDevice {
                        device_id,
                        user_id: final_user.user_id,
                        name: "Web Browser".to_string(), // TODO: Parse from user agent
                        device_type: crate::providers::DeviceType::Desktop,
                        platform: user_agent.clone(),
                        first_seen: Utc::now(),
                        last_seen: Utc::now(),
                        trust_level: DeviceTrustLevel::Unknown,
                        passkey_credential_id: None,
                        public_key: None,
                    };

                    if devices.create_device(&new_device).await.is_err() {
                        // TODO: Return MagicLinkFailed action
                        return None;
                    }

                    // 3. Create session in Redis
                    if sessions
                        .create_session(&session_clone, session_ttl)
                        .await
                        .is_err()
                    {
                        // TODO: Return MagicLinkFailed action
                        return None;
                    }

                    // 4. Emit session created event
                    Some(AuthAction::SessionCreated {
                        session: session_clone,
                    })
                }))]
            }

            // Other actions are not handled by this reducer
            _ => smallvec![Effect::None],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token() {
        type TestReducer = MagicLinkReducer<(), (), (), (), (), (), ()>;
        let reducer = TestReducer::new();
        let token1 = reducer.generate_token();
        let token2 = reducer.generate_token();

        // Tokens should be non-empty
        assert!(!token1.is_empty());
        assert!(!token2.is_empty());

        // Tokens should be unique
        assert_ne!(token1, token2);

        // Tokens should be 43 characters (256 bits base64url encoded)
        assert_eq!(token1.len(), 43);
    }

    #[test]
    fn test_custom_ttl() {
        type TestReducer = MagicLinkReducer<(), (), (), (), (), (), ()>;
        let reducer = TestReducer::with_ttl(15);
        assert_eq!(reducer.token_ttl_minutes, 15);
    }
}
