//! Projection manager setup for the ticketing system.
//!
//! This module starts projection consumers that consume events from global action
//! channels and update PostgreSQL projections.
//!
//! # Architecture
//!
//! ```text
//! Aggregates → Global Channels (tokio::broadcast) → Projections (PostgreSQL)
//! ```
//!
//! Each projection runs with:
//! - Subscription to relevant global action channels
//! - Error handling (continue on failure, log errors)
//! - Graceful shutdown support

use crate::config::Config;
use crate::projections::{
    PostgresAvailableSeatsProjection, PostgresEventsProjection, PostgresPaymentsProjection,
    PostgresReservationsProjection, TicketingEvent,
};
use crate::types::GlobalActionChannels;
use composable_rust_core::projection::Projection;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::watch;

/// Setup function to create and configure all projection consumers.
///
/// Creates projection streams for:
/// - Events Projection (event data queries)
/// - Available Seats Projection (seat availability queries)
/// - Payments Projection (payment history queries)
/// - Reservations Projection (reservation history queries)
///
/// Returns task handles and shutdown senders for graceful termination.
///
/// # Arguments
///
/// - `config`: Application configuration
/// - `global_channels`: Global action channels for intra-process communication
///
/// # Errors
///
/// Returns error if projection database connection fails.
pub async fn setup_projection_managers(
    config: &Config,
    global_channels: GlobalActionChannels,
) -> Result<ProjectionManagers, Box<dyn std::error::Error>> {
    // Connect to projection database (separate from event store)
    let projection_pool = PgPool::connect(&config.projections.url).await?;

    // Note: Projection migrations are handled by main.rs before this is called

    // Setup Events Projection
    let events_projection = PostgresEventsProjection::new(Arc::new(projection_pool.clone()));

    // Subscribe to event actions
    let event_receiver = global_channels.event_actions.subscribe();

    // Setup Available Seats Projection
    let available_seats_projection =
        PostgresAvailableSeatsProjection::new(Arc::new(projection_pool.clone()));

    // Subscribe to inventory actions
    let inventory_receiver = global_channels.inventory_actions.subscribe();

    // Setup Payments Projection
    let payments_projection =
        PostgresPaymentsProjection::new(Arc::new(projection_pool.clone()));

    // Subscribe to payment actions
    let payment_receiver = global_channels.payment_actions.subscribe();

    // Setup Reservations Projection
    let reservations_projection =
        PostgresReservationsProjection::new(Arc::new(projection_pool.clone()));

    // Subscribe to reservation actions
    let reservation_receiver = global_channels.reservation_actions.subscribe();

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx_events) = watch::channel(false);
    let shutdown_rx_seats = shutdown_tx.subscribe();
    let shutdown_rx_payments = shutdown_tx.subscribe();
    let shutdown_rx_reservations = shutdown_tx.subscribe();

    Ok(ProjectionManagers {
        events: EventsProjectionRunner {
            projection: events_projection,
            receiver: event_receiver,
            shutdown: shutdown_rx_events,
        },
        available_seats: AvailableSeatsProjectionRunner {
            projection: available_seats_projection,
            receiver: inventory_receiver,
            shutdown: shutdown_rx_seats,
        },
        payments: PaymentsProjectionRunner {
            projection: payments_projection,
            receiver: payment_receiver,
            shutdown: shutdown_rx_payments,
        },
        reservations: ReservationsProjectionRunner {
            projection: reservations_projection,
            receiver: reservation_receiver,
            shutdown: shutdown_rx_reservations,
        },
        shutdown: shutdown_tx,
    })
}

/// Runner for events projection with manual event loop.
struct EventsProjectionRunner {
    projection: PostgresEventsProjection,
    receiver: tokio::sync::broadcast::Receiver<crate::aggregates::EventAction>,
    shutdown: watch::Receiver<bool>,
}

