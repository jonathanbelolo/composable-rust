//! Authentication module for ticketing system
//!
//! This module integrates the `composable-rust-auth` framework with
//! the ticketing application. It provides:
//! - Re-exports of framework components
//! - Custom email provider (console output for demo)
//! - Environment setup helpers
//! - Custom auth handlers with testing support

// Custom email provider for ticketing
pub mod email;
// Authentication setup (build environment and store)
pub mod setup;
// Authentication middleware (extractors for protected routes)
pub mod middleware;
// Custom authentication handlers (with testing support)
pub mod handlers;

// Re-export framework components
pub use composable_rust_auth::{
    // Core types
    AuthAction,
    // Environment
    AuthEnvironment,
    // Error types
    AuthError,
    AuthReducer,
    AuthState,
    Result,
    // Provider traits
    providers::{
        ChallengeStore, DeviceRepository, EmailProvider, OAuth2Provider, OAuthTokenStore,
        RateLimiter, RiskCalculator, SessionStore, TokenStore, UserRepository, WebAuthnProvider,
    },
    // State types
    state::{DeviceId, Session, SessionId, UserId},
    // Store implementations
    stores::{
        PostgresDeviceRepository, PostgresUserRepository, RedisChallengeStore,
        RedisOAuthTokenStore, RedisRateLimiter, RedisSessionStore, RedisTokenStore,
    },
};

// Re-export our custom email provider
pub use email::ConsoleEmailProvider;

// Note: auth_router is available via composable_rust_auth::auth_router (behind axum feature)
// We don't re-export it here to avoid feature flag complexity
