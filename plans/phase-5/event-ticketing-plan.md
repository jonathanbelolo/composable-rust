# Event Ticketing System - Complete Implementation Plan

**Goal**: Build a production-ready, medium-size example demonstrating real-world business complexity, concurrency challenges, and sophisticated saga patterns.

**Status**: 📋 PLANNED - Ready for Implementation

**Estimated Effort**: 2-3 days (1,500-2,000 lines of code, 30-40 comprehensive tests)

---

## Table of Contents

1. [Business Requirements](#business-requirements)
2. [Architecture Overview](#architecture-overview)
3. [Domain Model](#domain-model)
4. [Aggregates Specification](#aggregates-specification)
5. [Saga Workflows](#saga-workflows)
6. [State Machines](#state-machines)
7. [Concurrency & Race Conditions](#concurrency--race-conditions)
8. [Read Models (Projections)](#read-models-projections)
9. [Testing Strategy](#testing-strategy)
10. [Implementation Phases](#implementation-phases)
11. [File Structure](#file-structure)
12. [Code Specifications](#code-specifications)

---

## Business Requirements

### Core Features

**As an Event Organizer**, I want to:
- Create events (concerts, sports, conferences) with venue layout
- Define pricing tiers (VIP, General Admission, Early Bird)
- Track available inventory per section/tier
- See real-time sales analytics
- Handle high-concurrency ticket sales (thousands per minute)

**As a Customer**, I want to:
- Browse available events
- Select specific seats or ticket tiers
- Reserve tickets with a time limit (5 minutes)
- Complete payment before timeout
- Receive ticket confirmation
- Transfer/resell tickets to others

**As the System**, I must:
- Prevent double-booking (100% accuracy)
- Handle race conditions for last tickets
- Automatically release expired reservations
- Process compensation on payment failures
- Provide eventual consistency for read models
- Scale to handle flash sales (10,000+ concurrent users)

### Business Rules

1. **Reservation Timeout**: 5 minutes to complete payment
2. **Overbooking Policy**: No overbooking (unlike airlines) - accuracy is critical
3. **Pricing Tiers**:
   - Early Bird: 30 days before event, 20% discount
   - Regular: Until 7 days before, standard price
   - Last Minute: Within 7 days, 10% markup
4. **Maximum Purchase**: 8 tickets per transaction
5. **Transfer Rules**: Tickets transferable up to 24 hours before event
6. **Cancellation Policy**:
   - Full refund: > 7 days before event
   - 50% refund: 3-7 days before
   - No refund: < 3 days before

---

## Architecture Overview

### System Components

```
┌─────────────────────────────────────────────────────────────┐
│                     EVENT TICKETING SYSTEM                   │
└─────────────────────────────────────────────────────────────┘

Write Side (Event Sourcing):
┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│    Event     │  │   Inventory  │  │ Reservation  │  │   Payment    │
│  Aggregate   │  │  Aggregate   │  │    (Saga)    │  │  Aggregate   │
└──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘
       │                 │                  │                  │
       └─────────────────┴──────────────────┴──────────────────┘
                                │
                          Event Stream
                                │
                                ▼
                       ┌─────────────────┐
                       │   Event Bus     │
                       │   (Redpanda)    │
                       └─────────────────┘
                                │
                                ▼
Read Side (Projections):
┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│   Available  │  │    Sales     │  │   Customer   │  │    Venue     │
│    Seats     │  │  Analytics   │  │   History    │  │    Layout    │
└──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘
```

### Aggregates (Write Side)

1. **Event Aggregate** - Event creation, venue, pricing tiers
2. **Inventory Aggregate** - Available seats per event/tier
3. **Reservation Saga** - Orchestrates ticket purchase with timeout
4. **Payment Aggregate** - Payment processing, refunds
5. **Customer Aggregate** - Purchase history, preferences (optional)

### Projections (Read Side)

1. **Available Seats View** - Real-time seat availability by event
2. **Sales Analytics** - Revenue, tickets sold, trends
3. **Customer History** - Past purchases, upcoming events
4. **Venue Layout** - Seat map with availability

---

## Domain Model

### Core Value Objects

```rust
// Identifiers
pub struct EventId(Uuid);
pub struct SeatId(Uuid);
pub struct ReservationId(Uuid);
pub struct PaymentId(Uuid);
pub struct CustomerId(Uuid);
pub struct TicketId(Uuid);

// Money (cents-based, no floats)
pub struct Money(u64);

impl Money {
    pub const fn from_cents(cents: u64) -> Self { Self(cents) }
    pub const fn from_dollars(dollars: u64) -> Self { Self(dollars * 100) }
    pub const fn cents(&self) -> u64 { self.0 }
}

// Time
pub struct EventDate(DateTime<Utc>);
pub struct ReservationExpiry(DateTime<Utc>);

// Capacity
pub struct Capacity(u32);
pub struct SeatNumber(String); // e.g., "A-12", "VIP-5"
```

### Domain Entities

```rust
// Event entity
pub struct Event {
    pub id: EventId,
    pub name: String,
    pub venue: Venue,
    pub date: EventDate,
    pub pricing_tiers: Vec<PricingTier>,
    pub status: EventStatus,
    pub created_at: DateTime<Utc>,
}

pub enum EventStatus {
    Draft,
    Published,
    SalesOpen,
    SalesClosed,
    Completed,
    Cancelled,
}

pub struct Venue {
    pub name: String,
    pub capacity: Capacity,
    pub sections: Vec<VenueSection>,
}

pub struct VenueSection {
    pub name: String,        // "VIP", "General", "Balcony"
    pub capacity: Capacity,
    pub seat_type: SeatType,
}

pub enum SeatType {
    Numbered { seats: Vec<SeatNumber> },  // Specific seats
    GeneralAdmission,                      // First-come, first-served
}

pub struct PricingTier {
    pub tier_type: TierType,
    pub section: String,
    pub base_price: Money,
    pub available_from: DateTime<Utc>,
    pub available_until: Option<DateTime<Utc>>,
}

pub enum TierType {
    EarlyBird,      // 30 days before, -20%
    Regular,        // Normal pricing
    LastMinute,     // 7 days before, +10%
}

// Inventory entity
pub struct Inventory {
    pub event_id: EventId,
    pub section: String,
    pub total_capacity: Capacity,
    pub reserved: u32,
    pub sold: u32,
}

impl Inventory {
    /// Returns the number of available seats (computed, not stored)
    pub fn available(&self) -> u32 {
        self.total_capacity.0 - self.reserved - self.sold
    }
}

// Reservation entity (Saga state)
pub struct Reservation {
    pub id: ReservationId,
    pub event_id: EventId,
    pub customer_id: CustomerId,
    pub seats: Vec<SeatId>,
    pub total_amount: Money,
    pub status: ReservationStatus,
    pub expires_at: ReservationExpiry,
    pub created_at: DateTime<Utc>,
}

pub enum ReservationStatus {
    Initiated,           // Just created
    SeatsReserved,       // Inventory locked
    PaymentPending,      // Awaiting payment
    PaymentCompleted,    // Payment successful
    PaymentFailed,       // Payment rejected
    Completed,           // Tickets issued
    Expired,             // Timeout reached
    Cancelled,           // User cancelled
    Compensated,         // Rolled back after failure
}

// Payment entity
pub struct Payment {
    pub id: PaymentId,
    pub reservation_id: ReservationId,
    pub customer_id: CustomerId,
    pub amount: Money,
    pub status: PaymentStatus,
    pub payment_method: PaymentMethod,
    pub processed_at: Option<DateTime<Utc>>,
}

pub enum PaymentStatus {
    Pending,
    Authorized,
    Captured,
    Failed { reason: String },
    Refunded { amount: Money },
}

pub enum PaymentMethod {
    CreditCard { last_four: String },
    PayPal { email: String },
    ApplePay,
}
```

---

## Aggregates Specification

### 1. Event Aggregate

**Responsibilities**:
- Create and configure events
- Define venue layout and capacity
- Set pricing tiers
- Manage event lifecycle (draft → published → sales open → completed)

**State**:
```rust
#[derive(State, Clone, Debug, Serialize, Deserialize)]
pub struct EventState {
    pub events: HashMap<EventId, Event>,
    pub last_error: Option<String>,
}
```

**Actions**:
```rust
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum EventAction {
    // Commands
    #[command]
    CreateEvent {
        id: EventId,
        name: String,
        venue: Venue,
        date: EventDate,
        pricing_tiers: Vec<PricingTier>,
    },

    #[command]
    PublishEvent { event_id: EventId },

    #[command]
    OpenSales { event_id: EventId },

    #[command]
    CloseSales { event_id: EventId },

    #[command]
    CancelEvent { event_id: EventId, reason: String },

    // Events
    #[event]
    EventCreated {
        id: EventId,
        name: String,
        venue: Venue,
        date: EventDate,
        pricing_tiers: Vec<PricingTier>,
        created_at: DateTime<Utc>,
    },

    #[event]
    EventPublished { event_id: EventId, published_at: DateTime<Utc> },

    #[event]
    SalesOpened { event_id: EventId, opened_at: DateTime<Utc> },

    #[event]
    SalesClosed { event_id: EventId, closed_at: DateTime<Utc> },

    #[event]
    EventCancelled { event_id: EventId, reason: String, cancelled_at: DateTime<Utc> },

    #[event]
    ValidationFailed { error: String },
}
```

**Validation Rules**:
- Event name must be non-empty (< 200 chars)
- Event date must be in future
- Venue capacity must be > 0
- At least one pricing tier required
- Pricing tiers must have positive prices
- Cannot cancel event < 24 hours before start
- Cannot open sales before event is published

### 2. Inventory Aggregate

**Responsibilities**:
- Track available inventory per event/section
- Reserve seats during purchase flow
- Release seats on timeout/cancellation
- Prevent double-booking (CRITICAL)

**State**:
```rust
#[derive(State, Clone, Debug, Serialize, Deserialize)]
pub struct InventoryState {
    pub inventories: HashMap<(EventId, String), Inventory>,  // Key: (event_id, section)
    pub seat_assignments: HashMap<SeatId, SeatAssignment>,   // Specific seat tracking
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeatAssignment {
    pub seat_id: SeatId,
    pub event_id: EventId,
    pub section: String,
    pub seat_number: Option<SeatNumber>,
    pub status: SeatStatus,
    pub reserved_by: Option<ReservationId>,
    pub sold_to: Option<CustomerId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeatStatus {
    Available,
    Reserved { expires_at: DateTime<Utc> },
    Sold,
    Held { reason: String },  // Held by organizer
}
```

**Actions**:
```rust
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum InventoryAction {
    // Commands
    #[command]
    InitializeInventory {
        event_id: EventId,
        section: String,
        capacity: Capacity,
        seat_numbers: Option<Vec<SeatNumber>>,  // None for general admission
    },

    #[command]
    ReserveSeats {
        reservation_id: ReservationId,
        event_id: EventId,
        section: String,
        quantity: u32,
        specific_seats: Option<Vec<SeatNumber>>,
        expires_at: DateTime<Utc>,
    },

    #[command]
    ConfirmReservation {
        reservation_id: ReservationId,
        customer_id: CustomerId,
    },

    #[command]
    ReleaseReservation { reservation_id: ReservationId },

    #[command]
    ExpireReservation { reservation_id: ReservationId },

    // Events
    #[event]
    InventoryInitialized {
        event_id: EventId,
        section: String,
        capacity: Capacity,
        seats: Vec<SeatId>,
        initialized_at: DateTime<Utc>,
    },

    #[event]
    SeatsReserved {
        reservation_id: ReservationId,
        event_id: EventId,
        section: String,
        seats: Vec<SeatId>,
        expires_at: DateTime<Utc>,
        reserved_at: DateTime<Utc>,
    },

    #[event]
    SeatsConfirmed {
        reservation_id: ReservationId,
        customer_id: CustomerId,
        seats: Vec<SeatId>,
        confirmed_at: DateTime<Utc>,
    },

    #[event]
    SeatsReleased {
        reservation_id: ReservationId,
        seats: Vec<SeatId>,
        released_at: DateTime<Utc>,
    },

    #[event]
    InsufficientInventory {
        event_id: EventId,
        section: String,
        requested: u32,
        available: u32,
    },

    #[event]
    ValidationFailed { error: String },
}
```

**Validation Rules**:
- Cannot reserve more seats than available
- Cannot reserve seats that are already sold
- Cannot reserve seats that are already reserved (unless expired)
- Cannot confirm reservation if seats released
- Specific seat numbers must exist in section
- Quantity must be > 0 and <= 8 (max purchase)

**Concurrency Handling** (CRITICAL):
```rust
impl InventoryReducer {
    fn reduce(&self, state: &mut InventoryState, action: InventoryAction, env: &Env)
        -> SmallVec<[Effect<InventoryAction>; 4]>
    {
        match action {
            InventoryAction::ReserveSeats {
                event_id, section, quantity, specific_seats, ..
            } => {
                let key = (event_id.clone(), section.clone());
                let inventory = state.inventories.get(&key)?;

                // CRITICAL: Check availability with reserved count
                let actually_available = inventory.total_capacity.0
                    - inventory.reserved
                    - inventory.sold;

                if actually_available < quantity {
                    // Emit InsufficientInventory event
                    let event = InventoryAction::InsufficientInventory {
                        event_id,
                        section,
                        requested: quantity,
                        available: actually_available,
                    };
                    Self::apply_event(state, &event);
                    return SmallVec::new();
                }

                // Select seats (first-available or specific)
                let seats = if let Some(specific) = specific_seats {
                    self.validate_specific_seats(state, &event_id, &section, &specific)?
                } else {
                    self.select_available_seats(state, &event_id, &section, quantity)
                };

                // Mark seats as reserved
                let event = InventoryAction::SeatsReserved {
                    reservation_id,
                    event_id,
                    section,
                    seats,
                    expires_at,
                    reserved_at: env.clock.now(),
                };

                Self::apply_event(state, &event);
                SmallVec::new()
            }
            // ... other cases
        }
    }

    // Helper: Select first N available seats
    fn select_available_seats(
        &self,
        state: &InventoryState,
        event_id: &EventId,
        section: &str,
        quantity: u32
    ) -> Vec<SeatId> {
        state.seat_assignments
            .values()
            .filter(|seat| {
                seat.event_id == *event_id
                && seat.section == section
                && seat.status == SeatStatus::Available
            })
            .take(quantity as usize)
            .map(|seat| seat.seat_id.clone())
            .collect()
    }
}
```

### 3. Reservation Saga

**Responsibilities**:
- Orchestrate ticket purchase workflow
- Coordinate inventory + payment aggregates
- Handle timeout-based expiration (5 minutes)
- Implement compensation on failures

**State**:
```rust
#[derive(State, Clone, Debug, Serialize, Deserialize)]
pub struct ReservationState {
    pub reservations: HashMap<ReservationId, Reservation>,
    pub last_error: Option<String>,
}
```

**Actions**:
```rust
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum ReservationAction {
    // Commands
    #[command]
    InitiateReservation {
        reservation_id: ReservationId,
        event_id: EventId,
        customer_id: CustomerId,
        section: String,
        quantity: u32,
        specific_seats: Option<Vec<SeatNumber>>,
    },

    #[command]
    CompletePayment {
        reservation_id: ReservationId,
        payment_id: PaymentId,
    },

    #[command]
    CancelReservation { reservation_id: ReservationId },

    #[command]
    ExpireReservation { reservation_id: ReservationId },  // Triggered by timeout

    // Events
    #[event]
    ReservationInitiated {
        reservation_id: ReservationId,
        event_id: EventId,
        customer_id: CustomerId,
        section: String,
        quantity: u32,
        expires_at: DateTime<Utc>,
        initiated_at: DateTime<Utc>,
    },

    #[event]
    SeatsAllocated {
        reservation_id: ReservationId,
        seats: Vec<SeatId>,
        total_amount: Money,
    },

    #[event]
    PaymentRequested {
        reservation_id: ReservationId,
        payment_id: PaymentId,
        amount: Money,
    },

    #[event]
    PaymentSucceeded {
        reservation_id: ReservationId,
        payment_id: PaymentId,
    },

    #[event]
    PaymentFailed {
        reservation_id: ReservationId,
        payment_id: PaymentId,
        reason: String,
    },

    #[event]
    ReservationCompleted {
        reservation_id: ReservationId,
        tickets_issued: Vec<TicketId>,
        completed_at: DateTime<Utc>,
    },

    #[event]
    ReservationExpired {
        reservation_id: ReservationId,
        expired_at: DateTime<Utc>,
    },

    #[event]
    ReservationCancelled {
        reservation_id: ReservationId,
        reason: String,
        cancelled_at: DateTime<Utc>,
    },

    #[event]
    ReservationCompensated {
        reservation_id: ReservationId,
        reason: String,
        compensated_at: DateTime<Utc>,
    },

    #[event]
    ValidationFailed { error: String },
}
```

**Saga Workflow**:
```rust
impl ReservationReducer {
    fn reduce(&self, state: &mut ReservationState, action: ReservationAction, env: &Env)
        -> SmallVec<[Effect<ReservationAction>; 4]>
    {
        match action {
            // Step 1: Initiate reservation
            ReservationAction::InitiateReservation {
                reservation_id,
                event_id,
                customer_id,
                section,
                quantity,
                specific_seats,
            } => {
                // Validate
                if quantity == 0 || quantity > 8 {
                    return self.emit_validation_error("Invalid quantity");
                }

                // Create reservation record
                let expires_at = env.clock.now() + Duration::minutes(5);
                let event = ReservationAction::ReservationInitiated {
                    reservation_id: reservation_id.clone(),
                    event_id: event_id.clone(),
                    customer_id,
                    section: section.clone(),
                    quantity,
                    expires_at,
                    initiated_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                // Effect 1: Reserve seats in Inventory aggregate
                let reserve_seats_cmd = InventoryAction::ReserveSeats {
                    reservation_id: reservation_id.clone(),
                    event_id,
                    section,
                    quantity,
                    specific_seats,
                    expires_at,
                };

                // Effect 2: Schedule expiration (delayed effect)
                let schedule_expiration = ReservationAction::ExpireReservation {
                    reservation_id,
                };

                smallvec![
                    Effect::PublishEvent(reserve_seats_cmd),
                    Effect::Delay {
                        duration: Duration::minutes(5),
                        action: schedule_expiration,
                    }
                ]
            }

            // Step 2: Inventory confirmed seats
            ReservationAction::SeatsAllocated {
                reservation_id,
                seats,
                total_amount,
            } => {
                // Update reservation with seat details
                Self::apply_event(state, &action);

                // Effect: Request payment
                let payment_id = PaymentId::new();
                let payment_event = ReservationAction::PaymentRequested {
                    reservation_id: reservation_id.clone(),
                    payment_id: payment_id.clone(),
                    amount: total_amount,
                };
                Self::apply_event(state, &payment_event);

                // Trigger payment processing
                let process_payment_cmd = PaymentAction::ProcessPayment {
                    payment_id,
                    reservation_id,
                    amount: total_amount,
                };

                smallvec![Effect::PublishEvent(process_payment_cmd)]
            }

            // Step 3a: Payment succeeded
            ReservationAction::PaymentSucceeded {
                reservation_id,
                payment_id,
            } => {
                Self::apply_event(state, &action);

                // Effect: Confirm seats in inventory (mark as sold)
                let confirm_cmd = InventoryAction::ConfirmReservation {
                    reservation_id: reservation_id.clone(),
                    customer_id: state.reservations[&reservation_id].customer_id.clone(),
                };

                // Effect: Issue tickets
                let tickets = state.reservations[&reservation_id]
                    .seats
                    .iter()
                    .map(|_| TicketId::new())
                    .collect();

                let completion_event = ReservationAction::ReservationCompleted {
                    reservation_id,
                    tickets_issued: tickets,
                    completed_at: env.clock.now(),
                };
                Self::apply_event(state, &completion_event);

                smallvec![Effect::PublishEvent(confirm_cmd)]
            }

            // Step 3b: Payment failed - COMPENSATE
            ReservationAction::PaymentFailed {
                reservation_id,
                reason,
                ..
            } => {
                Self::apply_event(state, &action);

                // COMPENSATION: Release seats back to inventory
                let release_cmd = InventoryAction::ReleaseReservation {
                    reservation_id: reservation_id.clone(),
                };

                let compensation_event = ReservationAction::ReservationCompensated {
                    reservation_id,
                    reason,
                    compensated_at: env.clock.now(),
                };
                Self::apply_event(state, &compensation_event);

                smallvec![Effect::PublishEvent(release_cmd)]
            }

            // Step 4: Timeout expired - COMPENSATE
            ReservationAction::ExpireReservation { reservation_id } => {
                // Check if still pending (not completed/cancelled)
                if let Some(reservation) = state.reservations.get(&reservation_id) {
                    if matches!(reservation.status, ReservationStatus::SeatsReserved | ReservationStatus::PaymentPending) {
                        let event = ReservationAction::ReservationExpired {
                            reservation_id: reservation_id.clone(),
                            expired_at: env.clock.now(),
                        };
                        Self::apply_event(state, &event);

                        // COMPENSATION: Release seats
                        let release_cmd = InventoryAction::ReleaseReservation {
                            reservation_id,
                        };

                        return smallvec![Effect::PublishEvent(release_cmd)];
                    }
                }

                SmallVec::new()  // Already completed, ignore
            }

            // Events (replay or from other aggregates)
            _ => {
                Self::apply_event(state, &action);
                SmallVec::new()
            }
        }
    }
}
```

### 4. Payment Aggregate

**Responsibilities**:
- Process payments
- Handle payment failures
- Issue refunds
- Track payment methods

**State**:
```rust
#[derive(State, Clone, Debug, Serialize, Deserialize)]
pub struct PaymentState {
    pub payments: HashMap<PaymentId, Payment>,
    pub last_error: Option<String>,
}
```

**Actions**:
```rust
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum PaymentAction {
    // Commands
    #[command]
    ProcessPayment {
        payment_id: PaymentId,
        reservation_id: ReservationId,
        amount: Money,
        payment_method: PaymentMethod,
    },

    #[command]
    RefundPayment {
        payment_id: PaymentId,
        amount: Money,
        reason: String,
    },

    #[command]
    SimulatePaymentFailure {
        payment_id: PaymentId,
        reservation_id: ReservationId,
        reason: String,
    },

    // Events
    #[event]
    PaymentProcessed {
        payment_id: PaymentId,
        reservation_id: ReservationId,
        amount: Money,
        payment_method: PaymentMethod,
        processed_at: DateTime<Utc>,
    },

    #[event]
    PaymentSucceeded {
        payment_id: PaymentId,
        transaction_id: String,
    },

    #[event]
    PaymentFailed {
        payment_id: PaymentId,
        reason: String,
        failed_at: DateTime<Utc>,
    },

    #[event]
    PaymentRefunded {
        payment_id: PaymentId,
        amount: Money,
        reason: String,
        refunded_at: DateTime<Utc>,
    },

    #[event]
    ValidationFailed { error: String },
}
```

**Simulation** (for demo purposes):
```rust
impl PaymentReducer {
    fn reduce(&self, state: &mut PaymentState, action: PaymentAction, env: &Env)
        -> SmallVec<[Effect<PaymentAction>; 4]>
    {
        match action {
            PaymentAction::ProcessPayment {
                payment_id,
                reservation_id,
                amount,
                payment_method,
            } => {
                // Record payment attempt
                let event = PaymentAction::PaymentProcessed {
                    payment_id: payment_id.clone(),
                    reservation_id: reservation_id.clone(),
                    amount,
                    payment_method: payment_method.clone(),
                    processed_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                // Simulate payment processing
                // In production: This would call a real payment gateway (Stripe, PayPal, etc.)
                // For demo: We'll always succeed to show the happy path
                // To test failure scenarios, add a command to simulate payment failure

                // Emit PaymentSucceeded
                let success = PaymentAction::PaymentSucceeded {
                    payment_id: payment_id.clone(),
                    transaction_id: format!("txn_{}", Uuid::new_v4()),
                };
                Self::apply_event(state, &success);

                // Notify reservation saga
                let result_event = ReservationAction::PaymentSucceeded {
                    reservation_id,
                    payment_id,
                };

                smallvec![Effect::PublishEvent(result_event)]
            }

            // Simulate payment failure (for testing compensation flows)
            PaymentAction::SimulatePaymentFailure {
                payment_id,
                reservation_id,
                reason,
            } => {
                // Emit PaymentFailed event
                let failure = PaymentAction::PaymentFailed {
                    payment_id: payment_id.clone(),
                    reason: reason.clone(),
                    failed_at: env.clock.now(),
                };
                Self::apply_event(state, &failure);

                // Notify reservation saga to trigger compensation
                let result_event = ReservationAction::PaymentFailed {
                    reservation_id,
                    payment_id,
                    reason,
                };

                smallvec![Effect::PublishEvent(result_event)]
            }

            // Refund processing
            PaymentAction::RefundPayment {
                payment_id,
                amount,
                reason,
            } => {
                // Validate payment exists and is captured
                if let Some(payment) = state.payments.get(&payment_id) {
                    if !matches!(payment.status, PaymentStatus::Captured) {
                        return self.emit_validation_error("Cannot refund uncaptured payment");
                    }
                }

                // Process refund (simulate)
                let event = PaymentAction::PaymentRefunded {
                    payment_id,
                    amount,
                    reason,
                    refunded_at: env.clock.now(),
                };
                Self::apply_event(state, &event);

                SmallVec::new()
            }

            _ => {
                Self::apply_event(state, &action);
                SmallVec::new()
            }
        }
    }
}
```

---

## Saga Workflows

### Primary Flow: Ticket Purchase

```
Initiator: Customer
Duration: 0-5 minutes
Success Rate: ~95%

Step 1: Initiate Reservation
├─→ Create reservation record
├─→ Set 5-minute expiration timer
└─→ Request seat reservation from Inventory

Step 2: Reserve Seats
├─→ Inventory checks availability
├─→ IF available:
│   ├─→ Mark seats as reserved
│   ├─→ Calculate total price
│   └─→ Emit SeatsAllocated event
└─→ IF unavailable:
    └─→ Emit InsufficientInventory event → Saga fails

Step 3: Request Payment
├─→ Create payment record
├─→ Process payment (external gateway)
└─→ Await payment result

Step 4a: Payment Success Path
├─→ Confirm seats in inventory (mark as sold)
├─→ Issue tickets
└─→ Complete reservation

Step 4b: Payment Failure Path (COMPENSATION)
├─→ Release seats back to inventory
└─→ Mark reservation as compensated

Step 5: Timeout Path (COMPENSATION)
├─→ Check if reservation still pending
├─→ IF pending:
│   ├─→ Mark reservation as expired
│   └─→ Release seats back to inventory
└─→ IF completed: Ignore (already processed)
```

### Compensation Flows

**Payment Failure Compensation**:
```
Trigger: PaymentFailed event
Actions:
1. Emit ReservationCompensated event
2. Send ReleaseReservation command to Inventory
3. Update seat status: Reserved → Available
4. Notify customer (via projection/notification service)
```

**Timeout Compensation**:
```
Trigger: 5-minute timer expires
Guard: Check reservation.status (must be pending)
Actions:
1. Emit ReservationExpired event
2. Send ReleaseReservation command to Inventory
3. Update seat status: Reserved → Available
4. Clean up reservation record
```

**Cancellation Compensation**:
```
Trigger: User cancels before payment
Actions:
1. Emit ReservationCancelled event
2. Send ReleaseReservation command to Inventory
3. Update seat status: Reserved → Available
4. Calculate refund amount (if any)
5. Process refund via Payment aggregate
```

---

## State Machines

### Reservation State Machine

```
States:
┌──────────────┐
│  Initiated   │ (Initial state)
└──────┬───────┘
       │ SeatsAllocated
       ▼
┌──────────────┐
│SeatsReserved │ (Seats locked, awaiting payment)
└──────┬───────┘
       │
       ├─── PaymentSucceeded ──────┐
       │                           ▼
       │                    ┌──────────────┐
       │                    │  Completed   │ (Final - success)
       │                    └──────────────┘
       │
       ├─── PaymentFailed ─────────┐
       │                           ▼
       │                    ┌──────────────┐
       │                    │ Compensated  │ (Final - failure)
       │                    └──────────────┘
       │
       └─── Timeout ──────────────┐
                                  ▼
                           ┌──────────────┐
                           │   Expired    │ (Final - timeout)
                           └──────────────┘

Transitions:
Initiated → SeatsReserved: When inventory confirms seats
SeatsReserved → Completed: When payment succeeds
SeatsReserved → Compensated: When payment fails
SeatsReserved → Expired: When 5-minute timer fires
```

### Seat Status State Machine

```
States:
┌──────────────┐
│  Available   │ (Initial state)
└──────┬───────┘
       │ ReserveSeats
       ▼
┌──────────────┐
│   Reserved   │ (Locked for reservation, with expiry)
└──────┬───────┘
       │
       ├─── ConfirmReservation ────┐
       │                           ▼
       │                    ┌──────────────┐
       │                    │     Sold     │ (Final)
       │                    └──────────────┘
       │
       └─── ReleaseReservation/Expire ─────┐
                                           ▼
                                    ┌──────────────┐
                                    │  Available   │ (Back to pool)
                                    └──────────────┘

Transitions:
Available → Reserved: When reservation locks seat
Reserved → Sold: When payment succeeds
Reserved → Available: When reservation released/expired
```

---

## Concurrency & Race Conditions

### Critical Race Condition: Last Seat

**Scenario**: Two customers try to reserve the last seat simultaneously.

**Without Proper Handling** (INCORRECT):
```rust
// BAD: Race condition possible
let available = inventory.total_capacity - inventory.sold;
if available >= quantity {
    // ⚠️ Another reducer could execute here!
    inventory.reserved += quantity;
    // Double-booking possible!
}
```

**With Proper Handling** (CORRECT):
```rust
// GOOD: Atomically check and reserve
let actually_available = inventory.total_capacity
    - inventory.reserved  // ← Include reserved count
    - inventory.sold;

if actually_available < quantity {
    // Not enough seats - emit InsufficientInventory
    return Err(InsufficientInventory {
        requested: quantity,
        available: actually_available
    });
}

// Safe to reserve
inventory.reserved += quantity;
```

**Why This Works**:
- Reducer executes atomically (single-threaded per aggregate)
- Reserved count prevents double-booking during concurrent reservations
- Event sourcing ensures all state changes are recorded
- Optimistic concurrency: let one win, others get InsufficientInventory

### Testing Concurrency

**Property-Based Test**:
```rust
#[test]
fn property_never_oversell_seats() {
    proptest!(|(
        reservations: Vec<ReservationRequest>,  // Random concurrent reservations
        capacity: u32 in 10..100,
    )| {
        let mut state = InventoryState::with_capacity(capacity);
        let reducer = InventoryReducer::new();

        // Simulate concurrent reservations
        for request in reservations {
            reducer.reduce(&mut state, request.into_action(), &env);
        }

        // Invariant: Never oversell
        let total_allocated = state.inventory.reserved + state.inventory.sold;
        assert!(total_allocated <= capacity,
            "Oversold! Allocated {} seats but capacity is {}",
            total_allocated, capacity);
    });
}
```

**Stress Test**:
```rust
#[tokio::test]
async fn stress_test_last_seat_contention() {
    let capacity = 100;
    let concurrent_requests = 150;  // Intentionally more than capacity

    let inventory_store = /* ... */;

    // Launch 150 concurrent reservation attempts
    let tasks: Vec<_> = (0..concurrent_requests)
        .map(|i| {
            let store = inventory_store.clone();
            tokio::spawn(async move {
                store.send(ReserveSeats {
                    reservation_id: ReservationId::new(),
                    event_id: test_event_id(),
                    section: "General".to_string(),
                    quantity: 1,
                    specific_seats: None,
                    expires_at: Utc::now() + Duration::minutes(5),
                }).await
            })
        })
        .collect();

    let results = join_all(tasks).await;

    // Count successes
    let successes = results.iter()
        .filter(|r| matches!(r, Ok(Ok(_))))
        .count();

    // Invariant: Exactly capacity succeeded, rest failed
    assert_eq!(successes, capacity as usize,
        "Expected {} successful reservations but got {}", capacity, successes);

    // Verify final state
    let final_state = inventory_store.state(|s| s.clone()).await;
    assert_eq!(final_state.inventory.reserved, capacity);
}
```

---

## Read Models (Projections)

### 1. Available Seats Projection

**Purpose**: Real-time seat availability for UI

**Schema**:
```sql
CREATE TABLE available_seats (
    event_id UUID NOT NULL,
    section TEXT NOT NULL,
    total_capacity INTEGER NOT NULL,
    available INTEGER NOT NULL,
    reserved INTEGER NOT NULL,
    sold INTEGER NOT NULL,
    last_updated TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (event_id, section)
);

CREATE INDEX idx_available_seats_event ON available_seats(event_id);
```

**Projection Logic**:
```rust
impl Projection for AvailableSeatsProjection {
    type Event = InventoryAction;

    fn name(&self) -> &str {
        "available_seats"
    }

    async fn apply_event(&self, event: &Self::Event) -> Result<()> {
        match event {
            InventoryAction::InventoryInitialized { event_id, section, capacity, .. } => {
                sqlx::query(
                    "INSERT INTO available_seats
                     (event_id, section, total_capacity, available, reserved, sold, last_updated)
                     VALUES ($1, $2, $3, $3, 0, 0, NOW())
                     ON CONFLICT (event_id, section) DO NOTHING"
                )
                .bind(event_id.as_uuid())
                .bind(section)
                .bind(capacity.0 as i32)
                .execute(&self.pool)
                .await?;
            }

            InventoryAction::SeatsReserved { event_id, section, seats, .. } => {
                let count = seats.len() as i32;
                sqlx::query(
                    "UPDATE available_seats
                     SET available = available - $3,
                         reserved = reserved + $3,
                         last_updated = NOW()
                     WHERE event_id = $1 AND section = $2"
                )
                .bind(event_id.as_uuid())
                .bind(section)
                .bind(count)
                .execute(&self.pool)
                .await?;
            }

            InventoryAction::SeatsConfirmed { seats, .. } => {
                let count = seats.len() as i32;
                // Move from reserved to sold
                sqlx::query(
                    "UPDATE available_seats
                     SET reserved = reserved - $3,
                         sold = sold + $3,
                         last_updated = NOW()
                     WHERE event_id = $1 AND section = $2"
                )
                .bind(event_id.as_uuid())
                .bind(section)
                .bind(count)
                .execute(&self.pool)
                .await?;
            }

            InventoryAction::SeatsReleased { event_id, section, seats, .. } => {
                let count = seats.len() as i32;
                // Return to available pool
                sqlx::query(
                    "UPDATE available_seats
                     SET available = available + $3,
                         reserved = reserved - $3,
                         last_updated = NOW()
                     WHERE event_id = $1 AND section = $2"
                )
                .bind(event_id.as_uuid())
                .bind(section)
                .bind(count)
                .execute(&self.pool)
                .await?;
            }

            _ => {}  // Ignore other events
        }

        Ok(())
    }
}
```

**Query API**:
```rust
impl AvailableSeatsProjection {
    pub async fn get_availability(&self, event_id: &EventId) -> Result<Vec<SectionAvailability>> {
        sqlx::query_as(
            "SELECT section, total_capacity, available, reserved, sold
             FROM available_seats
             WHERE event_id = $1
             ORDER BY section"
        )
        .bind(event_id.as_uuid())
        .fetch_all(&self.pool)
        .await
    }

    pub async fn check_availability(
        &self,
        event_id: &EventId,
        section: &str,
        quantity: u32
    ) -> Result<bool> {
        let row: (i32,) = sqlx::query_as(
            "SELECT available FROM available_seats
             WHERE event_id = $1 AND section = $2"
        )
        .bind(event_id.as_uuid())
        .bind(section)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0 >= quantity as i32)
    }
}
```

### 2. Sales Analytics Projection

**Purpose**: Revenue tracking, tickets sold, trends

**Schema**:
```sql
CREATE TABLE sales_analytics (
    event_id UUID NOT NULL,
    date DATE NOT NULL,
    tickets_sold INTEGER NOT NULL DEFAULT 0,
    revenue_cents BIGINT NOT NULL DEFAULT 0,
    refunds_cents BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (event_id, date)
);

CREATE INDEX idx_sales_event ON sales_analytics(event_id);
CREATE INDEX idx_sales_date ON sales_analytics(date);
```

### 3. Customer History Projection

**Purpose**: Past purchases, upcoming events

**Schema**:
```sql
CREATE TABLE customer_purchases (
    customer_id UUID NOT NULL,
    reservation_id UUID NOT NULL,
    event_id UUID NOT NULL,
    event_name TEXT NOT NULL,
    event_date TIMESTAMPTZ NOT NULL,
    tickets_count INTEGER NOT NULL,
    total_paid_cents BIGINT NOT NULL,
    purchased_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (customer_id, reservation_id)
);

CREATE INDEX idx_customer_history ON customer_purchases(customer_id, event_date);
```

---

## Testing Strategy

### Unit Tests (Reducer-Level)

**Event Aggregate**:
- ✅ Create event with valid data
- ✅ Reject empty event name
- ✅ Reject past event date
- ✅ Reject zero capacity
- ✅ Publish event transitions state
- ✅ Cannot cancel < 24 hours before

**Inventory Aggregate**:
- ✅ Initialize inventory with capacity
- ✅ Reserve seats decrements available
- ✅ Reject reservation exceeding capacity
- ✅ Confirm reservation moves to sold
- ✅ Release reservation returns to available
- ✅ Cannot reserve already-sold seats
- ✅ Expire reservations automatically

**Reservation Saga**:
- ✅ Happy path: Initiate → Reserve → Pay → Complete
- ✅ Payment failure triggers compensation
- ✅ Timeout triggers compensation
- ✅ Cannot exceed max quantity (8 tickets)
- ✅ Expiration timer is set correctly
- ✅ Completed reservations ignore timeout

**Payment Aggregate**:
- ✅ Process payment successfully
- ✅ Process payment failure
- ✅ Refund captured payment
- ✅ Cannot refund uncaptured payment

### Integration Tests

**Saga Integration**:
```rust
#[tokio::test]
async fn test_full_reservation_flow() {
    let event_id = EventId::new();
    let customer_id = CustomerId::new();

    // Setup: Create event and initialize inventory
    event_store.send(CreateEvent { /* ... */ }).await?;
    inventory_store.send(InitializeInventory {
        event_id: event_id.clone(),
        section: "General".to_string(),
        capacity: Capacity(100),
        seat_numbers: None,
    }).await?;

    // Act: Initiate reservation
    let reservation_id = ReservationId::new();
    reservation_store.send(InitiateReservation {
        reservation_id: reservation_id.clone(),
        event_id: event_id.clone(),
        customer_id: customer_id.clone(),
        section: "General".to_string(),
        quantity: 2,
        specific_seats: None,
    }).await?;

    // Wait for saga to complete
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Assert: Reservation completed
    let res_state = reservation_store.state(|s| s.clone()).await;
    let reservation = res_state.reservations.get(&reservation_id).unwrap();
    assert_eq!(reservation.status, ReservationStatus::Completed);

    // Assert: Inventory updated
    let inv_state = inventory_store.state(|s| s.clone()).await;
    let inventory = inv_state.inventories.get(&(event_id, "General".to_string())).unwrap();
    assert_eq!(inventory.sold, 2);
    assert_eq!(inventory.reserved, 0);
}
```

**Timeout Test**:
```rust
#[tokio::test]
async fn test_reservation_expires_after_timeout() {
    // Setup
    let reservation_id = ReservationId::new();

    // Use FixedClock for deterministic time
    let clock = Arc::new(FixedClock::new(Utc::now()));
    let env = ReservationEnvironment { clock: clock.clone() };

    // Initiate reservation
    reservation_store.send(InitiateReservation { /* ... */ }).await?;

    // Advance time by 5 minutes
    clock.advance(Duration::minutes(5));

    // Trigger expiration check (in real system, automated)
    reservation_store.send(ExpireReservation { reservation_id }).await?;

    // Assert: Reservation expired
    let state = reservation_store.state(|s| s.clone()).await;
    let reservation = state.reservations.get(&reservation_id).unwrap();
    assert_eq!(reservation.status, ReservationStatus::Expired);

    // Assert: Seats released back to inventory
    let inv_state = inventory_store.state(|s| s.clone()).await;
    assert_eq!(inv_state.inventory.reserved, 0);
    assert_eq!(inv_state.inventory.available, 100);
}
```

### Concurrency Tests

**Last Seat Contention**:
```rust
#[tokio::test]
async fn test_last_seat_race_condition() {
    let capacity = 1;  // Only 1 seat

    // Setup inventory with 1 seat
    inventory_store.send(InitializeInventory {
        event_id: event_id.clone(),
        section: "VIP".to_string(),
        capacity: Capacity(capacity),
        seat_numbers: None,
    }).await?;

    // Launch 10 concurrent reservation attempts
    let tasks: Vec<_> = (0..10)
        .map(|i| {
            let store = inventory_store.clone();
            let event_id = event_id.clone();
            tokio::spawn(async move {
                store.send(ReserveSeats {
                    reservation_id: ReservationId::new(),
                    event_id,
                    section: "VIP".to_string(),
                    quantity: 1,
                    specific_seats: None,
                    expires_at: Utc::now() + Duration::minutes(5),
                }).await
            })
        })
        .collect();

    let results = join_all(tasks).await;

    // Count successes and failures
    let successes = results.iter().filter(|r| matches!(r, Ok(Ok(_)))).count();
    let failures = results.iter().filter(|r| matches!(r, Ok(Err(_)))).count();

    // Assertions
    assert_eq!(successes, 1, "Exactly 1 should succeed");
    assert_eq!(failures, 9, "Exactly 9 should fail");

    // Verify no oversell
    let state = inventory_store.state(|s| s.clone()).await;
    assert_eq!(state.inventory.reserved, 1);
    assert_eq!(state.inventory.available, 0);
}
```

### Property-Based Tests

**Invariant: Never Oversell**:
```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn prop_never_oversell(
        capacity in 10u32..100,
        reservations in prop::collection::vec(
            (1u32..8, any::<bool>()),  // (quantity, succeeds)
            10..50
        )
    ) {
        let mut state = InventoryState::with_capacity(capacity);
        let reducer = InventoryReducer::new();
        let env = test_env();

        let mut successful_reservations = Vec::new();

        for (quantity, _) in reservations {
            let action = ReserveSeats {
                reservation_id: ReservationId::new(),
                event_id: test_event_id(),
                section: "General".to_string(),
                quantity,
                specific_seats: None,
                expires_at: Utc::now() + Duration::minutes(5),
            };

            let effects = reducer.reduce(&mut state, action.clone(), &env);

            // Track successful reservations
            if !effects.iter().any(|e| matches!(e, Effect::Event(InventoryAction::InsufficientInventory { .. }))) {
                successful_reservations.push(quantity);
            }
        }

        // Invariant: Total allocated never exceeds capacity
        let total_allocated = state.inventory.reserved + state.inventory.sold;
        prop_assert!(
            total_allocated <= capacity,
            "Oversold! capacity={}, allocated={}", capacity, total_allocated
        );
    }
}
```

### Performance Tests

**Throughput Benchmark**:
```rust
#[bench]
fn bench_reservation_throughput(b: &mut Bencher) {
    let rt = Runtime::new().unwrap();
    let store = /* setup store */;

    b.iter(|| {
        rt.block_on(async {
            store.send(ReserveSeats { /* ... */ }).await
        })
    });
}
```

---

## Implementation Phases

### Phase 1: Core Aggregates (Day 1 Morning)

**Deliverables** (4 hours):
1. Create project structure: `examples/ticketing/`
2. Implement `types.rs` with all domain types
3. Implement `Event` aggregate
4. Implement `Inventory` aggregate
5. Unit tests for both aggregates (15 tests)

**Success Criteria**:
- Can create events
- Can initialize inventory
- Can reserve/release seats
- All validation works
- Tests pass

### Phase 2: Reservation Saga (Day 1 Afternoon)

**Deliverables** (4 hours):
1. Implement `Reservation` saga reducer
2. Implement `Payment` aggregate (simplified)
3. Wire up saga workflow
4. Implement timeout mechanism
5. Implement compensation flows
6. Integration tests (10 tests)

**Success Criteria**:
- Full reservation flow works
- Timeout triggers compensation
- Payment failure triggers compensation
- Saga tests pass

### Phase 3: Concurrency & Edge Cases (Day 2 Morning)

**Deliverables** (4 hours):
1. Implement concurrency handling in inventory
2. Add property-based tests
3. Add stress tests (last seat contention)
4. Handle all edge cases
5. Add comprehensive error handling

**Success Criteria**:
- Last seat race condition handled correctly
- Property tests pass (never oversell)
- Stress test shows correct behavior
- All edge cases covered

### Phase 4: Read Models (Day 2 Afternoon)

**Deliverables** (3 hours):
1. Implement `AvailableSeatsProjection`
2. Implement `SalesAnalyticsProjection`
3. Implement `CustomerHistoryProjection`
4. Add projection tests (5 tests)
5. Verify eventual consistency

**Success Criteria**:
- Projections update from events
- Query APIs work
- Tests demonstrate eventual consistency

### Phase 5: CLI Demo & Documentation (Day 3)

**Deliverables** (6 hours):
1. Build interactive CLI application
2. Add comprehensive README with:
   - Architecture diagrams
   - Usage examples
   - Key concepts explained
3. Add inline documentation
4. Code review and polish
5. Final testing

**Success Criteria**:
- CLI demonstrates all features
- README is comprehensive
- Code is well-documented
- All tests pass (30-40 total)
- Example is production-quality

---

## File Structure

```
examples/ticketing/
├── Cargo.toml
├── README.md                    # Comprehensive documentation
├── src/
│   ├── lib.rs                  # Library entry point with docs
│   ├── types.rs                # All domain types (400 lines)
│   ├── aggregates/
│   │   ├── mod.rs
│   │   ├── event.rs            # Event aggregate (200 lines)
│   │   ├── inventory.rs        # Inventory aggregate (300 lines)
│   │   ├── reservation.rs      # Reservation saga (400 lines)
│   │   └── payment.rs          # Payment aggregate (150 lines)
│   ├── projections/
│   │   ├── mod.rs
│   │   ├── available_seats.rs  # Seat availability (150 lines)
│   │   ├── sales_analytics.rs  # Sales metrics (100 lines)
│   │   └── customer_history.rs # Purchase history (100 lines)
│   └── main.rs                 # Interactive CLI demo (200 lines)
├── tests/
│   ├── unit/
│   │   ├── event_tests.rs      # Event aggregate tests
│   │   ├── inventory_tests.rs  # Inventory tests
│   │   ├── reservation_tests.rs # Saga tests
│   │   └── payment_tests.rs    # Payment tests
│   ├── integration/
│   │   ├── full_flow_tests.rs  # End-to-end scenarios
│   │   ├── timeout_tests.rs    # Timeout/expiration
│   │   └── concurrency_tests.rs # Race conditions
│   └── properties/
│       └── invariant_tests.rs  # Property-based tests
└── docs/
    ├── architecture.md         # System architecture
    └── saga-patterns.md        # Saga pattern explanation
```

**Estimated Total**: 1,500-2,000 lines of production code + 500-800 lines of tests

---

## Code Specifications

### Error Handling

**Validation Errors**:
```rust
#[derive(Debug, thiserror::Error)]
pub enum TicketingError {
    #[error("Event not found: {0}")]
    EventNotFound(EventId),

    #[error("Insufficient inventory: requested {requested}, available {available}")]
    InsufficientInventory { requested: u32, available: u32 },

    #[error("Reservation expired: {0}")]
    ReservationExpired(ReservationId),

    #[error("Invalid quantity: must be 1-8, got {0}")]
    InvalidQuantity(u32),

    #[error("Event date must be in future")]
    PastEventDate,

    #[error("Cannot modify event less than 24 hours before start")]
    TooCloseToEvent,
}
```

### Logging & Observability

**Critical Operations**:
```rust
impl InventoryReducer {
    fn reduce(&self, state: &mut InventoryState, action: InventoryAction, env: &Env)
        -> SmallVec<[Effect<InventoryAction>; 4]>
    {
        match action {
            InventoryAction::ReserveSeats { reservation_id, quantity, .. } => {
                tracing::info!(
                    reservation_id = %reservation_id,
                    quantity = quantity,
                    "Attempting seat reservation"
                );

                let actually_available = /* calculate */;

                if actually_available < quantity {
                    tracing::warn!(
                        reservation_id = %reservation_id,
                        requested = quantity,
                        available = actually_available,
                        "Insufficient inventory for reservation"
                    );
                    // Emit InsufficientInventory event
                }

                tracing::info!(
                    reservation_id = %reservation_id,
                    quantity = quantity,
                    "Seats reserved successfully"
                );

                // ...
            }
        }
    }
}
```

### Performance Considerations

**Optimizations**:
1. **Inventory Lookup**: HashMap for O(1) section lookup
2. **Seat Selection**: Early exit when capacity reached
3. **Projection Updates**: Batch updates where possible
4. **Read Models**: Indexed queries for common paths
5. **Event Serialization**: Use bincode for performance

**Benchmarks**:
- Reservation latency: < 10ms (p99)
- Inventory check: < 1ms (p99)
- Projection update: < 5ms (p99)
- Concurrent throughput: > 1000 req/sec

---

## Success Criteria

### Functional Requirements ✅

- ✅ Create events with venue and pricing
- ✅ Initialize inventory per section
- ✅ Reserve seats with timeout
- ✅ Process payment (simulated)
- ✅ Confirm or release seats based on payment
- ✅ Automatic timeout-based release
- ✅ Compensation on failures
- ✅ No double-booking (race condition handled)
- ✅ Read models for queries

### Non-Functional Requirements ✅

- ✅ 30-40 comprehensive tests
- ✅ Property-based tests for invariants
- ✅ Stress tests for concurrency
- ✅ Comprehensive documentation
- ✅ Production-quality code
- ✅ Interactive CLI demo
- ✅ Demonstrates real-world complexity

### Educational Value ✅

- ✅ Shows time-based sagas (timeout)
- ✅ Shows compensation patterns
- ✅ Shows concurrency handling
- ✅ Shows saga coordination
- ✅ Shows read model projections
- ✅ Shows realistic business rules
- ✅ Shows production patterns

---

## Conclusion

This plan provides a complete blueprint for a production-ready Event Ticketing system that demonstrates:

1. **Real-World Complexity**: Multiple aggregates, sophisticated workflows
2. **Saga Pattern**: Time-based workflows with compensation
3. **Concurrency**: Race condition handling, stress testing
4. **CQRS**: Separate read models with eventual consistency
5. **Production Quality**: Comprehensive testing, error handling, observability

**Estimated Effort**: 2-3 days for complete, polished implementation

**Next Step**: Begin Phase 1 (Core Aggregates) 🚀
