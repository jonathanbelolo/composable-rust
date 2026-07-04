//! In-memory inventory projection infrastructure for testing.
//!
//! This module provides:
//! - [`InMemoryInventoryProjections`]: Shared store for inventory projection data
//! - [`InMemoryInventoryProjector`]: Writes to the store when events are persisted
//! - [`InMemoryInventoryQueryFetcher`]: Reads from the store for command validation

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use composable_rust_next::{
    FetchResult, ProjectionError, ProjectionQueries, Projector, QueryFetcher, SerializedEvent,
};
use tokio::sync::RwLock;

use crate::next::inventory::{
    InventoryCommand, InventoryDto, InventoryEvent, ReservationDto, SectionAvailabilityDto,
};
use crate::types::{EventId, ReservationId, SeatId};

// ═══════════════════════════════════════════════════════════════════════════
// Shared Projection Store
// ═══════════════════════════════════════════════════════════════════════════

/// In-memory inventory data for a single section.
#[derive(Debug, Clone)]
pub struct InventoryProjectionData {
    /// Event ID
    pub event_id: EventId,
    /// Section name
    pub section: String,
    /// Total capacity
    pub total_capacity: u32,
    /// Available seat IDs
    pub available_seats: Vec<SeatId>,
    /// Reserved seats by reservation ID
    pub reserved_seats: HashMap<ReservationId, ReservedSeatData>,
    /// Sold seat IDs
    pub sold_seats: Vec<SeatId>,
}

/// Data for a reservation's seats.
#[derive(Debug, Clone)]
pub struct ReservedSeatData {
    /// Seat IDs in this reservation
    pub seats: Vec<SeatId>,
    /// When the reservation expires
    pub expires_at: DateTime<Utc>,
}

/// In-memory projection store for inventory data.
///
/// This store is shared between:
/// - [`InMemoryInventoryProjector`]: Writes when events are projected
/// - [`InMemoryInventoryQueryFetcher`]: Reads when preparing commands
///
/// # Thread Safety
///
/// Uses `Arc<RwLock<...>>` for safe concurrent access. The projector takes
/// a write lock, the query fetcher takes a read lock.
#[derive(Debug, Clone, Default)]
pub struct InMemoryInventoryProjections {
    /// Inventory data keyed by (event_id, section)
    inventories: Arc<RwLock<HashMap<(EventId, String), InventoryProjectionData>>>,
    /// Reservation data keyed by reservation_id (for Confirm/Release lookups)
    reservations: Arc<RwLock<HashMap<ReservationId, ReservationLookup>>>,
}

/// Lookup data for finding a reservation's inventory.
#[derive(Debug, Clone)]
struct ReservationLookup {
    event_id: EventId,
    section: String,
}

impl InMemoryInventoryProjections {
    /// Create a new empty projection store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get inventory DTO for a section.
    ///
    /// Returns `None` if inventory hasn't been initialized.
    pub async fn get_inventory(&self, event_id: EventId, section: &str) -> Option<InventoryDto> {
        let inventories = self.inventories.read().await;
        inventories
            .get(&(event_id, section.to_string()))
            .map(|data| InventoryDto {
                initialized: true,
                capacity: data.total_capacity,
                available_count: data.available_seats.len() as u32,
                available_seats: data.available_seats.clone(),
            })
    }

    /// Get reservation DTO for validation.
    ///
    /// Returns `None` if reservation doesn't exist or isn't active.
    pub async fn get_reservation(&self, reservation_id: ReservationId) -> Option<ReservationDto> {
        let reservations = self.reservations.read().await;
        let lookup = reservations.get(&reservation_id)?;

        let inventories = self.inventories.read().await;
        let inventory = inventories.get(&(lookup.event_id, lookup.section.clone()))?;

        inventory
            .reserved_seats
            .get(&reservation_id)
            .map(|data| ReservationDto {
                seats: data.seats.clone(),
                expires_at: data.expires_at,
            })
    }

    /// Get section availability DTO.
    pub async fn get_section_availability(
        &self,
        event_id: EventId,
        section: &str,
    ) -> Option<SectionAvailabilityDto> {
        let inventories = self.inventories.read().await;
        inventories
            .get(&(event_id, section.to_string()))
            .map(|data| {
                let reserved: u32 = data
                    .reserved_seats
                    .values()
                    .map(|r| r.seats.len() as u32)
                    .sum();
                SectionAvailabilityDto {
                    event_id: data.event_id,
                    section: data.section.clone(),
                    total_capacity: data.total_capacity,
                    available_seats: data.available_seats.len() as u32,
                    reserved_seats: reserved,
                    sold_seats: data.sold_seats.len() as u32,
                }
            })
    }

