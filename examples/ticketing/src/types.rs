//! Domain types for the Event Ticketing System.
//!
//! This module contains all value objects, entities, and state types for the ticketing system.
//! Demonstrates a complete event-sourced ticketing platform with inventory management,
//! reservation sagas, and payment processing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use uuid::Uuid;

// ============================================================================
// Identifiers
// ============================================================================

/// Unique identifier for an event
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(Uuid);

impl EventId {
    /// Creates a new random `EventId`
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create an `EventId` from a `Uuid`
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the inner UUID
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a seat
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SeatId(Uuid);

impl SeatId {
    /// Creates a new random `SeatId`
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SeatId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SeatId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a reservation
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReservationId(Uuid);

impl ReservationId {
    /// Creates a new random `ReservationId`
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a `ReservationId` from a `Uuid`
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the inner UUID
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for ReservationId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ReservationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a payment
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PaymentId(Uuid);

impl PaymentId {
    /// Creates a new random `PaymentId`
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a `PaymentId` from a `Uuid`
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the inner UUID
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for PaymentId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PaymentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a customer
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CustomerId(Uuid);

impl CustomerId {
    /// Creates a new random `CustomerId`
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a `CustomerId` from a `Uuid`
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the inner UUID
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for CustomerId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CustomerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a ticket
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TicketId(Uuid);

impl TicketId {
    /// Creates a new random `TicketId`
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TicketId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TicketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================================
// Money Value Object (cents-based to avoid floating point errors)
// ============================================================================

/// Represents money in cents to avoid floating-point arithmetic errors
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Money(u64);

impl Money {
    /// Creates a `Money` value from cents
    #[must_use]
    pub const fn from_cents(cents: u64) -> Self {
        Self(cents)
    }

    /// Creates a `Money` value from dollars
    ///
    /// # Panics
    ///
    /// Panics if the conversion would overflow (dollars * 100 > `u64::MAX`).
    /// Use `checked_from_dollars` for non-panicking conversion.
    #[must_use]
    #[allow(clippy::panic)]
    pub const fn from_dollars(dollars: u64) -> Self {
        match dollars.checked_mul(100) {
            Some(cents) => Self(cents),
            None => panic!("Money::from_dollars overflow"),
        }
    }

    /// Creates a `Money` value from dollars with overflow checking
    #[must_use]
    pub const fn checked_from_dollars(dollars: u64) -> Option<Self> {
        match dollars.checked_mul(100) {
            Some(cents) => Some(Self(cents)),
            None => None,
        }
    }

    /// Returns the amount in cents
    #[must_use]
    pub const fn cents(&self) -> u64 {
        self.0
    }

    /// Returns the amount in dollars (rounded down)
    #[must_use]
    pub const fn dollars(&self) -> u64 {
        self.0 / 100
    }

    /// Checks if the amount is zero
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.0 == 0
    }

    /// Adds two money amounts with overflow checking
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(result) => Some(Self(result)),
            None => None,
        }
    }

    /// Adds two money amounts
    ///
    /// # Panics
    ///
    /// Panics if the addition would overflow.
    /// Use `checked_add` for non-panicking addition.
    #[must_use]
    #[allow(clippy::panic)]
    pub const fn add(self, other: Self) -> Self {
        match self.checked_add(other) {
            Some(result) => result,
            None => panic!("Money::add overflow"),
        }
    }

    /// Subtracts two money amounts (returns None if result would be negative)
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        if self.0 >= other.0 {
            Some(Self(self.0 - other.0))
        } else {
            None
        }
    }

    /// Multiplies money by a quantity with overflow checking
    #[must_use]
    pub const fn checked_multiply(self, quantity: u32) -> Option<Self> {
        match self.0.checked_mul(quantity as u64) {
            Some(result) => Some(Self(result)),
            None => None,
        }
    }

    /// Multiplies money by a quantity
    ///
    /// # Panics
    ///
    /// Panics if the multiplication would overflow.
    /// Use `checked_multiply` for non-panicking multiplication.
    #[must_use]
    #[allow(clippy::panic)]
    pub const fn multiply(self, quantity: u32) -> Self {
        match self.checked_multiply(quantity) {
            Some(result) => result,
            None => panic!("Money::multiply overflow"),
        }
    }