impl EventsProjectionRunner {
    /// Start the projection consumer loop.
    ///
    /// Continuously consumes events from the event channel,
    /// converts to `TicketingEvent`, and applies to projection.
    async fn run(mut self) {
        let projection_name = self.projection.name();
        tracing::info!(
            projection = projection_name,
            "Starting projection consumer"
        );

        loop {
            tokio::select! {
                // Process next event from channel
                Ok(action) = self.receiver.recv() => {
                    // Extract respond_to channel from EVENTS (not commands)
                    // Events carry respond_to for projection completion signaling
                    let respond_to = match &action {
                        crate::aggregates::EventAction::EventCreated { respond_to, .. } => respond_to.clone(),
                        crate::aggregates::EventAction::EventUpdated { respond_to, .. } => respond_to.clone(),
                        crate::aggregates::EventAction::VenueSectionsAdded { respond_to, .. } => respond_to.clone(),
                        crate::aggregates::EventAction::PricingTiersUpdated { respond_to, .. } => respond_to.clone(),
                        _ => crate::types::ResponseChannel::none(),
                    };

                    // Convert to TicketingEvent
                    let event = TicketingEvent::Event(action);

                    // Apply to projection
                    let result = self.projection.apply_event(&event).await;

                    // Send completion notification using framework ResponseChannel
                    let notification = result.as_ref().copied().map_err(|e| {
                        format!("{projection_name}: {e}")
                    });
                    let _ = respond_to.send(notification);

                    // Log result
                    match result {
                        Ok(()) => {
                            tracing::debug!(
                                projection = projection_name,
                                "Successfully applied event to projection"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                projection = projection_name,
                                error = ?e,
                                "Failed to apply event to projection"
                            );
                        }
                    }
                }

                // Handle shutdown
                _ = self.shutdown.changed() => {
                    if *self.shutdown.borrow() {
                        tracing::info!(projection = projection_name, "Shutdown signal received");
                        break;
                    }
                }
            }
        }

        tracing::info!(projection = projection_name, "Projection consumer stopped");
    }
}

/// Runner for available seats projection with manual event loop.
struct AvailableSeatsProjectionRunner {
    projection: PostgresAvailableSeatsProjection,
    receiver: tokio::sync::broadcast::Receiver<crate::aggregates::InventoryAction>,
    shutdown: watch::Receiver<bool>,
}

impl AvailableSeatsProjectionRunner {
    /// Start the projection consumer loop.
    ///
    /// Continuously consumes events from the inventory channel,
    /// converts to TicketingEvent, and applies to projection.
    async fn run(mut self) {
        let projection_name = self.projection.name();
        tracing::info!(
            projection = projection_name,
            "Starting projection consumer"
        );

        loop {
            tokio::select! {
                // Process next event from channel
                Ok(action) = self.receiver.recv() => {
                    // Extract respond_to channel from EVENTS (not commands)
                    // InventoryInitialized carries respond_to for projection completion signaling
                    let respond_to = match &action {
                        crate::aggregates::InventoryAction::InventoryInitialized { respond_to, .. } => respond_to.clone(),
                        _ => crate::types::ResponseChannel::none(),
                    };

                    // Convert to TicketingEvent
                    let event = TicketingEvent::Inventory(action);

                    // Apply to projection
                    let result = self.projection.apply_event(&event).await;

                    // Send completion notification using framework ResponseChannel
                    let notification = result.as_ref().copied().map_err(|e| {
                        format!("{projection_name}: {e}")
                    });
                    let _ = respond_to.send(notification);

                    // Log result
                    match result {
                        Ok(()) => {
                            tracing::debug!(
                                projection = projection_name,
                                "Successfully applied event to projection"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                projection = projection_name,
                                error = ?e,
                                "Failed to apply event to projection"
                            );
                        }
                    }
                }

                // Handle shutdown
                _ = self.shutdown.changed() => {
                    if *self.shutdown.borrow() {
                        tracing::info!(projection = projection_name, "Shutdown signal received");
                        break;
                    }
                }
            }
        }

        tracing::info!(projection = projection_name, "Projection consumer stopped");
    }
}

/// Runner for payments projection with manual event loop.
struct PaymentsProjectionRunner {
    projection: PostgresPaymentsProjection,
    receiver: tokio::sync::broadcast::Receiver<crate::aggregates::PaymentAction>,
    shutdown: watch::Receiver<bool>,
}

