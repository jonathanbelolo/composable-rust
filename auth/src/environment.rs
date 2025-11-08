//! Authentication environment.
//!
//! This module defines the environment type for dependency injection
//! in auth reducers.

use crate::providers::{
    OAuth2Provider, EmailProvider, WebAuthnProvider, SessionStore,
    UserRepository, DeviceRepository, RiskCalculator,
};

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
/// - `U`: User repository
/// - `D`: Device repository
/// - `R`: Risk calculator
pub struct AuthEnvironment<O, E, W, S, U, D, R>
where
    O: OAuth2Provider,
    E: EmailProvider,
    W: WebAuthnProvider,
    S: SessionStore,
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

    /// User repository (PostgreSQL).
    pub users: U,

    /// Device repository (PostgreSQL).
    pub devices: D,

    /// Risk calculator.
    pub risk: R,
}

impl<O, E, W, S, U, D, R> AuthEnvironment<O, E, W, S, U, D, R>
where
    O: OAuth2Provider,
    E: EmailProvider,
    W: WebAuthnProvider,
    S: SessionStore,
    U: UserRepository,
    D: DeviceRepository,
    R: RiskCalculator,
{
    /// Create a new authentication environment.
    #[must_use]
    pub const fn new(
        oauth: O,
        email: E,
        webauthn: W,
        sessions: S,
        users: U,
        devices: D,
        risk: R,
    ) -> Self {
        Self {
            oauth,
            email,
            webauthn,
            sessions,
            users,
            devices,
            risk,
        }
    }
}
