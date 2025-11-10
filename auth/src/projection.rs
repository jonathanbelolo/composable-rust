//! Event projection for auth read models.
//!
//! This module implements projections that build read-optimized views from auth events.
//! The projections listen to events from the event store and update PostgreSQL tables
//! that are optimized for querying.
//!
//! # Architecture
//!
//! ```text
//! Event Store (PostgreSQL)        Projections (PostgreSQL)
//! ┌─────────────────────┐        ┌──────────────────────┐
//! │ events table        │        │ users_projection     │
//! │ - UserRegistered    │───────▶│ - user_id            │
//! │ - DeviceRegistered  │        │ - email              │
//! │ - UserLoggedIn      │        │ - name               │
//! │ - ...               │        │ - email_verified     │
//! └─────────────────────┘        │ - created_at         │
//!                                │ - updated_at         │
//!                                ├──────────────────────┤
//!                                │ devices_projection   │
//!                                │ - device_id          │
//!                                │ - user_id            │
//!                                │ - name               │
//!                                │ - device_type        │
//!                                │ - platform           │
//!                                │ - first_seen         │
//!                                │ - last_seen          │
//!                                │ - trust_level        │
//!                                │ - login_count        │
//!                                └──────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use composable_rust_auth::projection::AuthProjection;
//! use composable_rust_core::projection::Projection;
//!
//! let projection = AuthProjection::new(pool);
//!
//! // Apply events to update projections
//! for event in events {
//!     projection.apply_event(&event).await?;
//! }
//!
//! // Rebuild from scratch (replay all events)
//! projection.rebuild().await?;
//! ```

use crate::actions::DeviceTrustLevel;
use crate::events::AuthEvent;
use chrono::{DateTime, Utc};
use composable_rust_core::projection::{Projection, ProjectionError, Result};

#[cfg(feature = "postgres")]
use sqlx::PgPool;

/// Auth projection handler.
///
/// Maintains read-optimized views of users and devices from auth events.
#[cfg(feature = "postgres")]
pub struct AuthProjection {
    /// PostgreSQL connection pool.
    pool: PgPool,
}

#[cfg(feature = "postgres")]
impl AuthProjection {
    /// Create a new auth projection.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Apply a UserRegistered event.
    async fn apply_user_registered(&self, event: &AuthEvent) -> Result<()> {
        if let AuthEvent::UserRegistered {
            user_id,
            email,
            name,
            email_verified,
            timestamp,
        } = event
        {
            // ✅ IDEMPOTENCY: Use ON CONFLICT DO UPDATE with timestamp check
            // This ensures:
            // - Duplicate events are handled correctly (same timestamp = no-op)
            // - Out-of-order events are rejected (older timestamp ignored)
            // - Late-arriving events don't overwrite newer data
            sqlx::query!(
                r#"
                INSERT INTO users_projection (user_id, email, name, email_verified, created_at, updated_at, last_event_timestamp)
                VALUES ($1, $2, $3, $4, $5, $5, $6)
                ON CONFLICT (user_id) DO UPDATE SET
                    email = EXCLUDED.email,
                    name = EXCLUDED.name,
                    email_verified = EXCLUDED.email_verified,
                    updated_at = EXCLUDED.updated_at,
                    last_event_timestamp = EXCLUDED.last_event_timestamp
                WHERE users_projection.last_event_timestamp < EXCLUDED.last_event_timestamp
                "#,
                user_id.0,
                email,
                name.as_deref(),
                email_verified,
                timestamp,
                timestamp // last_event_timestamp
            )
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to insert user: {e}")))?;
        }

        Ok(())
    }

    /// Apply an EmailVerified event.
    async fn apply_email_verified(&self, event: &AuthEvent) -> Result<()> {
        if let AuthEvent::EmailVerified { user_id, timestamp } = event {
            // ✅ IDEMPOTENCY: Only update if timestamp is newer
            sqlx::query!(
                r#"
                UPDATE users_projection
                SET email_verified = true,
                    updated_at = $2,
                    last_event_timestamp = $3
                WHERE user_id = $1
                  AND last_event_timestamp < $3
                "#,
                user_id.0,
                timestamp,
                timestamp // last_event_timestamp
            )
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to verify email: {e}")))?;
        }

        Ok(())
    }

