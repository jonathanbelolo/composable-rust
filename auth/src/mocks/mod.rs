//! Mock provider implementations for testing.
//!
//! This module provides simple, in-memory implementations of all provider traits
//! for use in unit and integration tests.

pub mod oauth;
pub mod session;
pub mod user;
pub mod device;
pub mod email;
pub mod webauthn;
pub mod risk;

pub use oauth::MockOAuth2Provider;
pub use session::MockSessionStore;
pub use user::MockUserRepository;
pub use device::MockDeviceRepository;
pub use email::MockEmailProvider;
pub use webauthn::MockWebAuthnProvider;
pub use risk::MockRiskCalculator;
