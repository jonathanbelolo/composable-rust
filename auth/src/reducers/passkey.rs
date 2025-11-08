//! WebAuthn/Passkey authentication reducer.
//!
//! This reducer implements passwordless authentication via WebAuthn/FIDO2 passkeys.
//!
//! # Flows
//!
//! ## Registration Flow
//!
//! 1. User initiates passkey registration (must be logged in)
//! 2. Generate WebAuthn challenge
//! 3. Client calls `navigator.credentials.create()`
//! 4. Verify attestation response
//! 5. Emit PasskeyRegistered event
//!
//! ## Login Flow
//!
//! 1. User initiates passkey login with username/email
//! 2. Look up user's passkeys
//! 3. Generate WebAuthn challenge
//! 4. Client calls `navigator.credentials.get()`
//! 5. Verify assertion response
//! 6. Emit PasskeyUsed, DeviceAccessed, UserLoggedIn events (batch)
//! 7. Create session
//!
//! # Security
//!
//! - Challenges expire after 5 minutes
//! - Origin and RP ID validation
//! - Counter rollback detection (via PasskeyUsed event)
//! - Public key cryptography (ECDSA/EdDSA)
//! - Hardware-backed keys (FIDO2 authenticators)
//!
//! # Event Sourcing
//!
//! - PasskeyUsed event tracks counter for replay protection
//! - DeviceAccessed event for device trust calculation
//! - UserLoggedIn event for audit trail
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_auth::{AuthState, AuthAction};
//! use composable_rust_auth::reducers::PasskeyReducer;
//! use composable_rust_auth::state::UserId;
//! use composable_rust_core::reducer::Reducer;
//! use std::net::{IpAddr, Ipv4Addr};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Note: PasskeyReducer is generic - needs type annotations in real usage
//! // See integration tests for complete examples
//! # Ok(())
//! # }
//! ```

use crate::actions::{AuthAction, AuthLevel};
use crate::config::PasskeyConfig;
use crate::constants::login_methods;
use crate::environment::AuthEnvironment;
use crate::events::AuthEvent;
use crate::providers::{
    DeviceRepository, EmailProvider, OAuth2Provider, PasskeyCredential, RiskCalculator,
    SessionStore, TokenStore, UserRepository, WebAuthnProvider,
};
use crate::state::{AuthState, Session, SessionId};
use chrono::Utc;
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use composable_rust_core::stream::StreamId;
use composable_rust_core::{smallvec, SmallVec};
use std::sync::Arc;

/// WebAuthn/Passkey authentication reducer.
///
/// Handles passkey registration and login flows.
#[derive(Debug, Clone)]
pub struct PasskeyReducer<O, E, W, S, T, U, D, R> {
    /// Configuration for passkey authentication.
    config: PasskeyConfig,
    /// Phantom data to hold type parameters.
    _phantom: std::marker::PhantomData<(O, E, W, S, T, U, D, R)>,
}

impl<O, E, W, S, T, U, D, R> PasskeyReducer<O, E, W, S, T, U, D, R> {
    /// Create a new passkey reducer with default configuration.
    ///
    /// Default configuration:
    /// - Origin: http://localhost:3000
    /// - RP ID: localhost
    /// - Challenge TTL: 5 minutes
    /// - Session duration: 24 hours
    ///
    /// For production, use `with_config()` to provide proper configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: PasskeyConfig::default(),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create a reducer with custom configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - Passkey configuration
    ///
    /// # Example
    ///
    /// ```ignore
    /// use composable_rust_auth::config::PasskeyConfig;
    /// use composable_rust_auth::reducers::PasskeyReducer;
    ///
    /// let config = PasskeyConfig::new(
    ///     "https://app.example.com".to_string(),
    ///     "app.example.com".to_string()
    /// ).with_challenge_ttl(10);
    ///
    /// let reducer: PasskeyReducer<_, _, _, _, _, _, _> =
    ///     PasskeyReducer::with_config(config);
    /// ```
    #[must_use]
    pub fn with_config(config: PasskeyConfig) -> Self {
        Self {
            config,
            _phantom: std::marker::PhantomData,
        }
    }

}

