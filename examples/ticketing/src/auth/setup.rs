//! Authentication setup - builds `AuthEnvironment` and `Store` for the ticketing system.
//!
//! This module wires together all 11 provider traits required by the auth framework:
//! - OAuth2Provider (mock for demo)
//! - EmailProvider (console output)
//! - WebAuthnProvider (mock for demo)
//! - SessionStore (Redis)
//! - TokenStore (Redis)
//! - ChallengeStore (Redis)
//! - OAuthTokenStore (Redis)
//! - UserRepository (PostgreSQL)
//! - DeviceRepository (PostgreSQL)
//! - RateLimiter (Redis)
//! - RiskCalculator (mock for demo)

use crate::config::Config;
use composable_rust_auth::{
    AuthAction, AuthEnvironment, AuthReducer, AuthState, Result,
    mocks::{MockOAuth2Provider, MockWebAuthnProvider, MockRiskCalculator},
    stores::{
        RedisSessionStore, RedisTokenStore, RedisChallengeStore,
        RedisOAuthTokenStore, RedisRateLimiter,
        PostgresUserRepository, PostgresDeviceRepository,
    },
};
use crate::auth::ConsoleEmailProvider;
use composable_rust_runtime::Store;
use composable_rust_postgres::PostgresEventStore;
use sqlx::PgPool;
use std::sync::Arc;

/// Type alias for the concrete `AuthEnvironment` used in this application.
///
/// Combines framework providers (Redis/PostgreSQL stores) with custom providers
/// (Console email, Mock OAuth/WebAuthn) for demo purposes.
pub type TicketingAuthEnvironment = AuthEnvironment<
    MockOAuth2Provider,
    ConsoleEmailProvider,
    MockWebAuthnProvider,
    RedisSessionStore,
    RedisTokenStore,
    PostgresUserRepository,
    PostgresDeviceRepository,
    MockRiskCalculator,
    RedisOAuthTokenStore,
    RedisChallengeStore,
    RedisRateLimiter,
>;

/// Type alias for the `AuthReducer` configured with ticketing providers.
pub type TicketingAuthReducer = AuthReducer<
    MockOAuth2Provider,
    ConsoleEmailProvider,
    MockWebAuthnProvider,
    RedisSessionStore,
    RedisTokenStore,
    PostgresUserRepository,
    PostgresDeviceRepository,
    MockRiskCalculator,
    RedisOAuthTokenStore,
    RedisChallengeStore,
    RedisRateLimiter,
>;

/// Type alias for the auth `Store` with all ticketing-specific types.
pub type TicketingAuthStore = Store<AuthState, AuthAction, TicketingAuthEnvironment, TicketingAuthReducer>;

/// Build `AuthEnvironment` with all 11 providers
///
/// # Errors
///
/// Returns error if Redis or PostgreSQL connections fail.
pub async fn build_auth_environment(
    config: &Config,
    pg_pool: PgPool,
) -> Result<TicketingAuthEnvironment> {
    let auth_config = &config.auth;
    let redis_url = &config.redis.url;
    let postgres_url = &config.postgres.url;

    // Create the event store for auth events
    let event_store = PostgresEventStore::new(postgres_url)
        .await
        .map_err(|e| composable_rust_auth::AuthError::InternalError(format!("Event store error: {e}")))?;

    // Generate encryption key for OAuth tokens (32 bytes for AES-256)
    // In production, load this from secure configuration
    let encryption_key = auth_config.jwt_secret.as_bytes()[..32].to_vec();

    Ok(AuthEnvironment {
        oauth: MockOAuth2Provider::new(),
        email: ConsoleEmailProvider::new(auth_config.base_url.clone()),
        webauthn: MockWebAuthnProvider,
        sessions: RedisSessionStore::new(redis_url).await?,
        tokens: RedisTokenStore::new(redis_url).await?,
        users: PostgresUserRepository::new(pg_pool.clone()),
        devices: PostgresDeviceRepository::new(pg_pool),
        risk: MockRiskCalculator::new(),
        oauth_tokens: RedisOAuthTokenStore::new(redis_url, encryption_key).await?,
        challenges: RedisChallengeStore::new(redis_url).await?,
        rate_limiter: RedisRateLimiter::new(redis_url).await?,
        event_store: Arc::new(event_store),
    })
}

/// Build auth `Store` with `AuthReducer`
///
/// # Errors
///
/// Returns error if environment setup fails.
pub async fn build_auth_store(
    config: &Config,
    pg_pool: PgPool,
) -> Result<Arc<TicketingAuthStore>> {
    let env = build_auth_environment(config, pg_pool).await?;
    let store = Store::new(
        AuthState::default(),
        TicketingAuthReducer::new(),
        env,
    );
    Ok(Arc::new(store))
}
