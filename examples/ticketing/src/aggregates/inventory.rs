//! Inventory aggregate for the Event Ticketing System.
//!
//! Manages seat availability and reservations. This aggregate is CRITICAL for preventing
//! double-booking in high-concurrency scenarios (the "last seat" problem).
//!
//! **Concurrency Strategy**: Optimistic concurrency control - check available seats including
//! reserved count to prevent overselling during concurrent reservation attempts.

use crate::projections::{EventProjectionQuery, TicketingEvent};
use crate::types::{
    Capacity, CustomerId, EventId, GlobalActionChannels, Inventory, InventoryState, Money, PricingTier,
    ReservationId, ResponseChannel, SeatAssignment, SeatId, SeatNumber, SeatStatus,
};
use chrono::{DateTime, Utc};
use composable_rust_core::{
    append_events, delay, effect::Effect, environment::Clock,
    event_store::EventStore, reducer::Reducer, smallvec,
    stream::{StreamId, Version},
    SmallVec,
};
use composable_rust_macros::Action;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ============================================================================
// Data Structures
// ============================================================================

/// Section availability data for query results.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SectionAvailabilityData {
    /// Section identifier
    pub section: String,
    /// Total capacity
    pub total_capacity: u32,
    /// Currently reserved seats (pending payment)
    pub reserved: u32,
    /// Sold seats (payment confirmed)
    pub sold: u32,
    /// Available seats (total - reserved - sold)
    pub available: u32,
}

// ============================================================================
// Projection Query Trait
// ============================================================================

/// Trait for querying inventory projection data.
///
/// This trait defines the read operations needed by the Inventory aggregate
/// to load state from the projection when processing commands.
///
/// # Pattern: State Loading from Projections
///
/// According to the state-loading-patterns spec, aggregates load state on-demand
/// by querying projections. This trait is injected via the Environment to enable
/// the reducer to trigger state loading effects.
///
/// Note: Returns `BoxFuture` instead of async fn to be dyn-compatible (object-safe).
#[allow(clippy::type_complexity)] // Complex future types required for dyn-compatibility
pub trait InventoryProjectionQuery: Send + Sync {
    /// Load inventory data for a specific event and section.
    ///
    /// Returns (counts, `seat_assignments`) where counts is (`total_capacity`, reserved, sold, available).
    /// The seat assignments provide the complete denormalized snapshot of individual seats.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn load_inventory(
        &self,
        event_id: &EventId,
        section: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<((u32, u32, u32, u32), Vec<SeatAssignment>)>, String>> + Send + '_>>;

    /// Query all sections for an event.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn get_all_sections(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<SectionAvailabilityData>, String>> + Send + '_>>;

    /// Query availability for a specific section.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn get_section_availability(
        &self,
        event_id: &EventId,
        section: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<SectionAvailabilityData>, String>> + Send + '_>>;

    /// Query total available seats across all sections for an event.
    ///
    /// # Errors
    ///
    /// Returns error if database query fails.
    fn get_total_available(
        &self,
        event_id: &EventId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u32, String>> + Send + '_>>;
}

// ============================================================================
// Actions (Commands + Events)
// ============================================================================

/// Actions for the Inventory aggregate
///
/// Handles seat reservation, release, and confirmation with atomic operations
/// to prevent double-booking.
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum InventoryAction {
    // Commands
    /// Initialize inventory for an event section
    #[command]
    InitializeInventory {
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// Total capacity
        capacity: Capacity,
        /// Optional specific seat numbers (None for general admission)
        seat_numbers: Option<Vec<SeatNumber>>,

        /// Response channel for projection completion
        #[serde(skip)]
        respond_to: ResponseChannel,
    },

    /// Reserve seats for a reservation
    #[command]
    ReserveSeats {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// Number of seats to reserve
        quantity: u32,
        /// Optional specific seat numbers
        specific_seats: Option<Vec<SeatNumber>>,
        /// When the reservation expires
        expires_at: DateTime<Utc>,
    },

    /// Confirm reservation (mark seats as sold)
    #[command]
    ConfirmReservation {
        /// Reservation to confirm
        reservation_id: ReservationId,
        /// Customer purchasing the seats
        customer_id: CustomerId,
    },

    /// Release reservation (return seats to available pool)
    #[command]
    ReleaseReservation {
        /// Reservation to release
        reservation_id: ReservationId,
    },

    /// Expire a reservation (timeout reached)
    #[command]
    ExpireReservation {
        /// Reservation to expire
        reservation_id: ReservationId,
    },

    /// Query all sections for an event
    #[command]
    GetAllSections {
        /// Event ID to query
        event_id: EventId,
    },

    /// Query availability for a specific section
    #[command]
    GetSectionAvailability {
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
    },

    /// Query total available seats across all sections for an event
    #[command]
    GetTotalAvailable {
        /// Event ID to query
        event_id: EventId,
    },

    // Events
    /// Inventory was initialized
    #[event]
    InventoryInitialized {
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// Capacity
        capacity: Capacity,
        /// Created seat IDs
        seats: Vec<SeatId>,
        /// When initialized
        initialized_at: DateTime<Utc>,

        /// Response channel for projection completion signaling
        #[serde(skip)]
        respond_to: ResponseChannel,
    },

    /// Seats were reserved
    #[event]
    SeatsReserved {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Event ID
        event_id: EventId,
        /// Section name
        section: String,
        /// Reserved seat IDs
        seats: Vec<SeatId>,
        /// Expiration time
        expires_at: DateTime<Utc>,
        /// When reserved
        reserved_at: DateTime<Utc>,
    },

    /// Seats were confirmed (sold)
    #[event]
    SeatsConfirmed {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Event ID (needed for projection rebuilding)
        event_id: EventId,
        /// Section name (needed for projection rebuilding)
        section: String,
        /// Customer ID
        customer_id: CustomerId,
        /// Confirmed seat IDs
        seats: Vec<SeatId>,
        /// When confirmed
        confirmed_at: DateTime<Utc>,
    },

    /// Seats were released back to available pool
    #[event]
    SeatsReleased {
        /// Reservation ID
        reservation_id: ReservationId,
        /// Event ID (needed for projection rebuilding)
        event_id: EventId,
        /// Section name (needed for projection rebuilding)
        section: String,
        /// Released seat IDs
        seats: Vec<SeatId>,
        /// When released
        released_at: DateTime<Utc>,
    },

    /// Insufficient inventory (concurrency - someone else got the last seats)
    #[event]
    InsufficientInventory {
        /// Event ID
        event_id: EventId,
        /// Section
        section: String,
        /// Requested quantity
        requested: u32,
        /// Actually available
        available: u32,
    },

    /// Validation failed
    #[event]
    ValidationFailed {
        /// Error message
        error: String,
    },

    /// Serialization failed
    #[event]
    SerializationFailed {
        /// Error message
        error: String,
    },

    /// Inventory state loaded from projection
    #[event]
    InventoryStateLoaded {
        /// Event ID
        event_id: EventId,
        /// Section
        section: String,
        /// Loaded inventory data (total, available, reserved, sold)
        inventory_data: Option<(u32, u32, u32, u32)>,
        /// Loaded seat assignments from projection (complete snapshot)
        seat_assignments: Vec<SeatAssignment>,
        /// Current stream version from event store (for optimistic concurrency)
        stream_version: Version,
    },

    /// Pricing queried from Event aggregate
    #[event]
    PricingQueried {
        /// Event ID
        event_id: EventId,
        /// Section
        section: String,
        /// Price per seat in cents (None if no pricing configured)
        price_per_seat: Option<u64>,
        /// Reservation ID this pricing is for
        reservation_id: ReservationId,
        /// Quantity
        quantity: u32,
        /// Specific seats requested (if any)
        specific_seats: Option<Vec<SeatNumber>>,
        /// Expiration time
        expires_at: DateTime<Utc>,
    },

    /// All sections were queried (query result)
    #[event]
    AllSectionsQueried {
        /// Event ID that was queried
        event_id: EventId,
        /// Section availability data
        sections: Vec<SectionAvailabilityData>,
    },

    /// Section availability was queried (query result)
    #[event]
    SectionAvailabilityQueried {
        /// Event ID that was queried
        event_id: EventId,
        /// Section that was queried
        section: String,
        /// Availability data (None if section not found)
        data: Option<SectionAvailabilityData>,
    },

    /// Total available seats were queried (query result)
    #[event]
    TotalAvailableQueried {
        /// Event ID that was queried
        event_id: EventId,
        /// Total available seats across all sections
        total_available: u32,
    },

    /// Stream version was updated after successful event append
    #[event]
    VersionUpdated {
        /// New version number
        version: Version,
    },

    /// Projection update confirmed
    #[event]
    InventoryProjectionConfirmed {
        /// Event ID
        event_id: EventId,
        /// Section
        section: String,
    },

    /// Projection update failed
    #[event]
    InventoryProjectionFailed {
        /// Event ID
        event_id: EventId,
        /// Section
        section: String,
        /// Failure reason
        reason: String,
    },
}

// ============================================================================
// Environment
// ============================================================================

/// Environment dependencies for the Inventory aggregate
#[derive(Clone)]
pub struct InventoryEnvironment {
    /// Clock for timestamps
    pub clock: Arc<dyn Clock>,
    /// Event store for persistence
    pub event_store: Arc<dyn EventStore>,
    /// Stream ID for this aggregate instance
    pub stream_id: StreamId,
    /// Projection query for loading inventory state on-demand
    pub inventory_projection: Arc<dyn InventoryProjectionQuery>,
    /// Event projection for pricing queries
    pub event_projection: Arc<dyn EventProjectionQuery>,
    /// Global action channels for cross-aggregate coordination
    pub global_actions: GlobalActionChannels,
}

impl InventoryEnvironment {
    /// Creates a new `InventoryEnvironment`
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        stream_id: StreamId,
        inventory_projection: Arc<dyn InventoryProjectionQuery>,
        event_projection: Arc<dyn EventProjectionQuery>,
        global_actions: GlobalActionChannels,
    ) -> Self {
        Self {
            clock,
            event_store,
            stream_id,
            inventory_projection,
            event_projection,
            global_actions,
        }
    }
}

