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
//! ## Query-Only Repositories
//!
//! **Important**: `UserRepository` and `DeviceRepository` are **query-only** interfaces.
//! They read from projections (read models) built from events. All writes happen
//! via event emission in reducers.
//!
//! ```text
//! Write Path (Command):              Read Path (Query):
//! ┌──────────────────┐              ┌──────────────────┐
//! │ Reducer          │              │ UserRepository   │
//! │ - Validates      │              │ (Query-Only)     │
//! │ - Emits Events   │              │                  │
//! │   • UserReg'd    │              │ Reads from:      │
//! │   • DeviceReg'd  │              │   users_proj.    │
//! └────────┬─────────┘              │   devices_proj.  │
//!          │                         └──────────────────┘
//!          ▼                                  ▲
//! ┌──────────────────┐                       │
//! │ Event Store      │                       │
//! │ (Source of Truth)│                       │
//! └────────┬─────────┘                       │
//!          │                                  │
//!          ▼                                  │
//! ┌──────────────────┐                       │
//! │ Projection       │───────────────────────┘
//! │ (Event Handler)  │  Updates projections
//! └──────────────────┘
//! ```
//!
//! This enables:
//! - **Testing**: Use mocks (in-memory, deterministic)
//! - **Production**: Use real services (PostgreSQL, Redis, SendGrid, etc.)
//! - **Development**: Use instrumented versions (logging, tracing)
//! - **CQRS**: Clear separation between write (events) and read (projections)

use crate::actions::{AuthLevel, DeviceTrustLevel};
use crate::state::{DeviceId, OAuthProvider, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

pub mod oauth;
pub mod oauth_token_store;
pub mod email;
pub mod webauthn;
pub mod session;
pub mod user;
pub mod device;
pub mod risk;
pub mod token_store;
pub mod challenge_store;
pub mod rate_limiter;
pub mod google;

// Re-export provider traits
pub use oauth::{OAuth2Provider, OAuthTokenResponse};
pub use google::GoogleOAuthProvider;
pub use oauth_token_store::{OAuthTokenStore, OAuthTokenData};
pub use email::EmailProvider;
pub use webauthn::WebAuthnProvider;
pub use session::SessionStore;
pub use user::UserRepository;
pub use device::DeviceRepository;
pub use risk::RiskCalculator;
pub use token_store::{TokenStore, TokenData, TokenType};
pub use challenge_store::{ChallengeStore, ChallengeData};
pub use rate_limiter::RateLimiter;

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

    /// Device fingerprint (for enhanced recognition).
    ///
    /// Stored as JSON to support evolving fingerprinting techniques.
    /// Use `fingerprint_hash` for quick comparisons.
    pub fingerprint: Option<DeviceFingerprint>,

    /// SHA-256 hash of the fingerprint (for quick matching).
    ///
    /// This is a deterministic hash of the canonicalized fingerprint,
    /// allowing fast device recognition without comparing all fields.
    pub fingerprint_hash: Option<String>,
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

/// Device fingerprint for enhanced device recognition.
///
/// This struct stores browser/device fingerprinting data collected on the client
/// side (e.g., via FingerprintJS, ClientJS, or custom fingerprinting logic).
///
/// # Purpose
///
/// - **Device Recognition**: Identify returning devices even without cookies
/// - **Risk Assessment**: Detect suspicious device changes or anomalies
/// - **Security**: Flag potential account takeover attempts
///
/// # Privacy Considerations
///
/// Fingerprinting can be privacy-invasive. Best practices:
/// - Only collect fingerprints for authenticated users (post-login)
/// - Store hashed fingerprints, not raw values
/// - Allow users to view/delete their device fingerprints
/// - Comply with GDPR/privacy regulations
///
/// # Client-Side Collection
///
/// This is a backend library - fingerprints must be collected client-side.
/// Example libraries:
/// - FingerprintJS (commercial, high accuracy)
/// - ClientJS (open source, basic)
/// - Custom canvas/WebGL/audio fingerprinting
///
/// # Fields
///
/// All fields are optional to support partial fingerprints and evolving techniques.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DeviceFingerprint {
    /// Canvas fingerprint (rendering variations).
    pub canvas: Option<String>,

    /// WebGL fingerprint (GPU/driver variations).
    pub webgl: Option<String>,

    /// Audio context fingerprint (audio processing variations).
    pub audio: Option<String>,

    /// Screen resolution (width x height).
    pub screen_resolution: Option<String>,

    /// Timezone offset from UTC (minutes).
    pub timezone_offset: Option<i32>,

    /// Browser plugins (semicolon-separated list).
    pub plugins: Option<String>,

    /// Fonts installed (comma-separated list).
    pub fonts: Option<String>,

    /// CPU architecture/cores.
    pub cpu_cores: Option<u8>,

    /// Device memory (GB).
    pub device_memory: Option<u8>,

    /// Hardware concurrency (logical processors).
    pub hardware_concurrency: Option<u8>,

    /// Color depth (bits per pixel).
    pub color_depth: Option<u8>,

    /// Platform (navigator.platform).
    pub platform: Option<String>,

    /// Language preferences (navigator.languages).
    pub languages: Option<Vec<String>>,

    /// Do Not Track setting.
    pub do_not_track: Option<bool>,

    /// Touch support (max touch points).
    pub max_touch_points: Option<u8>,

    /// Vendor (navigator.vendor).
    pub vendor: Option<String>,

    /// Renderer (WebGL renderer string).
    pub renderer: Option<String>,

    /// Additional custom fields (extensibility).
    #[serde(flatten)]
    pub custom: std::collections::HashMap<String, serde_json::Value>,
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

    /// Device fingerprint (if provided by client).
    ///
    /// Used for enhanced device recognition and risk assessment.
    /// If provided, the risk calculator can:
    /// - Match against known devices for this user
    /// - Detect device changes/anomalies
    /// - Calculate fingerprint similarity scores
    pub fingerprint: Option<DeviceFingerprint>,
}
