//! PostgreSQL device repository implementation.
//!
//! This module provides persistent device storage using PostgreSQL.
//!
//! # Architecture
//!
//! Devices are stored permanently for:
//! - Audit trail (who accessed from which device)
//! - Trust level calculation (device age, login count)
//! - Multi-factor authentication decisions
//! - Passkey credential association
//!
//! # Example
//!
//! ```no_run
//! use composable_rust_auth::stores::PostgresDeviceRepository;
//! use sqlx::PgPool;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let pool = PgPool::connect("postgresql://localhost/auth").await?;
//! let repo = PostgresDeviceRepository::new(pool);
//! # Ok(())
//! # }
//! ```

use crate::actions::DeviceTrustLevel;
use crate::error::{AuthError, Result};
use crate::providers::{Device, DeviceRepository, DeviceType};
use crate::state::{DeviceId, UserId};
use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// `PostgreSQL` device repository.
///
/// Provides persistent storage for device registry using `PostgreSQL`.
#[derive(Clone)]
pub struct PostgresDeviceRepository {
    /// `PostgreSQL` connection pool.
    pool: PgPool,
}

impl PostgresDeviceRepository {
    /// Create a new `PostgreSQL` device repository.
    ///
    /// # Arguments
    ///
    /// * `pool` - `PostgreSQL` connection pool
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Run database migrations.
    ///
    /// # Errors
    ///
    /// Returns error if migrations fail.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| AuthError::DatabaseError(format!("Migration failed: {e}")))?;
        Ok(())
    }
}