    /// Applies a percentage discount with overflow checking
    #[must_use]
    pub const fn checked_apply_discount(self, percent: u32) -> Option<Self> {
        // Check that percent doesn't cause overflow when multiplied
        let discount = match self.0.checked_mul(percent as u64) {
            Some(product) => product / 100,
            None => return None,
        };

        // Discount should never exceed the original amount
        if discount > self.0 {
            return None;
        }

        Some(Self(self.0 - discount))
    }

    /// Applies a percentage discount
    ///
    /// # Panics
    ///
    /// Panics if the calculation would overflow.
    /// Use `checked_apply_discount` for non-panicking discount.
    #[must_use]
    #[allow(clippy::panic)]
    pub const fn apply_discount(self, percent: u32) -> Self {
        match self.checked_apply_discount(percent) {
            Some(result) => result,
            None => panic!("Money::apply_discount overflow"),
        }
    }

    /// Applies a percentage markup with overflow checking
    #[must_use]
    pub const fn checked_apply_markup(self, percent: u32) -> Option<Self> {
        // Calculate markup
        let markup = match self.0.checked_mul(percent as u64) {
            Some(product) => product / 100,
            None => return None,
        };

        // Add markup to original
        match self.0.checked_add(markup) {
            Some(result) => Some(Self(result)),
            None => None,
        }
    }

    /// Applies a percentage markup
    ///
    /// # Panics
    ///
    /// Panics if the calculation would overflow.
    /// Use `checked_apply_markup` for non-panicking markup.
    #[must_use]
    #[allow(clippy::panic)]
    pub const fn apply_markup(self, percent: u32) -> Self {
        match self.checked_apply_markup(percent) {
            Some(result) => result,
            None => panic!("Money::apply_markup overflow"),
        }
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}.{:02}", self.dollars(), self.0 % 100)
    }
}

// ============================================================================
// Time Value Objects
// ============================================================================

/// Wrapper for event date with ordering and comparison
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EventDate(DateTime<Utc>);

impl EventDate {
    /// Creates a new `EventDate`
    #[must_use]
    pub const fn new(date: DateTime<Utc>) -> Self {
        Self(date)
    }

    /// Returns the inner `DateTime`
    #[must_use]
    pub const fn inner(&self) -> DateTime<Utc> {
        self.0
    }
}

impl fmt::Display for EventDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.format("%Y-%m-%d %H:%M UTC"))
    }
}

/// Wrapper for reservation expiry with ordering and comparison
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReservationExpiry(DateTime<Utc>);

impl ReservationExpiry {
    /// Creates a new `ReservationExpiry`
    #[must_use]
    pub const fn new(expiry: DateTime<Utc>) -> Self {
        Self(expiry)
    }

    /// Returns the inner `DateTime`
    #[must_use]
    pub const fn inner(&self) -> DateTime<Utc> {
        self.0
    }

    /// Checks if the reservation has expired
    #[must_use]
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.0
    }
}

impl fmt::Display for ReservationExpiry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.format("%Y-%m-%d %H:%M:%S UTC"))
    }
}

// ============================================================================
// Capacity and Seat Numbers
// ============================================================================

/// Represents capacity for a venue or section
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Capacity(pub u32);

