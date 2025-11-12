//! Authentication domain events.
//!
//! This module defines the event-sourced domain events for the authentication system.
//! All state changes are persisted as events and can be replayed to rebuild state.
//!
//! # Event Sourcing
//!
//! The auth system follows the composable-rust event sourcing pattern:
//!
//! 1. **Commands** → Reducers validate and produce events
//! 2. **Events** → Persisted to event store (` PostgreSQL`)
//! 3. **Projections** → Read models rebuilt from events
//! 4. **State** → Derived from event replay
//!
//! # Aggregate Roots
//!
//! - **User**: Identity aggregate (email, verification status)
//! - **Device**: Device trust and fingerprinting
//! - **Session**: Ephemeral (NOT event sourced - stored in Redis)
//!
//! # Example
//!
//! ```ignore
//! // Command
//! AuthAction::RegisterUser { email, ... }
//!   ↓ Reducer validates
//!   ↓ Emits event
//! AuthEvent::UserRegistered { user_id, email, timestamp }
//!   ↓ Persisted to event store
//!   ↓ Projection updates
//! users_projection table updated
//! ```

use crate::actions::AuthLevel;
use crate::state::{DeviceId, OAuthProvider, SessionId, UserId};
use chrono::{DateTime, Utc};
use composable_rust_core::event::SerializedEvent;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Authentication domain events.
///
/// These events represent facts that have happened in the authentication domain.
/// All state changes are event-sourced and can be replayed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AuthEvent {
    // ═══════════════════════════════════════════════════════════════════════
    // User Events
    // ═══════════════════════════════════════════════════════════════════════
    /// User account was created.
    ///
    /// Triggered by: Magic link, `OAuth`, or passkey registration
    UserRegistered {
        /// User identifier
        user_id: UserId,
        /// Email address
        email: String,
        /// User name (if provided)
        name: Option<String>,
        /// Email verified at registration (true for magic link, false for `OAuth`)
        email_verified: bool,
        /// When the user was registered
        timestamp: DateTime<Utc>,
    },

    /// User email was verified.
    ///
    /// Triggered by: Email verification link click
    EmailVerified {
        /// User identifier
        user_id: UserId,
        /// When the email was verified
        timestamp: DateTime<Utc>,
    },

    /// User profile was updated.
    UserUpdated {
        /// User identifier
        user_id: UserId,
        /// New name (if changed)
        name: Option<String>,
        /// When the update occurred
        timestamp: DateTime<Utc>,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Device Events
    // ═══════════════════════════════════════════════════════════════════════
    /// Device was registered for a user.
    ///
    /// Triggered by: First login from a new device
    DeviceRegistered {
        /// Device identifier
        device_id: DeviceId,
        /// User identifier
        user_id: UserId,
        /// Device name (e.g., "iPhone 15 Pro")
        name: String,
        /// Device type (mobile, desktop, tablet)
        device_type: String,
        /// Platform/user agent
        platform: String,
        /// IP address of registration
        ip_address: IpAddr,
        /// When the device was first seen
        timestamp: DateTime<Utc>,
    },

    /// Device trust level was changed by user.
    ///
    /// Triggered by: User marks device as trusted
    DeviceTrustedByUser {
        /// Device identifier
        device_id: DeviceId,
        /// User identifier
        user_id: UserId,
        /// Whether user marked as trusted
        trusted: bool,
        /// When the trust level changed
        timestamp: DateTime<Utc>,
    },

    /// Device was accessed (login occurred).
    ///
    /// Triggered by: Successful authentication
    DeviceAccessed {
        /// Device identifier
        device_id: DeviceId,
        /// User identifier
        user_id: UserId,
        /// IP address of access
        ip_address: IpAddr,
        /// Authentication level used (Basic, `MultiFactor`, `HardwareBacked`)
        auth_level: AuthLevel,
        /// When the access occurred
        timestamp: DateTime<Utc>,
    },

    /// Device was revoked.
    ///
    /// Triggered by: User removes device from account
    DeviceRevoked {
        /// Device identifier
        device_id: DeviceId,
        /// User identifier
        user_id: UserId,
        /// Reason for revocation
        reason: String,
        /// When the device was revoked
        timestamp: DateTime<Utc>,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // OAuth Events
    // ═══════════════════════════════════════════════════════════════════════
    /// `OAuth` account was linked.
    ///
    /// Triggered by: Successful `OAuth` flow
    OAuthAccountLinked {
        /// User identifier
        user_id: UserId,
        /// `OAuth` provider (Google, GitHub, Microsoft)
        provider: OAuthProvider,
        /// Provider's user ID
        provider_user_id: String,
        /// Provider's email
        provider_email: String,
        /// When the account was linked
        timestamp: DateTime<Utc>,
    },

    /// `OAuth` account was unlinked.
    ///
    /// Triggered by: User disconnects `OAuth` provider
    OAuthAccountUnlinked {
        /// User identifier
        user_id: UserId,
        /// `OAuth` provider
        provider: OAuthProvider,
        /// When the account was unlinked
        timestamp: DateTime<Utc>,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Passkey Events
    // ═══════════════════════════════════════════════════════════════════════
    /// Passkey credential was registered.
    ///
    /// Triggered by: `WebAuthn` registration completion
    PasskeyRegistered {
        /// Credential identifier
        credential_id: String,
        /// User identifier
        user_id: UserId,
        /// Device identifier
        device_id: DeviceId,
        /// Public key (for verification)
        public_key: Vec<u8>,
        /// Initial counter value
        counter: u32,
        /// When the passkey was registered
        timestamp: DateTime<Utc>,
    },

    /// Passkey was used for authentication.
    ///
    /// Triggered by: `WebAuthn` assertion verification
    PasskeyUsed {
        /// Credential identifier
        credential_id: String,
        /// User identifier
        user_id: UserId,
        /// Device identifier
        device_id: DeviceId,
        /// New counter value (for replay protection)
        counter: u32,
        /// IP address of use
        ip_address: IpAddr,
        /// When the passkey was used
        timestamp: DateTime<Utc>,
    },

    /// Passkey was revoked.
    ///
    /// Triggered by: User removes passkey from device
    PasskeyRevoked {
        /// Credential identifier
        credential_id: String,
        /// User identifier
        user_id: UserId,
        /// When the passkey was revoked
        timestamp: DateTime<Utc>,
    },

    /// Passkey counter rollback detected (SECURITY EVENT).
    ///
    /// Triggered by: Counter rollback detection during passkey authentication
    ///
    /// This indicates either:
    /// - Cloned authenticator (security compromise)
    /// - Replay attack attempt
    /// - Hardware malfunction (rare)
    ///
    /// **CRITICAL**: This should trigger security alerts and monitoring.
    CounterRollbackDetected {
        /// Credential identifier
        credential_id: String,
        /// User identifier
        user_id: UserId,
        /// Device identifier
        device_id: DeviceId,
        /// Stored counter value
        stored_counter: u32,
        /// Received counter value (from authenticator)
        received_counter: u32,
        /// IP address of attempt
        ip_address: IpAddr,
        /// When the rollback was detected
        timestamp: DateTime<Utc>,
    },

    // ═══════════════════════════════════════════════════════════════════════
    // Login Events (Audit Trail)
    // ═══════════════════════════════════════════════════════════════════════
    /// Login attempt was made.
    ///
    /// Triggered by: Any authentication attempt
    LoginAttempted {
        /// Email attempted
        email: String,
        /// Authentication method (`magic_link`, oauth, passkey)
        method: String,
        /// IP address of attempt
        ip_address: IpAddr,
        /// User agent
        user_agent: String,
        /// Whether attempt succeeded
        success: bool,
        /// Failure reason (if failed)
        failure_reason: Option<String>,
        /// When the attempt occurred
        timestamp: DateTime<Utc>,
    },

    /// User logged in successfully.
    ///
    /// Triggered by: Successful authentication
    UserLoggedIn {
        /// User identifier
        user_id: UserId,
        /// Device identifier
        device_id: DeviceId,
        /// Session identifier (ephemeral - not event sourced)
        session_id: SessionId,
        /// Authentication method
        method: String,
        /// Authentication level
        auth_level: AuthLevel,
        /// IP address
        ip_address: IpAddr,
        /// User agent
        user_agent: String,
        /// Risk score (0.0 = safe, 1.0 = high risk)
        risk_score: f64,
        /// When the login occurred
        timestamp: DateTime<Utc>,
    },

    /// User logged out.
    ///
    /// Triggered by: Explicit logout action
    UserLoggedOut {
        /// User identifier
        user_id: UserId,
        /// Session identifier
        session_id: SessionId,
        /// When the logout occurred
        timestamp: DateTime<Utc>,
    },
}

impl AuthEvent {
    /// Get the event type for serialization.
    ///
    /// Event types are versioned (e.g., "UserRegistered.v1") to support
    /// schema evolution.
    #[must_use]
    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::UserRegistered { .. } => "UserRegistered.v1",
            Self::EmailVerified { .. } => "EmailVerified.v1",
            Self::UserUpdated { .. } => "UserUpdated.v1",
            Self::DeviceRegistered { .. } => "DeviceRegistered.v1",
            Self::DeviceTrustedByUser { .. } => "DeviceTrustedByUser.v1",
            Self::DeviceAccessed { .. } => "DeviceAccessed.v1",
            Self::DeviceRevoked { .. } => "DeviceRevoked.v1",
            Self::OAuthAccountLinked { .. } => "OAuthAccountLinked.v1",
            Self::OAuthAccountUnlinked { .. } => "OAuthAccountUnlinked.v1",
            Self::PasskeyRegistered { .. } => "PasskeyRegistered.v1",
            Self::PasskeyUsed { .. } => "PasskeyUsed.v1",
            Self::PasskeyRevoked { .. } => "PasskeyRevoked.v1",
            Self::CounterRollbackDetected { .. } => "CounterRollbackDetected.v1",
            Self::LoginAttempted { .. } => "LoginAttempted.v1",
            Self::UserLoggedIn { .. } => "UserLoggedIn.v1",
            Self::UserLoggedOut { .. } => "UserLoggedOut.v1",
        }
    }

    /// Serialize this event to a `SerializedEvent`.
    ///
    /// # Errors
    ///
    /// Returns error if serialization fails.
    pub fn to_serialized(&self) -> Result<SerializedEvent, String> {
        let data =
            bincode::serialize(self).map_err(|e| format!("Failed to serialize event: {e}"))?;

        Ok(SerializedEvent::new(
            self.event_type().to_string(),
            data,
            None,
        ))
    }

    /// Deserialize an event from a `SerializedEvent`.
    ///
    /// # Errors
    ///
    /// Returns error if deserialization fails.
    pub fn from_serialized(serialized: &SerializedEvent) -> Result<Self, String> {
        bincode::deserialize(&serialized.data)
            .map_err(|e| format!("Failed to deserialize event: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_event_type() {
        let event = AuthEvent::UserRegistered {
            user_id: UserId::new(),
            email: "test@example.com".to_string(),
            name: None,
            email_verified: true,
            timestamp: Utc::now(),
        };
        assert_eq!(event.event_type(), "UserRegistered.v1");
    }

    #[test]
    #[allow(clippy::unwrap_used)] // Test code
    fn test_event_serialization_roundtrip() {
        let original = AuthEvent::DeviceRegistered {
            device_id: DeviceId::new(),
            user_id: UserId::new(),
            name: "iPhone 15 Pro".to_string(),
            device_type: "mobile".to_string(),
            platform: "iOS 17".to_string(),
            ip_address: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)),
            timestamp: Utc::now(),
        };

        // Serialize
        let serialized = original.to_serialized().unwrap();
        assert_eq!(serialized.event_type, "DeviceRegistered.v1");

        // Deserialize
        let deserialized = AuthEvent::from_serialized(&serialized).unwrap();

        // Verify
        assert_eq!(original, deserialized);
        assert_eq!(original.event_type(), deserialized.event_type());
    }

    #[test]
    #[allow(clippy::unwrap_used)] // Test code
    fn test_passkey_event() {
        let event = AuthEvent::PasskeyUsed {
            credential_id: "cred-123".to_string(),
            user_id: UserId::new(),
            device_id: DeviceId::new(),
            counter: 42,
            ip_address: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            timestamp: Utc::now(),
        };

        let serialized = event.to_serialized().unwrap();
        let deserialized = AuthEvent::from_serialized(&serialized).unwrap();

        assert_eq!(event, deserialized);
    }
}
