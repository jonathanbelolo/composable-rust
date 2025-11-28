//! Shared test utilities for the ticketing example.
//!
//! This module provides reusable mock implementations and helper functions
//! to reduce duplication across unit and integration tests.
//!
//! # Usage
//!
//! In unit tests (within `src/`):
//! ```rust,ignore
//! use crate::test_utils::{MockEventProjectionQuery, MockInventoryQuery, create_test_global_channels};
//! ```
//!
//! In integration tests (within `tests/`):
//! ```rust,ignore
//! use ticketing::test_utils::{MockEventProjectionQuery, MockInventoryQuery, create_test_global_channels};
//! ```

use crate::aggregates::inventory::{InventoryProjectionQuery, SectionAvailabilityData};
use crate::projections::EventProjectionQuery;
use crate::types::{
    Capacity, Event, EventId, EventStatus, GlobalActionChannels, Money, PricingTier,
    SeatAssignment, SeatType, TierType, Venue, VenueSection,
};
use async_trait::async_trait;
use chrono::Utc;
use std::pin::Pin;
use tokio::sync::broadcast;

// ═══════════════════════════════════════════════════════════════════════════
// MOCK PROJECTION QUERIES
// ═══════════════════════════════════════════════════════════════════════════

/// Mock event projection query that returns `None` for all queries.
///
/// Use this when you want tests to rely on event sourcing (loading from event store)
/// rather than projection data.
#[derive(Clone, Debug, Default)]
pub struct MockEventProjectionQuery;

#[async_trait]
impl EventProjectionQuery for MockEventProjectionQuery {
    async fn load_event(&self, _event_id: &EventId) -> Result<Option<Event>, String> {
        Ok(None)
    }

    async fn load_events(
        &self,
        _status_filter: Option<EventStatus>,
    ) -> Result<Vec<Event>, String> {
        Ok(vec![])
    }
}

/// Mock inventory projection query that returns `None` for all queries.
///
/// Use this when you want tests to rely on event sourcing (loading from event store)
/// rather than projection data.
#[derive(Clone, Debug, Default)]
pub struct MockInventoryQuery;

impl InventoryProjectionQuery for MockInventoryQuery {
    fn load_inventory(
        &self,
        _event_id: &EventId,
        _section: &str,
    ) -> Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Option<((u32, u32, u32, u32), Vec<SeatAssignment>)>, String>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move { Ok(None) })
    }

    fn get_all_sections(
        &self,
        _event_id: &EventId,
    ) -> Pin<
        Box<dyn std::future::Future<Output = Result<Vec<SectionAvailabilityData>, String>> + Send + '_>,
    > {
        Box::pin(async move { Ok(vec![]) })
    }

    fn get_section_availability(
        &self,
        _event_id: &EventId,
        _section: &str,
    ) -> Pin<
        Box<
            dyn std::future::Future<Output = Result<Option<SectionAvailabilityData>, String>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move { Ok(None) })
    }

    fn get_total_available(
        &self,
        _event_id: &EventId,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<u32, String>> + Send + '_>> {
        Box::pin(async move { Ok(0) })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GLOBAL CHANNELS FACTORY
// ═══════════════════════════════════════════════════════════════════════════

/// Creates test global action channels for cross-aggregate coordination.
///
/// These channels are used for intra-process communication between aggregates.
/// In tests, they typically aren't actively consumed, but the channels must exist
/// for the environment to be properly initialized.
///
/// # Example
///
/// ```rust,ignore
/// let env = InventoryEnvironment::new(
///     clock,
///     event_store,
///     stream_id,
///     query,
///     event_query,
///     create_test_global_channels(),
/// );
/// ```
#[must_use]
pub fn create_test_global_channels() -> GlobalActionChannels {
    let (event_actions, _) = broadcast::channel(1000);
    let (inventory_actions, _) = broadcast::channel(1000);
    let (reservation_actions, _) = broadcast::channel(1000);
    let (payment_actions, _) = broadcast::channel(1000);

    GlobalActionChannels {
        event_actions,
        inventory_actions,
        reservation_actions,
        payment_actions,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TEST DATA FACTORIES
// ═══════════════════════════════════════════════════════════════════════════

/// Creates a test venue with standard sections (VIP, General, Balcony).
///
/// This provides a consistent venue structure for tests that need to create
/// events with inventory.
#[must_use]
pub fn create_test_venue() -> Venue {
    Venue {
        name: "Test Arena".to_string(),
        capacity: Capacity::new(350), // 50 + 200 + 100
        sections: vec![
            VenueSection {
                name: "VIP".to_string(),
                capacity: Capacity::new(50),
                seat_type: SeatType::Numbered { seats: vec![] },
            },
            VenueSection {
                name: "General".to_string(),
                capacity: Capacity::new(200),
                seat_type: SeatType::GeneralAdmission,
            },
            VenueSection {
                name: "Balcony".to_string(),
                capacity: Capacity::new(100),
                seat_type: SeatType::Numbered { seats: vec![] },
            },
        ],
    }
}

/// Creates test pricing tiers for common sections.
///
/// Returns a `Vec` with pricing for VIP ($150), General ($50), and Balcony ($75).
#[must_use]
pub fn create_test_pricing_tiers() -> Vec<PricingTier> {
    let now = Utc::now();
    vec![
        PricingTier {
            tier_type: TierType::Regular,
            section: "VIP".to_string(),
            base_price: Money::from_cents(15000), // $150.00
            available_from: now,
            available_until: None,
        },
        PricingTier {
            tier_type: TierType::Regular,
            section: "General".to_string(),
            base_price: Money::from_cents(5000), // $50.00
            available_from: now,
            available_until: None,
        },
        PricingTier {
            tier_type: TierType::Regular,
            section: "Balcony".to_string(),
            base_price: Money::from_cents(7500), // $75.00
            available_from: now,
            available_until: None,
        },
    ]
}