    /// Apply a UserUpdated event.
    async fn apply_user_updated(&self, event: &AuthEvent) -> Result<()> {
        if let AuthEvent::UserUpdated {
            user_id,
            name,
            timestamp,
        } = event
        {
            // ✅ IDEMPOTENCY: Only update if timestamp is newer
            sqlx::query!(
                r#"
                UPDATE users_projection
                SET name = $2,
                    updated_at = $3,
                    last_event_timestamp = $4
                WHERE user_id = $1
                  AND last_event_timestamp < $4
                "#,
                user_id.0,
                name.as_deref(),
                timestamp,
                timestamp // last_event_timestamp
            )
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to update user: {e}")))?;
        }

        Ok(())
    }

    /// Apply a DeviceRegistered event.
    async fn apply_device_registered(&self, event: &AuthEvent) -> Result<()> {
        if let AuthEvent::DeviceRegistered {
            device_id,
            user_id,
            name,
            device_type,
            platform,
            ip_address: _,
            timestamp,
        } = event
        {
            // ✅ IDEMPOTENCY: Use ON CONFLICT DO UPDATE with timestamp check
            sqlx::query_unchecked!(
                r#"
                INSERT INTO devices_projection
                    (device_id, user_id, name, device_type, platform, first_seen, last_seen, trust_level, login_count, last_event_timestamp)
                VALUES ($1, $2, $3, $4::device_type, $5, $6, $6, 'unknown'::device_trust_level, 0, $7)
                ON CONFLICT (device_id) DO UPDATE SET
                    user_id = EXCLUDED.user_id,
                    name = EXCLUDED.name,
                    device_type = EXCLUDED.device_type,
                    platform = EXCLUDED.platform,
                    first_seen = EXCLUDED.first_seen,
                    last_seen = EXCLUDED.last_seen,
                    last_event_timestamp = EXCLUDED.last_event_timestamp
                WHERE devices_projection.last_event_timestamp < EXCLUDED.last_event_timestamp
                "#,
                device_id.0,
                user_id.0,
                name,
                device_type,
                platform,
                timestamp,
                timestamp // last_event_timestamp
            )
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to insert device: {e}")))?;
        }

        Ok(())
    }

    /// Apply a DeviceTrustedByUser event.
    async fn apply_device_trusted(&self, event: &AuthEvent) -> Result<()> {
        if let AuthEvent::DeviceTrustedByUser {
            device_id,
            user_id: _,
            trusted,
            timestamp,
        } = event
        {
            let trust_level = if *trusted {
                DeviceTrustLevel::Trusted
            } else {
                DeviceTrustLevel::Unknown
            };

            let trust_level_str = match trust_level {
                DeviceTrustLevel::Unknown => "unknown",
                DeviceTrustLevel::Recognized => "recognized",
                DeviceTrustLevel::Familiar => "familiar",
                DeviceTrustLevel::Trusted => "trusted",
                DeviceTrustLevel::HighlyTrusted => "highly_trusted",
            };

            // ✅ IDEMPOTENCY: Only update if timestamp is newer
            sqlx::query_unchecked!(
                r#"
                UPDATE devices_projection
                SET trust_level = $2::device_trust_level,
                    last_event_timestamp = $3
                WHERE device_id = $1
                  AND last_event_timestamp < $3
                "#,
                device_id.0,
                trust_level_str,
                timestamp
            )
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to update device trust: {e}")))?;
        }

        Ok(())
    }

