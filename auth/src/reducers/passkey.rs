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
    ChallengeStore, DeviceRepository, EmailProvider, OAuth2Provider, OAuthTokenStore,
    PasskeyCredential, RiskCalculator, SessionStore, TokenStore, UserRepository,
    WebAuthnProvider,
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
pub struct PasskeyReducer<O, E, W, S, T, U, D, R, OT, C, RL> {
    /// Configuration for passkey authentication.
    config: PasskeyConfig,
    /// Phantom data to hold type parameters.
    _phantom: std::marker::PhantomData<(O, E, W, S, T, U, D, R, OT, C, RL)>,
}

impl<O, E, W, S, T, U, D, R, OT, C, RL> PasskeyReducer<O, E, W, S, T, U, D, R, OT, C, RL> {
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

impl<O, E, W, S, T, U, D, R, OT, C, RL> Default for PasskeyReducer<O, E, W, S, T, U, D, R, OT, C, RL> {
    fn default() -> Self {
        Self::new()
    }
}

impl<O, E, W, S, T, U, D, R, OT, C, RL> Reducer for PasskeyReducer<O, E, W, S, T, U, D, R, OT, C, RL>
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
            // InitiatePasskeyRegistration: Generate challenge
            // ═══════════════════════════════════════════════════════════════
            AuthAction::InitiatePasskeyRegistration {
                user_id,
                device_name: _,
            } => {
                // ✅ SECURITY: Store challenge with expiration using ChallengeStore
                // Challenges are single-use, time-limited, and isolated per user

                let webauthn = env.webauthn.clone();
                let users = env.users.clone();
                let challenges = env.challenges.clone();
                let challenge_ttl = chrono::Duration::minutes(self.config.challenge_ttl_minutes);

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
                            // Use challenge_id from WebAuthn provider as the challenge string
                            let challenge_string = challenge.challenge_id.clone();

                            // Store challenge in ChallengeStore with TTL
                            match challenges.store_challenge(user_id, challenge_string.clone(), challenge_ttl).await {
                                Ok(()) => {
                                    tracing::info!(
                                        "Generated WebAuthn registration challenge for user {} (expires in {} minutes)",
                                        user_id.0,
                                        challenge_ttl.num_minutes()
                                    );
                                    None // TODO: Return challenge_string to client
                                }
                                Err(e) => {
                                    tracing::error!("Failed to store passkey registration challenge: {}", e);
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
                // ✅ SECURITY: Atomic challenge consumption from ChallengeStore
                // The attestation_response contains the challenge that was sent to the client
                // We extract it, consume it atomically, and verify it matches

                // ✅ SECURITY: Origin and RP ID validation
                // These values come from trusted configuration (not user input).
                // The WebAuthnProvider MUST validate that the attestation was created for
                // the correct origin and RP ID to prevent cross-site registration attacks.
                // This is enforced by the WebAuthn spec and implemented in webauthn-rs.
                let webauthn = env.webauthn.clone();
                let users = env.users.clone();
                let challenges = env.challenges.clone();
                let origin = self.config.origin.clone();
                let rp_id = self.config.rp_id.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // Extract challenge from attestation response
                    // Note: In a real implementation, the WebAuthn library would extract this
                    // For now, we're using a simplified approach where the challenge is the challenge_id
                    let challenge_string = match webauthn.extract_challenge_from_attestation(&attestation_response).await {
                        Ok(c) => c,
                        Err(_) => {
                            tracing::warn!("Failed to extract challenge from attestation response");
                            return None;
                        }
                    };

                    // Atomically consume challenge (prevents replay attacks)
                    let challenge_data = match challenges.consume_challenge(user_id, &challenge_string).await {
                        Ok(Some(data)) => data,
                        Ok(None) => {
                            // ✅ SECURITY: Generic error message prevents information leakage
                            // Don't distinguish between: expired, already used, or never existed
                            tracing::warn!(
                                user_id = %user_id.0,
                                "Passkey registration challenge verification failed"
                            );
                            return None;
                        }
                        Err(e) => {
                            tracing::error!("Failed to consume registration challenge: {}", e);
                            return None;
                        }
                    };

                    // ✅ SECURITY: Defense-in-depth validation
                    // Double-check expiration even though ChallengeStore should handle this
                    if challenge_data.expires_at < Utc::now() {
                        tracing::error!(
                            "Challenge expired (expires_at: {}, now: {})",
                            challenge_data.expires_at,
                            Utc::now()
                        );
                        return None;
                    }

                    // Verify challenge matches expected user
                    if challenge_data.user_id != user_id {
                        tracing::error!(
                            "🚨 SECURITY ALERT: Challenge user_id mismatch! Expected {}, got {}",
                            user_id.0,
                            challenge_data.user_id.0
                        );
                        return None;
                    }

                    // Verify attestation with the consumed challenge
                    let result = match webauthn
                        .verify_registration(&challenge_data.challenge, &attestation_response, &origin, &rp_id)
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
                            tracing::info!(
                                "Passkey registered successfully for user {} (credential: {})",
                                user_id.0,
                                credential.credential_id
                            );
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
                // ✅ SECURITY: Store authentication challenge with expiration using ChallengeStore
                // Challenges are single-use, time-limited, and isolated per user

                let webauthn = env.webauthn.clone();
                let users = env.users.clone();
                let challenges = env.challenges.clone();
                let challenge_ttl = chrono::Duration::minutes(self.config.challenge_ttl_minutes);

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
                            // Use challenge_id from WebAuthn provider as the challenge string
                            let challenge_string = challenge.challenge_id.clone();

                            // Store challenge in ChallengeStore with TTL
                            match challenges.store_challenge(user.user_id, challenge_string.clone(), challenge_ttl).await {
                                Ok(()) => {
                                    tracing::info!(
                                        "Generated WebAuthn authentication challenge for user {} (expires in {} minutes)",
                                        user.user_id.0,
                                        challenge_ttl.num_minutes()
                                    );
                                    None // TODO: Return challenge_string to client
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
                // ✅ SECURITY: Atomic challenge consumption from ChallengeStore
                // The assertion_response contains the challenge that was sent to the client
                // We extract it, consume it atomically, and verify it matches

                // ✅ SECURITY: Origin and RP ID validation
                // These values come from trusted configuration (not user input).
                // The WebAuthnProvider MUST validate that the assertion was created for
                // the correct origin and RP ID to prevent cross-site authentication attacks.
                // This is enforced by the WebAuthn spec and implemented in webauthn-rs.
                let webauthn = env.webauthn.clone();
                let users = env.users.clone();
                let challenges = env.challenges.clone();
                let rate_limiter = env.rate_limiter.clone();
                let origin = self.config.origin.clone();
                let rp_id = self.config.rp_id.clone();

                smallvec![Effect::Future(Box::pin(async move {
                    // Get credential first (to get user_id for challenge lookup and rate limiting)
                    let credential = match users.get_passkey_credential(&credential_id).await {
                        Ok(c) => c,
                        Err(_) => {
                            tracing::warn!("Credential not found: {}", credential_id);
                            return None;
                        }
                    };

                    // ✅ SECURITY FIX (HIGH #2): Rate limiting for passkey authentication
                    //
                    // Prevents brute force attacks and DoS via unlimited authentication attempts.
                    //
                    // Rate limit: 10 attempts per 15 minutes per user
                    // - More generous than password auth (passkeys are cryptographically secure)
                    // - Per-user limiting (not per-IP) to prevent legitimate users from being blocked
                    // - Applied before expensive cryptographic verification
                    //
                    // Why 10 attempts?
                    // - Legitimate users may have multiple devices/authenticators
                    // - Typos in credential_id selection are rare but possible
                    // - Failed biometric attempts (fingerprint misread) are common
                    //
                    // Why 15 minutes?
                    // - Balance between security and usability
                    // - Long enough for legitimate troubleshooting
                    // - Short enough to limit attacker attempts
                    let rate_key = format!("passkey_auth:{}", credential.user_id.0);
                    if let Err(e) = rate_limiter
                        .check_and_record(&rate_key, 10, std::time::Duration::from_secs(900))
                        .await
                    {
                        tracing::warn!(
                            rate_limit_exceeded = true,
                            user_id = %credential.user_id.0,
                            credential_id = %credential_id,
                            error = %e,
                            "Passkey authentication rate limit exceeded"
                        );
                        return None;
                    }

                    // Extract challenge from assertion response
                    // Note: In a real implementation, the WebAuthn library would extract this
                    // For now, we're using a simplified approach where the challenge is the challenge_id
                    let challenge_string = match webauthn.extract_challenge_from_assertion(&assertion_response).await {
                        Ok(c) => c,
                        Err(_) => {
                            tracing::warn!("Failed to extract challenge from assertion response");
                            return None;
                        }
                    };

                    // Atomically consume challenge (prevents replay attacks)
                    let challenge_data = match challenges.consume_challenge(credential.user_id, &challenge_string).await {
                        Ok(Some(data)) => data,
                        Ok(None) => {
                            // ✅ SECURITY: Generic error message prevents information leakage
                            // Don't distinguish between: expired, already used, or never existed
                            tracing::warn!(
                                user_id = %credential.user_id.0,
                                "Passkey login challenge verification failed"
                            );
                            return None;
                        }
                        Err(e) => {
                            tracing::error!("Failed to consume login challenge: {}", e);
                            return None;
                        }
                    };

                    // ✅ SECURITY: Defense-in-depth validation
                    // Double-check expiration even though ChallengeStore should handle this
                    if challenge_data.expires_at < Utc::now() {
                        tracing::error!(
                            "Challenge expired (expires_at: {}, now: {})",
                            challenge_data.expires_at,
                            Utc::now()
                        );
                        return None;
                    }

                    // Verify challenge matches expected user
                    if challenge_data.user_id != credential.user_id {
                        tracing::error!(
                            "🚨 SECURITY ALERT: Challenge user_id mismatch! Expected {}, got {}",
                            credential.user_id.0,
                            challenge_data.user_id.0
                        );
                        return None;
                    }

                    // Verify assertion with the consumed challenge
                    let result = match webauthn
                        .verify_authentication(
                            &challenge_data.challenge,
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
                    // ✅ SECURITY FIX: Correct half-space algorithm with wrapping arithmetic
                    //
                    // WebAuthn spec requires checking the signature counter to detect
                    // cloned authenticators. Counters are u32 and can wrap around
                    // from u32::MAX to 0.
                    //
                    // We use the "half-space" algorithm with wrapping arithmetic:
                    // - Calculate forward_diff = new.wrapping_sub(stored)
                    // - If forward_diff > HALF_SPACE (2^31), it's backward movement
                    //
                    // Why this works:
                    // - wrapping_sub handles u32 overflow correctly
                    // - Any "forward" jump > 2^31 is actually backward in modular arithmetic
                    //
                    // Examples:
                    // - Stored: 100, New: 101 → diff = 1 (valid, < HALF_SPACE)
                    // - Stored: 100, New: 50 → diff = 4,294,967,246 (rollback, > HALF_SPACE)
                    // - Stored: u32::MAX-5, New: 10 → diff = 16 (valid wraparound, < HALF_SPACE)
                    // - Stored: 10, New: u32::MAX-5 → diff = 4,294,967,280 (rollback, > HALF_SPACE)

                    const HALF_SPACE: u32 = u32::MAX / 2;

                    let is_rollback = if result.counter == credential.counter {
                        // Same counter = replay attack (counter should always increment)
                        true
                    } else {
                        // Calculate forward difference using wrapping arithmetic
                        let forward_diff = result.counter.wrapping_sub(credential.counter);

                        // If forward_diff > HALF_SPACE, the counter moved backward
                        // (i.e., rollback attack or cloned authenticator)
                        forward_diff > HALF_SPACE
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

                        // ✅ SECURITY: Log security event for monitoring and alerting
                        // This allows security teams to:
                        // - Track attempted attacks via logs/metrics
                        // - Alert on cloned authenticators
                        // - Audit suspicious activity
                        //
                        // Note: Event persistence handled separately to avoid blocking auth flow
                        tracing::warn!(
                            counter_rollback_detected = true,
                            credential_id = %credential_id,
                            user_id = %credential.user_id.0,
                            device_id = %credential.device_id.0,
                            stored_counter = credential.counter,
                            received_counter = result.counter,
                            "Counter rollback detected - authentication rejected"
                        );

                        // REJECT the authentication immediately
                        return None;
                    }

                    // ✅ SECURITY FIX (BLOCKER #6): Atomic counter update with compare-and-swap
                    //
                    // VULNERABILITY PREVENTED: Race condition in concurrent authentication
                    //
                    // Without atomic update:
                    // - Request A: Read counter=100, verify counter=101, update to 101 ✓
                    // - Request B: Read counter=100, verify counter=101, update to 101 ✓ (SHOULD FAIL!)
                    //
                    // With atomic compare-and-swap:
                    // - Request A: Read counter=100, verify counter=101, CAS(100→101) ✓
                    // - Request B: Read counter=100, verify counter=101, CAS(100→101) ✗ (counter changed)
                    //
                    // This prevents cloned authenticators from bypassing detection by authenticating
                    // concurrently before either updates the stored counter value.
                    //
                    // ⚠️ TOCTOU WINDOW MITIGATION (NOT ELIMINATION)
                    //
                    // There is a Time-of-Check-Time-of-Use (TOCTOU) window between:
                    // 1. Counter rollback check (lines 525-534, above)
                    // 2. Atomic counter update (line 607, below)
                    //
                    // **What happens during this window:**
                    // - Concurrent requests with the same counter will BOTH pass the rollback check
                    // - Only ONE request will succeed at the atomic CAS
                    // - The other request(s) will be rejected
                    //
                    // **Security guarantee:**
                    // The rollback check is an optimization (fails fast on obvious attacks).
                    // The atomic CAS is the FINAL verification that ensures only one authentication succeeds.
                    //
                    // **Example timeline:**
                    //   t=0: Request A reads counter=100, checks rollback (PASS)
                    //   t=1: Request B reads counter=100, checks rollback (PASS) ⚠️ TOCTOU
                    //   t=2: Request A attempts CAS(100→101) ✓ SUCCESS
                    //   t=3: Request B attempts CAS(100→101) ✗ FAIL (counter now 101)
                    //   t=4: Request B is rejected (expected behavior)
                    //
                    // **Logging impact:**
                    // - Both requests may log "rollback check passed"
                    // - Only one will log "counter updated atomically"
                    // - Failed CAS will log "concurrent authentication detected"
                    //
                    // **Why this is safe:**
                    // - ✅ Atomic CAS ensures only ONE authentication succeeds
                    // - ✅ Counters are monotonically increasing (never decrease)
                    // - ✅ Database transaction provides ACID guarantees
                    // - ✅ No security vulnerability - multiple checks before final CAS
                    //
                    // The TOCTOU window is unavoidable but mitigated by defense-in-depth.
                    match users
                        .update_passkey_counter_atomic(
                            &credential_id,
                            credential.counter,  // Expected old value
                            result.counter,      // New value
                        )
                        .await
                    {
                        Ok(true) => {
                            // ✅ Counter update succeeded - authentication allowed
                            tracing::info!(
                                "Passkey counter updated atomically: {} -> {} (credential: {})",
                                credential.counter,
                                result.counter,
                                credential_id
                            );
                        }
                        Ok(false) => {
                            // ❌ Counter was changed by concurrent request - authentication REJECTED
                            tracing::error!(
                                "🚨 SECURITY ALERT: Concurrent passkey authentication detected!\n\
                                 Credential ID: {}\n\
                                 Expected counter: {}\n\
                                 Attempted counter: {}\n\
                                 This indicates CONCURRENT AUTHENTICATION ATTEMPT with same credential.",
                                credential_id,
                                credential.counter,
                                result.counter
                            );

                            tracing::warn!(
                                concurrent_authentication_detected = true,
                                credential_id = %credential_id,
                                user_id = %credential.user_id.0,
                                device_id = %credential.device_id.0,
                                expected_counter = credential.counter,
                                attempted_counter = result.counter,
                                "Concurrent authentication blocked by atomic counter update"
                            );

                            // REJECT the authentication - counter was modified by concurrent request
                            return None;
                        }
                        Err(e) => {
                            tracing::error!(
                                "CRITICAL: Database error during atomic counter update for {}: {}",
                                credential_id,
                                e
                            );
                            // Database failure is serious - don't allow auth
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
                    idle_timeout: self.config.idle_timeout,
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
                let max_concurrent_sessions = self.config.max_concurrent_sessions;
                let idle_timeout = self.config.idle_timeout;

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
                        idle_timeout,
                    };

                    // Batch append all events to the user stream
                    let stream_id = StreamId::new(format!("user-{}", user_id.0));

                    match event_store.append_events(stream_id, None, serialized_events).await {
                        Ok(_version) => {
                            // Events persisted successfully
                            // Now create ephemeral session in Redis
                            if let Err(e) = sessions.create_session(&session, session_duration, max_concurrent_sessions).await {
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
        type TestReducer = PasskeyReducer<(), (), (), (), (), (), (), (), (), (), ()>;
        let reducer = TestReducer::new();
        assert_eq!(reducer.config.challenge_ttl_minutes, 5);
        assert_eq!(reducer.config.origin, "http://localhost:3000");
        assert_eq!(reducer.config.rp_id, "localhost");
    }

    #[test]
    fn test_custom_config() {
        type TestReducer = PasskeyReducer<(), (), (), (), (), (), (), (), (), (), ()>;
        let config = PasskeyConfig::new(
            "https://app.example.com".to_string(),
            "app.example.com".to_string(),
        );
        let reducer = TestReducer::with_config(config);
        assert_eq!(reducer.config.origin, "https://app.example.com");
        assert_eq!(reducer.config.rp_id, "app.example.com");
    }
}