// ============================================================================
// Reducer
// ============================================================================

/// Reducer for the Inventory aggregate
///
/// CRITICAL: This reducer implements atomic seat reservation to prevent double-booking.
/// The key is checking `reserved + sold` against capacity, NOT just `sold`.
#[derive(Clone, Debug)]
pub struct InventoryReducer;

impl InventoryReducer {
    /// Creates a new `InventoryReducer`
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Calculate price per seat from Event's pricing tiers.
    ///
    /// Selects the appropriate pricing tier based on:
    /// - Section match
    /// - Current time (tier availability window)
    ///
    /// Pricing tiers have time-based availability (`EarlyBird`, `Regular`, `LastMinute`).
    /// Returns the `base_price` from the first matching tier.
    ///
    /// # Returns
    ///
    /// Price in cents, or None if no matching tier found
    fn calculate_price_from_tiers(
        pricing_tiers: &[PricingTier],
        section: &str,
        now: DateTime<Utc>,
    ) -> Option<u64> {
        // Find first tier that matches section and is currently available
        pricing_tiers
            .iter()
            .find(|tier| {
                tier.section == section
                    && tier.available_from <= now
                    && tier.available_until.is_none_or(|until| now <= until)
            })
            .map(|tier| tier.base_price.cents())
    }

    /// Fallback pricing for when Event has no configured pricing tiers.
    ///
    /// This is a simplified pricing model for backwards compatibility.
    /// Used only when Event projection returns None or has empty `pricing_tiers`.
    ///
    /// # Pricing Logic
    ///
    /// - Sections containing "VIP" or "Premium": $10,000 cents ($100)
    /// - Sections containing "General": $3,000 cents ($30)
    /// - All other sections (Regular): $5,000 cents ($50)
    ///
    /// # Returns
    ///
    /// Price in cents
    fn fallback_section_price(section: &str) -> u64 {
        let section_lower = section.to_lowercase();

        if section_lower.contains("vip") || section_lower.contains("premium") {
            10_000 // $100 per seat
        } else if section_lower.contains("general") {
            3_000 // $30 per seat
        } else {
            5_000 // $50 per seat (default for regular sections)
        }
    }

    /// Creates effects for persisting events (`PostgreSQL` only, no Redpanda)
    ///
    /// With direct orchestration, we use local channels for coordination,
    /// so Redpanda publishing is no longer needed.
    fn create_effects(
        event: InventoryAction,
        expected_version: Version,
        env: &InventoryEnvironment,
    ) -> SmallVec<[Effect<InventoryAction>; 4]> {
        let ticketing_event = TicketingEvent::Inventory(event.clone());
        let serialized = match ticketing_event.serialize() {
            Ok(s) => s,
            Err(e) => {
                return smallvec![Effect::Future(Box::pin(async move {
                    Some(InventoryAction::SerializationFailed {
                        error: format!("Failed to serialize event: {e}"),
                    })
                }))];
            }
        };

        smallvec![
            append_events! {
                store: env.event_store,
                stream: env.stream_id.as_str(),
                expected_version: Some(expected_version),
                events: vec![serialized],
                on_success: |version| Some(InventoryAction::VersionUpdated { version }),
                on_error: |error| Some(InventoryAction::ValidationFailed {
                    error: error.to_string()
                })
            },
            // Echo the event back as an action so it broadcasts to action_broadcast channel
            // This allows send_and_wait_for to receive it (e.g., SeatsConfirmed, SeatsReleased)
            Effect::Future(Box::pin(async move {
                Some(event)
            }))
        ]
    }

    /// Validates `InitializeInventory` command
    fn validate_initialize_inventory(
        state: &InventoryState,
        event_id: &EventId,
        section: &str,
        capacity: Capacity,
    ) -> Result<(), String> {
        // Check if inventory already exists for this event/section
        if state
            .get_inventory(event_id, section)
            .is_some()
        {
            return Err(format!(
                "Inventory for event {event_id}, section '{section}' already exists"
            ));
        }

        // Capacity must be > 0
        if capacity.value() == 0 {
            return Err("Capacity must be greater than zero".to_string());
        }

        Ok(())
    }