impl DeviceRepository for PostgresDeviceRepository {
    async fn get_device(&self, user_id: UserId, device_id: DeviceId) -> Result<Device> {
        // ✅ Authorization: Filter by both user_id AND device_id
        let row = sqlx::query!(
            r#"
            SELECT device_id, user_id, name, device_type AS "device_type: DeviceType",
                   platform, first_seen, last_seen, trust_level AS "trust_level: DeviceTrustLevel",
                   login_count, passkey_credential_id, public_key
            FROM devices_projection
            WHERE device_id = $1 AND user_id = $2
            "#,
            device_id.0,
            user_id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to get device: {e}")))?
        .ok_or(AuthError::ResourceNotFound)?; // Returns ResourceNotFound for both missing and unauthorized

        Ok(Device {
            device_id: DeviceId(row.device_id),
            user_id: UserId(row.user_id),
            name: row.name,
            device_type: row.device_type,
            platform: row.platform,
            first_seen: row.first_seen,
            last_seen: row.last_seen,
            trust_level: row.trust_level,
            login_count: row.login_count,
            passkey_credential_id: row.passkey_credential_id,
            public_key: row.public_key,
            fingerprint: None, // TODO: Add to schema and query
            fingerprint_hash: None, // TODO: Add to schema and query
        })
    }

    async fn get_user_devices(
        &self,
        user_id: UserId,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<Device>> {
        // ✅ Cap limit to prevent DoS via unbounded queries
        const MAX_LIMIT: i64 = 1000;
        const DEFAULT_LIMIT: i64 = 100;

        let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        let offset = offset.unwrap_or(0).max(0); // Prevent negative offsets

        let rows = sqlx::query!(
            r#"
            SELECT device_id, user_id, name, device_type AS "device_type: DeviceType",
                   platform, first_seen, last_seen, trust_level AS "trust_level: DeviceTrustLevel",
                   login_count, passkey_credential_id, public_key
            FROM devices_projection
            WHERE user_id = $1
            ORDER BY last_seen DESC
            LIMIT $2 OFFSET $3
            "#,
            user_id.0,
            limit,
            offset
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to get user devices: {e}")))?;

        let devices = rows
            .into_iter()
            .map(|row| Device {
                device_id: DeviceId(row.device_id),
                user_id: UserId(row.user_id),
                name: row.name,
                device_type: row.device_type,
                platform: row.platform,
                first_seen: row.first_seen,
                last_seen: row.last_seen,
                trust_level: row.trust_level,
                login_count: row.login_count,
                passkey_credential_id: row.passkey_credential_id,
                public_key: row.public_key,
                fingerprint: None, // TODO: Add to schema and query
                fingerprint_hash: None, // TODO: Add to schema and query
            })
            .collect();

        Ok(devices)
    }

    async fn create_device(&self, device: &Device) -> Result<Device> {
        // ✅ Validate inputs before database insertion (XSS/injection prevention)
        crate::utils::validate_device_name(&device.name)?;
        crate::utils::validate_platform(&device.platform)?;

        sqlx::query!(
            r#"
            INSERT INTO devices_projection
                (device_id, user_id, name, device_type, platform, first_seen, last_seen,
                 trust_level, login_count, passkey_credential_id, public_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            "#,
            device.device_id.0,
            device.user_id.0,
            device.name,
            device.device_type as DeviceType,
            device.platform,
            device.first_seen,
            device.last_seen,
            device.trust_level as DeviceTrustLevel,
            device.login_count,
            device.passkey_credential_id,
            device.public_key,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to create device: {e}")))?;

        Ok(device.clone())
    }

    async fn update_device(&self, user_id: UserId, device: &Device) -> Result<Device> {
        // ✅ Authorization: Filter by both device_id AND user_id
        // Also verify device.user_id == user_id (prevent device transfer)
        let result = sqlx::query!(
            r#"
            UPDATE devices_projection
            SET name = $2,
                device_type = $3,
                platform = $4,
                last_seen = $5,
                trust_level = $6,
                login_count = $7,
                passkey_credential_id = $8,
                public_key = $9
            WHERE device_id = $1 AND user_id = $10
            "#,
            device.device_id.0,
            device.name,
            device.device_type as DeviceType,
            device.platform,
            device.last_seen,
            device.trust_level as DeviceTrustLevel,
            device.login_count,
            device.passkey_credential_id,
            device.public_key,
            user_id.0, // ✅ Authorization check
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to update device: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AuthError::ResourceNotFound); // Device not found or unauthorized
        }

        // ✅ Additional check: Verify device.user_id matches user_id (prevent device transfer)
        if device.user_id != user_id {
            return Err(AuthError::ResourceNotFound);
        }

        Ok(device.clone())
    }

    async fn update_device_trust_level(
        &self,
        user_id: UserId,
        device_id: DeviceId,
        trust_level: DeviceTrustLevel,
    ) -> Result<()> {
        // ✅ Authorization: Filter by both device_id AND user_id
        let result = sqlx::query!(
            r#"
            UPDATE devices_projection
            SET trust_level = $2
            WHERE device_id = $1 AND user_id = $3
            "#,
            device_id.0,
            trust_level as DeviceTrustLevel,
            user_id.0, // ✅ Authorization check
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to update device trust level: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AuthError::ResourceNotFound); // Device not found or unauthorized
        }

        Ok(())
    }

    async fn update_device_last_seen(
        &self,
        user_id: UserId,
        device_id: DeviceId,
        last_seen: DateTime<Utc>,
    ) -> Result<()> {
        // ✅ Authorization: Filter by both device_id AND user_id
        let result = sqlx::query!(
            r#"
            UPDATE devices_projection
            SET last_seen = $2,
                login_count = login_count + 1
            WHERE device_id = $1 AND user_id = $3
            "#,
            device_id.0,
            last_seen,
            user_id.0, // ✅ Authorization check
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to update device last seen: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AuthError::ResourceNotFound); // Device not found or unauthorized
        }

        Ok(())
    }

    async fn delete_device(&self, user_id: UserId, device_id: DeviceId) -> Result<()> {
        // ✅ Authorization: Filter by both device_id AND user_id
        let result = sqlx::query!(
            r#"
            DELETE FROM devices_projection
            WHERE device_id = $1 AND user_id = $2
            "#,
            device_id.0,
            user_id.0, // ✅ Authorization check
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to delete device: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AuthError::ResourceNotFound); // Device not found or unauthorized
        }

        Ok(())
    }

    async fn find_device_by_fingerprint(
        &self,
        user_id: UserId,
        user_agent: &str,
        platform: &str,
    ) -> Result<Option<Device>> {
        let row = sqlx::query!(
            r#"
            SELECT device_id, user_id, name, device_type AS "device_type: DeviceType",
                   platform, first_seen, last_seen, trust_level AS "trust_level: DeviceTrustLevel",
                   login_count, passkey_credential_id, public_key
            FROM devices_projection
            WHERE user_id = $1
              AND platform = $2
            ORDER BY last_seen DESC
            LIMIT 1
            "#,
            user_id.0,
            platform,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to find device: {e}")))?;

        match row {
            Some(row) => {
                // Check if user agent is similar (simple check - could be more sophisticated)
                let ua_match = user_agent.contains(&row.platform) || row.platform.contains(user_agent);

                if ua_match {
                    Ok(Some(Device {
                        device_id: DeviceId(row.device_id),
                        user_id: UserId(row.user_id),
                        name: row.name,
                        device_type: row.device_type,
                        platform: row.platform,
                        first_seen: row.first_seen,
                        last_seen: row.last_seen,
                        trust_level: row.trust_level,
                        login_count: row.login_count,
                        passkey_credential_id: row.passkey_credential_id,
                        public_key: row.public_key,
                        fingerprint: None, // TODO: Add to schema and query
                        fingerprint_hash: None, // TODO: Add to schema and query
                    }))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }
}

/// Calculate device trust level based on usage metrics.
///
/// This function implements the progressive trust algorithm:
/// - **Unknown**: New device (< 7 days, < 5 logins)
/// - **Recognized**: Seen before (7-30 days or 5-20 logins)
/// - **Familiar**: Regular device (30+ days or 20+ logins)
///
/// **Note**: This does NOT handle `Trusted` or `HighlyTrusted` levels,
/// which are set manually via `DeviceTrustedByUser` events.
///
/// **Note**: This function is primarily used for testing. The actual projection
/// uses `calculate_progressive_trust` in `projection.rs`.
///
/// # Arguments
///
/// * `user_marked_trusted` - Whether user explicitly marked device as trusted
/// * `login_count` - Number of successful logins from this device
/// * `first_seen` - When the device was first registered
///
/// # Returns
///
/// The calculated trust level (Unknown, Recognized, Familiar, or Trusted)
#[cfg(test)]
fn calculate_trust_level(
    user_marked_trusted: bool,
    login_count: i32,
    first_seen: DateTime<Utc>,
) -> DeviceTrustLevel {
    // Manual trust override
    if user_marked_trusted {
        return DeviceTrustLevel::Trusted;
    }

    let age_days = (Utc::now() - first_seen).num_days();

    // Progressive trust based on usage patterns
    match (age_days, login_count) {
        (days, count) if days >= 30 || count >= 20 => DeviceTrustLevel::Familiar,
        (days, count) if days >= 7 || count >= 5 => DeviceTrustLevel::Recognized,
        _ => DeviceTrustLevel::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trust_level_calculation() {
        let now = Utc::now();

        // New device
        let level = calculate_trust_level(false, 1, now);
        assert_eq!(level, DeviceTrustLevel::Unknown);

        // Recognized device (age-based)
        let level = calculate_trust_level(false, 2, now - chrono::Duration::days(10));
        assert_eq!(level, DeviceTrustLevel::Recognized);

        // Recognized device (login-based)
        let level = calculate_trust_level(false, 10, now - chrono::Duration::days(2));
        assert_eq!(level, DeviceTrustLevel::Recognized);

        // Familiar device (age-based)
        let level = calculate_trust_level(false, 5, now - chrono::Duration::days(35));
        assert_eq!(level, DeviceTrustLevel::Familiar);

        // Familiar device (login-based)
        let level = calculate_trust_level(false, 25, now - chrono::Duration::days(5));
        assert_eq!(level, DeviceTrustLevel::Familiar);

        // User marked trusted
        let level = calculate_trust_level(true, 1, now);
        assert_eq!(level, DeviceTrustLevel::Trusted);
    }
}
