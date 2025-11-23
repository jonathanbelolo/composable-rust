//! Local projection stream using tokio broadcast channels.
//!
//! Replaces Redpanda-based `ProjectionStream` for in-process projections.
//!
//! # Key Differences from Redpanda
//!
//! - **No checkpointing**: Projections rebuild from PostgreSQL on restart
//! - **No consumer groups**: Each projection gets its own receiver
//! - **Simpler**: No offset management, no external dependencies
//! - **Faster**: 100-1000x faster for same-process communication
//! - **Lagging strategy**: Drop old messages (eventually consistent)
//!
//! # Usage
//!
//! ```rust,ignore
//! let mut stream = LocalProjectionStream::new(
//!     resources.inventory_actions.clone(),
//!     "available-seats"
//! );
//!
//! loop {
//!     match stream.recv().await {
//!         Ok(action) => projection.process_action(action).await?,
//!         Err(RecvError::ChannelClosed) => break,
//!     }
//! }
//! ```

use tokio::sync::broadcast;

/// Projection stream backed by local broadcast channel.
///
/// This replaces `ProjectionStream` (Redpanda-based) for in-process projections.
/// Actions are broadcast from aggregates via global channels, and projections
/// subscribe to receive updates in real-time.
///
/// # Lagging Strategy
///
/// If a projection falls behind and the channel buffer is full, old messages
/// are dropped. This is acceptable because:
/// - Projections are eventually consistent
/// - Projections rebuild from PostgreSQL on server restart
/// - Real-time updates are best-effort
pub struct LocalProjectionStream<A> {
    receiver: broadcast::Receiver<A>,
    name: String,
}

impl<A: Clone> LocalProjectionStream<A> {
    /// Create a new projection stream from a broadcast channel.
    ///
    /// # Arguments
    ///
    /// * `channel` - Broadcast sender to subscribe to
    /// * `name` - Name of the projection (for logging)
    ///
    /// # Returns
    ///
    /// A new stream that will receive all actions broadcast on the channel.
    #[must_use]
    pub fn new(channel: broadcast::Sender<A>, name: impl Into<String>) -> Self {
        Self {
            receiver: channel.subscribe(),
            name: name.into(),
        }
    }

    /// Receive the next action from the stream.
    ///
    /// This method blocks until an action is available or the channel is closed.
    ///
    /// # Returns
    ///
    /// - `Ok(action)` - Successfully received an action
    /// - `Err(RecvError::ChannelClosed)` - Channel closed, no more actions
    ///
    /// # Lagging
    ///
    /// If the projection falls behind and messages are dropped, a warning
    /// is logged and the stream continues with the next available message.
    ///
    /// # Errors
    ///
    /// Returns `RecvError::ChannelClosed` if the broadcast channel is closed.
    pub async fn recv(&mut self) -> Result<A, RecvError> {
        match self.receiver.recv().await {
            Ok(action) => Ok(action),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // Projection fell behind, dropped messages
                // This is OK - projection rebuilds from PostgreSQL on restart
                tracing::warn!(
                    projection = %self.name,
                    dropped = n,
                    "Projection lagged, dropped messages"
                );
                // Continue with next message
                self.receiver
                    .recv()
                    .await
                    .map_err(|_| RecvError::ChannelClosed)
            }
            Err(broadcast::error::RecvError::Closed) => Err(RecvError::ChannelClosed),
        }
    }
}

/// Errors that can occur when receiving from a projection stream.
#[derive(Debug)]
pub enum RecvError {
    /// The broadcast channel has been closed.
    ///
    /// This indicates that all senders have been dropped and no more
    /// actions will be received. The projection should gracefully shut down.
    ChannelClosed,
}

impl std::fmt::Display for RecvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelClosed => write!(f, "Broadcast channel closed"),
        }
    }
}

impl std::error::Error for RecvError {}
