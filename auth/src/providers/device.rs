//! Device repository trait.
//!
//! # Query-Only Repository (Event Sourced)
//!
//! This repository reads from projections (read models) built from events.
//! All writes happen via event emission in reducers.
//!
//! **Architecture**:
//! - ✅ Queries: Read from `devices_projection` table
//! - ❌ Writes: Use event emission (reducers emit `DeviceRegistered`, `DeviceAccessed` events)
//! - 🔄 Projections: `AuthProjection` listens to events and updates read models

use crate::error::Result;
use crate::actions::DeviceTrustLevel;
use crate::state::{DeviceId, UserId};
use super::Device;

/// Device repository (query-only).
///
/// This trait provides read access to device data from projections.
///
/// **Event Sourcing Note**: This repository reads from `devices_projection` table,
/// which is updated by the `AuthProjection` event handler. All device state changes
/// happen via event emission in reducers (e.g., `DeviceRegistered`, `DeviceAccessed` events).
///
/// # Implementation Notes
///
/// - Devices are permanent (audit trail via events)
/// - Track first seen, last seen, trust level (from events)
/// - Link to passkey credentials (via `PasskeyRegistered` events)
pub trait DeviceRepository: Send + Sync {
    /// Get device by ID.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - Device not found → `AuthError::ResourceNotFound`
    fn get_device(
        &self,
        device_id: DeviceId,
    ) -> impl std::future::Future<Output = Result<Device>> + Send;

    /// Get all devices for a user.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn get_user_devices(
        &self,
        user_id: UserId,
    ) -> impl std::future::Future<Output = Result<Vec<Device>>> + Send;

    /// Create device.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - Device ID already exists
    fn create_device(
        &self,
        device: &Device,
    ) -> impl std::future::Future<Output = Result<Device>> + Send;

    /// Update device.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Database query fails
    /// - Device not found
    fn update_device(
        &self,
        device: &Device,
    ) -> impl std::future::Future<Output = Result<Device>> + Send;

    /// Update device trust level.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn update_device_trust_level(
        &self,
        device_id: DeviceId,
        trust_level: DeviceTrustLevel,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Update device last seen.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn update_device_last_seen(
        &self,
        device_id: DeviceId,
        last_seen: chrono::DateTime<chrono::Utc>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Delete device.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn delete_device(
        &self,
        device_id: DeviceId,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Find device by fingerprint.
    ///
    /// Attempts to find an existing device based on user agent,
    /// platform, and other fingerprinting signals.
    ///
    /// # Returns
    ///
    /// The device if found, or `None` if this is a new device.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn find_device_by_fingerprint(
        &self,
        user_id: UserId,
        user_agent: &str,
        platform: &str,
    ) -> impl std::future::Future<Output = Result<Option<Device>>> + Send;
}
