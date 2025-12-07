//! `PostgreSQL` LISTEN/NOTIFY listener.
//!
//! This module provides a background task that listens to `PostgreSQL`
//! notification channels and broadcasts events to WebSocket clients.

use super::{notification::RawNotification, EventNotification, WsManager};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

/// Configuration for the `pg_notify` listener.
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::websocket::ListenerConfig;
///
/// let config = ListenerConfig::new()
///     .with_channel("sales_events")
///     .with_channel("inventory_events");
/// ```
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    /// Channels to listen on.
    ///
    /// Must be explicitly configured - there are no default channels.
    pub channels: Vec<String>,

    /// Reconnection delay after connection failure.
    pub reconnect_delay: Duration,
}

impl Default for ListenerConfig {
    fn default() -> Self {
        Self {
            channels: Vec::new(),
            reconnect_delay: Duration::from_secs(5),
        }
    }
}

impl ListenerConfig {
    /// Create a new configuration with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the channels to listen on.
    #[must_use]
    pub fn with_channels(mut self, channels: Vec<String>) -> Self {
        self.channels = channels;
        self
    }

    /// Add a channel to listen on.
    #[must_use]
    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channels.push(channel.into());
        self
    }

    /// Set the reconnection delay.
    #[must_use]
    pub const fn with_reconnect_delay(mut self, delay: Duration) -> Self {
        self.reconnect_delay = delay;
        self
    }
}

/// Start the `PostgreSQL` notification listener.
///
/// This function runs indefinitely, listening for `PostgreSQL` `NOTIFY` messages
/// and broadcasting them to WebSocket clients via the [`WsManager`].
///
/// # Reconnection
///
/// If the connection is lost, the listener will automatically reconnect
/// after a configurable delay (default: 5 seconds).
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::websocket::{pg_notify_listener, WsManager, ListenerConfig};
/// use std::sync::Arc;
///
/// let manager = Arc::new(WsManager::new(1000));
/// let config = ListenerConfig::new()
///     .with_channel("custom_events");
///
/// tokio::spawn(pg_notify_listener(pool, manager, config));
/// ```
pub async fn pg_notify_listener(pool: PgPool, manager: Arc<WsManager>, config: ListenerConfig) {
    loop {
        match run_listener(&pool, &manager, &config).await {
            Ok(()) => {
                tracing::info!("PostgreSQL listener connection closed, reconnecting...");
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    delay_secs = config.reconnect_delay.as_secs(),
                    "PostgreSQL listener error, reconnecting..."
                );
            }
        }

        tokio::time::sleep(config.reconnect_delay).await;
    }
}

/// Run the LISTEN loop until connection is lost.
#[allow(clippy::cognitive_complexity)]
async fn run_listener(
    pool: &PgPool,
    manager: &WsManager,
    config: &ListenerConfig,
) -> Result<(), sqlx::Error> {
    let mut listener = sqlx::postgres::PgListener::connect_with(pool).await?;

    // Subscribe to all configured channels
    for channel in &config.channels {
        listener.listen(channel).await?;
        tracing::debug!(channel = %channel, "Listening on PostgreSQL channel");
    }

    tracing::info!(
        channels = ?config.channels,
        "PostgreSQL LISTEN connection established"
    );

    // Process notifications
    loop {
        let notification = listener.recv().await?;

        tracing::trace!(
            channel = %notification.channel(),
            payload_len = notification.payload().len(),
            "Received PostgreSQL notification"
        );

        // Parse the notification payload
        match serde_json::from_str::<RawNotification>(notification.payload()) {
            Ok(raw) => {
                match EventNotification::try_from(raw) {
                    Ok(event) => {
                        tracing::debug!(
                            global_id = event.global_id(),
                            context = %event.context(),
                            event_type = %event.event_type(),
                            "Broadcasting event to WebSocket clients"
                        );
                        manager.broadcast(event);
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            channel = %notification.channel(),
                            "Failed to parse event timestamp"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    channel = %notification.channel(),
                    payload = %notification.payload(),
                    "Failed to parse notification payload"
                );
            }
        }
    }
}

/// Start a listener with default configuration.
///
/// Convenience function that uses [`ListenerConfig::default()`].
///
/// # Example
///
/// ```rust,ignore
/// use composable_rust_pg_gateway::websocket::{pg_notify_listener_default, WsManager};
/// use std::sync::Arc;
///
/// let manager = Arc::new(WsManager::new(1000));
/// tokio::spawn(pg_notify_listener_default(pool, manager));
/// ```
pub async fn pg_notify_listener_default(pool: PgPool, manager: Arc<WsManager>) {
    pg_notify_listener(pool, manager, ListenerConfig::default()).await;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn listener_config_defaults() {
        let config = ListenerConfig::default();

        // No default channels - must be explicitly configured
        assert!(config.channels.is_empty());
        assert_eq!(config.reconnect_delay, Duration::from_secs(5));
    }

    #[test]
    fn listener_config_builder() {
        let config = ListenerConfig::new()
            .with_channels(vec!["custom1".to_string()])
            .with_channel("custom2")
            .with_reconnect_delay(Duration::from_secs(10));

        assert_eq!(config.channels, vec!["custom1", "custom2"]);
        assert_eq!(config.reconnect_delay, Duration::from_secs(10));
    }
}