impl Capacity {
    /// Creates a new `Capacity`
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the capacity value
    #[must_use]
    pub const fn value(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for Capacity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Represents a seat number (e.g., "A-12", "VIP-5")
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SeatNumber(String);

impl SeatNumber {
    /// Creates a new `SeatNumber`
    #[must_use]
    pub const fn new(number: String) -> Self {
        Self(number)
    }

    /// Returns the seat number as a string reference
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SeatNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================================
// Domain Entities
// ============================================================================

/// Event entity representing a concert, sports game, or conference
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Unique event identifier
    pub id: EventId,
    /// Event name (e.g., "Taylor Swift Concert")
    pub name: String,
    /// Venue information
    pub venue: Venue,
    /// Event date and time
    pub date: EventDate,
    /// Pricing tiers for tickets
    pub pricing_tiers: Vec<PricingTier>,
    /// Current event status
    pub status: EventStatus,
    /// When the event was created
    pub created_at: DateTime<Utc>,
}

impl Event {
    /// Creates a new `Event`
    #[must_use]
    pub const fn new(
        id: EventId,
        name: String,
        venue: Venue,
        date: EventDate,
        pricing_tiers: Vec<PricingTier>,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            name,
            venue,
            date,
            pricing_tiers,
            status: EventStatus::Draft,
            created_at,
        }
    }
}

/// Event lifecycle status
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventStatus {
    /// Event is being configured (not visible to public)
    Draft,
    /// Event is published (visible but sales not open)
    Published,
    /// Sales are open
    SalesOpen,
    /// Sales are closed (event approaching or sold out)
    SalesClosed,
    /// Event has completed
    Completed,
    /// Event was cancelled
    Cancelled,
}

/// Venue information
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Venue {
    /// Venue name (e.g., "Madison Square Garden")
    pub name: String,
    /// Total venue capacity
    pub capacity: Capacity,
    /// Venue sections (VIP, General, Balcony, etc.)
    pub sections: Vec<VenueSection>,
}

impl Venue {
    /// Creates a new `Venue`
    #[must_use]
    pub const fn new(name: String, capacity: Capacity, sections: Vec<VenueSection>) -> Self {
        Self {
            name,
            capacity,
            sections,
        }
    }
}

/// A section within a venue
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VenueSection {
    /// Section name (e.g., "VIP", "General", "Balcony")
    pub name: String,
    /// Section capacity
    pub capacity: Capacity,
    /// Type of seating
    pub seat_type: SeatType,
}

impl VenueSection {
    /// Creates a new `VenueSection`
    #[must_use]
    pub const fn new(name: String, capacity: Capacity, seat_type: SeatType) -> Self {
        Self {
            name,
            capacity,
            seat_type,
        }
    }
}

/// Type of seating in a section
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeatType {
    /// Numbered seats (specific seat assignments)
    Numbered {
        /// Specific seat numbers
        seats: Vec<SeatNumber>,
    },
    /// General admission (first-come, first-served)
    GeneralAdmission,
}

/// Pricing tier for tickets
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PricingTier {
    /// Type of tier (`EarlyBird`, Regular, `LastMinute`)
    pub tier_type: TierType,
    /// Section this tier applies to
    pub section: String,
    /// Base price for this tier
    pub base_price: Money,
    /// When this tier becomes available
    pub available_from: DateTime<Utc>,
    /// When this tier expires (None = never)
    pub available_until: Option<DateTime<Utc>>,
}

impl PricingTier {
    /// Creates a new `PricingTier`
    #[must_use]
    pub const fn new(
        tier_type: TierType,
        section: String,
        base_price: Money,
        available_from: DateTime<Utc>,
        available_until: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            tier_type,
            section,
            base_price,
            available_from,
            available_until,
        }
    }
}

/// Pricing tier types
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TierType {
    /// Early bird pricing (30 days before, -20%)
    EarlyBird,
    /// Regular pricing
    Regular,
    /// Last minute pricing (7 days before, +10%)
    LastMinute,
}

/// Inventory entity tracking available seats
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    /// Event this inventory belongs to
    pub event_id: EventId,
    /// Section name
    pub section: String,
    /// Total capacity
    pub total_capacity: Capacity,
    /// Currently reserved seats (not yet confirmed)
    pub reserved: u32,
    /// Sold seats (confirmed)
    pub sold: u32,
}

impl Inventory {
    /// Creates a new `Inventory`
    #[must_use]
    pub const fn new(event_id: EventId, section: String, total_capacity: Capacity) -> Self {
        Self {
            event_id,
            section,
            total_capacity,
            reserved: 0,
            sold: 0,
        }
    }

    /// Returns the number of available seats (computed, not stored)
    #[must_use]
    pub const fn available(&self) -> u32 {
        self.total_capacity.0 - self.reserved - self.sold
    }

    /// Checks if the requested quantity is available
    #[must_use]
    pub const fn has_availability(&self, quantity: u32) -> bool {
        self.available() >= quantity
    }
}

