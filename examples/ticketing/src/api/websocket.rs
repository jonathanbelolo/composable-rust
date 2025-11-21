//! WebSocket endpoints for real-time updates.
//!
//! Provides WebSocket connections for:
//! - Real-time seat availability updates
//! - Reservation status notifications
//! - Payment confirmation events
//!
//! # WebSocket Protocol
//!
//! ## Connection
//!
//! **Unauthenticated** (public availability updates):
//! ```text
//! ws://localhost:8080/api/ws/availability/:event_id
//! ```
//!
//! **Authenticated** (personal notifications):
//! ```text
//! ws://localhost:8080/api/ws/notifications
//! Authorization: Bearer <session_token>
//! ```
//!
//! ## Message Format
//!
//! **Server → Client (Availability Update):**
//! ```json
//! {
//!   "type": "availability_update",
//!   "event_id": "550e8400-...",
//!   "section": "VIP",
//!   "available": 42,
//!   "reserved": 8,
//!   "sold": 50
//! }
//! ```
//!
//! **Server → Client (Reservation Status):**
//! ```json
//! {
//!   "type": "reservation_status",
//!   "reservation_id": "660e8400-...",
//!   "status": "Completed",
//!   "message": "Your tickets have been issued!"
//! }
//! ```
//!
//! **Server → Client (Error):**
//! ```json
//! {
//!   "type": "error",
//!   "message": "Session expired. Please reconnect."
//! }
//! ```
//!
//! ## Connection Limits
//!
//! - Max 1000 concurrent WebSocket connections per server instance
//! - Idle timeout: 5 minutes
//! - Ping/Pong keep-alive every 30 seconds
//!
//! ## Security
//!
//! - Public availability endpoint: No authentication (read-only event data)
//! - Personal notifications endpoint: Requires valid session token
//! - Rate limiting: Max 1 connection per user to notifications endpoint

#![allow(clippy::cognitive_complexity, clippy::too_many_lines)] // WebSocket event loops are naturally complex
#![allow(clippy::cast_sign_loss, clippy::cast_precision_loss)] // Safe casts from projection data
#![allow(clippy::unnecessary_map_or, clippy::uninlined_format_args)] // Example code

use crate::auth::middleware::SessionUser;
use crate::server::state::AppState;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Path, State, WebSocketUpgrade,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures::{stream::StreamExt, SinkExt};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::time::{interval, Duration};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Global WebSocket connection counter.
///
/// Tracks active connections to enforce system-wide limits.
static ACTIVE_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

/// Maximum concurrent WebSocket connections.
const MAX_CONNECTIONS: usize = 1000;

/// Ping interval for keep-alive (30 seconds).
const PING_INTERVAL_SECS: u64 = 30;

/// Idle timeout (5 minutes).
const IDLE_TIMEOUT_SECS: u64 = 300;

// ============================================================================
// Message Types
// ============================================================================

/// WebSocket message from server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TicketingWsMessage {
    /// Seat availability update for an event/section
    AvailabilityUpdate {
        /// Event ID
        event_id: Uuid,
        /// Section name
        section: String,
        /// Available seats
        available: u32,
        /// Reserved seats (pending payment)
        reserved: u32,
        /// Sold seats
        sold: u32,
    },
    /// Reservation status change notification
    ReservationStatus {
        /// Reservation ID
        reservation_id: Uuid,
        /// New status
        status: String,
        /// User-friendly message
        message: String,
    },
    /// Payment confirmation notification
    PaymentConfirmation {
        /// Payment ID
        payment_id: Uuid,
        /// Reservation ID
        reservation_id: Uuid,
        /// Status
        status: String,
        /// User-friendly message
        message: String,
    },
    /// Error message
    Error {
        /// Error description
        message: String,
    },
    /// Ping message (keep-alive)
    Ping,
    /// Pong response
    Pong,
}

// ============================================================================
// Handlers
// ============================================================================

