//! Authentication module for ticketing system
//!
//! This module integrates the `composable-rust-auth` framework with
//! the ticketing application. It provides:
//! - Re-exports of framework components
//! - Custom email provider (console output for demo)
//! - Environment setup helpers

// Custom email provider for ticketing
pub mod email;
// Authentication setup (build environment and store)
pub mod setup;

// Re-export framework components
pub use composable_rust_auth::{
    // Core types
    AuthAction, AuthReducer, AuthState,
    // State types
    state::{Session, SessionId, UserId, DeviceId},
    // Error types
    AuthError, Result,
    // Store implementations
    stores::{
        RedisSessionStore,
        RedisTokenStore,
        RedisChallengeStore,
        RedisOAuthTokenStore,
        RedisRateLimiter,
        PostgresUserRepository,
        PostgresDeviceRepository,
    },
    // Provider traits
    providers::{
        EmailProvider,
        OAuth2Provider,
        WebAuthnProvider,
        SessionStore,
        TokenStore,
        ChallengeStore,
        OAuthTokenStore,
        UserRepository,
        DeviceRepository,
        RateLimiter,
        RiskCalculator,
    },
    // Environment
    AuthEnvironment,
};

// Re-export our custom email provider
pub use email::ConsoleEmailProvider;

// Note: auth_router is available via composable_rust_auth::auth_router (behind axum feature)
// We don't re-export it here to avoid feature flag complexity