/// Seat assignment tracking
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatAssignment {
    /// Unique seat identifier
    pub seat_id: SeatId,
    /// Event this seat belongs to
    pub event_id: EventId,
    /// Section name
    pub section: String,
    /// Specific seat number (if numbered seating)
    pub seat_number: Option<SeatNumber>,
    /// Current seat status
    pub status: SeatStatus,
    /// Reservation holding this seat (if reserved)
    pub reserved_by: Option<ReservationId>,
    /// Customer who purchased this seat (if sold)
    pub sold_to: Option<CustomerId>,
}

impl SeatAssignment {
    /// Creates a new available `SeatAssignment`
    #[must_use]
    pub const fn new(
        seat_id: SeatId,
        event_id: EventId,
        section: String,
        seat_number: Option<SeatNumber>,
    ) -> Self {
        Self {
            seat_id,
            event_id,
            section,
            seat_number,
            status: SeatStatus::Available,
            reserved_by: None,
            sold_to: None,
        }
    }
}

/// Seat status
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeatStatus {
    /// Available for purchase
    Available,
    /// Reserved (temporary hold with expiry)
    Reserved {
        /// When the reservation expires
        expires_at: DateTime<Utc>,
    },
    /// Sold (permanent)
    Sold,
    /// Held by organizer
    Held,
}

/// Reservation entity (saga state)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reservation {
    /// Unique reservation identifier
    pub id: ReservationId,
    /// Event being reserved
    pub event_id: EventId,
    /// Customer making the reservation
    pub customer_id: CustomerId,
    /// Reserved seats
    pub seats: Vec<SeatId>,
    /// Total amount to pay
    pub total_amount: Money,
    /// Current reservation status
    pub status: ReservationStatus,
    /// When the reservation expires
    pub expires_at: ReservationExpiry,
    /// When the reservation was created
    pub created_at: DateTime<Utc>,
}

impl Reservation {
    /// Creates a new `Reservation`
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        id: ReservationId,
        event_id: EventId,
        customer_id: CustomerId,
        seats: Vec<SeatId>,
        total_amount: Money,
        expires_at: ReservationExpiry,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            event_id,
            customer_id,
            seats,
            total_amount,
            status: ReservationStatus::Initiated,
            expires_at,
            created_at,
        }
    }
}

/// Reservation status (saga state machine)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReservationStatus {
    /// Just created
    Initiated,
    /// Inventory locked
    SeatsReserved,
    /// Awaiting payment
    PaymentPending,
    /// Payment successful
    PaymentCompleted,
    /// Payment rejected
    PaymentFailed {
        /// Failure reason
        reason: String,
    },
    /// Tickets issued
    Completed,
    /// Timeout reached
    Expired,
    /// User cancelled
    Cancelled,
    /// Rolled back after failure
    Compensated,
}

/// Payment entity
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Payment {
    /// Unique payment identifier
    pub id: PaymentId,
    /// Reservation this payment is for
    pub reservation_id: ReservationId,
    /// Customer making the payment
    pub customer_id: CustomerId,
    /// Amount to charge
    pub amount: Money,
    /// Current payment status
    pub status: PaymentStatus,
    /// Payment method used
    pub payment_method: PaymentMethod,
    /// When the payment was processed
    pub processed_at: Option<DateTime<Utc>>,
}

impl Payment {
    /// Creates a new `Payment`
    #[must_use]
    pub const fn new(
        id: PaymentId,
        reservation_id: ReservationId,
        customer_id: CustomerId,
        amount: Money,
        payment_method: PaymentMethod,
    ) -> Self {
        Self {
            id,
            reservation_id,
            customer_id,
            amount,
            status: PaymentStatus::Pending,
            payment_method,
            processed_at: None,
        }
    }
}

/// Payment status
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentStatus {
    /// Payment initiated
    Pending,
    /// Payment authorized (funds held)
    Authorized,
    /// Payment captured (funds transferred)
    Captured,
    /// Payment failed
    Failed {
        /// Failure reason
        reason: String,
    },
    /// Payment refunded
    Refunded {
        /// Refunded amount
        amount: Money,
    },
}

/// Payment method
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentMethod {
    /// Credit card payment
    CreditCard {
        /// Last four digits of card
        last_four: String,
    },
    /// `PayPal` payment
    PayPal {
        /// `PayPal` email
        email: String,
    },
    /// Apple Pay
    ApplePay,
}