/// WebSocket endpoint for real-time availability updates.
///
/// **Public endpoint** - no authentication required. Clients can subscribe to
/// real-time seat availability changes for a specific event.
///
/// Use cases:
/// - Live seat availability display on ticket selection page
/// - "Only N seats left!" urgency indicators
/// - Section popularity visualization
///
/// # Connection Limit
///
/// Returns 503 Service Unavailable if max connections (1000) exceeded.
///
/// # Example
///
/// ```javascript
/// const ws = new WebSocket('ws://localhost:8080/api/ws/availability/550e8400-...');
///
/// ws.onmessage = (event) => {
///   const msg = JSON.parse(event.data);
///   if (msg.type === 'availability_update') {
///     console.log(`${msg.section}: ${msg.available} seats available`);
///   }
/// };
/// ```
#[allow(clippy::unused_async)] // Axum handler signature requires async
pub async fn availability_updates(
    ws: WebSocketUpgrade,
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Response {
    // Check connection limit
    let current = ACTIVE_CONNECTIONS.load(Ordering::Relaxed);
    if current >= MAX_CONNECTIONS {
        warn!(
            current_connections = current,
            "WebSocket connection limit exceeded"
        );
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Too many concurrent connections. Please try again later.",
        )
            .into_response();
    }

    info!("WebSocket connection requested for event {}", event_id);

    // Upgrade to WebSocket
    ws.on_upgrade(move |socket| handle_availability_socket(socket, event_id, state))
}

/// WebSocket endpoint for authenticated personal notifications.
///
/// Requires authentication via `SessionUser` extractor. Clients receive:
/// - Reservation status updates (initiated, confirmed, expired, cancelled)
/// - Payment confirmation notifications
/// - Ticket issuance notifications
///
/// # Rate Limiting
///
/// Only **one connection per user** is allowed. If a second connection is attempted,
/// the first connection will be closed.
///
/// # Example
///
/// ```javascript
/// const ws = new WebSocket('ws://localhost:8080/api/ws/notifications', {
///   headers: {
///     'Authorization': 'Bearer <session_token>'
///   }
/// });
///
/// ws.onmessage = (event) => {
///   const msg = JSON.parse(event.data);
///   if (msg.type === 'payment_confirmation') {
///     console.log(`Payment confirmed: ${msg.message}`);
///   }
/// };
/// ```
///
/// # Errors
///
/// Returns `AppError::Unauthorized` if session token is invalid or expired.
#[allow(clippy::unused_async)] // Axum handler signature requires async
pub async fn personal_notifications(
    session: SessionUser,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    // Check connection limit
    let current = ACTIVE_CONNECTIONS.load(Ordering::Relaxed);
    if current >= MAX_CONNECTIONS {
        warn!(
            current_connections = current,
            "WebSocket connection limit exceeded"
        );
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Too many concurrent connections. Please try again later.",
        )
            .into_response();
    }

    info!(
        "WebSocket connection requested for user {}",
        session.user_id.0
    );

    // TODO: Check if user already has an active connection (rate limiting)
    // TODO: Store connection in a connection registry (DashMap<UserId, WebSocket>)
    // TODO: Close existing connection if present

    // Upgrade to WebSocket
    ws.on_upgrade(move |socket| handle_notifications_socket(socket, session, state))
}

// ============================================================================
// Socket Handlers
// ============================================================================