impl PaymentsProjectionRunner {
    /// Start the projection consumer loop.
    ///
    /// Continuously consumes events from the payment channel,
    /// converts to TicketingEvent, and applies to projection.
    async fn run(mut self) {
        let projection_name = self.projection.name();
        tracing::info!(
            projection = projection_name,
            "Starting projection consumer"
        );

        loop {
            tokio::select! {
                // Process next event from channel
                Ok(action) = self.receiver.recv() => {
                    // Extract respond_to channel before converting to event
                    let respond_to = match &action {
                        crate::aggregates::PaymentAction::ProcessPayment { respond_to, .. } => respond_to.clone(),
                        _ => crate::types::ResponseChannel::none(),
                    };

                    // Convert to TicketingEvent
                    let event = TicketingEvent::Payment(action);

                    // Apply to projection
                    let result = self.projection.apply_event(&event).await;

                    // Send completion notification using framework ResponseChannel
                    let notification = result.as_ref().copied().map_err(|e| {
                        format!("{projection_name}: {e}")
                    });
                    let _ = respond_to.send(notification);

                    // Log result
                    match result {
                        Ok(()) => {
                            tracing::debug!(
                                projection = projection_name,
                                "Successfully applied event to projection"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                projection = projection_name,
                                error = ?e,
                                "Failed to apply event to projection"
                            );
                        }
                    }
                }

                // Handle shutdown
                _ = self.shutdown.changed() => {
                    if *self.shutdown.borrow() {
                        tracing::info!(projection = projection_name, "Shutdown signal received");
                        break;
                    }
                }
            }
        }

        tracing::info!(projection = projection_name, "Projection consumer stopped");
    }
}

/// Runner for reservations projection with manual event loop.
struct ReservationsProjectionRunner {
    projection: PostgresReservationsProjection,
    receiver: tokio::sync::broadcast::Receiver<crate::aggregates::ReservationAction>,
    shutdown: watch::Receiver<bool>,
}

impl ReservationsProjectionRunner {
    /// Start the projection consumer loop.
    ///
    /// Continuously consumes events from the reservation channel,
    /// converts to `TicketingEvent`, and applies to projection.
    async fn run(mut self) {
        let projection_name = self.projection.name();
        tracing::info!(
            projection = projection_name,
            "Starting projection consumer"
        );

        loop {
            tokio::select! {
                // Process next event from channel
                Ok(action) = self.receiver.recv() => {
                    // Extract respond_to channel before converting to event
                    let respond_to = match &action {
                        crate::aggregates::ReservationAction::InitiateReservation { respond_to, .. } => respond_to.clone(),
                        _ => crate::types::ResponseChannel::none(),
                    };

                    // Convert to TicketingEvent
                    let event = TicketingEvent::Reservation(action);

                    // Apply to projection
                    let result = self.projection.apply_event(&event).await;

                    // Send completion notification using framework ResponseChannel
                    let notification = result.as_ref().copied().map_err(|e| {
                        format!("{projection_name}: {e}")
                    });
                    let _ = respond_to.send(notification);

                    // Log result
                    match result {
                        Ok(()) => {
                            tracing::debug!(
                                projection = projection_name,
                                "Successfully applied event to projection"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                projection = projection_name,
                                error = ?e,
                                "Failed to apply event to projection"
                            );
                        }
                    }
                }

                // Handle shutdown
                _ = self.shutdown.changed() => {
                    if *self.shutdown.borrow() {
                        tracing::info!(projection = projection_name, "Shutdown signal received");
                        break;
                    }
                }
            }
        }

        tracing::info!(projection = projection_name, "Projection consumer stopped");
    }
}

/// Container for all projection runners and shutdown sender.
pub struct ProjectionManagers {
    /// Events projection runner
    events: EventsProjectionRunner,
    /// Available seats projection runner
    available_seats: AvailableSeatsProjectionRunner,
    /// Payments projection runner
    payments: PaymentsProjectionRunner,
    /// Reservations projection runner
    reservations: ReservationsProjectionRunner,
    /// Shared shutdown signal (dropped when struct is dropped, signaling shutdown)
    #[allow(dead_code)]
    shutdown: watch::Sender<bool>,
}

impl ProjectionManagers {
    /// Start all projection consumers.
    ///
    /// Spawns each consumer in its own tokio task.
    /// Consumers run until shutdown signal is received.
    #[must_use]
    pub fn start_all(self) -> Vec<tokio::task::JoinHandle<()>> {
        let mut handles = Vec::new();

        // Start events projection
        let events = self.events;
        handles.push(tokio::spawn(async move {
            events.run().await;
        }));

        // Start available seats projection
        let available_seats = self.available_seats;
        handles.push(tokio::spawn(async move {
            available_seats.run().await;
        }));

        // Start payments projection
        let payments = self.payments;
        handles.push(tokio::spawn(async move {
            payments.run().await;
        }));

        // Start reservations projection
        let reservations = self.reservations;
        handles.push(tokio::spawn(async move {
            reservations.run().await;
        }));

        handles
    }
}
