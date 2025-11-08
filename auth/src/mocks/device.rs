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
        device_id: DeviceId,
    ) -> impl Future<Output = Result<Device>> + Send {
        let devices = Arc::clone(&self.devices);

        async move {
            devices
                .lock()
                .map_err(|_| AuthError::InternalError)?
                .get(&device_id)
                .cloned()
                .ok_or(AuthError::ResourceNotFound)
        }
    }

    fn get_user_devices(
        &self,
        user_id: UserId,
    ) -> impl Future<Output = Result<Vec<Device>>> + Send {
        let devices = Arc::clone(&self.devices);

        async move {
            let devices_guard = devices.lock().map_err(|_| AuthError::InternalError)?;
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
            let mut devices_guard = devices.lock().map_err(|_| AuthError::InternalError)?;

            if devices_guard.contains_key(&device.device_id) {
                return Err(AuthError::DatabaseError("Device ID already exists".to_string()));
            }

            devices_guard.insert(device.device_id, device.clone());
            Ok(device)
        }
    }

    fn update_device(
        &self,
        device: &Device,
    ) -> impl Future<Output = Result<Device>> + Send {
        let devices = Arc::clone(&self.devices);
        let device = device.clone();

        async move {
            let mut devices_guard = devices.lock().map_err(|_| AuthError::InternalError)?;

            if !devices_guard.contains_key(&device.device_id) {
                return Err(AuthError::ResourceNotFound);
            }

            devices_guard.insert(device.device_id, device.clone());
            Ok(device)
        }
    }

    fn update_device_trust_level(
        &self,
        device_id: DeviceId,
        trust_level: DeviceTrustLevel,
    ) -> impl Future<Output = Result<()>> + Send {
        let devices = Arc::clone(&self.devices);

        async move {
            let mut devices_guard = devices.lock().map_err(|_| AuthError::InternalError)?;

            if let Some(device) = devices_guard.get_mut(&device_id) {
                device.trust_level = trust_level;
                Ok(())
            } else {
                Err(AuthError::ResourceNotFound)
            }
        }
    }

    fn update_device_last_seen(
        &self,
        device_id: DeviceId,
        last_seen: chrono::DateTime<chrono::Utc>,
    ) -> impl Future<Output = Result<()>> + Send {
        let devices = Arc::clone(&self.devices);

        async move {
            let mut devices_guard = devices.lock().map_err(|_| AuthError::InternalError)?;

            if let Some(device) = devices_guard.get_mut(&device_id) {
                device.last_seen = last_seen;
                Ok(())
            } else {
                Err(AuthError::ResourceNotFound)
            }
        }
    }

    fn delete_device(
        &self,
        device_id: DeviceId,
    ) -> impl Future<Output = Result<()>> + Send {
        let devices = Arc::clone(&self.devices);

        async move {
            devices
                .lock()
                .map_err(|_| AuthError::InternalError)?
                .remove(&device_id);
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
            let devices_guard = devices.lock().map_err(|_| AuthError::InternalError)?;

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