    /// Validates `ReserveSeats` command
    ///
    /// CRITICAL: This is where we prevent double-booking.
    fn validate_reserve_seats(
        state: &InventoryState,
        event_id: &EventId,
        section: &str,
        quantity: u32,
    ) -> Result<(), String> {
        // Quantity must be > 0 and <= 8 (max purchase)
        if quantity == 0 {
            return Err("Quantity must be greater than zero".to_string());
        }

        if quantity > 8 {
            return Err(format!(
                "Cannot reserve more than 8 seats at once (requested: {quantity})"
            ));
        }

        // Inventory must exist
        let Some(inventory) = state.get_inventory(event_id, section) else {
            return Err(format!(
                "Inventory for event {event_id}, section '{section}' not found"
            ));
        };

        // CRITICAL: Check actual availability (including reserved seats)
        let actually_available = inventory.available();

        if actually_available < quantity {
            return Err(format!(
                "Insufficient inventory: requested {quantity}, available {actually_available}"
            ));
        }

        Ok(())
    }

    /// Selects available seats for reservation
    ///
    /// For general admission, picks the first N available seats.
    fn select_available_seats(
        state: &InventoryState,
        event_id: &EventId,
        section: &str,
        quantity: u32,
    ) -> Vec<SeatId> {
        // IMPORTANT: Sort seats by ID to ensure deterministic selection
        // HashMap iteration order is non-deterministic, which would cause
        // different seats to be selected during event replay
        let mut available: Vec<SeatId> = state
            .seat_assignments
            .values()
            .filter(|seat| {
                seat.event_id == *event_id
                    && seat.section == *section
                    && seat.status == SeatStatus::Available
            })
            .map(|seat| seat.seat_id)
            .collect();

        // Sort to ensure consistent ordering
        available.sort();

        // Take only the requested quantity
        available.into_iter().take(quantity as usize).collect()
    }

    /// Select specific seats by seat numbers.
    ///
    /// # Errors
    ///
    /// Returns error message if:
    /// - Any requested seat doesn't exist
    /// - Any seat doesn't belong to the event/section
    /// - Any seat is not available
    fn select_specific_seats(
        state: &InventoryState,
        event_id: &EventId,
        section: &str,
        seat_numbers: &[SeatNumber],
    ) -> Result<Vec<SeatId>, String> {
        let mut seat_ids = Vec::with_capacity(seat_numbers.len());

        for seat_number in seat_numbers {
            // Find seat by number in the requested event and section
            let seat = state
                .seat_assignments
                .values()
                .find(|seat| {
                    seat.event_id == *event_id
                        && seat.section == *section
                        && seat.seat_number.as_ref() == Some(seat_number)
                });

            let Some(seat) = seat else {
                return Err(format!(
                    "Seat {} not found in section {} for this event",
                    seat_number.as_str(),
                    section
                ));
            };

            // Verify seat is available
            if seat.status != SeatStatus::Available {
                return Err(format!(
                    "Seat {} is not available (status: {:?})",
                    seat_number.as_str(),
                    seat.status
                ));
            }

            seat_ids.push(seat.seat_id);
        }

        Ok(seat_ids)
    }

    /// Finds seats by reservation ID
    fn find_seats_by_reservation(
        state: &InventoryState,
        reservation_id: &ReservationId,
    ) -> Vec<SeatId> {
        state
            .seat_assignments
            .values()
            .filter(|seat| seat.reserved_by == Some(*reservation_id))
            .map(|seat| seat.seat_id)
            .collect()
    }

    /// Finds `event_id` and section for a reservation
    /// Returns None if reservation not found
    fn find_reservation_location(
        state: &InventoryState,
        reservation_id: &ReservationId,
    ) -> Option<(EventId, String)> {
        state
            .seat_assignments
            .values()
            .find(|seat| seat.reserved_by == Some(*reservation_id))
            .map(|seat| (seat.event_id, seat.section.clone()))
    }

    /// Handles releasing seats back to the available pool.
    ///
    /// Used by both `ReleaseReservation` (manual release) and `ExpireReservation` (timeout).
    /// Returns empty effects if reservation not found (idempotent behavior).
    fn handle_release_seats(
        state: &mut InventoryState,
        reservation_id: ReservationId,
        env: &InventoryEnvironment,
    ) -> SmallVec<[Effect<InventoryAction>; 4]> {
        // Find seats for this reservation
        let seats = Self::find_seats_by_reservation(state, &reservation_id);

        if seats.is_empty() {
            // Silently ignore - reservation might have already been released
            return SmallVec::new();
        }

        // Find event_id and section for this reservation
        let Some((event_id, section)) = Self::find_reservation_location(state, &reservation_id)
        else {
            // Silently ignore - reservation might have already been released
            return SmallVec::new();
        };

        // Create and apply event
        let event = InventoryAction::SeatsReleased {
            reservation_id,
            event_id,
            section,
            seats,
            released_at: env.clock.now(),
        };
        let expected_version = state.version;
        Self::apply_event(state, &event);

        Self::create_effects(event, expected_version, env)
    }

