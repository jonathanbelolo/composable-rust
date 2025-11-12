//! Authentication reducers.
//!
//! This module contains pure reducer functions for authentication.
//!
//! Reducers are pure functions: `(State, Action, Environment) → (State, Effects)`.

pub mod magic_link;
pub mod oauth;
pub mod passkey;

use crate::{AuthAction, AuthState, AuthEnvironment};
use composable_rust_core::{effect::Effect, reducer::Reducer, SmallVec};

// Re-export
pub use magic_link::MagicLinkReducer;
pub use oauth::OAuthReducer;
pub use passkey::PasskeyReducer;

/// Unified authentication reducer.
///
/// Combines `OAuth`, Magic Link, and Passkey flows into a single reducer.
/// Routes actions to the appropriate sub-reducer based on action type.
#[derive(Clone, Debug)]
pub struct AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>
where
    O: crate::providers::OAuth2Provider + Clone + 'static,
    E: crate::providers::EmailProvider + Clone + 'static,
    W: crate::providers::WebAuthnProvider + Clone + 'static,
    S: crate::providers::SessionStore + Clone + 'static,
    T: crate::providers::TokenStore + Clone + 'static,
    U: crate::providers::UserRepository + Clone + 'static,
    D: crate::providers::DeviceRepository + Clone + 'static,
    R: crate::providers::RiskCalculator + Clone + 'static,
    OT: crate::providers::OAuthTokenStore + Clone + 'static,
    C: crate::providers::ChallengeStore + Clone + 'static,
    RL: crate::providers::RateLimiter + Clone + 'static,
{
    oauth: OAuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>,
    magic_link: MagicLinkReducer<O, E, W, S, T, U, D, R, OT, C, RL>,
    passkey: PasskeyReducer<O, E, W, S, T, U, D, R, OT, C, RL>,
}

impl<O, E, W, S, T, U, D, R, OT, C, RL> AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>
where
    O: crate::providers::OAuth2Provider + Clone + 'static,
    E: crate::providers::EmailProvider + Clone + 'static,
    W: crate::providers::WebAuthnProvider + Clone + 'static,
    S: crate::providers::SessionStore + Clone + 'static,
    T: crate::providers::TokenStore + Clone + 'static,
    U: crate::providers::UserRepository + Clone + 'static,
    D: crate::providers::DeviceRepository + Clone + 'static,
    R: crate::providers::RiskCalculator + Clone + 'static,
    OT: crate::providers::OAuthTokenStore + Clone + 'static,
    C: crate::providers::ChallengeStore + Clone + 'static,
    RL: crate::providers::RateLimiter + Clone + 'static,
{
    /// Create a new unified auth reducer with default configurations.
    #[must_use]
    pub fn new() -> Self {
        Self {
            oauth: OAuthReducer::new(),
            magic_link: MagicLinkReducer::new(),
            passkey: PasskeyReducer::new(),
        }
    }
}

impl<O, E, W, S, T, U, D, R, OT, C, RL> Default for AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>
where
    O: crate::providers::OAuth2Provider + Clone + 'static,
    E: crate::providers::EmailProvider + Clone + 'static,
    W: crate::providers::WebAuthnProvider + Clone + 'static,
    S: crate::providers::SessionStore + Clone + 'static,
    T: crate::providers::TokenStore + Clone + 'static,
    U: crate::providers::UserRepository + Clone + 'static,
    D: crate::providers::DeviceRepository + Clone + 'static,
    R: crate::providers::RiskCalculator + Clone + 'static,
    OT: crate::providers::OAuthTokenStore + Clone + 'static,
    C: crate::providers::ChallengeStore + Clone + 'static,
    RL: crate::providers::RateLimiter + Clone + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<O, E, W, S, T, U, D, R, OT, C, RL> Reducer for AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>
where
    O: crate::providers::OAuth2Provider + Clone + 'static,
    E: crate::providers::EmailProvider + Clone + 'static,
    W: crate::providers::WebAuthnProvider + Clone + 'static,
    S: crate::providers::SessionStore + Clone + 'static,
    T: crate::providers::TokenStore + Clone + 'static,
    U: crate::providers::UserRepository + Clone + 'static,
    D: crate::providers::DeviceRepository + Clone + 'static,
    R: crate::providers::RiskCalculator + Clone + 'static,
    OT: crate::providers::OAuthTokenStore + Clone + 'static,
    C: crate::providers::ChallengeStore + Clone + 'static,
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
        // Route to appropriate sub-reducer based on action type
        match action {
            // OAuth actions
            AuthAction::InitiateOAuth { .. }
            | AuthAction::OAuthCallback { .. }
            | AuthAction::OAuthAuthorizationUrlReady { .. }
            | AuthAction::OAuthSuccess { .. }
            | AuthAction::RefreshOAuthToken { .. }
            | AuthAction::OAuthTokenRefreshed { .. }
            | AuthAction::OAuthFailed { .. } => {
                self.oauth.reduce(state, action, env)
            }

            // Magic Link actions
            AuthAction::SendMagicLink { .. }
            | AuthAction::MagicLinkSent { .. }
            | AuthAction::VerifyMagicLink { .. }
            | AuthAction::MagicLinkVerified { .. }
            | AuthAction::MagicLinkFailed { .. } => {
                self.magic_link.reduce(state, action, env)
            }

            // Passkey actions
            AuthAction::InitiatePasskeyRegistration { .. }
            | AuthAction::CompletePasskeyRegistration { .. }
            | AuthAction::PasskeyRegistrationChallengeGenerated { .. }
            | AuthAction::PasskeyRegistrationSuccess { .. }
            | AuthAction::PasskeyRegistrationFailed { .. }
            | AuthAction::InitiatePasskeyLogin { .. }
            | AuthAction::CompletePasskeyLogin { .. }
            | AuthAction::PasskeyLoginChallengeGenerated { .. }
            | AuthAction::PasskeyLoginSuccess { .. }
            | AuthAction::ListPasskeyCredentials { .. }
            | AuthAction::PasskeyCredentialsListed { .. }
            | AuthAction::DeletePasskeyCredential { .. }
            | AuthAction::PasskeyCredentialDeleted { .. }
            | AuthAction::PasskeyCredentialDeletionFailed { .. }
            | AuthAction::PasskeyAuthenticationFailed { .. } => {
                self.passkey.reduce(state, action, env)
            }

            // Session management
            AuthAction::SessionCreated { .. }
            | AuthAction::ValidateSession { .. }
            | AuthAction::SessionValidated { .. }
            | AuthAction::SessionExpired { .. }
            | AuthAction::Logout { .. }
            | AuthAction::LogoutSuccess { .. }
            | AuthAction::RevokeDevice { .. }
            | AuthAction::RevokeAllSessions { .. }
            | AuthAction::RequestStepUp { .. }
            | AuthAction::StepUpCompleted { .. }
            | AuthAction::UpdateDeviceTrust { .. } => {
                // Route to magic_link reducer for session management
                // (could be handled by a dedicated session reducer in future)
                self.magic_link.reduce(state, action, env)
            }

            // Event persistence
            AuthAction::EventPersisted { .. }
            | AuthAction::EventPersistenceFailed { .. } => {
                // Event persistence actions don't produce effects
                SmallVec::new()
            }

            // Error actions
            AuthAction::SessionCreationFailed { .. } => {
                // Error actions typically don't produce effects
                SmallVec::new()
            }
        }
    }
}
