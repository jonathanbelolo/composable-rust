//! Mock device repository for testing.

use crate::error::{AuthError, Result};
use crate::providers::{Device, DeviceRepository};
use crate::actions::DeviceTrustLevel;
use crate::state::{DeviceId, UserId};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

/// Mock device repository.
///
/// Uses in-memory storage for testing.
#[derive(Debug, Clone)]
pub struct MockDeviceRepository {
    devices: Arc<Mutex<HashMap<DeviceId, Device>>>,
}

impl MockDeviceRepository {
    /// Create a new mock device repository.
    #[must_use]
    pub fn new() -> Self {
        Self {
            devices: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for MockDeviceRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceRepository for MockDeviceRepository {
    fn get_device(
        &self,
        user_id: UserId,
        device_id: DeviceId,
    ) -> impl Future<Output = Result<Device>> + Send {
        let devices = Arc::clone(&self.devices);

        async move {
            let devices_guard = devices
                .lock()
                .map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            let device = devices_guard
                .get(&device_id)
                .ok_or(AuthError::ResourceNotFound)?;

            // ✅ Authorization check: Verify device belongs to user
            if device.user_id != user_id {
                return Err(AuthError::ResourceNotFound); // Don't leak existence
            }

            Ok(device.clone())
        }
    }

    fn get_user_devices(
        &self,
        user_id: UserId,
    ) -> impl Future<Output = Result<Vec<Device>>> + Send {
        let devices = Arc::clone(&self.devices);

        async move {
            let devices_guard = devices.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;
            let user_devices: Vec<Device> = devices_guard
                .values()
                .filter(|d| d.user_id == user_id)
                .cloned()
                .collect();
            Ok(user_devices)
        }
    }

    fn create_device(
        &self,
        device: &Device,
    ) -> impl Future<Output = Result<Device>> + Send {
        let devices = Arc::clone(&self.devices);
        let device = device.clone();

        async move {
            // ✅ Validate inputs before storage (XSS/injection prevention)
            crate::utils::validate_device_name(&device.name)?;
            crate::utils::validate_platform(&device.platform)?;

            let mut devices_guard = devices.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            if devices_guard.contains_key(&device.device_id) {
                return Err(AuthError::DatabaseError("Device ID already exists".to_string()));
            }

            devices_guard.insert(device.device_id, device.clone());
            Ok(device)
        }
    }

    fn update_device(
        &self,
        user_id: UserId,
        device: &Device,
    ) -> impl Future<Output = Result<Device>> + Send {
        let devices = Arc::clone(&self.devices);
        let device = device.clone();

        async move {
            let mut devices_guard = devices.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            let existing = devices_guard
                .get(&device.device_id)
                .ok_or(AuthError::ResourceNotFound)?;

            // ✅ Authorization check: Verify device belongs to user
            if existing.user_id != user_id {
                return Err(AuthError::ResourceNotFound);
            }

            // ✅ Authorization check: Prevent device transfer between accounts
            if device.user_id != user_id {
                return Err(AuthError::ResourceNotFound);
            }

            devices_guard.insert(device.device_id, device.clone());
            Ok(device)
        }
    }

    fn update_device_trust_level(
        &self,
        user_id: UserId,
        device_id: DeviceId,
        trust_level: DeviceTrustLevel,
    ) -> impl Future<Output = Result<()>> + Send {
        let devices = Arc::clone(&self.devices);

        async move {
            let mut devices_guard = devices.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            let device = devices_guard
                .get_mut(&device_id)
                .ok_or(AuthError::ResourceNotFound)?;

            // ✅ Authorization check: Verify device belongs to user
            if device.user_id != user_id {
                return Err(AuthError::ResourceNotFound);
            }

            device.trust_level = trust_level;
            Ok(())
        }
    }

    fn update_device_last_seen(
        &self,
        user_id: UserId,
        device_id: DeviceId,
        last_seen: chrono::DateTime<chrono::Utc>,
    ) -> impl Future<Output = Result<()>> + Send {
        let devices = Arc::clone(&self.devices);

        async move {
            let mut devices_guard = devices.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            let device = devices_guard
                .get_mut(&device_id)
                .ok_or(AuthError::ResourceNotFound)?;

            // ✅ Authorization check: Verify device belongs to user
            if device.user_id != user_id {
                return Err(AuthError::ResourceNotFound);
            }

            device.last_seen = last_seen;
            Ok(())
        }
    }

    fn delete_device(
        &self,
        user_id: UserId,
        device_id: DeviceId,
    ) -> impl Future<Output = Result<()>> + Send {
        let devices = Arc::clone(&self.devices);

        async move {
            let mut devices_guard = devices
                .lock()
                .map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            // Check device exists and belongs to user
            let device = devices_guard
                .get(&device_id)
                .ok_or(AuthError::ResourceNotFound)?;

            // ✅ Authorization check: Verify device belongs to user
            if device.user_id != user_id {
                return Err(AuthError::ResourceNotFound);
            }

            devices_guard.remove(&device_id);
            Ok(())
        }
    }

    fn find_device_by_fingerprint(
        &self,
        user_id: UserId,
        user_agent: &str,
        platform: &str,
    ) -> impl Future<Output = Result<Option<Device>>> + Send {
        let devices = Arc::clone(&self.devices);
        let user_agent = user_agent.to_string();
        let platform = platform.to_string();

        async move {
            let devices_guard = devices.lock().map_err(|_| AuthError::InternalError("Mutex lock failed".to_string()))?;

            let found_device = devices_guard
                .values()
                .find(|d| {
                    d.user_id == user_id
                        && d.platform == platform
                        && d.name.contains(&user_agent)
                })
                .cloned();

            Ok(found_device)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::DeviceTrustLevel;
    use crate::providers::DeviceType;

    fn create_test_device(user_id: UserId) -> Device {
        Device {
            device_id: DeviceId::new(),
            user_id,
            name: "Test Device".to_string(),
            device_type: DeviceType::Desktop,
            platform: "Linux".to_string(),
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            trust_level: DeviceTrustLevel::Unknown,
            passkey_credential_id: None,
            public_key: None,
        }
    }

    #[tokio::test]
    async fn test_cross_user_get_device_prevented() {
        let repo = MockDeviceRepository::new();

        let user1 = UserId::new();
        let user2 = UserId::new();

        // User 1 creates a device
        let device = create_test_device(user1);
        let created = repo.create_device(&device).await.unwrap();

        // User 2 tries to access User 1's device
        let result = repo.get_device(user2, created.device_id).await;

        assert!(
            matches!(result, Err(AuthError::ResourceNotFound)),
            "Cross-user device access should be prevented"
        );

        // User 1 can access their own device
        let result = repo.get_device(user1, created.device_id).await;
        assert!(result.is_ok(), "Owner should be able to access their device");
    }

    #[tokio::test]
    async fn test_cross_user_update_device_prevented() {
        let repo = MockDeviceRepository::new();

        let user1 = UserId::new();
        let user2 = UserId::new();

        // User 1 creates a device
        let device = create_test_device(user1);
        let created = repo.create_device(&device).await.unwrap();

        // User 2 tries to update User 1's device
        let mut tampered = created.clone();
        tampered.name = "Hacked Device".to_string();
        let result = repo.update_device(user2, &tampered).await;

        assert!(
            matches!(result, Err(AuthError::ResourceNotFound)),
            "Cross-user device update should be prevented"
        );
    }

    #[tokio::test]
    async fn test_cross_user_trust_manipulation_prevented() {
        let repo = MockDeviceRepository::new();

        let user1 = UserId::new();
        let user2 = UserId::new();

        // User 1 creates a device
        let device = create_test_device(user1);
        let created = repo.create_device(&device).await.unwrap();

        // User 2 tries to mark User 1's device as trusted
        let result = repo
            .update_device_trust_level(user2, created.device_id, DeviceTrustLevel::HighlyTrusted)
            .await;

        assert!(
            matches!(result, Err(AuthError::ResourceNotFound)),
            "Cross-user trust manipulation should be prevented"
        );
    }

    #[tokio::test]
    async fn test_cross_user_delete_device_prevented() {
        let repo = MockDeviceRepository::new();

        let user1 = UserId::new();
        let user2 = UserId::new();

        // User 1 creates a device
        let device = create_test_device(user1);
        let created = repo.create_device(&device).await.unwrap();

        // User 2 tries to delete User 1's device
        let result = repo.delete_device(user2, created.device_id).await;

        assert!(
            matches!(result, Err(AuthError::ResourceNotFound)),
            "Cross-user device deletion should be prevented"
        );

        // Verify device still exists
        let result = repo.get_device(user1, created.device_id).await;
        assert!(result.is_ok(), "Device should still exist after failed delete");
    }

    #[tokio::test]
    async fn test_device_transfer_prevented() {
        let repo = MockDeviceRepository::new();

        let user1 = UserId::new();
        let user2 = UserId::new();

        // User 1 creates a device
        let device = create_test_device(user1);
        let created = repo.create_device(&device).await.unwrap();

        // User 2 tries to transfer the device to their account
        let mut transfer_attempt = created.clone();
        transfer_attempt.user_id = user2; // Try to change ownership
        let result = repo.update_device(user2, &transfer_attempt).await;

        assert!(
            matches!(result, Err(AuthError::ResourceNotFound)),
            "Device transfer between accounts should be prevented"
        );
    }

    #[tokio::test]
    async fn test_update_device_last_seen_authorization() {
        let repo = MockDeviceRepository::new();

        let user1 = UserId::new();
        let user2 = UserId::new();

        // User 1 creates a device
        let device = create_test_device(user1);
        let created = repo.create_device(&device).await.unwrap();

        // User 2 tries to update last_seen on User 1's device
        let result = repo
            .update_device_last_seen(user2, created.device_id, chrono::Utc::now())
            .await;

        assert!(
            matches!(result, Err(AuthError::ResourceNotFound)),
            "Cross-user last_seen update should be prevented"
        );

        // User 1 can update their own device
        let result = repo
            .update_device_last_seen(user1, created.device_id, chrono::Utc::now())
            .await;

        assert!(result.is_ok(), "Owner should be able to update last_seen");
    }
}