    /// Applies an event to state
    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // Complex state management required
    fn apply_event(state: &mut InventoryState, action: &InventoryAction) {
        match action {
            InventoryAction::InventoryInitialized {
                event_id,
                section,
                capacity,
                seats,
                ..
            } => {
                // Create inventory record
                let inventory = Inventory::new(*event_id, section.clone(), *capacity);
                state
                    .inventories
                    .insert((*event_id, section.clone()), inventory);

                // Create seat assignments
                for seat_id in seats {
                    let assignment = SeatAssignment::new(
                        *seat_id,
                        *event_id,
                        section.clone(),
                        None, // General admission (no specific seat numbers for now)
                    );
                    state.seat_assignments.insert(*seat_id, assignment);
                }

                state.last_error = None;
            }

            InventoryAction::SeatsReserved {
                reservation_id,
                event_id,
                section,
                seats,
                expires_at,
                ..
            } => {
                // Idempotent: Count only seats not already reserved by this reservation
                let mut newly_reserved_count = 0u32;
                for seat_id in seats {
                    if let Some(seat) = state.seat_assignments.get_mut(seat_id) {
                        // Only count and mark if not already reserved by this reservation
                        if seat.reserved_by != Some(*reservation_id) {
                            newly_reserved_count += 1;
                            seat.status = SeatStatus::Reserved {
                                expires_at: *expires_at,
                            };
                            seat.reserved_by = Some(*reservation_id);
                        }
                    }
                }

                // Update inventory reserved count only for newly reserved seats
                let key = (*event_id, section.clone());
                if let Some(inventory) = state.inventories.get_mut(&key) {
                    inventory.reserved += newly_reserved_count;
                }

                state.last_error = None;
            }

            InventoryAction::SeatsConfirmed {
                customer_id, seats, ..
            } => {
                // Find which inventory this belongs to
                if let Some(first_seat) = seats.first() {
                    if let Some(seat_assignment) = state.seat_assignments.get(first_seat) {
                        let key = (seat_assignment.event_id, seat_assignment.section.clone());
                        if let Some(inventory) = state.inventories.get_mut(&key) {
                            // Move from reserved to sold
                            #[allow(clippy::cast_possible_truncation)]
                            {
                                inventory.reserved = inventory.reserved.saturating_sub(seats.len() as u32);
                                inventory.sold += seats.len() as u32;
                            }
                        }
                    }
                }

                // Mark seats as sold
                for seat_id in seats {
                    if let Some(seat) = state.seat_assignments.get_mut(seat_id) {
                        seat.status = SeatStatus::Sold;
                        seat.sold_to = Some(*customer_id);
                        seat.reserved_by = None;
                    }
                }

                state.last_error = None;
            }

            InventoryAction::SeatsReleased { seats, .. } => {
                // Find which inventory this belongs to
                if let Some(first_seat) = seats.first() {
                    if let Some(seat_assignment) = state.seat_assignments.get(first_seat) {
                        let key = (seat_assignment.event_id, seat_assignment.section.clone());
                        if let Some(inventory) = state.inventories.get_mut(&key) {
                            // Return from reserved to available
                            #[allow(clippy::cast_possible_truncation)]
                            {
                                inventory.reserved = inventory.reserved.saturating_sub(seats.len() as u32);
                            }
                        }
                    }
                }

                // Mark seats as available
                for seat_id in seats {
                    if let Some(seat) = state.seat_assignments.get_mut(seat_id) {
                        seat.status = SeatStatus::Available;
                        seat.reserved_by = None;
                    }
                }

                state.last_error = None;
            }

            InventoryAction::ValidationFailed { error }
            | InventoryAction::SerializationFailed { error } => {
                state.last_error = Some(error.clone());
            }

            InventoryAction::InventoryStateLoaded {
                event_id,
                section,
                inventory_data,
                seat_assignments,
                stream_version,
            } => {
                tracing::debug!(
                    "InventoryStateLoaded: event_id={}, section={}, data={:?}, seats={}, stream_version={:?}",
                    event_id,
                    section,
                    inventory_data,
                    seat_assignments.len(),
                    stream_version
                );

                // Mark as loaded
                state.mark_loaded(*event_id, section.clone());

                // **Critical**: Set the stream version for optimistic concurrency
                // This ensures subsequent append operations use the correct expected version
                state.version = *stream_version;

                // If data was found in projection, reconstruct the inventory
                // Note: projection returns (total_capacity, reserved, sold, available) + seat assignments
                if let Some((total, reserved, sold, _available)) = inventory_data {
                    tracing::debug!(
                        "Reconstructing inventory from projection snapshot: total={}, reserved={}, sold={}, seats={}",
                        total,
                        reserved,
                        sold,
                        seat_assignments.len()
                    );

                    let mut inventory = Inventory::new(*event_id, section.clone(), Capacity::new(*total));
                    // Note: 'available' is derived (total - reserved - sold), not stored
                    inventory.reserved = *reserved;
                    inventory.sold = *sold;

                    state.inventories.insert((*event_id, section.clone()), inventory);

                    // Load seat assignments from projection snapshot (no more placeholder generation!)
                    for assignment in seat_assignments {
                        state.seat_assignments.insert(assignment.seat_id, assignment.clone());
                    }

                    tracing::debug!(
                        "Inventory loaded from projection snapshot. State now has {} inventories and {} seat assignments, version={}",
                        state.inventories.len(),
                        state.seat_assignments.len(),
                        u64::from(state.version)
                    );
                } else {
                    tracing::warn!(
                        "No inventory data found in projection for event_id={}, section={}",
                        event_id,
                        section
                    );
                }

                state.last_error = None;
            }

            InventoryAction::VersionUpdated { version } => {
                state.version = *version;
            }

            InventoryAction::PricingQueried {
                event_id,
                section,
                price_per_seat,
                ..
            } => {
                tracing::debug!(
                    "PricingQueried: event_id={}, section={}, price_per_seat={:?}",
                    event_id,
                    section,
                    price_per_seat
                );

                // Cache the pricing for this (event_id, section)
                if let Some(price) = price_per_seat {
                    state.pricing_cache.insert((*event_id, section.clone()), *price);
                    tracing::debug!("Cached pricing: {} cents for section {}", price, section);
                } else {
                    tracing::warn!(
                        "No pricing configured for event {} section {}. Will use fallback pricing.",
                        event_id,
                        section
                    );
                }
            }

            // Commands and informational events don't modify state
            InventoryAction::InsufficientInventory { .. }
            | InventoryAction::InitializeInventory { .. }
            | InventoryAction::ReserveSeats { .. }
            | InventoryAction::ConfirmReservation { .. }
            | InventoryAction::ReleaseReservation { .. }
            | InventoryAction::ExpireReservation { .. }
            | InventoryAction::GetAllSections { .. }
            | InventoryAction::GetSectionAvailability { .. }
            | InventoryAction::GetTotalAvailable { .. }
            | InventoryAction::AllSectionsQueried { .. }
            | InventoryAction::SectionAvailabilityQueried { .. }
            | InventoryAction::TotalAvailableQueried { .. }
            | InventoryAction::InventoryProjectionConfirmed { .. }
            | InventoryAction::InventoryProjectionFailed { .. } => {
                // Commands don't modify state
                // Query actions and results are handled in reducer
                // InsufficientInventory is informational - no state change needed
                // Projection confirmation actions are logged but don't modify aggregate state
                // Don't clear last_error - this represents a failure condition
            }
        }
    }
}

impl Default for InventoryReducer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer for InventoryReducer {
    type State = InventoryState;
    type Action = InventoryAction;
    type Environment = InventoryEnvironment;

    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // Complex business logic required
    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // ========== Commands ==========
            InventoryAction::InitializeInventory {
                event_id,
                section,
                capacity,
                seat_numbers: _,
                respond_to: _,
            } => {
                // Validate
                if let Err(error) =
                    Self::validate_initialize_inventory(state, &event_id, &section, capacity)
                {
                    Self::apply_event(state, &InventoryAction::ValidationFailed { error });
                    return SmallVec::new();
                }

                // Create seat IDs
                let seat_count = capacity.value();
                let seats: Vec<SeatId> = (0..seat_count).map(|_| SeatId::new()).collect();

                // Capture values for closures BEFORE creating the event
                let initialized_at = env.clock.now();
                let seats_for_channel = seats.clone();
                let event_id_for_channel = event_id;
                let section_for_channel = section.clone();
                let capacity_for_channel = capacity;
                let initialized_at_for_channel = initialized_at;
                let event_id_for_success = event_id;
                let section_for_success = section.clone();
                let event_id_for_error = event_id;
                let section_for_error = section.clone();

                // Create and apply event
                let event = InventoryAction::InventoryInitialized {
                    event_id,
                    section: section.clone(),
                    capacity,
                    seats,
                    initialized_at,
                    respond_to: crate::types::ResponseChannel::none(),
                };
                let expected_version = state.version;
                Self::apply_event(state, &event);

                // Clone event for feedback - this is the domain event we'll return on success
                let event_for_feedback = event.clone();

                // Serialize event
                let ticketing_event = TicketingEvent::Inventory(event);
                let serialized = match ticketing_event.serialize() {
                    Ok(s) => s,
                    Err(e) => {
                        Self::apply_event(
                            state,
                            &InventoryAction::SerializationFailed {
                                error: format!("Failed to serialize event: {e}"),
                            },
                        );
                        return SmallVec::new();
                    }
                };

                // Create base effects for persistence (no Redpanda)
                // On success, return the domain event (InventoryInitialized) not a technical action
                let mut effects = smallvec![
                    append_events! {
                        store: env.event_store,
                        stream: env.stream_id.as_str(),
                        expected_version: Some(expected_version),
                        events: vec![serialized],
                        on_success: |_version| Some(event_for_feedback.clone()),
                        on_error: |error| Some(InventoryAction::ValidationFailed {
                            error: error.to_string()
                        })
                    }
                ];

                // Publish the domain EVENT to global channel and wait for projection completion
                // Uses Effect::PublishWithResponse to ensure synchronous handling
                effects.push(Effect::PublishWithResponse {
                    channel: env.global_actions.inventory_actions.clone(),
                    create_action: Box::new(move |respond_to| InventoryAction::InventoryInitialized {
                        event_id: event_id_for_channel,
                        section: section_for_channel,
                        capacity: capacity_for_channel,
                        seats: seats_for_channel,
                        initialized_at: initialized_at_for_channel,
                        respond_to,
                    }),
                    on_success: Box::new(move || {
                        Some(InventoryAction::InventoryProjectionConfirmed {
                            event_id: event_id_for_success,
                            section: section_for_success.clone(),
                        })
                    }),
                    on_error: Box::new(move |reason| {
                        Some(InventoryAction::InventoryProjectionFailed {
                            event_id: event_id_for_error,
                            section: section_for_error.clone(),
                            reason,
                        })
                    }),
                });

                effects
            }