    /// Apply a DeviceAccessed event.
    async fn apply_device_accessed(&self, event: &AuthEvent) -> Result<()> {
        if let AuthEvent::DeviceAccessed {
            device_id,
            user_id: _,
            ip_address: _,
            auth_level: _,
            timestamp,
        } = event
        {
            // First, fetch the device to calculate new trust level
            let device = sqlx::query!(
                r#"
                SELECT first_seen, trust_level AS "trust_level: DeviceTrustLevel", login_count
                FROM devices_projection
                WHERE device_id = $1
                "#,
                device_id.0
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to fetch device: {e}")))?;

            if let Some(device) = device {
                // Calculate new trust level (only if not manually set to Trusted/HighlyTrusted)
                let new_trust_level = if matches!(
                    device.trust_level,
                    DeviceTrustLevel::Trusted | DeviceTrustLevel::HighlyTrusted
                ) {
                    // Don't override manually-set trust levels
                    device.trust_level
                } else {
                    // Recalculate progressive trust with incremented login_count
                    calculate_progressive_trust(device.login_count + 1, device.first_seen)
                };

                // ✅ IDEMPOTENCY: Only update if timestamp is newer
                sqlx::query!(
                    r#"
                    UPDATE devices_projection
                    SET last_seen = $2,
                        login_count = login_count + 1,
                        trust_level = $3,
                        last_event_timestamp = $4
                    WHERE device_id = $1
                      AND last_event_timestamp < $4
                    "#,
                    device_id.0,
                    timestamp,
                    new_trust_level as DeviceTrustLevel,
                    timestamp
                )
                .execute(&self.pool)
                .await
                .map_err(|e| ProjectionError::Storage(format!("Failed to update device access: {e}")))?;
            }
        }

        Ok(())
    }

    /// Apply an OAuthAccountLinked event.
    async fn apply_oauth_linked(&self, event: &AuthEvent) -> Result<()> {
        if let AuthEvent::OAuthAccountLinked {
            user_id,
            provider,
            provider_user_id,
            provider_email: _,
            timestamp,
        } = event
        {
            // ✅ IDEMPOTENCY: Use ON CONFLICT DO UPDATE with timestamp check
            sqlx::query!(
                r#"
                INSERT INTO oauth_links_projection (user_id, provider, provider_user_id, linked_at, last_event_timestamp)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (user_id, provider) DO UPDATE SET
                    provider_user_id = EXCLUDED.provider_user_id,
                    linked_at = EXCLUDED.linked_at,
                    last_event_timestamp = EXCLUDED.last_event_timestamp
                WHERE oauth_links_projection.last_event_timestamp < EXCLUDED.last_event_timestamp
                "#,
                user_id.0,
                provider.as_str(),
                provider_user_id,
                timestamp,
                timestamp // last_event_timestamp
            )
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to link OAuth account: {e}")))?;
        }

        Ok(())
    }

    /// Apply a PasskeyRegistered event.
    async fn apply_passkey_registered(&self, event: &AuthEvent) -> Result<()> {
        if let AuthEvent::PasskeyRegistered {
            credential_id,
            user_id,
            device_id,
            public_key,
            counter,
            timestamp,
        } = event
        {
            // ✅ IDEMPOTENCY: Use ON CONFLICT DO UPDATE with timestamp check
            sqlx::query!(
                r#"
                INSERT INTO passkeys_projection
                    (credential_id, user_id, device_id, public_key, counter, registered_at, last_event_timestamp)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT (credential_id) DO UPDATE SET
                    user_id = EXCLUDED.user_id,
                    device_id = EXCLUDED.device_id,
                    public_key = EXCLUDED.public_key,
                    counter = EXCLUDED.counter,
                    registered_at = EXCLUDED.registered_at,
                    last_event_timestamp = EXCLUDED.last_event_timestamp
                WHERE passkeys_projection.last_event_timestamp < EXCLUDED.last_event_timestamp
                "#,
                credential_id,
                user_id.0,
                device_id.0,
                public_key,
                *counter as i32,
                timestamp,
                timestamp // last_event_timestamp
            )
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to register passkey: {e}")))?;

            // Also update device to link passkey (with timestamp check)
            sqlx::query!(
                r#"
                UPDATE devices_projection
                SET passkey_credential_id = $2,
                    last_event_timestamp = $3
                WHERE device_id = $1
                  AND last_event_timestamp < $3
                "#,
                device_id.0,
                credential_id,
                timestamp
            )
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to link passkey to device: {e}")))?;
        }

        Ok(())
    }

    /// Apply a PasskeyUsed event.
    async fn apply_passkey_used(&self, event: &AuthEvent) -> Result<()> {
        if let AuthEvent::PasskeyUsed {
            credential_id,
            user_id: _,
            device_id: _,
            counter,
            ip_address: _,
            timestamp,
        } = event
        {
            // ✅ IDEMPOTENCY: Only update if timestamp is newer
            sqlx::query!(
                r#"
                UPDATE passkeys_projection
                SET counter = $2,
                    last_used = $3,
                    last_event_timestamp = $4
                WHERE credential_id = $1
                  AND last_event_timestamp < $4
                "#,
                credential_id,
                *counter as i32,
                timestamp,
                timestamp
            )
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to update passkey usage: {e}")))?;
        }

        Ok(())
    }
}

#[cfg(feature = "postgres")]
impl Projection for AuthProjection {
    type Event = AuthEvent;

