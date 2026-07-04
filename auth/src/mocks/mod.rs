//! Mock provider implementations for testing.
//!
//! This module provides simple, in-memory implementations of all provider traits
//! for use in unit and integration tests.

pub mod challenge_store;
pub mod device;
pub mod email;
pub mod oauth;
pub mod oauth_token_store;
pub mod rate_limiter;
pub mod risk;
pub mod session;
pub mod token_store;
pub mod user;
pub mod webauthn;

pub use challenge_store::MockChallengeStore;
pub use device::MockDeviceRepository;
pub use email::MockEmailProvider;
pub use oauth::MockOAuth2Provider;
pub use oauth_token_store::MockOAuthTokenStore;
pub use rate_limiter::MockRateLimiter;
pub use risk::MockRiskCalculator;
pub use session::MockSessionStore;
pub use token_store::MockTokenStore;
pub use user::MockUserRepository;
pub use webauthn::MockWebAuthnProvider;