            InventoryAction::ReserveSeats {
                reservation_id,
                event_id,
                section,
                quantity,
                specific_seats,
                expires_at,
            } => {
                tracing::debug!(
                    "ReserveSeats: reservation_id={}, event_id={}, section={}, quantity={}, state.inventories.len()={}",
                    reservation_id,
                    event_id,
                    section,
                    quantity,
                    state.inventories.len()
                );

                // ===== EARLY SYNC VALIDATION (fail fast before async load) =====
                // These checks don't require state to be loaded

                // Validate quantity is positive
                if quantity == 0 {
                    let error = "Cannot reserve 0 seats".to_string();
                    tracing::warn!("Early validation failed: {}", error);
                    Self::apply_event(
                        state,
                        &InventoryAction::ValidationFailed { error: error.clone() },
                    );
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(InventoryAction::ValidationFailed { error })
                    }))];
                }

                // Validate specific_seats length matches quantity (if provided)
                if let Some(ref seat_numbers) = specific_seats {
                    if seat_numbers.len() != quantity as usize {
                        let error = format!(
                            "Quantity mismatch: requested {} seats but provided {} specific seat numbers",
                            quantity,
                            seat_numbers.len()
                        );
                        tracing::warn!("Early validation failed: {}", error);
                        Self::apply_event(
                            state,
                            &InventoryAction::ValidationFailed { error: error.clone() },
                        );
                        return smallvec![Effect::Future(Box::pin(async move {
                            Some(InventoryAction::ValidationFailed { error })
                        }))];
                    }
                }

                // ===== END EARLY SYNC VALIDATION =====

                // Check if state has been loaded from projection
                if !state.is_loaded(&event_id, &section) {
                    tracing::debug!(
                        "State not loaded for event_id={}, section={}. Triggering parallel load of inventory + pricing.",
                        event_id,
                        section
                    );

                    // Mark as loading to prevent duplicate load requests
                    state.mark_loading(event_id, section.clone());

                    // Clone dependencies for async closures
                    let inventory_projection = env.inventory_projection.clone();
                    let event_projection = env.event_projection.clone();
                    let event_store = env.event_store.clone();
                    let clock = env.clock.clone();
                    let event_id_copy = event_id;
                    let section_copy = section.clone();
                    let section_copy2 = section.clone();
                    let specific_seats_copy = specific_seats.clone();
                    // Build stream_id for querying event store version
                    let stream_id = StreamId::new(format!("inventory-{}", event_id.as_uuid()));
                    let original_command = InventoryAction::ReserveSeats {
                        reservation_id,
                        event_id,
                        section: section.clone(),
                        quantity,
                        specific_seats,
                        expires_at,
                    };

                    // Use Sequential: 1) Parallel load (inventory + pricing), 2) retry original command
                    return smallvec![Effect::Sequential(vec![
                        // Load inventory state and pricing in parallel
                        Effect::Parallel(vec![
                            Effect::Future(Box::pin(async move {
                                // Load inventory data from projection
                                let result = inventory_projection
                                    .load_inventory(&event_id_copy, &section_copy)
                                    .await
                                    .ok()
                                    .flatten();

                                // Destructure into counts and seat assignments
                                let (inventory_data, seat_assignments) = match result {
                                    Some((counts, seats)) => (Some(counts), seats),
                                    None => (None, Vec::new()),
                                };

                                // Query event store for current stream version
                                // This is critical for optimistic concurrency control
                                let stream_version = match event_store.get_stream_version(stream_id).await {
                                    Ok(version) => version,
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to get stream version: {}. Using version 0.",
                                            e
                                        );
                                        Version::new(0)
                                    }
                                };

                                // Return StateLoaded event with complete snapshot
                                Some(InventoryAction::InventoryStateLoaded {
                                    event_id: event_id_copy,
                                    section: section_copy,
                                    inventory_data,
                                    seat_assignments,
                                    stream_version,
                                })
                            })),
                            Effect::Future(Box::pin(async move {
                                // Load Event to get pricing configuration
                                let event_result = event_projection
                                    .load_event(&event_id_copy)
                                    .await
                                    .ok()
                                    .flatten();

                                // Calculate price per seat based on Event's pricing tiers
                                let price_per_seat = event_result.and_then(|event| {
                                    Self::calculate_price_from_tiers(
                                        &event.pricing_tiers,
                                        &section_copy2,
                                        clock.now(),
                                    )
                                });

                                // Return pricing queried event
                                Some(InventoryAction::PricingQueried {
                                    event_id: event_id_copy,
                                    section: section_copy2,
                                    price_per_seat,
                                    reservation_id,
                                    quantity,
                                    specific_seats: specific_seats_copy,
                                    expires_at,
                                })
                            })),
                        ]),
                        // After both loads complete, retry the original command
                        Effect::Future(Box::pin(async move {
                            Some(original_command)
                        })),
                    ])];
                }

                tracing::debug!(
                    "State already loaded. Proceeding with validation. Has inventory: {}",
                    state.get_inventory(&event_id, &section).is_some()
                );

                // Note: specific_seats length validation moved to early sync validation phase
                // (before async load) to fail fast

                // Validate
                if let Err(error) =
                    Self::validate_reserve_seats(state, &event_id, &section, quantity)
                {
                    tracing::warn!(
                        "Validation failed for ReserveSeats: {}",
                        error
                    );

                    // Apply ValidationFailed event to state
                    let validation_failed = InventoryAction::ValidationFailed {
                        error: error.clone(),
                    };
                    Self::apply_event(state, &validation_failed);

                    // Also apply InsufficientInventory for saga coordination
                    if error.contains("Insufficient inventory") {
                        if let Some(inventory) = state.get_inventory(&event_id, &section) {
                            let event = InventoryAction::InsufficientInventory {
                                event_id,
                                section,
                                requested: quantity,
                                available: inventory.available(),
                            };
                            Self::apply_event(state, &event);
                        }
                    }

                    // Return effect that broadcasts ValidationFailed for send_and_wait_for
                    return smallvec![Effect::Future(Box::pin(async move {
                        Some(validation_failed)
                    }))];
                }

                tracing::debug!(
                    "Validation passed. Creating SeatsReserved event."
                );

                // Select seats (specific or automatic)
                let seats = if let Some(ref seat_numbers) = specific_seats {
                    // User requested specific seats - validate and reserve them
                    match Self::select_specific_seats(state, &event_id, &section, seat_numbers) {
                        Ok(seat_ids) => seat_ids,
                        Err(error) => {
                            // Specific seat selection failed - return validation error
                            let validation_failed = InventoryAction::ValidationFailed {
                                error: error.clone(),
                            };
                            Self::apply_event(state, &validation_failed);
                            tracing::warn!("Specific seat selection failed: {}", error);
                            // Return effect that broadcasts ValidationFailed for send_and_wait_for
                            return smallvec![Effect::Future(Box::pin(async move {
                                Some(validation_failed)
                            }))];
                        }
                    }
                } else {
                    // Automatic seat selection - pick any available seats
                    Self::select_available_seats(state, &event_id, &section, quantity)
                };

                // Clone seats for later use in reservation response
                let seats_for_response = seats.clone();

                // Create and apply event
                let event = InventoryAction::SeatsReserved {
                    reservation_id,
                    event_id,
                    section: section.clone(),
                    seats,
                    expires_at,
                    reserved_at: env.clock.now(),
                };
                let expected_version = state.version;
                Self::apply_event(state, &event);

                // Clone event for feedback - this is the domain event we'll return on success
                // so that send_and_wait_for can observe SeatsReserved
                let event_for_feedback = event.clone();

                // Serialize event
                let ticketing_event = TicketingEvent::Inventory(event);
                let serialized = match ticketing_event.serialize() {
                    Ok(s) => s,
                    Err(e) => {
                        Self::apply_event(
                            state,
                            &InventoryAction::SerializationFailed {
                                error: format!("Failed to serialize event: {e}"),
                            },
                        );
                        return SmallVec::new();
                    }
                };

                // Calculate timeout duration
                let now = env.clock.now();
                let timeout_duration = if expires_at > now {
                    let diff = expires_at - now;
                    #[allow(clippy::cast_sign_loss)]
                    std::time::Duration::from_secs(diff.num_seconds() as u64)
                } else {
                    std::time::Duration::from_secs(0)
                };

                // Calculate pricing: Use cached pricing from Event, fallback to simple logic
                let price_per_seat = state
                    .pricing_cache
                    .get(&(event_id, section.clone()))
                    .copied()
                    .unwrap_or_else(|| {
                        tracing::warn!(
                            "No cached pricing for event {} section {}. Using fallback pricing.",
                            event_id,
                            section
                        );
                        Self::fallback_section_price(&section)
                    });
                let total_amount = Money::from_cents(price_per_seat * u64::from(quantity));

                tracing::debug!(
                    "Calculated pricing: section={}, quantity={}, price_per_seat={}, total={}",
                    section,
                    quantity,
                    price_per_seat,
                    total_amount.dollars()
                );

                // Direct orchestration: Send response to reservation aggregate
                let reservation_response = crate::aggregates::ReservationAction::SeatsAllocated {
                    reservation_id,
                    seats: seats_for_response,
                    total_amount,
                };
                let reservation_channel = env.global_actions.reservation_actions.clone();

                // Return effects: persist, notify reservation, and schedule expiration (no Redpanda)
                smallvec![
                    append_events! {
                        store: env.event_store,
                        stream: env.stream_id.as_str(),
                        expected_version: Some(expected_version),
                        events: vec![serialized],
                        on_success: |_version| Some(event_for_feedback.clone()),
                        on_error: |error| Some(InventoryAction::ValidationFailed {
                            error: error.to_string()
                        })
                    },
                    Effect::Future(Box::pin(async move {
                        if let Err(e) = reservation_channel.send(reservation_response) {
                            tracing::error!(error = %e, "Failed to send SeatsAllocated to reservation channel");
                        }
                        None
                    })),
                    delay! {
                        duration: timeout_duration,
                        action: InventoryAction::ExpireReservation { reservation_id }
                    }
                ]
            }

            InventoryAction::ConfirmReservation {
                reservation_id,
                customer_id,
            } => {
                // Find seats for this reservation
                let seats = Self::find_seats_by_reservation(state, &reservation_id);

                if seats.is_empty() {
                    Self::apply_event(
                        state,
                        &InventoryAction::ValidationFailed {
                            error: format!("No seats found for reservation {reservation_id}"),
                        },
                    );
                    return SmallVec::new();
                }

                // Find event_id and section for this reservation
                let Some((event_id, section)) = Self::find_reservation_location(state, &reservation_id) else {
                    Self::apply_event(
                        state,
                        &InventoryAction::ValidationFailed {
                            error: format!("Could not find location for reservation {reservation_id}"),
                        },
                    );
                    return SmallVec::new();
                };

                // Create and apply event
                let event = InventoryAction::SeatsConfirmed {
                    reservation_id,
                    event_id,
                    section,
                    customer_id,
                    seats,
                    confirmed_at: env.clock.now(),
                };
                let expected_version = state.version;
                Self::apply_event(state, &event);

                Self::create_effects(event, expected_version, env)
            }

            InventoryAction::ReleaseReservation { reservation_id } => {
                Self::handle_release_seats(state, reservation_id, env)
            }

            InventoryAction::ExpireReservation { reservation_id } => {
                // Uses same logic as ReleaseReservation
                // In production, might add different analytics/metrics here
                Self::handle_release_seats(state, reservation_id, env)
            }

            // ========== Query Actions ==========
            InventoryAction::GetAllSections { event_id } => {
                let projection = env.inventory_projection.clone();
                let event_id_clone = event_id;
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.get_all_sections(&event_id_clone).await {
                        Ok(sections) => Some(InventoryAction::AllSectionsQueried {
                            event_id: event_id_clone,
                            sections,
                        }),
                        Err(e) => Some(InventoryAction::ValidationFailed {
                            error: format!("Failed to query sections: {e}"),
                        }),
                    }
                }))]
            }

            InventoryAction::GetSectionAvailability { event_id, section } => {
                let projection = env.inventory_projection.clone();
                let event_id_clone = event_id;
                let section_clone = section.clone();
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.get_section_availability(&event_id_clone, &section_clone).await {
                        Ok(data) => Some(InventoryAction::SectionAvailabilityQueried {
                            event_id: event_id_clone,
                            section: section_clone,
                            data,
                        }),
                        Err(e) => Some(InventoryAction::ValidationFailed {
                            error: format!("Failed to query section availability: {e}"),
                        }),
                    }
                }))]
            }

            InventoryAction::GetTotalAvailable { event_id } => {
                let projection = env.inventory_projection.clone();
                let event_id_clone = event_id;
                smallvec![Effect::Future(Box::pin(async move {
                    match projection.get_total_available(&event_id_clone).await {
                        Ok(total_available) => Some(InventoryAction::TotalAvailableQueried {
                            event_id: event_id_clone,
                            total_available,
                        }),
                        Err(e) => Some(InventoryAction::ValidationFailed {
                            error: format!("Failed to query total available: {e}"),
                        }),
                    }
                }))]
            }

            // ========== Events (from event store replay) ==========
            event => {
                Self::apply_event(state, &event);
                SmallVec::new()
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::test_utils::{
        create_test_global_channels, MockEventProjectionQuery, MockInventoryQuery,
    };
    use crate::types::TierType;
    use composable_rust_core::environment::SystemClock;
    use composable_rust_testing::{mocks::InMemoryEventStore, ReducerTest};

    fn create_test_env() -> InventoryEnvironment {
        InventoryEnvironment::new(
            Arc::new(SystemClock),
            Arc::new(InMemoryEventStore::new()),
            StreamId::new("inventory-test"),
            Arc::new(MockInventoryQuery),
            Arc::new(MockEventProjectionQuery),
            create_test_global_channels(),
        )
    }

    #[test]
    fn test_initialize_inventory() {
        let event_id = EventId::new();

        ReducerTest::new(InventoryReducer::new())
            .with_env(create_test_env())
            .given_state(InventoryState::new())
            .when_action(InventoryAction::InitializeInventory {
                event_id,
                section: "General".to_string(),
                capacity: Capacity::new(100),
                seat_numbers: None,
                respond_to: ResponseChannel::none(),
            })
            .then_state(move |state| {
                assert_eq!(state.count_inventories(), 1);
                let inventory = state.get_inventory(&event_id, "General").unwrap();
                assert_eq!(inventory.total_capacity.value(), 100);
                assert_eq!(inventory.available(), 100);
                assert_eq!(inventory.reserved, 0);
                assert_eq!(inventory.sold, 0);
                assert_eq!(state.count_seats(), 100);
            })
            .then_effects(|effects| {
                // Should return 2 effects: AppendEvents + Channel Send (no Redpanda)
                assert_eq!(effects.len(), 2);
            })
            .run();
    }

    #[test]
    fn test_reserve_seats_success() {
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();

        ReducerTest::new(InventoryReducer::new())
            .with_env(create_test_env())
            .given_state({
                // Initialize inventory first
                let mut state = InventoryState::new();
                let reducer = InventoryReducer::new();
                let env = create_test_env();
                reducer.reduce(
                    &mut state,
                    InventoryAction::InitializeInventory {
                        event_id,
                        section: "General".to_string(),
                        capacity: Capacity::new(100),
                        seat_numbers: None,
                        respond_to: ResponseChannel::none(),
                    },
                    &env,
                );
                // Mark state as loaded to avoid load-then-process flow
                state.mark_loaded(event_id, "General".to_string());
                state
            })
            .when_action(InventoryAction::ReserveSeats {
                reservation_id,
                event_id,
                section: "General".to_string(),
                quantity: 2,
                specific_seats: None,
                expires_at: Utc::now() + chrono::Duration::minutes(5),
            })
            .then_state(move |state| {
                let inventory = state.get_inventory(&event_id, "General").unwrap();
                assert_eq!(inventory.reserved, 2);
                assert_eq!(inventory.sold, 0);
                assert_eq!(inventory.available(), 98);
            })
            .then_effects(|effects| {
                // Should return 3 effects: AppendEvents + Channel Send + Delay (for expiration, no Redpanda)
                assert_eq!(effects.len(), 3);
            })
            .run();
    }

    #[test]
    fn test_reserve_seats_insufficient_inventory() {
        let event_id = EventId::new();

        ReducerTest::new(InventoryReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = InventoryState::new();
                let reducer = InventoryReducer::new();
                let env = create_test_env();
                // Initialize with only 5 seats
                reducer.reduce(
                    &mut state,
                    InventoryAction::InitializeInventory {
                        event_id,
                        section: "VIP".to_string(),
                        capacity: Capacity::new(5),
                        seat_numbers: None,
                        respond_to: ResponseChannel::none(),
                    },
                    &env,
                );
                // Mark state as loaded to avoid load-then-process flow
                state.mark_loaded(event_id, "VIP".to_string());
                state
            })
            .when_action(InventoryAction::ReserveSeats {
                reservation_id: ReservationId::new(),
                event_id,
                section: "VIP".to_string(),
                quantity: 10, // More than available
                specific_seats: None,
                expires_at: Utc::now() + chrono::Duration::minutes(5),
            })
            .then_state(move |state| {
                // No seats should be reserved
                let inventory = state.get_inventory(&event_id, "VIP").unwrap();
                assert_eq!(inventory.reserved, 0);
                assert!(state.last_error.is_some());
            })
            .then_effects(|effects| {
                // Validation failure broadcasts ValidationFailed for saga coordination
                assert_eq!(effects.len(), 1);
            })
            .run();
    }

    #[test]
    fn test_confirm_reservation() {
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();

        ReducerTest::new(InventoryReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = InventoryState::new();
                let reducer = InventoryReducer::new();
                let env = create_test_env();

                // Initialize and reserve
                reducer.reduce(
                    &mut state,
                    InventoryAction::InitializeInventory {
                        event_id,
                        section: "General".to_string(),
                        capacity: Capacity::new(100),
                        seat_numbers: None,
                        respond_to: ResponseChannel::none(),
                    },
                    &env,
                );
                // Mark state as loaded
                state.mark_loaded(event_id, "General".to_string());
                reducer.reduce(
                    &mut state,
                    InventoryAction::ReserveSeats {
                        reservation_id,
                        event_id,
                        section: "General".to_string(),
                        quantity: 2,
                        specific_seats: None,
                        expires_at: Utc::now() + chrono::Duration::minutes(5),
                    },
                    &env,
                );
                state
            })
            .when_action(InventoryAction::ConfirmReservation {
                reservation_id,
                customer_id,
            })
            .then_state(move |state| {
                let inventory = state.get_inventory(&event_id, "General").unwrap();
                assert_eq!(inventory.reserved, 0); // Moved from reserved to sold
                assert_eq!(inventory.sold, 2);
                assert_eq!(inventory.available(), 98);
            })
            .then_effects(|effects| {
                // Should return 2 effects: AppendEvents + Echo (no Redpanda)
                assert_eq!(effects.len(), 2);
            })
            .run();
    }

    #[test]
    fn test_release_reservation() {
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();

        ReducerTest::new(InventoryReducer::new())
            .with_env(create_test_env())
            .given_state({
                let mut state = InventoryState::new();
                let reducer = InventoryReducer::new();
                let env = create_test_env();

                // Initialize and reserve
                reducer.reduce(
                    &mut state,
                    InventoryAction::InitializeInventory {
                        event_id,
                        section: "General".to_string(),
                        capacity: Capacity::new(100),
                        seat_numbers: None,
                        respond_to: ResponseChannel::none(),
                    },
                    &env,
                );
                // Mark state as loaded
                state.mark_loaded(event_id, "General".to_string());
                reducer.reduce(
                    &mut state,
                    InventoryAction::ReserveSeats {
                        reservation_id,
                        event_id,
                        section: "General".to_string(),
                        quantity: 2,
                        specific_seats: None,
                        expires_at: Utc::now() + chrono::Duration::minutes(5),
                    },
                    &env,
                );
                state
            })
            .when_action(InventoryAction::ReleaseReservation { reservation_id })
            .then_state(move |state| {
                let inventory = state.get_inventory(&event_id, "General").unwrap();
                assert_eq!(inventory.reserved, 0); // Back to available
                assert_eq!(inventory.sold, 0);
                assert_eq!(inventory.available(), 100);
            })
            .then_effects(|effects| {
                // Should return 2 effects: AppendEvents + Echo (no Redpanda)
                assert_eq!(effects.len(), 2);
            })
            .run();
    }

    #[test]
    fn test_last_seat_race_condition() {
        // This test simulates the critical "last seat" scenario
        let event_id = EventId::new();
        let reservation1 = ReservationId::new();
        let reservation2 = ReservationId::new();

        let mut state = InventoryState::new();
        let reducer = InventoryReducer::new();
        let env = create_test_env();

        // Initialize with only 1 seat
        reducer.reduce(
            &mut state,
            InventoryAction::InitializeInventory {
                event_id,
                section: "VIP".to_string(),
                capacity: Capacity::new(1),
                seat_numbers: None,
                respond_to: ResponseChannel::none(),
            },
            &env,
        );

        // Mark state as loaded
        state.mark_loaded(event_id, "VIP".to_string());

        // First reservation gets the seat
        reducer.reduce(
            &mut state,
            InventoryAction::ReserveSeats {
                reservation_id: reservation1,
                event_id,
                section: "VIP".to_string(),
                quantity: 1,
                specific_seats: None,
                expires_at: Utc::now() + chrono::Duration::minutes(5),
            },
            &env,
        );

        let inventory = state.get_inventory(&event_id, "VIP").unwrap();
        assert_eq!(inventory.reserved, 1);
        assert_eq!(inventory.available(), 0);

        // Second reservation should fail (no seats available)
        reducer.reduce(
            &mut state,
            InventoryAction::ReserveSeats {
                reservation_id: reservation2,
                event_id,
                section: "VIP".to_string(),
                quantity: 1,
                specific_seats: None,
                expires_at: Utc::now() + chrono::Duration::minutes(5),
            },
            &env,
        );

        // Verify: still only 1 reserved, not 2 (no double-booking)
        let inventory = state.get_inventory(&event_id, "VIP").unwrap();
        assert_eq!(inventory.reserved, 1); // CRITICAL: Not 2!
        assert_eq!(inventory.sold, 0);
        assert!(state.last_error.is_some());
    }

    // ==================== Pricing Calculation Tests ====================

    #[test]
    fn test_calculate_price_from_tiers_active_tier() {
        let now = Utc::now();
        let pricing_tiers = vec![
            PricingTier::new(
                TierType::EarlyBird,
                "General".to_string(),
                Money::from_dollars(30),
                now - chrono::Duration::days(1),
                Some(now + chrono::Duration::days(7)),
            ),
            PricingTier::new(
                TierType::Regular,
                "General".to_string(),
                Money::from_dollars(50),
                now + chrono::Duration::days(7),
                None,
            ),
        ];

        let price = InventoryReducer::calculate_price_from_tiers(&pricing_tiers, "General", now);
        assert_eq!(price, Some(3000)); // EarlyBird price
    }

    #[test]
    fn test_calculate_price_from_tiers_future_tier() {
        let now = Utc::now();
        let pricing_tiers = vec![
            PricingTier::new(
                TierType::EarlyBird,
                "General".to_string(),
                Money::from_dollars(30),
                now - chrono::Duration::days(10),
                Some(now - chrono::Duration::days(1)),
            ),
            PricingTier::new(
                TierType::Regular,
                "General".to_string(),
                Money::from_dollars(50),
                now,
                None,
            ),
        ];

        let price = InventoryReducer::calculate_price_from_tiers(&pricing_tiers, "General", now);
        assert_eq!(price, Some(5000)); // Regular price (EarlyBird expired)
    }

    #[test]
    fn test_calculate_price_from_tiers_expired_tier() {
        let now = Utc::now();
        let pricing_tiers = vec![
            PricingTier::new(
                TierType::EarlyBird,
                "General".to_string(),
                Money::from_dollars(30),
                now - chrono::Duration::days(10),
                Some(now - chrono::Duration::days(1)),
            ),
        ];

        let price = InventoryReducer::calculate_price_from_tiers(&pricing_tiers, "General", now);
        assert_eq!(price, None); // All tiers expired
    }

    #[test]
    fn test_calculate_price_from_tiers_wrong_section() {
        let now = Utc::now();
        let pricing_tiers = vec![
            PricingTier::new(
                TierType::Regular,
                "VIP".to_string(),
                Money::from_dollars(100),
                now,
                None,
            ),
        ];

        let price = InventoryReducer::calculate_price_from_tiers(&pricing_tiers, "General", now);
        assert_eq!(price, None); // No pricing for requested section
    }

    #[test]
    fn test_calculate_price_from_tiers_multiple_sections() {
        let now = Utc::now();
        let pricing_tiers = vec![
            PricingTier::new(
                TierType::Regular,
                "VIP".to_string(),
                Money::from_dollars(100),
                now,
                None,
            ),
            PricingTier::new(
                TierType::Regular,
                "General".to_string(),
                Money::from_dollars(50),
                now,
                None,
            ),
        ];

        let vip_price = InventoryReducer::calculate_price_from_tiers(&pricing_tiers, "VIP", now);
        assert_eq!(vip_price, Some(10_000));

        let general_price = InventoryReducer::calculate_price_from_tiers(&pricing_tiers, "General", now);
        assert_eq!(general_price, Some(5000));
    }

    #[test]
    fn test_fallback_section_price_vip() {
        let price = InventoryReducer::fallback_section_price("VIP");
        assert_eq!(price, 10_000); // $100
    }

    #[test]
    fn test_fallback_section_price_premium() {
        let price = InventoryReducer::fallback_section_price("Premium Seating");
        assert_eq!(price, 10_000); // $100 (contains "premium")
    }

    #[test]
    fn test_fallback_section_price_general() {
        let price = InventoryReducer::fallback_section_price("General Admission");
        assert_eq!(price, 3_000); // $30 (contains "general")
    }

    #[test]
    fn test_fallback_section_price_default() {
        let price = InventoryReducer::fallback_section_price("Balcony");
        assert_eq!(price, 5_000); // $50 (default)
    }

    // Note: Pricing cache behavior is tested implicitly through integration tests
    // that exercise the full ReserveSeats flow with pricing lookup. The unit tests
    // above comprehensively test the pricing calculation functions themselves.
}