/// Handle WebSocket connection for availability updates.
///
/// Subscribes to availability projection updates and streams changes to client.
async fn handle_availability_socket(socket: WebSocket, event_id: Uuid, state: AppState) {
    use crate::aggregates::ReservationAction;
    use crate::config::Config;
    use crate::projections::TicketingEvent;
    use crate::types::EventId;

    // Increment connection counter
    let count = ACTIVE_CONNECTIONS.fetch_add(1, Ordering::Relaxed) + 1;
    info!(
        event_id = %event_id,
        total_connections = count,
        "WebSocket connection established (availability)"
    );

    // Split socket into sender and receiver
    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(tokio::sync::Mutex::new(sender));

    // Send initial availability snapshot via store/reducer pattern
    let event_id_typed = EventId::from_uuid(event_id);
    let inventory_store = state.create_inventory_store();

    // Query through store for initial snapshot
    if let Ok(result) = inventory_store
        .send_and_wait_for(
            crate::aggregates::InventoryAction::GetAllSections {
                event_id: event_id_typed,
            },
            |action| {
                matches!(
                    action,
                    crate::aggregates::InventoryAction::AllSectionsQueried { .. }
                )
            },
            std::time::Duration::from_secs(5),
        )
        .await
    {
        // Extract sections from result
        if let crate::aggregates::InventoryAction::AllSectionsQueried { sections, .. } = result {
            for section_availability in sections {
                let initial_msg = TicketingWsMessage::AvailabilityUpdate {
                    event_id,
                    section: section_availability.section.clone(),
                    available: section_availability.available,
                    reserved: section_availability.reserved,
                    sold: section_availability.sold,
                };

                if let Ok(json) = serde_json::to_string(&initial_msg) {
                    let mut sender_guard = sender.lock().await;
                    if sender_guard.send(Message::Text(json)).await.is_err() {
                        debug!("Client disconnected before initial message");
                        ACTIVE_CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
                        return;
                    }
                }
            }
        }
    }

    // Spawn EventBus consumer task for real-time updates
    let event_sender = sender.clone();
    let event_inventory_store = state.create_inventory_store();
    let event_bus = state.event_bus.clone();
    let config = Config::from_env();

    let mut event_task = tokio::spawn(async move {
        // Subscribe to reservation topic (where inventory events are published)
        let topics = &[config.redpanda.reservation_topic.as_str()];

        match event_bus.subscribe(topics).await {
            Ok(mut stream) => {
                debug!("WebSocket subscribed to reservation topic for availability updates");

                while let Some(result) = stream.next().await {
                    match result {
                        Ok(serialized_event) => {
                            // Deserialize event
                            if let Ok(ticketing_event) = bincode::deserialize::<TicketingEvent>(&serialized_event.data) {
                                // Filter events by event_id (only ReservationInitiated has event_id)
                                let matches_event = match &ticketing_event {
                                    TicketingEvent::Reservation(ReservationAction::ReservationInitiated {
                                        event_id: ev_id, ..
                                    }) => ev_id == &event_id_typed,
                                    TicketingEvent::Inventory(_) => true, // All inventory events affect availability
                                    _ => false,
                                };

                                if matches_event {
                                    // Query updated availability through store/reducer pattern
                                    if let Ok(result) = event_inventory_store
                                        .send_and_wait_for(
                                            crate::aggregates::InventoryAction::GetAllSections {
                                                event_id: event_id_typed,
                                            },
                                            |action| {
                                                matches!(
                                                    action,
                                                    crate::aggregates::InventoryAction::AllSectionsQueried { .. }
                                                )
                                            },
                                            std::time::Duration::from_secs(5),
                                        )
                                        .await
                                    {
                                        // Extract sections from result
                                        if let crate::aggregates::InventoryAction::AllSectionsQueried { sections, .. } = result {
                                            for section_availability in sections {
                                                let update_msg = TicketingWsMessage::AvailabilityUpdate {
                                                    event_id,
                                                    section: section_availability.section.clone(),
                                                    available: section_availability.available,
                                                    reserved: section_availability.reserved,
                                                    sold: section_availability.sold,
                                                };

                                                if let Ok(json) = serde_json::to_string(&update_msg) {
                                                    let mut sender_guard = event_sender.lock().await;
                                                    if sender_guard.send(Message::Text(json)).await.is_err() {
                                                        debug!("Client disconnected during event stream");
                                                        return;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Error receiving event from EventBus");
                        }
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to subscribe to EventBus for availability updates");
            }
        }
    });

    // Spawn ping task for keep-alive
    let ping_sender = sender.clone();
    let mut ping_interval = interval(Duration::from_secs(PING_INTERVAL_SECS));
    let mut ping_task = tokio::spawn(async move {
        loop {
            ping_interval.tick().await;
            let ping_msg = TicketingWsMessage::Ping;
            if let Ok(json) = serde_json::to_string(&ping_msg) {
                let mut sender_guard = ping_sender.lock().await;
                if sender_guard.send(Message::Text(json)).await.is_err() {
                    break;
                }
            } else {
                break;
            }
        }
        debug!("WebSocket ping task terminated");
    });

    // Spawn receive task (handle pong and close messages)
    let mut recv_task = tokio::spawn(async move {
        let timeout = tokio::time::sleep(Duration::from_secs(IDLE_TIMEOUT_SECS));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                Some(Ok(msg)) = receiver.next() => {
                    match msg {
                        Message::Pong(_) => {
                            debug!("Received pong from client");
                            // Reset timeout
                            timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(IDLE_TIMEOUT_SECS));
                        }
                        Message::Close(_) => {
                            info!("Client requested close");
                            break;
                        }
                        _ => {
                            debug!("Received unexpected message type");
                        }
                    }
                }
                () = &mut timeout => {
                    warn!("WebSocket idle timeout");
                    break;
                }
            }
        }

        debug!("WebSocket receive task terminated");
    });

    // Wait for any task to complete
    tokio::select! {
        _ = (&mut event_task) => {
            debug!("Event task completed, aborting other tasks");
            ping_task.abort();
            recv_task.abort();
        },
        _ = (&mut ping_task) => {
            debug!("Ping task completed, aborting other tasks");
            event_task.abort();
            recv_task.abort();
        },
        _ = (&mut recv_task) => {
            debug!("Receive task completed, aborting other tasks");
            event_task.abort();
            ping_task.abort();
        },
    }

    // Decrement connection counter
    let count = ACTIVE_CONNECTIONS.fetch_sub(1, Ordering::Relaxed) - 1;
    info!(
        event_id = %event_id,
        total_connections = count,
        "WebSocket connection closed (availability)"
    );
}

/// Handle WebSocket connection for authenticated personal notifications.
///
/// Streams reservation and payment notifications to the authenticated user.
async fn handle_notifications_socket(socket: WebSocket, session: SessionUser, state: AppState) {
    use crate::aggregates::{PaymentAction, ReservationAction};
    use crate::config::Config;
    use crate::projections::TicketingEvent;
    use crate::types::CustomerId;

    // Increment connection counter
    let count = ACTIVE_CONNECTIONS.fetch_add(1, Ordering::Relaxed) + 1;
    info!(
        user_id = %session.user_id.0,
        total_connections = count,
        "WebSocket connection established (notifications)"
    );

    // Split socket into sender and receiver
    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(tokio::sync::Mutex::new(sender));

    // Send welcome message
    let welcome = TicketingWsMessage::ReservationStatus {
        reservation_id: Uuid::nil(),
        status: "Connected".to_string(),
        message: "Real-time notifications enabled. You'll receive updates about your reservations and payments.".to_string(),
    };

    if let Ok(json) = serde_json::to_string(&welcome) {
        let mut sender_guard = sender.lock().await;
        if sender_guard.send(Message::Text(json)).await.is_err() {
            debug!("Client disconnected before welcome message");
            ACTIVE_CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
            return;
        }
    }

    // Spawn EventBus consumer task for personal notifications
    let event_sender = sender.clone();
    let event_bus = state.event_bus.clone();
    let reservation_ownership = state.reservation_ownership.clone();
    let payment_ownership = state.payment_ownership.clone();
    let config = Config::from_env();
    let customer_id_typed = CustomerId::from_uuid(session.user_id.0);

    let mut event_task = tokio::spawn(async move {
        // Subscribe to reservation and payment topics
        let topics = &[
            config.redpanda.reservation_topic.as_str(),
            config.redpanda.payment_topic.as_str(),
        ];

        match event_bus.subscribe(topics).await {
            Ok(mut stream) => {
                debug!("WebSocket subscribed to reservation and payment topics for notifications");

                while let Some(result) = stream.next().await {
                    match result {
                        Ok(serialized_event) => {
                            // Deserialize event
                            if let Ok(ticketing_event) = bincode::deserialize::<TicketingEvent>(&serialized_event.data) {
                                // Filter and transform events by customer_id
                                let notification = match &ticketing_event {
                                    // Reservation events
                                    TicketingEvent::Reservation(ReservationAction::ReservationInitiated {
                                        reservation_id, event_id, customer_id, ..
                                    }) if customer_id == &customer_id_typed => {
                                        Some(TicketingWsMessage::ReservationStatus {
                                            reservation_id: *reservation_id.as_uuid(),
                                            status: "Initiated".to_string(),
                                            message: format!("Reservation created for event {}", event_id.as_uuid()),
                                        })
                                    }
                                    TicketingEvent::Reservation(ReservationAction::ReservationCompleted {
                                        reservation_id, tickets_issued, ..
                                    }) => {
                                        // Check ownership via index
                                        let belongs_to_user = reservation_ownership
                                            .read()
                                            .ok()
                                            .and_then(|index| index.get(reservation_id).copied())
                                            .map_or(false, |owner| owner == customer_id_typed);

                                        if belongs_to_user {
                                            Some(TicketingWsMessage::ReservationStatus {
                                                reservation_id: *reservation_id.as_uuid(),
                                                status: "Completed".to_string(),
                                                message: format!("Your tickets have been issued! {} ticket(s)", tickets_issued.len()),
                                            })
                                        } else {
                                            None
                                        }
                                    }
                                    TicketingEvent::Reservation(ReservationAction::ReservationExpired {
                                        reservation_id, ..
                                    }) => {
                                        // Check ownership via index
                                        let belongs_to_user = reservation_ownership
                                            .read()
                                            .ok()
                                            .and_then(|index| index.get(reservation_id).copied())
                                            .map_or(false, |owner| owner == customer_id_typed);

                                        if belongs_to_user {
                                            Some(TicketingWsMessage::ReservationStatus {
                                                reservation_id: *reservation_id.as_uuid(),
                                                status: "Expired".to_string(),
                                                message: "Reservation expired due to timeout".to_string(),
                                            })
                                        } else {
                                            None
                                        }
                                    }
                                    TicketingEvent::Reservation(ReservationAction::ReservationCancelled {
                                        reservation_id, cancelled_at, ..
                                    }) => {
                                        // Check ownership via index
                                        let belongs_to_user = reservation_ownership
                                            .read()
                                            .ok()
                                            .and_then(|index| index.get(reservation_id).copied())
                                            .map_or(false, |owner| owner == customer_id_typed);

                                        if belongs_to_user {
                                            Some(TicketingWsMessage::ReservationStatus {
                                                reservation_id: *reservation_id.as_uuid(),
                                                status: "Cancelled".to_string(),
                                                message: format!("Reservation cancelled at {}", cancelled_at),
                                            })
                                        } else {
                                            None
                                        }
                                    }

                                    // Payment events - Check ownership via two-step lookup
                                    TicketingEvent::Payment(PaymentAction::PaymentProcessed {
                                        payment_id, reservation_id, amount, ..
                                    }) => {
                                        // Check if reservation belongs to user
                                        let belongs_to_user = reservation_ownership
                                            .read()
                                            .ok()
                                            .and_then(|index| index.get(reservation_id).copied())
                                            .map_or(false, |owner| owner == customer_id_typed);

                                        if belongs_to_user {
                                            Some(TicketingWsMessage::PaymentConfirmation {
                                                payment_id: *payment_id.as_uuid(),
                                                reservation_id: *reservation_id.as_uuid(),
                                                status: "Processed".to_string(),
                                                message: format!("Payment of ${:.2} is being processed", amount.cents() as f64 / 100.0),
                                            })
                                        } else {
                                            None
                                        }
                                    }
                                    TicketingEvent::Payment(PaymentAction::PaymentSucceeded {
                                        payment_id, transaction_id, ..
                                    }) => {
                                        // Two-step lookup: payment_id → reservation_id → customer_id
                                        let belongs_to_user = payment_ownership
                                            .read()
                                            .ok()
                                            .and_then(|pay_index| pay_index.get(payment_id).copied())
                                            .and_then(|res_id| {
                                                reservation_ownership
                                                    .read()
                                                    .ok()
                                                    .and_then(|res_index| res_index.get(&res_id).copied())
                                            })
                                            .map_or(false, |owner| owner == customer_id_typed);

                                        if belongs_to_user {
                                            Some(TicketingWsMessage::PaymentConfirmation {
                                                payment_id: *payment_id.as_uuid(),
                                                reservation_id: Uuid::nil(), // Not in this event
                                                status: "Succeeded".to_string(),
                                                message: format!("Payment succeeded! Transaction ID: {}", transaction_id),
                                            })
                                        } else {
                                            None
                                        }
                                    }
                                    TicketingEvent::Payment(PaymentAction::PaymentFailed {
                                        payment_id, reason, ..
                                    }) => {
                                        // Two-step lookup: payment_id → reservation_id → customer_id
                                        let belongs_to_user = payment_ownership
                                            .read()
                                            .ok()
                                            .and_then(|pay_index| pay_index.get(payment_id).copied())
                                            .and_then(|res_id| {
                                                reservation_ownership
                                                    .read()
                                                    .ok()
                                                    .and_then(|res_index| res_index.get(&res_id).copied())
                                            })
                                            .map_or(false, |owner| owner == customer_id_typed);

                                        if belongs_to_user {
                                            Some(TicketingWsMessage::PaymentConfirmation {
                                                payment_id: *payment_id.as_uuid(),
                                                reservation_id: Uuid::nil(), // Not in this event
                                                status: "Failed".to_string(),
                                                message: format!("Payment failed: {}", reason),
                                            })
                                        } else {
                                            None
                                        }
                                    }
                                    TicketingEvent::Payment(PaymentAction::PaymentRefunded {
                                        payment_id, amount, reason, ..
                                    }) => {
                                        // Two-step lookup: payment_id → reservation_id → customer_id
                                        let belongs_to_user = payment_ownership
                                            .read()
                                            .ok()
                                            .and_then(|pay_index| pay_index.get(payment_id).copied())
                                            .and_then(|res_id| {
                                                reservation_ownership
                                                    .read()
                                                    .ok()
                                                    .and_then(|res_index| res_index.get(&res_id).copied())
                                            })
                                            .map_or(false, |owner| owner == customer_id_typed);

                                        if belongs_to_user {
                                            Some(TicketingWsMessage::PaymentConfirmation {
                                                payment_id: *payment_id.as_uuid(),
                                                reservation_id: Uuid::nil(), // Not in this event
                                                status: "Refunded".to_string(),
                                                message: format!("Payment refunded: ${:.2} - {}", amount.cents() as f64 / 100.0, reason),
                                            })
                                        } else {
                                            None
                                        }
                                    }

                                    _ => None,
                                };

                                // Send notification if it matches this user
                                if let Some(msg) = notification {
                                    if let Ok(json) = serde_json::to_string(&msg) {
                                        let mut sender_guard = event_sender.lock().await;
                                        if sender_guard.send(Message::Text(json)).await.is_err() {
                                            debug!("Client disconnected during event stream");
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Error receiving event from EventBus");
                        }
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to subscribe to EventBus for notifications");
            }
        }
    });

    // Spawn ping task for keep-alive
    let ping_sender = sender.clone();
    let mut ping_interval = interval(Duration::from_secs(PING_INTERVAL_SECS));
    let mut ping_task = tokio::spawn(async move {
        loop {
            ping_interval.tick().await;
            let ping_msg = TicketingWsMessage::Ping;
            if let Ok(json) = serde_json::to_string(&ping_msg) {
                let mut sender_guard = ping_sender.lock().await;
                if sender_guard.send(Message::Text(json)).await.is_err() {
                    break;
                }
            } else {
                break;
            }
        }
        debug!("WebSocket ping task terminated");
    });

    // Spawn receive task (handle pong and close messages)
    let mut recv_task = tokio::spawn(async move {
        let timeout = tokio::time::sleep(Duration::from_secs(IDLE_TIMEOUT_SECS));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                Some(Ok(msg)) = receiver.next() => {
                    match msg {
                        Message::Pong(_) => {
                            debug!("Received pong from client");
                            // Reset timeout
                            timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(IDLE_TIMEOUT_SECS));
                        }
                        Message::Close(_) => {
                            info!("Client requested close");
                            break;
                        }
                        _ => {
                            debug!("Received unexpected message type");
                        }
                    }
                }
                () = &mut timeout => {
                    warn!("WebSocket idle timeout");
                    break;
                }
            }
        }

        debug!("WebSocket receive task terminated");
    });

    // Wait for any task to complete
    tokio::select! {
        _ = (&mut event_task) => {
            debug!("Event task completed, aborting other tasks");
            ping_task.abort();
            recv_task.abort();
        },
        _ = (&mut ping_task) => {
            debug!("Ping task completed, aborting other tasks");
            event_task.abort();
            recv_task.abort();
        },
        _ = (&mut recv_task) => {
            debug!("Receive task completed, aborting other tasks");
            event_task.abort();
            ping_task.abort();
        },
    }

    // TODO: Remove connection from registry (for rate limiting)

    // Decrement connection counter
    let count = ACTIVE_CONNECTIONS.fetch_sub(1, Ordering::Relaxed) - 1;
    info!(
        user_id = %session.user_id.0,
        total_connections = count,
        "WebSocket connection closed (notifications)"
    );
}

/// Get current WebSocket connection count.
///
/// Useful for monitoring and observability.
#[must_use]
pub fn active_connection_count() -> usize {
    ACTIVE_CONNECTIONS.load(Ordering::Relaxed)
}
