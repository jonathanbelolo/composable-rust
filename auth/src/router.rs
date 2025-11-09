//! Authentication router composition.
//!
//! Composes all authentication handlers into a single Axum router.

use crate::{AuthAction, AuthEnvironment, AuthReducer, AuthState};
use crate::handlers::{magic_link, oauth, passkey, session};
use axum::{
    routing::{get, post},
    Router,
};
use composable_rust_runtime::Store;
use std::sync::Arc;

/// Create authentication router with all auth endpoints.
///
/// # Routes
///
/// ## Magic Link
/// - `POST /magic-link/send` - Send magic link email
/// - `GET /magic-link/verify` - Verify magic link token
///
/// ## OAuth
/// - `GET /oauth/:provider/authorize` - Redirect to OAuth provider
/// - `GET /oauth/:provider/callback` - Handle OAuth callback
///
/// ## Passkey
/// - `POST /passkey/register/begin` - Begin passkey registration
/// - `POST /passkey/register/complete` - Complete passkey registration
/// - `POST /passkey/login/begin` - Begin passkey login
/// - `POST /passkey/login/complete` - Complete passkey login
///
/// ## Session
/// - `GET /session` - Get session info
/// - `POST /logout` - Logout (destroy session)
///
/// # Example
///
/// ```rust,ignore
/// let store = Arc::new(Store::new(
///     AuthState::default(),
///     AuthReducer::new(),
///     environment,
/// ));
///
/// let app = Router::new()
///     .nest("/api/v1/auth", auth_router(store))
///     .layer(TraceLayer::new_for_http());
/// ```
pub fn auth_router<O, E, W, S, T, U, D, R, OT, C, RL>(
    store: Arc<Store<AuthState, AuthAction, AuthEnvironment<O, E, W, S, T, U, D, R, OT, C, RL>, AuthReducer<O, E, W, S, T, U, D, R, OT, C, RL>>>,
) -> Router
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
    Router::new()
        // Magic Link routes
        .route("/magic-link/send", post(magic_link::send_magic_link::<O, E, W, S, T, U, D, R, OT, C, RL>))
        .route("/magic-link/verify", get(magic_link::verify_magic_link::<O, E, W, S, T, U, D, R, OT, C, RL>))

        // OAuth routes
        .route("/oauth/:provider/authorize", get(oauth::oauth_authorize::<O, E, W, S, T, U, D, R, OT, C, RL>))
        .route("/oauth/:provider/callback", get(oauth::oauth_callback::<O, E, W, S, T, U, D, R, OT, C, RL>))

        // Passkey routes
        .route("/passkey/register/begin", post(passkey::begin_passkey_registration::<O, E, W, S, T, U, D, R, OT, C, RL>))
        .route("/passkey/register/complete", post(passkey::complete_passkey_registration::<O, E, W, S, T, U, D, R, OT, C, RL>))
        .route("/passkey/login/begin", post(passkey::begin_passkey_login::<O, E, W, S, T, U, D, R, OT, C, RL>))
        .route("/passkey/login/complete", post(passkey::complete_passkey_login::<O, E, W, S, T, U, D, R, OT, C, RL>))

        // Session routes
        .route("/session", get(session::get_session::<O, E, W, S, T, U, D, R, OT, C, RL>))
        .route("/logout", post(session::logout::<O, E, W, S, T, U, D, R, OT, C, RL>))

        .with_state(store)
}