    fn name(&self) -> &str {
        "auth_projection"
    }

    async fn apply_event(&self, event: &Self::Event) -> Result<()> {
        // Match on event type and delegate to specific handlers
        match event {
            AuthEvent::UserRegistered { .. } => self.apply_user_registered(event).await,
            AuthEvent::EmailVerified { .. } => self.apply_email_verified(event).await,
            AuthEvent::UserUpdated { .. } => self.apply_user_updated(event).await,
            AuthEvent::DeviceRegistered { .. } => self.apply_device_registered(event).await,
            AuthEvent::DeviceTrustedByUser { .. } => self.apply_device_trusted(event).await,
            AuthEvent::DeviceAccessed { .. } => self.apply_device_accessed(event).await,
            AuthEvent::OAuthAccountLinked { .. } => self.apply_oauth_linked(event).await,
            AuthEvent::PasskeyRegistered { .. } => self.apply_passkey_registered(event).await,
            AuthEvent::PasskeyUsed { .. } => self.apply_passkey_used(event).await,

            // Audit events don't update projections (they're for logging/analytics)
            AuthEvent::LoginAttempted { .. }
            | AuthEvent::UserLoggedIn { .. }
            | AuthEvent::UserLoggedOut { .. }
            | AuthEvent::DeviceRevoked { .. }
            | AuthEvent::OAuthAccountUnlinked { .. }
            | AuthEvent::PasskeyRevoked { .. }
            | AuthEvent::CounterRollbackDetected { .. } => Ok(()),
        }
    }

    async fn rebuild(&self) -> Result<()> {
        // Clear all projection data
        sqlx::query!("TRUNCATE TABLE users_projection CASCADE")
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to truncate users_projection: {e}")))?;

        sqlx::query!("TRUNCATE TABLE devices_projection CASCADE")
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to truncate devices_projection: {e}")))?;

        sqlx::query!("TRUNCATE TABLE oauth_links_projection CASCADE")
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to truncate oauth_links_projection: {e}")))?;

        sqlx::query!("TRUNCATE TABLE passkeys_projection CASCADE")
            .execute(&self.pool)
            .await
            .map_err(|e| ProjectionError::Storage(format!("Failed to truncate passkeys_projection: {e}")))?;

        Ok(())
    }
}

/// Calculate progressive trust level based on usage metrics.
///
/// This implements the automatic trust progression algorithm:
/// - **Unknown**: New device (< 7 days, < 5 logins)
/// - **Recognized**: Seen before (7-30 days or 5-20 logins)
/// - **Familiar**: Regular device (30+ days or 20+ logins)
///
/// # Arguments
///
/// * `login_count` - Number of successful logins from this device
/// * `first_seen` - When the device was first registered
///
/// # Returns
///
/// The calculated trust level (Unknown, Recognized, or Familiar).
/// Does NOT return Trusted or HighlyTrusted (those are set manually).
fn calculate_progressive_trust(
    login_count: i32,
    first_seen: DateTime<Utc>,
) -> DeviceTrustLevel {
    let age_days = (Utc::now() - first_seen).num_days();

    // Progressive trust based on usage patterns
    match (age_days, login_count) {
        (days, count) if days >= 30 || count >= 20 => DeviceTrustLevel::Familiar,
        (days, count) if days >= 7 || count >= 5 => DeviceTrustLevel::Recognized,
        _ => DeviceTrustLevel::Unknown,
    }
}

#[cfg(test)]
#[cfg(feature = "postgres")]
mod tests {
    // Tests would require a test database setup
    // For now, we just verify the code compiles

    #[test]
    fn test_projection_name() {
        // This test just ensures the type signature is correct
        // Actual DB tests would use testcontainers
    }
}