    /// Get availability for all sections of an event.
    pub async fn get_event_availability(&self, event_id: EventId) -> Vec<SectionAvailabilityDto> {
        let inventories = self.inventories.read().await;
        inventories
            .iter()
            .filter(|((eid, _), _)| *eid == event_id)
            .map(|(_, data)| {
                let reserved: u32 = data
                    .reserved_seats
                    .values()
                    .map(|r| r.seats.len() as u32)
                    .sum();
                SectionAvailabilityDto {
                    event_id: data.event_id,
                    section: data.section.clone(),
                    total_capacity: data.total_capacity,
                    available_seats: data.available_seats.len() as u32,
                    reserved_seats: reserved,
                    sold_seats: data.sold_seats.len() as u32,
                }
            })
            .collect()
    }

    /// Get total available seats for an event.
    pub async fn get_total_available(&self, event_id: EventId) -> u32 {
        let inventories = self.inventories.read().await;
        inventories
            .iter()
            .filter(|((eid, _), _)| *eid == event_id)
            .map(|(_, data)| data.available_seats.len() as u32)
            .sum()
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Write Methods (used by projector)
    // ═══════════════════════════════════════════════════════════════════════

    /// Initialize inventory for a section.
    pub(crate) async fn initialize(
        &self,
        event_id: EventId,
        section: String,
        capacity: u32,
        seats: Vec<SeatId>,
    ) {
        let mut inventories = self.inventories.write().await;
        inventories.insert(
            (event_id, section.clone()),
            InventoryProjectionData {
                event_id,
                section,
                total_capacity: capacity,
                available_seats: seats,
                reserved_seats: HashMap::new(),
                sold_seats: Vec::new(),
            },
        );
    }

    /// Reserve seats.
    pub(crate) async fn reserve_seats(
        &self,
        reservation_id: ReservationId,
        event_id: EventId,
        section: String,
        seats: Vec<SeatId>,
        expires_at: DateTime<Utc>,
    ) {
        // Update inventory
        let mut inventories = self.inventories.write().await;
        if let Some(inventory) = inventories.get_mut(&(event_id, section.clone())) {
            // Remove from available
            inventory.available_seats.retain(|s| !seats.contains(s));
            // Add to reserved
            inventory.reserved_seats.insert(
                reservation_id,
                ReservedSeatData {
                    seats: seats.clone(),
                    expires_at,
                },
            );
        }
        drop(inventories);

        // Add reservation lookup
        let mut reservations = self.reservations.write().await;
        reservations.insert(reservation_id, ReservationLookup { event_id, section });
    }

    /// Confirm a reservation (seats become sold).
    pub(crate) async fn confirm_reservation(&self, reservation_id: ReservationId) {
        // Look up the reservation
        let reservations = self.reservations.read().await;
        let Some(lookup) = reservations.get(&reservation_id).cloned() else {
            return;
        };
        drop(reservations);

        // Update inventory
        let mut inventories = self.inventories.write().await;
        if let Some(inventory) = inventories.get_mut(&(lookup.event_id, lookup.section)) {
            if let Some(reserved) = inventory.reserved_seats.remove(&reservation_id) {
                // Move seats from reserved to sold
                inventory.sold_seats.extend(reserved.seats);
            }
        }
    }

    /// Release a reservation (seats return to available).
    pub(crate) async fn release_reservation(&self, reservation_id: ReservationId) {
        // Look up the reservation
        let reservations = self.reservations.read().await;
        let Some(lookup) = reservations.get(&reservation_id).cloned() else {
            return;
        };
        drop(reservations);

        // Update inventory
        let mut inventories = self.inventories.write().await;
        if let Some(inventory) = inventories.get_mut(&(lookup.event_id, lookup.section)) {
            if let Some(reserved) = inventory.reserved_seats.remove(&reservation_id) {
                // Return seats to available
                inventory.available_seats.extend(reserved.seats);
            }
        }

        // Remove reservation lookup
        let mut reservations = self.reservations.write().await;
        reservations.remove(&reservation_id);
    }
}

impl ProjectionQueries for InMemoryInventoryProjections {
    type Error = std::convert::Infallible;
}

// ═══════════════════════════════════════════════════════════════════════════
// Projector Implementation
// ═══════════════════════════════════════════════════════════════════════════

/// In-memory projector that writes to [`InMemoryInventoryProjections`].
///
/// This projector deserializes inventory events and updates the shared
/// projection store, enabling the query fetcher to read the latest state.
#[derive(Debug, Clone)]
pub struct InMemoryInventoryProjector {
    projections: InMemoryInventoryProjections,
}

impl InMemoryInventoryProjector {
    /// Create a new projector with the given shared store.
    #[must_use]
    pub fn new(projections: InMemoryInventoryProjections) -> Self {
        Self { projections }
    }
}

/// Inventory event type names for filtering.
///
/// These are the exact event type names from `InventoryBusinessLogic::event_type_name()`.
/// Note: `SeatsAllocated` is a SAGA event (not inventory) so we use exact matching.
const INVENTORY_EVENT_TYPES: &[&str] = &[
    "InventoryInitialized",
    "SeatsReserved",
    "SeatsConfirmed",
    "SeatsReleased",
];

impl Projector for InMemoryInventoryProjector {
    fn project(
        &self,
        events: &[SerializedEvent],
    ) -> impl std::future::Future<Output = Result<(), ProjectionError>> + Send {
        let events_owned: Vec<SerializedEvent> = events.to_vec();
        let projections = self.projections.clone();

        async move {
            for event in events_owned {
                // Skip events that aren't inventory events
                // Use exact matching because saga events like "SeatsAllocated" would
                // incorrectly match a "Seats*" prefix filter.
                if !INVENTORY_EVENT_TYPES.contains(&event.event_type.as_str()) {
                    continue;
                }

                let inventory_event: InventoryEvent = bincode::deserialize(&event.payload)
                    .map_err(|e| {
                        ProjectionError::Deserialization(format!(
                            "failed to deserialize inventory event: {e}"
                        ))
                    })?;

                match inventory_event {
                    InventoryEvent::Initialized {
                        event_id,
                        section,
                        capacity,
                        seats,
                        ..
                    } => {
                        projections
                            .initialize(event_id, section, capacity, seats)
                            .await;
                    },

                    InventoryEvent::SeatsReserved {
                        reservation_id,
                        event_id,
                        section,
                        seats,
                        expires_at,
                        ..
                    } => {
                        projections
                            .reserve_seats(reservation_id, event_id, section, seats, expires_at)
                            .await;
                    },

                    InventoryEvent::SeatsConfirmed { reservation_id, .. } => {
                        projections.confirm_reservation(reservation_id).await;
                    },

                    InventoryEvent::SeatsReleased { reservation_id, .. } => {
                        projections.release_reservation(reservation_id).await;
                    },
                }
            }

            Ok(())
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Query Fetcher Implementation
// ═══════════════════════════════════════════════════════════════════════════

/// In-memory query fetcher that reads from [`InMemoryInventoryProjections`].
///
/// This fetcher populates the `fetched` field in inventory commands with
/// data from the shared projection store.
#[derive(Debug, Clone)]
pub struct InMemoryInventoryQueryFetcher {
    projections: InMemoryInventoryProjections,
}

impl InMemoryInventoryQueryFetcher {
    /// Create a new query fetcher with the given shared store.
    #[must_use]
    pub fn new(projections: InMemoryInventoryProjections) -> Self {
        Self { projections }
    }
}

impl<PQ: ProjectionQueries> QueryFetcher<InventoryCommand, PQ> for InMemoryInventoryQueryFetcher {
    type Error = std::convert::Infallible;

    fn fetch(
        &self,
        input: InventoryCommand,
        _projections: &PQ, // Ignored - we use our own projections
    ) -> impl std::future::Future<Output = Result<FetchResult<InventoryCommand>, Self::Error>> + Send
    {
        let projections = self.projections.clone();

        async move {
            let prepared = match input {
                // ═══════════════════════════════════════════════════════════
                // Write Commands - need validation data
                // ═══════════════════════════════════════════════════════════
                InventoryCommand::Initialize {
                    event_id,
                    section,
                    capacity,
                    fetched: _,
                } => {
                    let fetched = projections.get_inventory(event_id, &section).await;
                    InventoryCommand::Initialize {
                        event_id,
                        section,
                        capacity,
                        fetched,
                    }
                },

                InventoryCommand::Reserve {
                    reservation_id,
                    event_id,
                    section,
                    quantity,
                    expires_at,
                    fetched: _,
                } => {
                    let fetched = projections.get_inventory(event_id, &section).await;
                    InventoryCommand::Reserve {
                        reservation_id,
                        event_id,
                        section,
                        quantity,
                        expires_at,
                        fetched,
                    }
                },

                InventoryCommand::Confirm {
                    reservation_id,
                    customer_id,
                    fetched: _,
                } => {
                    let fetched = projections.get_reservation(reservation_id).await;
                    InventoryCommand::Confirm {
                        reservation_id,
                        customer_id,
                        fetched,
                    }
                },

                InventoryCommand::Release {
                    reservation_id,
                    reason,
                    fetched: _,
                } => {
                    let fetched = projections.get_reservation(reservation_id).await;
                    InventoryCommand::Release {
                        reservation_id,
                        reason,
                        fetched,
                    }
                },

                // ═══════════════════════════════════════════════════════════
                // Query Commands - need read data
                // ═══════════════════════════════════════════════════════════
                InventoryCommand::GetSectionAvailability {
                    event_id,
                    section,
                    fetched: _,
                } => {
                    let fetched = projections
                        .get_section_availability(event_id, &section)
                        .await;
                    InventoryCommand::GetSectionAvailability {
                        event_id,
                        section,
                        fetched,
                    }
                },

                InventoryCommand::GetEventAvailability {
                    event_id,
                    fetched: _,
                } => {
                    let fetched = projections.get_event_availability(event_id).await;
                    InventoryCommand::GetEventAvailability { event_id, fetched }
                },

                InventoryCommand::GetTotalAvailable {
                    event_id,
                    fetched: _,
                } => {
                    let fetched = projections.get_total_available(event_id).await;
                    InventoryCommand::GetTotalAvailable { event_id, fetched }
                },
            };

            Ok(FetchResult::new_entity(prepared))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Test Harness
// ═══════════════════════════════════════════════════════════════════════════

use composable_rust_next::{
    Clock, HandleResult, Handler, HandlerEnvironment, HandlerError, MetadataContext,
    NoOpCallExecutor,
    testing::{InMemoryEventBus, InMemoryEventStore},
};

use crate::next::inventory::{InventoryBusinessLogic, InventoryError, InventoryResponse};

/// Test environment that uses our custom projector type.
///
/// This is necessary because `TestEnvironment` has a fixed `InMemoryProjector` type,
/// but we need our [`InMemoryInventoryProjector`] to update the shared projection store.
#[derive(Clone)]
pub struct InventoryTestEnvironment<C: Clock> {
    /// The clock
    pub clock: C,
    /// Shared event store
    pub event_store: InMemoryEventStore,
    /// Projector that writes to shared projections
    pub projector: InMemoryInventoryProjector,
    /// Event bus for broadcasts
    pub event_bus: InMemoryEventBus,
    /// Shared projections
    pub projections: InMemoryInventoryProjections,
    /// Metadata context
    pub metadata: MetadataContext,
}

impl<C: Clock + Send + Sync> HandlerEnvironment for InventoryTestEnvironment<C> {
    type Clock = C;
    type EventStore = InMemoryEventStore;
    type Projector = InMemoryInventoryProjector;
    type EventBus = InMemoryEventBus;
    type Projections = InMemoryInventoryProjections;

    fn clock(&self) -> &Self::Clock {
        &self.clock
    }

    fn event_store(&self) -> &Self::EventStore {
        &self.event_store
    }

    fn projector(&self) -> Option<&Self::Projector> {
        Some(&self.projector)
    }

    fn event_bus(&self) -> Option<&Self::EventBus> {
        Some(&self.event_bus)
    }

    fn broadcast_topic(&self) -> &str {
        "inventory-events"
    }

    fn projections(&self) -> &Self::Projections {
        &self.projections
    }

    fn metadata(&self) -> &MetadataContext {
        &self.metadata
    }
}

/// Fully-wired test harness for Inventory handler integration tests.
///
/// Bundles all the infrastructure together with proper data sharing between
/// the projector (writer) and query fetcher (reader).
///
/// # Example
///
/// ```ignore
/// let harness = InventoryTestHarness::new();
///
/// // Initialize inventory - events are persisted and projected
/// harness.handle(InventoryCommand::Initialize {
///     event_id,
///     section: "VIP".to_string(),
///     capacity: Capacity::new(100),
///     fetched: None, // Will be populated by QueryFetcher
/// }).await?;
///
/// // Reserve seats - QueryFetcher reads from projections
/// harness.handle(InventoryCommand::Reserve {
///     reservation_id: ReservationId::new(),
///     event_id,
///     section: "VIP".to_string(),
///     quantity: 5,
///     expires_at: Utc::now() + Duration::minutes(15),
///     fetched: None,
/// }).await?;
///
/// // Verify projection state
/// let dto = harness.get_inventory(event_id, "VIP").await.unwrap();
/// assert_eq!(dto.available_count, 95);
/// ```
pub struct InventoryTestHarness {
    projections: InMemoryInventoryProjections,
    handler: Handler<
        InventoryBusinessLogic,
        NoOpCallExecutor,
        InMemoryInventoryQueryFetcher,
        InventoryTestEnvironment<composable_rust_next::FixedClock>,
    >,
    event_store: InMemoryEventStore,
    event_bus: InMemoryEventBus,
}

impl InventoryTestHarness {
    /// Create a new test harness with all infrastructure wired together.
    #[must_use]
    pub fn new() -> Self {
        Self::with_clock(composable_rust_next::FixedClock::new(chrono::Utc::now()))
    }

    /// Create a test harness with a specific clock.
    #[must_use]
    pub fn with_clock(clock: composable_rust_next::FixedClock) -> Self {
        let projections = InMemoryInventoryProjections::new();
        let projector = InMemoryInventoryProjector::new(projections.clone());
        let query_fetcher = InMemoryInventoryQueryFetcher::new(projections.clone());
        let event_store = InMemoryEventStore::new();
        let event_bus = InMemoryEventBus::new();

        let env = InventoryTestEnvironment {
            clock,
            event_store: event_store.clone(),
            projector,
            event_bus: event_bus.clone(),
            projections: projections.clone(),
            metadata: MetadataContext::new(),
        };

        let handler = Handler::new(InventoryBusinessLogic, NoOpCallExecutor, query_fetcher, env);

        Self {
            projections,
            handler,
            event_store,
            event_bus,
        }
    }

    /// Handle an inventory command through the full pipeline.
    ///
    /// This will:
    /// 1. Fetch validation data from projections (via `QueryFetcher`)
    /// 2. Process business logic
    /// 3. Persist events to event store
    /// 4. Project events to update projections
    /// 5. Broadcast events to event bus
    pub async fn handle(
        &self,
        command: InventoryCommand,
    ) -> Result<HandleResult<InventoryResponse>, HandlerError<InventoryError>> {
        self.handler.handle(command).await
    }

    /// Get inventory DTO for a section (for test assertions).
    pub async fn get_inventory(&self, event_id: EventId, section: &str) -> Option<InventoryDto> {
        self.projections.get_inventory(event_id, section).await
    }

    /// Get reservation DTO (for test assertions).
    pub async fn get_reservation(&self, reservation_id: ReservationId) -> Option<ReservationDto> {
        self.projections.get_reservation(reservation_id).await
    }

    /// Get section availability (for test assertions).
    pub async fn get_section_availability(
        &self,
        event_id: EventId,
        section: &str,
    ) -> Option<SectionAvailabilityDto> {
        self.projections
            .get_section_availability(event_id, section)
            .await
    }

    /// Get total event count in the event store (for test assertions).
    #[must_use]
    pub fn total_event_count(&self) -> usize {
        self.event_store.total_event_count()
    }

    /// Get published events from the event bus (for test assertions).
    #[must_use]
    pub fn published_events(&self) -> Vec<(String, SerializedEvent)> {
        self.event_bus.published_events()
    }

    /// Access the shared projections directly.
    #[must_use]
    pub fn projections(&self) -> &InMemoryInventoryProjections {
        &self.projections
    }
}

impl Default for InventoryTestHarness {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn projections_store_initialized_inventory() {
        let projections = InMemoryInventoryProjections::new();
        let event_id = EventId::new();
        let seats: Vec<SeatId> = (0..10).map(|_| SeatId::new()).collect();

        projections
            .initialize(event_id, "VIP".to_string(), 10, seats.clone())
            .await;

        let dto = projections.get_inventory(event_id, "VIP").await.unwrap();
        assert!(dto.initialized);
        assert_eq!(dto.capacity, 10);
        assert_eq!(dto.available_count, 10);
        assert_eq!(dto.available_seats.len(), 10);
    }

    #[tokio::test]
    async fn projections_track_reserved_seats() {
        let projections = InMemoryInventoryProjections::new();
        let event_id = EventId::new();
        let seats: Vec<SeatId> = (0..10).map(|_| SeatId::new()).collect();
        let reservation_id = ReservationId::new();
        let reserved_seats = seats[0..3].to_vec();
        let expires_at = Utc::now() + chrono::Duration::minutes(15);

        // Initialize
        projections
            .initialize(event_id, "VIP".to_string(), 10, seats)
            .await;

        // Reserve
        projections
            .reserve_seats(
                reservation_id,
                event_id,
                "VIP".to_string(),
                reserved_seats.clone(),
                expires_at,
            )
            .await;

        // Check inventory
        let dto = projections.get_inventory(event_id, "VIP").await.unwrap();
        assert_eq!(dto.available_count, 7); // 10 - 3

        // Check reservation
        let res = projections.get_reservation(reservation_id).await.unwrap();
        assert_eq!(res.seats.len(), 3);
        assert_eq!(res.expires_at, expires_at);
    }

    #[tokio::test]
    async fn projections_release_returns_seats() {
        let projections = InMemoryInventoryProjections::new();
        let event_id = EventId::new();
        let seats: Vec<SeatId> = (0..10).map(|_| SeatId::new()).collect();
        let reservation_id = ReservationId::new();
        let reserved_seats = seats[0..3].to_vec();
        let expires_at = Utc::now() + chrono::Duration::minutes(15);

        // Initialize and reserve
        projections
            .initialize(event_id, "VIP".to_string(), 10, seats)
            .await;
        projections
            .reserve_seats(
                reservation_id,
                event_id,
                "VIP".to_string(),
                reserved_seats,
                expires_at,
            )
            .await;

        // Release
        projections.release_reservation(reservation_id).await;

        // Seats should be back
        let dto = projections.get_inventory(event_id, "VIP").await.unwrap();
        assert_eq!(dto.available_count, 10);

        // Reservation should be gone
        let res = projections.get_reservation(reservation_id).await;
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn projector_deserializes_and_updates_store() {
        let projections = InMemoryInventoryProjections::new();
        let projector = InMemoryInventoryProjector::new(projections.clone());

        let event_id = EventId::new();
        let seats: Vec<SeatId> = (0..5).map(|_| SeatId::new()).collect();

        // Create a serialized event
        let event = InventoryEvent::Initialized {
            event_id,
            section: "GA".to_string(),
            capacity: 5,
            seats: seats.clone(),
            initialized_at: Utc::now(),
        };

        let serialized = SerializedEvent {
            event_type: "InventoryInitialized".to_string(),
            payload: bincode::serialize(&event).unwrap(),
            metadata: None,
            version: None,
        };

        // Project
        projector.project(&[serialized]).await.unwrap();

        // Verify
        let dto = projections.get_inventory(event_id, "GA").await.unwrap();
        assert!(dto.initialized);
        assert_eq!(dto.capacity, 5);
    }

    #[tokio::test]
    async fn query_fetcher_populates_fetched_field() {
        let projections = InMemoryInventoryProjections::new();
        let event_id = EventId::new();
        let seats: Vec<SeatId> = (0..10).map(|_| SeatId::new()).collect();

        // Pre-seed projections
        projections
            .initialize(event_id, "VIP".to_string(), 10, seats)
            .await;

        let fetcher = InMemoryInventoryQueryFetcher::new(projections.clone());

        // Create command with fetched: None
        let command = InventoryCommand::Reserve {
            reservation_id: ReservationId::new(),
            event_id,
            section: "VIP".to_string(),
            quantity: 2,
            expires_at: Utc::now() + chrono::Duration::minutes(15),
            fetched: None,
        };

        // Fetch should populate fetched
        let result = fetcher.fetch(command, &projections).await.unwrap();

        match result.input {
            InventoryCommand::Reserve { fetched, .. } => {
                let dto = fetched.expect("fetched should be populated");
                assert!(dto.initialized);
                assert_eq!(dto.available_count, 10);
            },
            _ => panic!("Expected Reserve command"),
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Handler Integration Tests (Level 2)
    // ═══════════════════════════════════════════════════════════════════════

    use crate::next::inventory::ReleaseReason;
    use crate::types::Capacity;

    #[tokio::test]
    async fn harness_initialize_and_reserve_full_flow() {
        let harness = InventoryTestHarness::new();
        let event_id = EventId::new();

        // Step 1: Initialize inventory
        let result = harness
            .handle(InventoryCommand::Initialize {
                event_id,
                section: "VIP".to_string(),
                capacity: Capacity::new(100),
                fetched: None, // QueryFetcher will populate
            })
            .await
            .expect("Initialize should succeed");

        assert!(result.is_command());
        assert_eq!(result.event_count(), 1);

        // Verify projection was updated
        let dto = harness.get_inventory(event_id, "VIP").await.unwrap();
        assert!(dto.initialized);
        assert_eq!(dto.capacity, 100);
        assert_eq!(dto.available_count, 100);

        // Step 2: Reserve some seats
        let reservation_id = ReservationId::new();
        let expires_at = Utc::now() + chrono::Duration::minutes(15);

        let result = harness
            .handle(InventoryCommand::Reserve {
                reservation_id,
                event_id,
                section: "VIP".to_string(),
                quantity: 5,
                expires_at,
                fetched: None, // QueryFetcher will populate with initialized inventory
            })
            .await
            .expect("Reserve should succeed");

        assert!(result.is_command());
        assert_eq!(result.event_count(), 1);

        // Verify projection was updated
        let dto = harness.get_inventory(event_id, "VIP").await.unwrap();
        assert_eq!(dto.available_count, 95); // 100 - 5

        // Verify reservation exists
        let res_dto = harness.get_reservation(reservation_id).await.unwrap();
        assert_eq!(res_dto.seats.len(), 5);
    }

    #[tokio::test]
    async fn harness_reserve_without_init_fails() {
        let harness = InventoryTestHarness::new();
        let event_id = EventId::new();

        // Try to reserve without initializing
        let result = harness
            .handle(InventoryCommand::Reserve {
                reservation_id: ReservationId::new(),
                event_id,
                section: "VIP".to_string(),
                quantity: 5,
                expires_at: Utc::now() + chrono::Duration::minutes(15),
                fetched: None,
            })
            .await;

        assert!(result.is_err());
        // The error should be NotInitialized
        match result {
            Err(HandlerError::Business(InventoryError::NotInitialized)) => {},
            other => panic!("Expected NotInitialized error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn harness_reserve_insufficient_inventory_fails() {
        let harness = InventoryTestHarness::new();
        let event_id = EventId::new();

        // Initialize with small capacity
        harness
            .handle(InventoryCommand::Initialize {
                event_id,
                section: "VIP".to_string(),
                capacity: Capacity::new(10),
                fetched: None,
            })
            .await
            .expect("Initialize should succeed");

        // Try to reserve more than available
        let result = harness
            .handle(InventoryCommand::Reserve {
                reservation_id: ReservationId::new(),
                event_id,
                section: "VIP".to_string(),
                quantity: 20, // More than 10 available
                expires_at: Utc::now() + chrono::Duration::minutes(15),
                fetched: None,
            })
            .await;

        assert!(result.is_err());
        match result {
            Err(HandlerError::Business(InventoryError::InsufficientInventory {
                requested,
                available,
            })) => {
                assert_eq!(requested, 20);
                assert_eq!(available, 10);
            },
            other => panic!("Expected InsufficientInventory error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn harness_confirm_reservation_full_flow() {
        let harness = InventoryTestHarness::new();
        let event_id = EventId::new();
        let customer_id = crate::types::CustomerId::new();
        let reservation_id = ReservationId::new();

        // Initialize
        harness
            .handle(InventoryCommand::Initialize {
                event_id,
                section: "VIP".to_string(),
                capacity: Capacity::new(100),
                fetched: None,
            })
            .await
            .unwrap();

        // Reserve
        harness
            .handle(InventoryCommand::Reserve {
                reservation_id,
                event_id,
                section: "VIP".to_string(),
                quantity: 5,
                expires_at: Utc::now() + chrono::Duration::minutes(15),
                fetched: None,
            })
            .await
            .unwrap();

        // Confirm
        let result = harness
            .handle(InventoryCommand::Confirm {
                reservation_id,
                customer_id,
                fetched: None,
            })
            .await
            .expect("Confirm should succeed");

        assert!(result.is_command());
        assert_eq!(result.event_count(), 1);

        // Reservation should be gone
        let res_dto = harness.get_reservation(reservation_id).await;
        assert!(res_dto.is_none());

        // Check availability - seats moved from reserved to sold
        let availability = harness
            .get_section_availability(event_id, "VIP")
            .await
            .unwrap();
        assert_eq!(availability.available_seats, 95);
        assert_eq!(availability.reserved_seats, 0);
        assert_eq!(availability.sold_seats, 5);
    }

    #[tokio::test]
    async fn harness_release_reservation_returns_seats() {
        let harness = InventoryTestHarness::new();
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();

        // Initialize
        harness
            .handle(InventoryCommand::Initialize {
                event_id,
                section: "VIP".to_string(),
                capacity: Capacity::new(100),
                fetched: None,
            })
            .await
            .unwrap();

        // Reserve
        harness
            .handle(InventoryCommand::Reserve {
                reservation_id,
                event_id,
                section: "VIP".to_string(),
                quantity: 10,
                expires_at: Utc::now() + chrono::Duration::minutes(15),
                fetched: None,
            })
            .await
            .unwrap();

        // Verify reserved
        let dto = harness.get_inventory(event_id, "VIP").await.unwrap();
        assert_eq!(dto.available_count, 90);

        // Release
        let result = harness
            .handle(InventoryCommand::Release {
                reservation_id,
                reason: ReleaseReason::CustomerCancelled,
                fetched: None,
            })
            .await
            .expect("Release should succeed");

        assert!(result.is_command());

        // Seats should be back
        let dto = harness.get_inventory(event_id, "VIP").await.unwrap();
        assert_eq!(dto.available_count, 100);

        // Reservation should be gone
        assert!(harness.get_reservation(reservation_id).await.is_none());
    }

    #[tokio::test]
    async fn harness_events_are_broadcast() {
        let harness = InventoryTestHarness::new();
        let event_id = EventId::new();

        // Initialize
        harness
            .handle(InventoryCommand::Initialize {
                event_id,
                section: "GA".to_string(),
                capacity: Capacity::new(50),
                fetched: None,
            })
            .await
            .unwrap();

        // Check event bus
        let published = harness.published_events();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].0, "inventory-events"); // topic
        assert_eq!(published[0].1.event_type, "InventoryInitialized");
    }

    #[tokio::test]
    async fn harness_double_init_fails() {
        let harness = InventoryTestHarness::new();
        let event_id = EventId::new();

        // First init succeeds
        harness
            .handle(InventoryCommand::Initialize {
                event_id,
                section: "VIP".to_string(),
                capacity: Capacity::new(100),
                fetched: None,
            })
            .await
            .unwrap();

        // Second init fails
        let result = harness
            .handle(InventoryCommand::Initialize {
                event_id,
                section: "VIP".to_string(),
                capacity: Capacity::new(200),
                fetched: None,
            })
            .await;

        assert!(result.is_err());
        match result {
            Err(HandlerError::Business(InventoryError::AlreadyInitialized)) => {},
            other => panic!("Expected AlreadyInitialized error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn harness_multiple_reservations() {
        let harness = InventoryTestHarness::new();
        let event_id = EventId::new();

        // Initialize
        harness
            .handle(InventoryCommand::Initialize {
                event_id,
                section: "VIP".to_string(),
                capacity: Capacity::new(100),
                fetched: None,
            })
            .await
            .unwrap();

        // Multiple reservations
        for _ in 0..5 {
            harness
                .handle(InventoryCommand::Reserve {
                    reservation_id: ReservationId::new(),
                    event_id,
                    section: "VIP".to_string(),
                    quantity: 10,
                    expires_at: Utc::now() + chrono::Duration::minutes(15),
                    fetched: None,
                })
                .await
                .unwrap();
        }

        // Check availability: 100 - (5 * 10) = 50
        let dto = harness.get_inventory(event_id, "VIP").await.unwrap();
        assert_eq!(dto.available_count, 50);

        // 6th reservation should fail (only 50 seats left, need 60)
        let result = harness
            .handle(InventoryCommand::Reserve {
                reservation_id: ReservationId::new(),
                event_id,
                section: "VIP".to_string(),
                quantity: 60,
                expires_at: Utc::now() + chrono::Duration::minutes(15),
                fetched: None,
            })
            .await;

        assert!(matches!(
            result,
            Err(HandlerError::Business(
                InventoryError::InsufficientInventory { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn harness_confirm_nonexistent_reservation_fails() {
        let harness = InventoryTestHarness::new();
        let event_id = EventId::new();
        let customer_id = crate::types::CustomerId::new();
        let reservation_id = ReservationId::new();

        // Initialize inventory (but don't create a reservation)
        harness
            .handle(InventoryCommand::Initialize {
                event_id,
                section: "VIP".to_string(),
                capacity: Capacity::new(100),
                fetched: None,
            })
            .await
            .unwrap();

        // Try to confirm a reservation that doesn't exist
        let result = harness
            .handle(InventoryCommand::Confirm {
                reservation_id,
                customer_id,
                fetched: None, // QueryFetcher will find no reservation
            })
            .await;

        assert!(result.is_err());
        match result {
            Err(HandlerError::Business(InventoryError::ReservationNotFound(_))) => {},
            other => panic!("Expected ReservationNotFound error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn harness_release_nonexistent_reservation_fails() {
        let harness = InventoryTestHarness::new();
        let event_id = EventId::new();
        let reservation_id = ReservationId::new();

        // Initialize inventory (but don't create a reservation)
        harness
            .handle(InventoryCommand::Initialize {
                event_id,
                section: "VIP".to_string(),
                capacity: Capacity::new(100),
                fetched: None,
            })
            .await
            .unwrap();

        // Try to release a reservation that doesn't exist
        let result = harness
            .handle(InventoryCommand::Release {
                reservation_id,
                reason: ReleaseReason::CustomerCancelled,
                fetched: None,
            })
            .await;

        assert!(result.is_err());
        match result {
            Err(HandlerError::Business(InventoryError::ReservationNotFound(_))) => {},
            other => panic!("Expected ReservationNotFound error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn projections_confirm_moves_to_sold() {
        let projections = InMemoryInventoryProjections::new();
        let event_id = EventId::new();
        let seats: Vec<SeatId> = (0..10).map(|_| SeatId::new()).collect();
        let reservation_id = ReservationId::new();
        let reserved_seats = seats[0..3].to_vec();
        let expires_at = Utc::now() + chrono::Duration::minutes(15);

        // Initialize and reserve
        projections
            .initialize(event_id, "VIP".to_string(), 10, seats)
            .await;
        projections
            .reserve_seats(
                reservation_id,
                event_id,
                "VIP".to_string(),
                reserved_seats,
                expires_at,
            )
            .await;

        // Verify reserved state
        let dto = projections.get_inventory(event_id, "VIP").await.unwrap();
        assert_eq!(dto.available_count, 7);

        // Confirm the reservation
        projections.confirm_reservation(reservation_id).await;

        // Verify: available should still be 7, reservation should be gone
        let dto = projections.get_inventory(event_id, "VIP").await.unwrap();
        assert_eq!(
            dto.available_count, 7,
            "Available count unchanged after confirm"
        );

        // Reservation should no longer exist
        let res = projections.get_reservation(reservation_id).await;
        assert!(res.is_none(), "Reservation should be gone after confirm");

        // Check availability shows seats as sold
        let availability = projections
            .get_section_availability(event_id, "VIP")
            .await
            .unwrap();
        assert_eq!(availability.sold_seats, 3, "Should have 3 sold seats");
        assert_eq!(
            availability.reserved_seats, 0,
            "Should have 0 reserved seats"
        );
    }

    #[tokio::test]
    async fn projector_skips_non_inventory_events() {
        let projections = InMemoryInventoryProjections::new();
        let projector = InMemoryInventoryProjector::new(projections.clone());

        // Create a non-inventory event (simulating a saga event)
        let fake_event = SerializedEvent {
            event_type: "SeatsAllocated".to_string(), // Saga event, NOT inventory
            payload: vec![1, 2, 3],                   // Invalid payload for inventory
            metadata: None,
            version: None,
        };

        // Project should succeed (skip the event, not fail)
        let result = projector.project(&[fake_event]).await;
        assert!(
            result.is_ok(),
            "Projector should skip non-inventory events without error"
        );
    }

    #[tokio::test]
    async fn query_fetcher_returns_none_for_missing_data() {
        let projections = InMemoryInventoryProjections::new();
        let fetcher = InMemoryInventoryQueryFetcher::new(projections.clone());
        let event_id = EventId::new();

        // Create command for non-existent inventory
        let command = InventoryCommand::Reserve {
            reservation_id: ReservationId::new(),
            event_id,
            section: "VIP".to_string(),
            quantity: 2,
            expires_at: Utc::now() + chrono::Duration::minutes(15),
            fetched: None,
        };

        // Fetch should succeed but with fetched = None
        let result = fetcher.fetch(command, &projections).await.unwrap();

        match result.input {
            InventoryCommand::Reserve { fetched, .. } => {
                assert!(
                    fetched.is_none(),
                    "fetched should be None for missing inventory"
                );
            },
            _ => panic!("Expected Reserve command"),
        }
    }
}
