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

/// PostgreSQL device repository.
///
/// Provides persistent storage for device registry using PostgreSQL.
#[derive(Clone)]
pub struct PostgresDeviceRepository {
    /// PostgreSQL connection pool.
    pool: PgPool,
}

impl PostgresDeviceRepository {
    /// Create a new PostgreSQL device repository.
    ///
    /// # Arguments
    ///
    /// * `pool` - PostgreSQL connection pool
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
    async fn get_device(&self, device_id: DeviceId) -> Result<Device> {
        let row = sqlx::query!(
            r#"
            SELECT device_id, user_id, name, device_type AS "device_type: DeviceType",
                   platform, first_seen, last_seen, user_marked_trusted,
                   requires_mfa, passkey_credential_id, public_key, login_count
            FROM registered_devices
            WHERE device_id = $1
            "#,
            device_id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to get device: {e}")))?
        .ok_or(AuthError::ResourceNotFound)?;

        let trust_level = calculate_trust_level(
            row.user_marked_trusted,
            row.login_count,
            row.first_seen,
        );

        Ok(Device {
            device_id: DeviceId(row.device_id),
            user_id: UserId(row.user_id),
            name: row.name,
            device_type: row.device_type,
            platform: row.platform,
            first_seen: row.first_seen,
            last_seen: row.last_seen,
            trust_level,
            passkey_credential_id: row.passkey_credential_id,
            public_key: row.public_key,
        })
    }

    async fn get_user_devices(&self, user_id: UserId) -> Result<Vec<Device>> {
        let rows = sqlx::query!(
            r#"
            SELECT device_id, user_id, name, device_type AS "device_type: DeviceType",
                   platform, first_seen, last_seen, user_marked_trusted,
                   requires_mfa, passkey_credential_id, public_key, login_count
            FROM registered_devices
            WHERE user_id = $1
            ORDER BY last_seen DESC
            "#,
            user_id.0
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to get user devices: {e}")))?;

        let devices = rows
            .into_iter()
            .map(|row| {
                let trust_level = calculate_trust_level(
                    row.user_marked_trusted,
                    row.login_count,
                    row.first_seen,
                );

                Device {
                    device_id: DeviceId(row.device_id),
                    user_id: UserId(row.user_id),
                    name: row.name,
                    device_type: row.device_type,
                    platform: row.platform,
                    first_seen: row.first_seen,
                    last_seen: row.last_seen,
                    trust_level,
                    passkey_credential_id: row.passkey_credential_id,
                    public_key: row.public_key,
                }
            })
            .collect();

        Ok(devices)
    }

    async fn create_device(&self, device: &Device) -> Result<Device> {
        let device_type_str = match device.device_type {
            DeviceType::Mobile => "mobile",
            DeviceType::Desktop => "desktop",
            DeviceType::Tablet => "tablet",
            DeviceType::Other => "unknown",
        };

        sqlx::query!(
            r#"
            INSERT INTO registered_devices
                (device_id, user_id, name, device_type, platform, first_seen, last_seen,
                 user_marked_trusted, requires_mfa, passkey_credential_id, public_key)
            VALUES ($1, $2, $3, $4::device_type, $5, $6, $7, $8, $9, $10, $11)
            "#,
            device.device_id.0,
            device.user_id.0,
            device.name,
            device_type_str,
            device.platform,
            device.first_seen,
            device.last_seen,
            false, // user_marked_trusted starts as false
            false, // requires_mfa starts as false
            device.passkey_credential_id,
            device.public_key,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to create device: {e}")))?;

        Ok(device.clone())
    }

    async fn update_device(&self, device: &Device) -> Result<Device> {
        let device_type_str = match device.device_type {
            DeviceType::Mobile => "mobile",
            DeviceType::Desktop => "desktop",
            DeviceType::Tablet => "tablet",
            DeviceType::Other => "unknown",
        };

        let result = sqlx::query!(
            r#"
            UPDATE registered_devices
            SET name = $2,
                device_type = $3::device_type,
                platform = $4,
                last_seen = $5,
                passkey_credential_id = $6,
                public_key = $7
            WHERE device_id = $1
            "#,
            device.device_id.0,
            device.name,
            device_type_str,
            device.platform,
            device.last_seen,
            device.passkey_credential_id,
            device.public_key,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to update device: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AuthError::ResourceNotFound);
        }

        Ok(device.clone())
    }

    async fn update_device_trust_level(
        &self,
        device_id: DeviceId,
        trust_level: DeviceTrustLevel,
    ) -> Result<()> {
        // Store user marking as trusted (all other trust levels are calculated)
        let user_marked = matches!(trust_level, DeviceTrustLevel::Trusted | DeviceTrustLevel::HighlyTrusted);

        let result = sqlx::query!(
            r#"
            UPDATE registered_devices
            SET user_marked_trusted = $2
            WHERE device_id = $1
            "#,
            device_id.0,
            user_marked,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to update device trust level: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AuthError::ResourceNotFound);
        }

        Ok(())
    }

    async fn update_device_last_seen(
        &self,
        device_id: DeviceId,
        last_seen: DateTime<Utc>,
    ) -> Result<()> {
        let result = sqlx::query!(
            r#"
            UPDATE registered_devices
            SET last_seen = $2,
                login_count = login_count + 1
            WHERE device_id = $1
            "#,
            device_id.0,
            last_seen,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to update device last seen: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AuthError::ResourceNotFound);
        }

        Ok(())
    }

    async fn delete_device(&self, device_id: DeviceId) -> Result<()> {
        sqlx::query!(
            r#"
            DELETE FROM registered_devices
            WHERE device_id = $1
            "#,
            device_id.0,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::DatabaseError(format!("Failed to delete device: {e}")))?;

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
                   platform, first_seen, last_seen, user_marked_trusted,
                   requires_mfa, passkey_credential_id, public_key, login_count
            FROM registered_devices
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
                    let trust_level = calculate_trust_level(
                        row.user_marked_trusted,
                        row.login_count,
                        row.first_seen,
                    );

                    Ok(Some(Device {
                        device_id: DeviceId(row.device_id),
                        user_id: UserId(row.user_id),
                        name: row.name,
                        device_type: row.device_type,
                        platform: row.platform,
                        first_seen: row.first_seen,
                        last_seen: row.last_seen,
                        trust_level,
                        passkey_credential_id: row.passkey_credential_id,
                        public_key: row.public_key,
                    }))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }
}

/// Calculate device trust level based on metrics.
///
/// # Trust Levels
///
/// - **Unknown**: New device (< 7 days, < 5 logins)
/// - **Recognized**: Seen before but not trusted (7-30 days or 5-20 logins)
/// - **Familiar**: Regular device (30+ days or 20+ logins)
/// - **Trusted**: User explicitly marked as trusted
/// - **HighlyTrusted**: Trusted + has passkey
fn calculate_trust_level(
    user_marked_trusted: bool,
    login_count: i32,
    first_seen: DateTime<Utc>,
) -> DeviceTrustLevel {
    if user_marked_trusted {
        return DeviceTrustLevel::Trusted;
    }

    let age_days = (Utc::now() - first_seen).num_days();

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
