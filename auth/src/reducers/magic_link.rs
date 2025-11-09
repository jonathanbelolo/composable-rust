//! Magic link authentication reducer with event sourcing.
//!
//! This reducer implements passwordless email authentication via "magic links" using
//! the composable-rust event sourcing pattern.
//!
//! # Flow
//!
//! 1. User requests magic link with email address
//! 2. Generate cryptographically secure token
//! 3. Store token in state with expiration
//! 4. Send email with link containing token
//! 5. User clicks link, submits token
//! 6. Verify token (not expired, not used)
//! 7. Emit UserRegistered event (if new user)
//! 8. Emit DeviceRegistered event
//! 9. Emit UserLoggedIn event (audit trail)
//! 10. Create session in Redis (ephemeral, not event-sourced)
//!
//! # Security
//!
//! - Tokens are 256-bit random values (base64url encoded)
//! - Tokens expire after 5-15 minutes (configurable)
//! - Tokens are single-use (invalidated after verification)
//! - Constant-time comparison for tokens (timing attack prevention)
//!
//! # Event Sourcing
//!
//! - All user and device state changes are event-sourced
//! - Events are persisted to PostgreSQL event store
//! - Projections are rebuilt from events for queries
//! - Sessions are ephemeral (Redis only, not event-sourced)

use crate::actions::{AuthAction, AuthLevel};
use crate::config::MagicLinkConfig;
use crate::constants::login_methods;
use crate::environment::AuthEnvironment;
use crate::events::AuthEvent;
use crate::providers::{
    ChallengeStore, DeviceRepository, EmailProvider, OAuth2Provider, OAuthTokenStore,
    RiskCalculator, SessionStore, TokenStore, UserRepository, WebAuthnProvider,
};
use crate::state::{AuthState, DeviceId, MagicLinkState, Session, SessionId, UserId};
use chrono::Utc;
use composable_rust_core::async_effect;
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use composable_rust_core::stream::StreamId;
use composable_rust_core::{smallvec, SmallVec};
use std::sync::Arc;

/// Magic link authentication reducer.
///
/// Handles passwordless email authentication flows with event sourcing.
#[derive(Debug, Clone)]
pub struct MagicLinkReducer<O, E, W, S, T, U, D, R, OT, C, RL> {
    /// Configuration for magic link authentication.
    config: MagicLinkConfig,
    /// Phantom data to hold type parameters.
    _phantom: std::marker::PhantomData<(O, E, W, S, T, U, D, R, OT, C, RL)>,
}