// ============================================================================
// Aggregate States
// ============================================================================

/// State for the Event aggregate
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventState {
    /// All events indexed by ID
    pub events: HashMap<EventId, Event>,
    /// Last validation error
    pub last_error: Option<String>,
}

impl EventState {
    /// Creates a new empty `EventState`
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: HashMap::new(),
            last_error: None,
        }
    }

    /// Gets an event by ID
    #[must_use]
    pub fn get(&self, id: &EventId) -> Option<&Event> {
        self.events.get(id)
    }

    /// Checks if an event exists
    #[must_use]
    pub fn exists(&self, id: &EventId) -> bool {
        self.events.contains_key(id)
    }

    /// Returns the number of events
    #[must_use]
    pub fn count(&self) -> usize {
        self.events.len()
    }
}

impl Default for EventState {
    fn default() -> Self {
        Self::new()
    }
}

/// State for the Inventory aggregate
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InventoryState {
    /// All inventories indexed by (`event_id`, section)
    pub inventories: HashMap<(EventId, String), Inventory>,
    /// All seat assignments indexed by `seat_id`
    pub seat_assignments: HashMap<SeatId, SeatAssignment>,
    /// Last validation error
    pub last_error: Option<String>,
}

impl InventoryState {
    /// Creates a new empty `InventoryState`
    #[must_use]
    pub fn new() -> Self {
        Self {
            inventories: HashMap::new(),
            seat_assignments: HashMap::new(),
            last_error: None,
        }
    }

    /// Gets inventory for an event and section
    #[must_use]
    pub fn get_inventory(&self, event_id: &EventId, section: &str) -> Option<&Inventory> {
        self.inventories.get(&(*event_id, section.to_string()))
    }

    /// Gets a seat assignment by ID
    #[must_use]
    pub fn get_seat(&self, seat_id: &SeatId) -> Option<&SeatAssignment> {
        self.seat_assignments.get(seat_id)
    }

    /// Returns the number of inventories
    #[must_use]
    pub fn count_inventories(&self) -> usize {
        self.inventories.len()
    }

    /// Returns the number of seat assignments
    #[must_use]
    pub fn count_seats(&self) -> usize {
        self.seat_assignments.len()
    }
}

impl Default for InventoryState {
    fn default() -> Self {
        Self::new()
    }
}

/// State for the Reservation saga
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReservationState {
    /// All reservations indexed by ID
    pub reservations: HashMap<ReservationId, Reservation>,
    /// Last validation error
    pub last_error: Option<String>,
}

impl ReservationState {
    /// Creates a new empty `ReservationState`
    #[must_use]
    pub fn new() -> Self {
        Self {
            reservations: HashMap::new(),
            last_error: None,
        }
    }

    /// Gets a reservation by ID
    #[must_use]
    pub fn get(&self, id: &ReservationId) -> Option<&Reservation> {
        self.reservations.get(id)
    }

    /// Checks if a reservation exists
    #[must_use]
    pub fn exists(&self, id: &ReservationId) -> bool {
        self.reservations.contains_key(id)
    }

    /// Returns the number of reservations
    #[must_use]
    pub fn count(&self) -> usize {
        self.reservations.len()
    }
}

impl Default for ReservationState {
    fn default() -> Self {
        Self::new()
    }
}

/// State for the Payment aggregate
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaymentState {
    /// All payments indexed by ID
    pub payments: HashMap<PaymentId, Payment>,
    /// Last validation error
    pub last_error: Option<String>,
}

impl PaymentState {
    /// Creates a new empty `PaymentState`
    #[must_use]
    pub fn new() -> Self {
        Self {
            payments: HashMap::new(),
            last_error: None,
        }
    }

    /// Gets a payment by ID
    #[must_use]
    pub fn get(&self, id: &PaymentId) -> Option<&Payment> {
        self.payments.get(id)
    }

    /// Checks if a payment exists
    #[must_use]
    pub fn exists(&self, id: &PaymentId) -> bool {
        self.payments.contains_key(id)
    }

    /// Returns the number of payments
    #[must_use]
    pub fn count(&self) -> usize {
        self.payments.len()
    }
}

impl Default for PaymentState {
    fn default() -> Self {
        Self::new()
    }
}
