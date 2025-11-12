//! Authentication environment.
//!
//! This module defines the environment type for dependency injection
//! in auth reducers.

use crate::providers::{
    ChallengeStore, OAuth2Provider, EmailProvider, WebAuthnProvider, SessionStore,
    UserRepository, DeviceRepository, RiskCalculator, TokenStore,
    OAuthTokenStore, RateLimiter,
};
use composable_rust_core::event_store::EventStore;
use std::sync::Arc;

/// Authentication environment.
///
/// Contains all external dependencies needed by auth reducers.
///
/// # Type Parameters
///
/// - `O`: `OAuth2` provider
/// - `E`: Email provider
/// - `W`: `WebAuthn` provider
/// - `S`: Session store
/// - `T`: Token store
/// - `U`: User repository
/// - `D`: Device repository
/// - `R`: Risk calculator
/// - `OT`: `OAuth` token store
/// - `C`: Challenge store
/// - `RL`: Rate limiter
#[derive(Clone)]
pub struct AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>
where
    O: OAuth2Provider + Clone,
    E: EmailProvider + Clone,
    W: WebAuthnProvider + Clone,
    S: SessionStore + Clone,
    T: TokenStore + Clone,
    U: UserRepository + Clone,
    D: DeviceRepository + Clone,
    R: RiskCalculator + Clone,
    OT: OAuthTokenStore + Clone,
    C: ChallengeStore + Clone,
    RL: RateLimiter + Clone,
{
    /// `OAuth2` provider.
    pub oauth: O,

    /// Email provider.
    pub email: E,

    /// `WebAuthn` provider.
    pub webauthn: W,

    /// Session store (`Redis`).
    pub sessions: S,

    /// Token store (`Redis` - one-time tokens with atomic consumption).
    pub tokens: T,

    /// User repository (`PostgreSQL` projection queries).
    pub users: U,

    /// Device repository (`PostgreSQL` projection queries).
    pub devices: D,

    /// Risk calculator.
    pub risk: R,

    /// `OAuth` token store (`PostgreSQL` - encrypted access/refresh tokens).
    pub oauth_tokens: OT,

    /// Challenge store (`Redis` - `WebAuthn` challenges with atomic consumption).
    pub challenges: C,

    /// Rate limiter (`Redis` - brute force protection).
    pub rate_limiter: RL,

    /// Event store for event sourcing (`PostgreSQL`).
    pub event_store: Arc<dyn EventStore>,
}

impl<O, E, W, S, T, U, D, R, OT, C, RL> AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>
where
    O: OAuth2Provider + Clone,
    E: EmailProvider + Clone,
    W: WebAuthnProvider + Clone,
    S: SessionStore + Clone,
    T: TokenStore + Clone,
    U: UserRepository + Clone,
    D: DeviceRepository + Clone,
    R: RiskCalculator + Clone,
    OT: OAuthTokenStore + Clone,
    C: ChallengeStore + Clone,
    RL: RateLimiter + Clone,
{
    /// Create a new authentication environment.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        oauth: O,
        email: E,
        webauthn: W,
        sessions: S,
        tokens: T,
        users: U,
        devices: D,
        risk: R,
        oauth_tokens: OT,
        challenges: C,
        rate_limiter: RL,
        event_store: Arc<dyn EventStore>,
    ) -> Self {
        Self {
            oauth,
            email,
            webauthn,
            sessions,
            tokens,
            users,
            devices,
            risk,
            oauth_tokens,
            challenges,
            rate_limiter,
            event_store,
        }
    }
}