impl<O, E, W, S, T, U, D, R, OT, C, RL> MagicLinkReducer<O, E, W, S, T, U, D, R, OT, C, RL> {
    /// Create a new magic link reducer with default settings.
    ///
    /// Default configuration:
    /// - Base URL: http://localhost:3000
    /// - Token TTL: 10 minutes
    /// - Session duration: 24 hours
    ///
    /// For production, use `with_config()` to provide proper configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: MagicLinkConfig::default(),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create a reducer with custom configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - Magic link configuration
    ///
    /// # Example
    ///
    /// ```ignore
    /// use composable_rust_auth::config::MagicLinkConfig;
    /// use composable_rust_auth::reducers::MagicLinkReducer;
    ///
    /// let config = MagicLinkConfig::new("https://app.example.com".to_string())
    ///     .with_token_ttl(15);
    ///
    /// let reducer: MagicLinkReducer<_, _, _, _, _, _, _, _> =
    ///     MagicLinkReducer::with_config(config);
    /// ```
    #[must_use]
    pub fn with_config(config: MagicLinkConfig) -> Self {
        Self {
            config,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create a reducer with custom token TTL.
    ///
    /// # Arguments
    ///
    /// * `ttl_minutes` - Token expiration time in minutes (recommended: 5-15)
    ///
    /// # Deprecated
    ///
    /// Use `with_config()` instead for full configuration.
    #[must_use]
    pub fn with_ttl(ttl_minutes: i64) -> Self {
        Self {
            config: MagicLinkConfig::default().with_token_ttl(ttl_minutes),
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

    /// Apply an event to state (for event replay and EventPersisted handling).
    fn apply_event(&self, _state: &mut AuthState, event: &AuthEvent) {
        match event {
            AuthEvent::UserRegistered { user_id, .. } => {
                // Update state to reflect user registration
                tracing::info!("Applied UserRegistered event for user {}", user_id.0);
                // State updates will happen when session is created
            }
            AuthEvent::DeviceRegistered { device_id, .. } => {
                tracing::info!("Applied DeviceRegistered event for device {}", device_id.0);
            }
            AuthEvent::UserLoggedIn { user_id, .. } => {
                tracing::info!("Applied UserLoggedIn event for user {}", user_id.0);
            }
            _ => {
                // Other events not handled by magic link reducer
            }
        }
    }
}

impl<O, E, W, S, T, U, D, R, OT, C, RL> Default for MagicLinkReducer<O, E, W, S, T, U, D, R, OT, C, RL> {
    fn default() -> Self {
        Self::new()
    }
}

impl<O, E, W, S, T, U, D, R, OT, C, RL> Reducer for MagicLinkReducer<O, E, W, S, T, U, D, R, OT, C, RL>
where
    O: OAuth2Provider + Clone + 'static,
    E: EmailProvider + Clone + 'static,
    W: WebAuthnProvider + Clone + 'static,
    S: SessionStore + Clone + 'static,
    T: TokenStore + Clone + 'static,
    U: UserRepository + Clone + 'static,
    D: DeviceRepository + Clone + 'static,
    R: RiskCalculator + Clone + 'static,
    OT: OAuthTokenStore + Clone + 'static,
    C: ChallengeStore + Clone + 'static,
    RL: crate::providers::RateLimiter + Clone + 'static,
{
    type State = AuthState;
    type Action = AuthAction;
    type Environment = AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>;

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
                correlation_id,
                email,
                ip_address: _,
                user_agent: _,
            } => {
                // Validate and normalize email (prevent account collision)
                let email = match crate::utils::normalize_email(&email) {
                    Ok(normalized) => normalized,
                    Err(e) => {
                        tracing::warn!("Invalid email format: {}", e);
                        return smallvec![Effect::None];
                    }
                };

                // Generate cryptographically secure token
                let token = self.generate_token();
                let expires_at = Utc::now() + chrono::Duration::minutes(self.config.token_ttl_minutes);

                // Store magic link state (keep for backward compatibility during migration)
                state.magic_link_state = Some(MagicLinkState {
                    email: email.clone(),
                    token: token.clone(),
                    expires_at,
                });

                // Store token in TokenStore for atomic single-use semantics
                let token_store = env.tokens.clone();
                let token_for_store = token.clone();
                let email_for_store = email.clone();
                let token_data = crate::providers::TokenData::new(
                    crate::providers::TokenType::MagicLink,
                    token.clone(),
                    serde_json::json!({"email": email}),
                    expires_at,
                );

                // Send email with magic link
                let email_provider = env.email.clone();
                let token_for_email = token.clone();
                let email_for_email = email.clone();
                let base_url = self.config.base_url.clone();
                let expires_at_clone = expires_at;

                smallvec![
                    // Effect 1: Store token atomically
                    async_effect! {
                        match token_store.store_token(&token_for_store, token_data).await {
                            Ok(()) => {
                                tracing::debug!("Magic link token stored for {}", email_for_store);
                                None
                            }
                            Err(e) => {
                                tracing::error!("Failed to store magic link token: {}", e);
                                Some(AuthAction::MagicLinkFailed {
                                    correlation_id,
                                    email: email_for_store,
                                    error: e.to_string(),
                                })
                            }
                        }
                    },
                    // Effect 2: Send email
                    async_effect! {
                        match email_provider
                            .send_magic_link(&email_for_email, &token_for_email, &base_url, expires_at_clone)
                            .await
                        {
                            Ok(()) => Some(AuthAction::MagicLinkSent {
                                correlation_id,
                                email: email_for_email.clone(),
                                token: token_for_email,
                                expires_at: expires_at_clone,
                            }),
                            Err(e) => {
                                tracing::error!("Failed to send magic link to {}: {}", email_for_email, e);
                                Some(AuthAction::MagicLinkFailed {
                                    correlation_id,
                                    email: email_for_email,
                                    error: e.to_string(),
                                })
                            }
                        }
                    }
                ]
            }

            // ═══════════════════════════════════════════════════════════════
            // MagicLinkSent: Confirmation event (no-op)
            // ═══════════════════════════════════════════════════════════════
            AuthAction::MagicLinkSent { correlation_id: _, .. } => {
                // Email sent successfully - this is just a confirmation event
                smallvec![Effect::None]
            }

            // ═══════════════════════════════════════════════════════════════
            // MagicLinkFailed: Email sending failed
            // ═══════════════════════════════════════════════════════════════
            AuthAction::MagicLinkFailed { correlation_id: _, .. } => {
                // Clear magic link state on failure
                state.magic_link_state = None;
                smallvec![Effect::None]
            }

            // ═══════════════════════════════════════════════════════════════
            // VerifyMagicLink: Validate token (ATOMIC CONSUMPTION)
            // ═══════════════════════════════════════════════════════════════
            AuthAction::VerifyMagicLink {
                correlation_id,
                token,
                ip_address,
                user_agent,
                fingerprint,
            } => {
                // ✅ INPUT VALIDATION: Defense-in-depth validation at entry point
                if let Err(e) = crate::utils::validate_ip_address(&ip_address.to_string()) {
                    tracing::warn!(
                        error = %e,
                        ip_address = %crate::utils::sanitize_ip_for_logging(&ip_address.to_string()),
                        "Invalid IP address during magic link verification"
                    );
                    return smallvec![async_effect! {
                        Some(AuthAction::MagicLinkFailed {
                            correlation_id,
                            email: String::new(), // Don't leak email on validation failure
                            error: "Invalid request parameters".to_string(),
                        })
                    }];
                }

                if let Err(e) = crate::utils::validate_user_agent(&user_agent) {
                    tracing::warn!(
                        error = %e,
                        user_agent_length = user_agent.len(),
                        "Invalid user agent during magic link verification"
                    );
                    return smallvec![async_effect! {
                        Some(AuthAction::MagicLinkFailed {
                            correlation_id,
                            email: String::new(), // Don't leak email on validation failure
                            error: "Invalid request parameters".to_string(),
                        })
                    }];
                }

                // ⚡ SECURITY FIX (BLOCKER #1): Atomic token consumption
                // Use TokenStore.consume_token() to atomically verify and delete
                // This prevents race conditions where two concurrent requests
                // both pass validation before either deletes the token.

                let token_store = env.tokens.clone();
                let token_for_consume = token.clone();
                let ip_clone = ip_address;
                let ua_clone = user_agent.clone();
                let fingerprint_clone = fingerprint.clone();

                smallvec![async_effect! {
                    // Atomically consume token (check + delete in one operation)
                    match token_store.consume_token(&token_for_consume, &token_for_consume).await {
                        Ok(Some(token_data)) => {
                            // Token was valid, not expired, and has been consumed
                            // Extract email from token data
                            let email = token_data.data["email"]
                                .as_str()
                                .unwrap_or("")
                                .to_string();

                            if email.is_empty() {
                                tracing::error!("Magic link token missing email data");
                                return None;
                            }

                            tracing::info!("Magic link verified for {}", email);

                            // Transition to verified handler
                            Some(AuthAction::MagicLinkVerified {
                                correlation_id,
                                email,
                                ip_address: ip_clone,
                                user_agent: ua_clone,
                                fingerprint: fingerprint_clone,
                            })
                        }
                        Ok(None) => {
                            // Token not found, already used, or expired
                            // Don't leak which one (information disclosure prevention)
                            tracing::warn!("Magic link verification failed");
                            None
                        }
                        Err(e) => {
                            tracing::error!("Magic link token consumption failed: {}", e);
                            None
                        }
                    }
                }]
            }

            // ═══════════════════════════════════════════════════════════════
            // MagicLinkVerified: Emit domain events (batch)
            // ═══════════════════════════════════════════════════════════════
            AuthAction::MagicLinkVerified {
                correlation_id,
                email,
                ip_address,
                user_agent,
                fingerprint,
            } => {
                // Generate IDs upfront
                let user_id = UserId::new();
                let device_id = DeviceId::new();
                let session_id = SessionId::new();
                let now = Utc::now();

                // Build session with placeholder risk score (will be updated via SessionCreated)
                let session = Session {
                    session_id,
                    user_id, // Will be updated with actual user_id from projection
                    device_id,
                    email: email.clone(),
                    created_at: now,
                    last_active: now,
                    expires_at: now + self.config.session_duration,
                    ip_address,
                    user_agent: user_agent.clone(),
                    oauth_provider: None,
                    login_risk_score: 0.1, // Placeholder - will be updated via SessionCreated
                    idle_timeout: self.config.idle_timeout,
                    enable_sliding_refresh: self.config.enable_sliding_session_refresh,
                };

                // Clear magic link state (token consumed, no longer valid)
                state.magic_link_state = None;

                // Update state immediately (sessions are ephemeral, not event-sourced)
                // The risk score will be corrected when SessionCreated action is processed
                state.session = Some(session.clone());

                // Query projection to check if user exists
                let users = env.users.clone();
                let event_store = Arc::clone(&env.event_store);
                let sessions = env.sessions.clone();
                let risk = env.risk.clone();
                let email_clone = email.clone();
                let user_agent_clone = user_agent.clone();
                let fingerprint_clone = fingerprint.clone();
                let session_duration = self.config.session_duration;
                let max_concurrent_sessions = self.config.max_concurrent_sessions;
                let idle_timeout = self.config.idle_timeout;
                let enable_sliding_refresh = self.config.enable_sliding_session_refresh;

                smallvec![async_effect! {
                    // Check if user exists in projection
                    let existing_user = users.get_user_by_email(&email_clone).await.ok();
                    let final_user_id = existing_user.as_ref().map_or(user_id, |u| u.user_id);

                    // Calculate login risk
                    let risk_assessment = risk.calculate_login_risk(&crate::providers::LoginContext {
                        user_id: Some(final_user_id),
                        email: email_clone.clone(),
                        ip_address,
                        user_agent: user_agent_clone.clone(),
                        device_id: Some(device_id),
                        last_login_location: None,
                        last_login_at: None,
                        fingerprint: fingerprint_clone.clone(),
                    }).await.ok().unwrap_or_else(|| {
                        // Fall back to safe default on error
                        crate::providers::RiskAssessment {
                            score: 0.1,
                            level: crate::providers::RiskLevel::Low,
                            factors: vec![],
                            recommended_auth_level: AuthLevel::Basic,
                        }
                    });

                    let login_risk_score = risk_assessment.score;

                    // Build events to emit (all independent, can be batched)
                    let mut events = Vec::new();

                    // 1. UserRegistered event (only if new user)
                    if existing_user.is_none() {
                        events.push(AuthEvent::UserRegistered {
                            user_id: final_user_id,
                            email: email_clone.clone(),
                            name: None,
                            email_verified: true,
                            timestamp: now,
                        });
                    }

                    // 2. DeviceRegistered event (always)
                    events.push(AuthEvent::DeviceRegistered {
                        device_id,
                        user_id: final_user_id,
                        name: crate::utils::parse_device_name(&user_agent_clone),
                        device_type: crate::utils::parse_device_type(&user_agent_clone).to_string(),
                        platform: user_agent_clone.clone(),
                        ip_address,
                        timestamp: now,
                    });

                    // 3. UserLoggedIn event (audit trail, always)
                    events.push(AuthEvent::UserLoggedIn {
                        user_id: final_user_id,
                        device_id,
                        session_id,
                        method: login_methods::MAGIC_LINK.to_string(),
                        auth_level: AuthLevel::Basic,
                        ip_address,
                        user_agent: user_agent_clone.clone(),
                        risk_score: login_risk_score as f64,
                        timestamp: now,
                    });

                    // Serialize all events
                    let serialized_events: Vec<_> = events
                        .iter()
                        .filter_map(|e| e.to_serialized().ok())
                        .collect();

                    if serialized_events.is_empty() {
                        tracing::error!("No events to persist");
                        return None;
                    }

                    // Build session (to pass to EventPersisted handler)
                    let session = Session {
                        session_id,
                        user_id: final_user_id,
                        device_id,
                        email: email_clone.clone(),
                        created_at: now,
                        last_active: now,
                        expires_at: now + session_duration,
                        ip_address,
                        user_agent: user_agent_clone.clone(),
                        oauth_provider: None,
                        login_risk_score,
                        idle_timeout,
                        enable_sliding_refresh,
                    };

                    // Batch append all events to the user stream
                    let stream_id = StreamId::new(format!("user-{}", final_user_id.0));

                    // We need to execute the append ourselves here since we're in a Future
                    // The append_events! macro is designed to be used in the reducer directly
                    match event_store.append_events(stream_id, None, serialized_events).await {
                        Ok(_version) => {
                            // Events persisted successfully
                            // Now create ephemeral session in Redis
                            if let Err(e) = sessions.create_session(&session, session_duration, max_concurrent_sessions).await {
                                tracing::error!("Failed to create session in Redis for user {}: {}", final_user_id.0, e);
                                return Some(AuthAction::SessionCreationFailed {
                                    correlation_id,
                                    user_id: final_user_id,
                                    device_id,
                                    error: e.to_string(),
                                });
                            }

                            // Emit SessionCreated event
                            Some(AuthAction::SessionCreated { correlation_id, session })
                        }
                        Err(e) => {
                            tracing::error!("Failed to persist events: {e}");
                            Some(AuthAction::EventPersistenceFailed {
                                stream_id: format!("user-{}", final_user_id.0),
                                error: e.to_string(),
                            })
                        }
                    }
                }]
            }

            // ═══════════════════════════════════════════════════════════════
            // SessionCreated: Set session in state
            // ═══════════════════════════════════════════════════════════════
            AuthAction::SessionCreated { correlation_id: _, session } => {
                // Set session in state (session now has correct risk score from RiskCalculator)
                state.session = Some(session.clone());
                smallvec![Effect::None]
            }

            // ═══════════════════════════════════════════════════════════════
            // EventPersisted: Apply event to state
            // ═══════════════════════════════════════════════════════════════
            AuthAction::EventPersisted { event, version: _ } => {
                self.apply_event(state, &event);
                smallvec![Effect::None]
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
        type TestReducer = MagicLinkReducer<(), (), (), (), (), (), (), (), (), (), ()>;
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
        type TestReducer = MagicLinkReducer<(), (), (), (), (), (), (), (), (), (), ()>;
        let reducer = TestReducer::with_ttl(15);
        assert_eq!(reducer.config.token_ttl_minutes, 15);
    }
}
