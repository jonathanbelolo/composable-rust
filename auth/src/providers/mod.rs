//! Authentication providers.
//!
//! This module defines traits for all external dependencies used by the
//! auth system. These traits enable dependency injection and make the
//! auth logic testable.
//!
//! # Architecture
//!
//! Providers are **interfaces**, not implementations. The reducer depends
//! on these traits, and the runtime provides concrete implementations.
//!
//! This enables:
//! - **Testing**: Use mocks (in-memory, deterministic)
//! - **Production**: Use real services (PostgreSQL, Redis, SendGrid, etc.)
//! - **Development**: Use instrumented versions (logging, tracing)

use crate::actions::{AuthLevel, DeviceTrustLevel};
use crate::state::{DeviceId, OAuthProvider, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

pub mod oauth;
pub mod email;
pub mod webauthn;
pub mod session;
pub mod user;
pub mod device;
pub mod risk;

// Re-export provider traits
pub use oauth::OAuth2Provider;
pub use email::EmailProvider;
pub use webauthn::WebAuthnProvider;
pub use session::SessionStore;
pub use user::UserRepository;
pub use device::DeviceRepository;
pub use risk::RiskCalculator;

/// User data model.
///
/// Stored in PostgreSQL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    /// User ID.
    pub user_id: UserId,

    /// Email address.
    pub email: String,

    /// Display name.
    pub name: Option<String>,

    /// Email verified flag.
    pub email_verified: bool,

    /// Account created timestamp.
    pub created_at: DateTime<Utc>,

    /// Last updated timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Device data model.
///
/// Stored in PostgreSQL (permanent audit trail).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    /// Device ID.
    pub device_id: DeviceId,

    /// User ID.
    pub user_id: UserId,

    /// Device name (e.g., "iPhone 15 Pro").
    pub name: String,

    /// Device type.
    pub device_type: DeviceType,

    /// Platform (e.g., "iOS 17.2").
    pub platform: String,

    /// First seen timestamp.
    pub first_seen: DateTime<Utc>,

    /// Last seen timestamp.
    pub last_seen: DateTime<Utc>,

    /// Trust level (progressive trust).
    pub trust_level: DeviceTrustLevel,

    /// Passkey credential ID (if registered).
    pub passkey_credential_id: Option<String>,

    /// Public key (if passkey registered).
    pub public_key: Option<Vec<u8>>,
}

/// Device type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "postgres", derive(sqlx::Type))]
#[cfg_attr(feature = "postgres", sqlx(type_name = "device_type", rename_all = "lowercase"))]
pub enum DeviceType {
    /// Mobile device (phone, tablet).
    Mobile,

    /// Desktop computer.
    Desktop,

    /// Tablet.
    Tablet,

    /// Other/unknown.
    #[cfg_attr(feature = "postgres", sqlx(rename = "unknown"))]
    Other,
}

/// OAuth link (user ↔ provider).
///
/// Stored in PostgreSQL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuthLink {
    /// User ID.
    pub user_id: UserId,

    /// OAuth provider.
    pub provider: OAuthProvider,

    /// Provider user ID (unique per provider).
    pub provider_user_id: String,

    /// Access token.
    pub access_token: String,

    /// Refresh token (if available).
    pub refresh_token: Option<String>,

    /// Token expiration.
    pub expires_at: Option<DateTime<Utc>>,

    /// Created timestamp.
    pub created_at: DateTime<Utc>,

    /// Updated timestamp.
    pub updated_at: DateTime<Utc>,
}

/// OAuth user info from provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuthUserInfo {
    /// Provider user ID.
    pub provider_user_id: String,

    /// Email address.
    pub email: String,

    /// Email verified flag.
    pub email_verified: bool,

    /// Display name.
    pub name: Option<String>,

    /// Profile picture URL.
    pub picture: Option<String>,
}

/// Magic link token.
///
/// Stored in database (hashed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MagicLinkToken {
    /// Email address.
    pub email: String,

    /// Token hash (SHA-256).
    pub token_hash: String,

    /// Expiration timestamp.
    pub expires_at: DateTime<Utc>,

    /// Used flag.
    pub used: bool,

    /// Created timestamp.
    pub created_at: DateTime<Utc>,
}

/// Passkey credential.
///
/// Stored in PostgreSQL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PasskeyCredential {
    /// Credential ID (unique).
    pub credential_id: String,

    /// User ID.
    pub user_id: UserId,

    /// Device ID.
    pub device_id: DeviceId,

    /// Public key (COSE format).
    pub public_key: Vec<u8>,

    /// Signature counter (replay protection).
    pub counter: u32,

    /// Created timestamp.
    pub created_at: DateTime<Utc>,

    /// Last used timestamp.
    pub last_used: Option<DateTime<Utc>>,
}

/// Risk assessment result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskAssessment {
    /// Risk score (0.0-1.0).
    pub score: f32,

    /// Risk level.
    pub level: RiskLevel,

    /// Factors that contributed to the score.
    pub factors: Vec<RiskFactor>,

    /// Recommended authentication level.
    pub recommended_auth_level: AuthLevel,
}

/// Risk level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskLevel {
    /// Low risk (<0.3).
    Low,

    /// Medium risk (0.3-0.6).
    Medium,

    /// High risk (0.6-0.8).
    High,

    /// Critical risk (>=0.8).
    Critical,
}

/// Risk factor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskFactor {
    /// Factor name.
    pub name: String,

    /// Factor weight (contribution to total score).
    pub weight: f32,

    /// Factor description.
    pub description: String,
}

/// Login context for risk assessment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoginContext {
    /// User ID (if known).
    pub user_id: Option<UserId>,

    /// Email address.
    pub email: String,

    /// IP address.
    pub ip_address: IpAddr,

    /// User agent.
    pub user_agent: String,

    /// Device ID (if recognized).
    pub device_id: Option<DeviceId>,

    /// Last login location (for impossible travel detection).
    pub last_login_location: Option<String>,

    /// Last login timestamp.
    pub last_login_at: Option<DateTime<Utc>>,
}
