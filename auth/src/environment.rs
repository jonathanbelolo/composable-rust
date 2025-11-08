//! Authentication environment.
//!
//! This module defines the environment type for dependency injection
//! in auth reducers.

use crate::providers::{
    OAuth2Provider, EmailProvider, WebAuthnProvider, SessionStore,
    UserRepository, DeviceRepository, RiskCalculator, TokenStore,
};
use composable_rust_core::event_store::EventStore;
use std::sync::Arc;

/// Authentication environment.
///
/// Contains all external dependencies needed by auth reducers.
///
/// # Type Parameters
///
/// - `O`: OAuth2 provider
/// - `E`: Email provider
/// - `W`: WebAuthn provider
/// - `S`: Session store
/// - `T`: Token store
/// - `U`: User repository
/// - `D`: Device repository
/// - `R`: Risk calculator
pub struct AuthEnvironment<O, E, W, S, T, U, D, R>
where
    O: OAuth2Provider,
    E: EmailProvider,
    W: WebAuthnProvider,
    S: SessionStore,
    T: TokenStore,
    U: UserRepository,
    D: DeviceRepository,
    R: RiskCalculator,
{
    /// OAuth2 provider.
    pub oauth: O,

    /// Email provider.
    pub email: E,

    /// WebAuthn provider.
    pub webauthn: W,

    /// Session store (Redis).
    pub sessions: S,

    /// Token store (Redis - one-time tokens with atomic consumption).
    pub tokens: T,

    /// User repository (PostgreSQL projection queries).
    pub users: U,

    /// Device repository (PostgreSQL projection queries).
    pub devices: D,

    /// Risk calculator.
    pub risk: R,

    /// Event store for event sourcing (PostgreSQL).
    pub event_store: Arc<dyn EventStore>,
}

impl<O, E, W, S, T, U, D, R> AuthEnvironment<O, E, W, S, T, U, D, R>
where
    O: OAuth2Provider,
    E: EmailProvider,
    W: WebAuthnProvider,
    S: SessionStore,
    T: TokenStore,
    U: UserRepository,
    D: DeviceRepository,
    R: RiskCalculator,
{
    /// Create a new authentication environment.
    #[must_use]
    pub fn new(
        oauth: O,
        email: E,
        webauthn: W,
        sessions: S,
        tokens: T,
        users: U,
        devices: D,
        risk: R,
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
            event_store,
        }
    }
}