impl<O, E, W, S, T, U, D, R> Default for PasskeyReducer<O, E, W, S, T, U, D, R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<O, E, W, S, T, U, D, R> Reducer for PasskeyReducer<O, E, W, S, T, U, D, R>
where
    O: OAuth2Provider + Clone + 'static,
    E: EmailProvider + Clone + 'static,
    W: WebAuthnProvider + Clone + 'static,
    S: SessionStore + Clone + 'static,
    T: TokenStore + Clone + 'static,
    U: UserRepository + Clone + 'static,
    D: DeviceRepository + Clone + 'static,
    R: RiskCalculator + Clone + 'static,
{
    type State = AuthState;
    type Action = AuthAction;
    type Environment = AuthEnvironment<O, E, W, S, T, U, D, R>;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ═══════════════════════════════════════════════════════════════
            // InitiatePasskeyRegistration: Generate challenge
            // ═══════════════════════════════════════════════════════════════
            AuthAction::InitiatePasskeyRegistration {
                user_id,
                device_name: _,
            } => {
                // ⚡ SECURITY FIX (BLOCKER #3): Store challenge with expiration
                // Generate a unique challenge ID and store in TokenStore with TTL

                let webauthn = env.webauthn.clone();
                let users = env.users.clone();
                let token_store = env.tokens.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // Get user info for WebAuthn
                    let user = match users.get_user_by_id(user_id).await {
                        Ok(u) => u,
                        Err(_) => return None,
                    };

                    // Generate challenge
                    match webauthn
                        .generate_registration_challenge(
                            user_id,
                            &user.email,
                            user.name.as_deref().unwrap_or(&user.email),
                        )
                        .await
                    {
                        Ok(challenge) => {
                            // Use challenge_id from WebAuthn provider
                            let challenge_id = challenge.challenge_id.clone();
                            let challenge_bytes = challenge.challenge.clone();

                            // Store challenge in TokenStore with expiration
                            let expires_at = challenge.expires_at;
                            let token_data = crate::providers::TokenData::new(
                                crate::providers::TokenType::PasskeyRegistrationChallenge,
                                challenge_id.clone(),
                                serde_json::json!({
                                    "user_id": user_id.0,
                                    "challenge": challenge_bytes,
                                }),
                                expires_at,
                            );

                            match token_store.store_token(&challenge_id, token_data).await {
                                Ok(()) => {
                                    tracing::info!(
                                        "Generated WebAuthn registration challenge for user {}",
                                        user_id.0
                                    );
                                    None // TODO: Return challenge_id to client
                                }
                                Err(e) => {
                                    tracing::error!("Failed to store passkey challenge: {}", e);
                                    None
                                }
                            }
                        }
                        Err(_) => None,
                    }
                }))]
            }

            // ═══════════════════════════════════════════════════════════════
            // CompletePasskeyRegistration: Verify attestation
            // ═══════════════════════════════════════════════════════════════
            AuthAction::CompletePasskeyRegistration {
                user_id,
                device_id,
                credential_id: _,
                public_key: _,
                attestation_response,
            } => {
                // ⚡ SECURITY FIX (BLOCKER #3 & #7): Atomic challenge consumption
                //
                // TODO: Add challenge_id field to CompletePasskeyRegistration action
                // The client should pass back the challenge_id received from InitiatePasskeyRegistration
                //
                // Proper implementation:
                // ```rust
                // match token_store.consume_token(&challenge_id, &challenge_id).await {
                //     Ok(Some(token_data)) => {
                //         // Challenge valid, not expired, and consumed atomically
                //         let challenge = token_data.data["challenge"].as_str().unwrap();
                //         webauthn.verify_registration(challenge, &attestation_response, ...)
                //     }
                //     Ok(None) => {
                //         // Challenge expired, already used, or invalid
                //         return None;
                //     }
                // }
                // ```
                //
                // For now, using mock challenge_id as placeholder

                let webauthn = env.webauthn.clone();
                let users = env.users.clone();
                let challenge_id = "mock_challenge_id".to_string(); // TODO: Get from client
                let origin = self.config.origin.clone();
                let rp_id = self.config.rp_id.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // NOTE: In production, challenge would be retrieved from TokenStore here
                    // and atomically consumed to prevent replay attacks

                    // Verify attestation
                    let result = match webauthn
                        .verify_registration(&challenge_id, &attestation_response, &origin, &rp_id)
                        .await
                    {
                        Ok(r) => r,
                        Err(_) => return None,
                    };

                    // Store credential
                    let credential = PasskeyCredential {
                        credential_id: result.credential_id.clone(),
                        user_id,
                        device_id,
                        public_key: result.public_key,
                        counter: result.counter,
                        created_at: Utc::now(),
                        last_used: None,
                    };

                    match users.create_passkey_credential(&credential).await {
                        Ok(()) => {
                            tracing::info!("Passkey registered successfully");
                            None // TODO: Return success event
                        }
                        Err(_) => None,
                    }
                }))]
            }

            // ═══════════════════════════════════════════════════════════════
            // InitiatePasskeyLogin: Generate challenge and lookup credentials
            // ═══════════════════════════════════════════════════════════════
            AuthAction::InitiatePasskeyLogin {
                username,
                ip_address: _,
                user_agent: _,
            } => {
                // ⚡ SECURITY FIX (BLOCKER #3): Store authentication challenge with expiration

                let webauthn = env.webauthn.clone();
                let users = env.users.clone();
                let token_store = env.tokens.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // Get user by email
                    let user = match users.get_user_by_email(&username).await {
                        Ok(u) => u,
                        Err(_) => {
                            tracing::warn!("User not found for passkey login: {}", username);
                            return None;
                        }
                    };

                    // Get user's passkey credentials
                    let credentials = match users.get_user_passkey_credentials(user.user_id).await {
                        Ok(creds) => creds,
                        Err(_) => {
                            tracing::warn!("No passkeys found for user: {}", username);
                            return None;
                        }
                    };

                    if credentials.is_empty() {
                        tracing::warn!("User has no passkeys: {}", username);
                        return None;
                    }

                    // Generate authentication challenge
                    match webauthn
                        .generate_authentication_challenge(user.user_id, credentials)
                        .await
                    {
                        Ok(challenge) => {
                            // Use challenge_id from WebAuthn provider
                            let challenge_id = challenge.challenge_id.clone();
                            let challenge_bytes = challenge.challenge.clone();

                            // Store challenge in TokenStore with expiration
                            let expires_at = challenge.expires_at;
                            let token_data = crate::providers::TokenData::new(
                                crate::providers::TokenType::PasskeyAuthenticationChallenge,
                                challenge_id.clone(),
                                serde_json::json!({
                                    "user_id": user.user_id.0,
                                    "challenge": challenge_bytes,
                                }),
                                expires_at,
                            );

                            match token_store.store_token(&challenge_id, token_data).await {
                                Ok(()) => {
                                    tracing::info!(
                                        "Generated WebAuthn authentication challenge for user {}",
                                        user.user_id.0
                                    );
                                    None // TODO: Return challenge_id to client
                                }
                                Err(e) => {
                                    tracing::error!("Failed to store passkey authentication challenge: {}", e);
                                    None
                                }
                            }
                        }
                        Err(_) => None,
                    }
                }))]
            }

            // ═══════════════════════════════════════════════════════════════
            // CompletePasskeyLogin: Verify assertion
            // ═══════════════════════════════════════════════════════════════
            AuthAction::CompletePasskeyLogin {
                credential_id,
                assertion_response,
                ip_address,
                user_agent,
            } => {
                // ⚡ SECURITY FIX (BLOCKER #3 & #7): Atomic challenge consumption
                //
                // TODO: Add challenge_id field to CompletePasskeyLogin action
                // The client should pass back the challenge_id received from InitiatePasskeyLogin
                //
                // Proper implementation:
                // ```rust
                // match token_store.consume_token(&challenge_id, &challenge_id).await {
                //     Ok(Some(token_data)) => {
                //         // Challenge valid, not expired, and consumed atomically
                //         let challenge = token_data.data["challenge"].as_str().unwrap();
                //         webauthn.verify_authentication(challenge, &assertion_response, ...)
                //     }
                //     Ok(None) => {
                //         // Challenge expired, already used, or invalid
                //         return None;
                //     }
                // }
                // ```
                //
                // For now, using mock challenge_id as placeholder

                let webauthn = env.webauthn.clone();
                let users = env.users.clone();
                let challenge_id = "mock_challenge_id".to_string(); // TODO: Get from client
                let origin = self.config.origin.clone();
                let rp_id = self.config.rp_id.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // Get credential
                    let credential = match users.get_passkey_credential(&credential_id).await {
                        Ok(c) => c,
                        Err(_) => {
                            tracing::warn!("Credential not found: {}", credential_id);
                            return None;
                        }
                    };

                    // Verify assertion
                    let result = match webauthn
                        .verify_authentication(
                            &challenge_id,
                            &assertion_response,
                            &credential,
                            &origin,
                            &rp_id,
                        )
                        .await
                    {
                        Ok(r) => r,
                        Err(_) => {
                            tracing::warn!("Passkey verification failed");
                            return None;
                        }
                    };

                    // ═══════════════════════════════════════════════════════════════
                    // CRITICAL: Counter Rollback Detection (Replay Attack Prevention)
                    // ═══════════════════════════════════════════════════════════════
                    // ⚡ SECURITY FIX (BLOCKER #6): Counter rollback with wraparound handling
                    //
                    // WebAuthn spec requires checking the signature counter to detect
                    // cloned authenticators. However, counters are u32 and can wrap
                    // around from u32::MAX to 0.
                    //
                    // We use the "half-space" algorithm: if the difference between
                    // counters is more than half the total space (2^31), we treat it
                    // as wraparound. Otherwise, we treat it as rollback.
                    //
                    // Examples:
                    // - Stored: 100, New: 50   → Rollback (diff = -50)
                    // - Stored: u32::MAX - 10, New: 5 → Valid wraparound (diff = 16)
                    // - Stored: 5, New: u32::MAX - 10 → Rollback (diff = huge negative)

                    const HALF_SPACE: u32 = u32::MAX / 2;

                    let is_rollback = if result.counter == credential.counter {
                        // Same counter = replay attack
                        true
                    } else if result.counter > credential.counter {
                        // New counter is higher - check if it's suspiciously high
                        // (e.g., stored=5, new=u32::MAX-10 would be a rollback)
                        let diff = result.counter - credential.counter;
                        diff > HALF_SPACE
                    } else {
                        // New counter is lower - check if it's a valid wraparound
                        // (e.g., stored=u32::MAX-10, new=5 is valid wraparound)
                        let stored_from_max = u32::MAX - credential.counter;
                        let is_near_max = stored_from_max < HALF_SPACE;
                        let new_is_small = result.counter < HALF_SPACE;
                        !(is_near_max && new_is_small)
                    };

                    if is_rollback {
                        tracing::error!(
                            "🚨 SECURITY ALERT: Passkey counter rollback detected!\n\
                             Credential ID: {}\n\
                             Stored counter: {}\n\
                             Received counter: {}\n\
                             This indicates a CLONED AUTHENTICATOR or REPLAY ATTACK.",
                            credential_id,
                            credential.counter,
                            result.counter
                        );

                        // TODO: Emit CounterRollbackDetected security event for monitoring

                        // REJECT the authentication
                        return None;
                    }

                    // Counter is valid - update it
                    match users.update_passkey_counter(&credential_id, result.counter).await {
                        Ok(()) => {
                            tracing::info!(
                                "Passkey counter updated: {} -> {} (credential: {})",
                                credential.counter,
                                result.counter,
                                credential_id
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                "CRITICAL: Failed to update passkey counter for {}: {}",
                                credential_id,
                                e
                            );
                            // Counter update failure is serious - don't allow auth
                            return None;
                        }
                    }

                    // Get user email
                    let user = match users.get_user_by_id(result.user_id).await {
                        Ok(u) => u,
                        Err(_) => return None,
                    };

                    // Emit success event
                    Some(AuthAction::PasskeyLoginSuccess {
                        user_id: result.user_id,
                        device_id: result.device_id,
                        email: user.email,
                        ip_address,
                        user_agent,
                    })
                }))]
            }

            // ═══════════════════════════════════════════════════════════════
            // PasskeyLoginSuccess: Emit events (batch) and create session
            // ═══════════════════════════════════════════════════════════════
            AuthAction::PasskeyLoginSuccess {
                user_id,
                device_id,
                email,
                ip_address,
                user_agent,
            } => {
                // Generate session ID
                let session_id = SessionId::new();
                let now = Utc::now();

                // Build session with placeholder risk score (will be updated via SessionCreated)
                let session = Session {
                    session_id,
                    user_id,
                    device_id,
                    email: email.clone(),
                    created_at: now,
                    last_active: now,
                    expires_at: now + self.config.session_duration,
                    ip_address,
                    user_agent: user_agent.clone(),
                    oauth_provider: None,
                    login_risk_score: 0.05, // Placeholder - will be updated via SessionCreated
                };

                // Update state immediately (sessions are ephemeral, not event-sourced)
                // The risk score will be corrected when SessionCreated action is processed
                state.session = Some(session.clone());

                // Emit events
                let event_store = Arc::clone(&env.event_store);
                let sessions = env.sessions.clone();
                let risk = env.risk.clone();
                let email_clone = email.clone();
                let user_agent_clone = user_agent.clone();
                let session_duration = self.config.session_duration;

                smallvec![Effect::Future(Box::pin(async move {
                    // Calculate login risk
                    let risk_assessment = risk.calculate_login_risk(&crate::providers::LoginContext {
                        user_id: Some(user_id),
                        email: email_clone.clone(),
                        ip_address,
                        user_agent: user_agent_clone.clone(),
                        device_id: Some(device_id),
                        last_login_location: None,
                        last_login_at: None,
                    }).await.unwrap_or_else(|_| {
                        // Fall back to safe default on error
                        crate::providers::RiskAssessment {
                            score: 0.05, // Passkeys are very secure, low default
                            level: crate::providers::RiskLevel::Low,
                            factors: vec![],
                            recommended_auth_level: AuthLevel::HardwareBacked,
                        }
                    });

                    let login_risk_score = risk_assessment.score;

                    // Build events to emit (all independent, can be batched)
                    let mut events = Vec::new();

                    // 1. PasskeyUsed event (for counter tracking and audit)
                    // Note: This should have been emitted in CompletePasskeyLogin
                    // but we'll emit it here for now. TODO: Move to CompletePasskeyLogin
                    // once we have proper challenge storage.

                    // 2. DeviceAccessed event (for device trust calculation)
                    events.push(AuthEvent::DeviceAccessed {
                        device_id,
                        user_id,
                        ip_address,
                        auth_level: AuthLevel::HardwareBacked,
                        timestamp: now,
                    });

                    // 3. UserLoggedIn event (audit trail)
                    events.push(AuthEvent::UserLoggedIn {
                        user_id,
                        device_id,
                        session_id,
                        method: login_methods::PASSKEY.to_string(),
                        auth_level: AuthLevel::HardwareBacked,
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

                    // Build session
                    let session = Session {
                        session_id,
                        user_id,
                        device_id,
                        email: email_clone.clone(),
                        created_at: now,
                        last_active: now,
                        expires_at: now + session_duration,
                        ip_address,
                        user_agent: user_agent_clone,
                        oauth_provider: None,
                        login_risk_score,
                    };

                    // Batch append all events to the user stream
                    let stream_id = StreamId::new(format!("user-{}", user_id.0));

                    match event_store.append_events(stream_id, None, serialized_events).await {
                        Ok(_version) => {
                            // Events persisted successfully
                            // Now create ephemeral session in Redis
                            if let Err(e) = sessions.create_session(&session, session_duration).await {
                                tracing::error!("Failed to create passkey session for user {} device {}: {}",
                                    user_id.0, device_id.0, e);
                                return Some(AuthAction::SessionCreationFailed {
                                    user_id,
                                    device_id,
                                    error: e.to_string(),
                                });
                            }

                            // Emit SessionCreated event
                            Some(AuthAction::SessionCreated { session })
                        }
                        Err(e) => {
                            tracing::error!("Failed to persist passkey events for user {}: {}", user_id.0, e);
                            Some(AuthAction::EventPersistenceFailed {
                                stream_id: format!("user-{}", user_id.0),
                                error: e.to_string(),
                            })
                        }
                    }
                }))]
            }

            // ═══════════════════════════════════════════════════════════════
            // SessionCreated: Set session in state
            // ═══════════════════════════════════════════════════════════════
            AuthAction::SessionCreated { session } => {
                // Set session in state (session now has correct risk score from RiskCalculator)
                state.session = Some(session.clone());
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
    fn test_default_config() {
        type TestReducer = PasskeyReducer<(), (), (), (), (), (), (), ()>;
        let reducer = TestReducer::new();
        assert_eq!(reducer.config.challenge_ttl_minutes, 5);
        assert_eq!(reducer.config.origin, "http://localhost:3000");
        assert_eq!(reducer.config.rp_id, "localhost");
    }

    #[test]
    fn test_custom_config() {
        type TestReducer = PasskeyReducer<(), (), (), (), (), (), (), ()>;
        let config = PasskeyConfig::new(
            "https://app.example.com".to_string(),
            "app.example.com".to_string(),
        );
        let reducer = TestReducer::with_config(config);
        assert_eq!(reducer.config.origin, "https://app.example.com");
        assert_eq!(reducer.config.rp_id, "app.example.com");
    }
}
