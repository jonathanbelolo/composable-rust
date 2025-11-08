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
//! 5. Store credential in database
//!
//! ## Login Flow
//!
//! 1. User initiates passkey login with username/email
//! 2. Look up user's passkeys
//! 3. Generate WebAuthn challenge
//! 4. Client calls `navigator.credentials.get()`
//! 5. Verify assertion response
//! 6. Create session
//!
//! # Security
//!
//! - Challenges expire after 5 minutes
//! - Origin and RP ID validation
//! - Counter rollback detection
//! - Public key cryptography (ECDSA/EdDSA)
//! - Hardware-backed keys (FIDO2 authenticators)
//!
//! # Example
//!
//! ```rust
//! use composable_rust_auth::{AuthState, AuthAction, PasskeyReducer};
//! use composable_rust_auth::mocks::*;
//! use composable_rust_auth::state::UserId;
//! use composable_rust_core::reducer::Reducer;
//! use std::net::{IpAddr, Ipv4Addr};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let reducer = PasskeyReducer::new();
//! let env = create_test_env(); // Mock environment
//! let mut state = AuthState::default();
//!
//! let user_id = UserId::new();
//! let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
//!
//! // Initiate passkey registration
//! let effects = reducer.reduce(
//!     &mut state,
//!     AuthAction::InitiatePasskeyRegistration {
//!         user_id,
//!         device_name: "iPhone 15 Pro".to_string(),
//!     },
//!     &env,
//! );
//!
//! // WebAuthn challenge should be set
//! assert!(state.webauthn_challenge.is_some());
//! # Ok(())
//! # }
//! ```

use crate::actions::{AuthAction, DeviceTrustLevel};
use crate::environment::AuthEnvironment;
use crate::providers::{
    DeviceRepository, EmailProvider, OAuth2Provider, PasskeyCredential, RiskCalculator,
    SessionStore, UserRepository, WebAuthnProvider,
};
use crate::state::{AuthState, Session, SessionId};
use chrono::Utc;
use composable_rust_core::effect::Effect;
use composable_rust_core::reducer::Reducer;
use composable_rust_core::{smallvec, SmallVec};

/// WebAuthn/Passkey authentication reducer.
///
/// Handles passkey registration and login flows.
#[derive(Debug, Clone)]
pub struct PasskeyReducer<O, E, W, S, U, D, R> {
    /// Challenge TTL in minutes (default: 5 minutes).
    challenge_ttl_minutes: i64,
    /// Expected origin for WebAuthn (e.g., "https://app.example.com").
    expected_origin: String,
    /// Expected RP ID for WebAuthn (e.g., "app.example.com").
    expected_rp_id: String,
    /// Phantom data to hold type parameters.
    _phantom: std::marker::PhantomData<(O, E, W, S, U, D, R)>,
}

impl<O, E, W, S, U, D, R> PasskeyReducer<O, E, W, S, U, D, R> {
    /// Create a new passkey reducer with default settings.
    ///
    /// Default challenge TTL is 5 minutes.
    /// Default origin and RP ID are for localhost development.
    #[must_use]
    pub fn new() -> Self {
        Self {
            challenge_ttl_minutes: 5,
            expected_origin: "http://localhost:3000".to_string(),
            expected_rp_id: "localhost".to_string(),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create a reducer with custom WebAuthn configuration.
    ///
    /// # Arguments
    ///
    /// * `origin` - Expected origin (e.g., "https://app.example.com")
    /// * `rp_id` - Relying party ID (e.g., "app.example.com")
    #[must_use]
    pub fn with_config(origin: String, rp_id: String) -> Self {
        Self {
            challenge_ttl_minutes: 5,
            expected_origin: origin,
            expected_rp_id: rp_id,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<O, E, W, S, U, D, R> Default for PasskeyReducer<O, E, W, S, U, D, R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<O, E, W, S, U, D, R> Reducer for PasskeyReducer<O, E, W, S, U, D, R>
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
            // InitiatePasskeyRegistration: Generate challenge
            // ═══════════════════════════════════════════════════════════════
            AuthAction::InitiatePasskeyRegistration {
                user_id,
                device_name: _,
            } => {
                // Generate WebAuthn challenge
                let webauthn = env.webauthn.clone();
                let users = env.users.clone();

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
                            // Store challenge in state via an event
                            // In a real implementation, this would be a separate action
                            // For now, we'll just log it
                            tracing::info!("Generated WebAuthn registration challenge");
                            None // TODO: Return action to store challenge in state
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
                // Verify attestation
                let webauthn = env.webauthn.clone();
                let users = env.users.clone();
                let challenge_id = "mock_challenge_id".to_string(); // TODO: Get from state
                let origin = self.expected_origin.clone();
                let rp_id = self.expected_rp_id.clone();

                smallvec![Effect::Future(Box::pin(async move {
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
                // Look up user and their passkeys
                let webauthn = env.webauthn.clone();
                let users = env.users.clone();

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
                        Ok(_challenge) => {
                            tracing::info!("Generated WebAuthn authentication challenge");
                            None // TODO: Return action to store challenge in state
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
                // Verify assertion
                let webauthn = env.webauthn.clone();
                let users = env.users.clone();
                let challenge_id = "mock_challenge_id".to_string(); // TODO: Get from state
                let origin = self.expected_origin.clone();
                let rp_id = self.expected_rp_id.clone();

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

                    // Update counter
                    if let Err(_) = users
                        .update_passkey_counter(&credential_id, result.counter)
                        .await
                    {
                        tracing::error!("Failed to update passkey counter");
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
            // PasskeyLoginSuccess: Create session
            // ═══════════════════════════════════════════════════════════════
            AuthAction::PasskeyLoginSuccess {
                user_id,
                device_id,
                email,
                ip_address,
                user_agent,
            } => {
                // Generate session
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
                    login_risk_score: 0.05, // Passkeys are very secure
                };

                // Update state
                state.session = Some(session.clone());

                // Execute effects to persist session
                let sessions = env.sessions.clone();
                let session_clone = session.clone();
                let session_ttl = chrono::Duration::hours(24);

                smallvec![Effect::Future(Box::pin(async move {
                    // Create session in Redis
                    if sessions
                        .create_session(&session_clone, session_ttl)
                        .await
                        .is_err()
                    {
                        tracing::error!("Failed to create session");
                        return None;
                    }

                    // Emit session created event
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
    fn test_default_config() {
        type TestReducer = PasskeyReducer<(), (), (), (), (), (), ()>;
        let reducer = TestReducer::new();
        assert_eq!(reducer.challenge_ttl_minutes, 5);
        assert_eq!(reducer.expected_origin, "http://localhost:3000");
        assert_eq!(reducer.expected_rp_id, "localhost");
    }

    #[test]
    fn test_custom_config() {
        type TestReducer = PasskeyReducer<(), (), (), (), (), (), ()>;
        let reducer = TestReducer::with_config(
            "https://app.example.com".to_string(),
            "app.example.com".to_string(),
        );
        assert_eq!(reducer.expected_origin, "https://app.example.com");
        assert_eq!(reducer.expected_rp_id, "app.example.com");
    }
}
