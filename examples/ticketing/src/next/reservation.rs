//! Reservation query aggregate for the next-generation architecture.
//!
//! This module provides the [`ReservationQueryLogic`] which implements
//! [`BusinessLogic`] for querying reservation data.
//!
//! # Note
//!
//! This is separate from [`ReservationSagaBusinessLogic`](super::ReservationSagaBusinessLogic)
//! which handles the reservation workflow (create, cancel, etc.). This aggregate
//! is purely for read operations.
//!
//! # Commands
//!
//! - [`ReservationQueryCommand::GetReservation`] - Get a single reservation by ID
//! - [`ReservationQueryCommand::ListUserReservations`] - List all reservations for a user

use chrono::{DateTime, Utc};
use composable_rust_next::{BusinessLogic, BusinessResult, Clock, StreamId};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;

use crate::types::{CustomerId, EventId, Money, ReservationId, ReservationStatus};

// ═══════════════════════════════════════════════════════════════════════════
// DTOs (Data Transfer Objects)
// ═══════════════════════════════════════════════════════════════════════════

/// Reservation details DTO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReservationDto {
    /// Reservation ID
    pub id: ReservationId,
    /// Event ID
    pub event_id: EventId,
    /// Customer ID
    pub customer_id: CustomerId,
    /// Section name
    pub section: String,
    /// Number of tickets
    pub quantity: u32,
    /// Current status
    pub status: ReservationStatus,
    /// Total amount
    pub total_amount: Money,
    /// When the reservation expires (if pending)
    pub expires_at: Option<DateTime<Utc>>,
    /// When the reservation was created
    pub created_at: DateTime<Utc>,
    /// When the reservation was completed (if applicable)
    pub completed_at: Option<DateTime<Utc>>,
}

/// Reservation summary for list views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReservationSummaryDto {
    /// Reservation ID
    pub id: ReservationId,
    /// Event ID
    pub event_id: EventId,
    /// Section name
    pub section: String,
    /// Number of tickets
    pub quantity: u32,
    /// Current status
    pub status: ReservationStatus,
    /// Total amount
    pub total_amount: Money,
    /// When created
    pub created_at: DateTime<Utc>,
}

/// List reservations response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReservationListDto {
    /// List of reservations
    pub reservations: Vec<ReservationSummaryDto>,
    /// Total count
    pub total: usize,
}

// ═══════════════════════════════════════════════════════════════════════════
// Commands (Input)
// ═══════════════════════════════════════════════════════════════════════════

/// Reservation query commands.
#[derive(Debug, Clone)]
pub enum ReservationQueryCommand {
    /// Get a single reservation by ID.
    ///
    /// Public endpoint - anyone can check reservation status.
    GetReservation {
        /// Reservation ID to get
        reservation_id: ReservationId,
        /// Pre-fetched reservation data (populated by QueryFetcher)
        fetched: Option<ReservationDto>,
    },

    /// List all reservations for a user.
    ///
    /// Requires authentication.
    ListUserReservations {
        /// Customer ID to list reservations for
        customer_id: CustomerId,
        /// Requesting user ID (for ownership verification)
        requesting_user_id: CustomerId,
        /// Pre-fetched reservations (populated by QueryFetcher)
        fetched: Option<ReservationListDto>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Events (Output)
// ═══════════════════════════════════════════════════════════════════════════

/// Reservation query events.
///
/// Queries are read-only, so events are minimal - just for audit logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReservationQueryEvent {
    /// Reservation was queried.
    ReservationQueried {
        /// Reservation ID
        reservation_id: ReservationId,
        /// When queried
        queried_at: DateTime<Utc>,
    },

    /// User reservations were listed.
    UserReservationsListed {
        /// Customer ID
        customer_id: CustomerId,
        /// Count returned
        count: usize,
        /// When queried
        queried_at: DateTime<Utc>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Response Types
// ═══════════════════════════════════════════════════════════════════════════

/// Reservation query responses.
#[derive(Debug, Clone)]
pub enum ReservationQueryResponse {
    /// Single reservation response.
    Reservation(ReservationDto),
    /// List of reservations response.
    ReservationList(ReservationListDto),
}

// ═══════════════════════════════════════════════════════════════════════════
// Errors
// ═══════════════════════════════════════════════════════════════════════════

/// Reservation query errors.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ReservationQueryError {
    /// Reservation not found.
    #[error("Reservation not found: {reservation_id}")]
    ReservationNotFound {
        /// The reservation ID that was not found.
        reservation_id: ReservationId,
    },

    /// Access denied - user doesn't own this resource.
    #[error("Access denied: user {requesting_user_id} cannot access reservations for {customer_id}")]
    AccessDenied {
        /// The customer ID whose reservations are being accessed.
        customer_id: CustomerId,
        /// The user ID making the request.
        requesting_user_id: CustomerId,
    },

    /// Data not fetched - QueryFetcher didn't populate the data.
    #[error("Data not fetched for query")]
    DataNotFetched,
}

// ═══════════════════════════════════════════════════════════════════════════
// State
// ═══════════════════════════════════════════════════════════════════════════

/// Reservation query state.
///
/// Query operations are stateless, so state is minimal.
#[derive(Debug, Clone, Default)]
pub struct ReservationQueryState {
    /// Placeholder to satisfy the trait
    _placeholder: (),
}

impl ReservationQueryState {
    /// Create a new reservation query state.
    #[must_use]
    pub const fn new() -> Self {
        Self { _placeholder: () }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Business Logic
// ═══════════════════════════════════════════════════════════════════════════

/// Reservation query business logic.
///
/// Handles validation and returns pre-fetched data from projections.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReservationQueryLogic;

impl BusinessLogic for ReservationQueryLogic {
    type State = ReservationQueryState;
    type Input = ReservationQueryCommand;
    type Event = ReservationQueryEvent;
    type Error = ReservationQueryError;
    type Call = Infallible; // No external calls
    type CallResult = Infallible; // No call results
    type Response = ReservationQueryResponse;

    fn stream_id(input: &Self::Input) -> StreamId {
        // Queries don't persist events to a meaningful stream
        match input {
            ReservationQueryCommand::GetReservation { reservation_id, .. } => {
                StreamId::new(format!("reservation-query-{}", reservation_id.as_uuid()))
            }
            ReservationQueryCommand::ListUserReservations { customer_id, .. } => {
                StreamId::new(format!("reservation-query-user-{}", customer_id.as_uuid()))
            }
        }
    }

    fn event_type_name(event: &Self::Event) -> &'static str {
        match event {
            ReservationQueryEvent::ReservationQueried { .. } => "ReservationQueried",
            ReservationQueryEvent::UserReservationsListed { .. } => "UserReservationsListed",
        }
    }

    fn process(
        &self,
        input: Self::Input,
        _clock: &dyn Clock,
    ) -> Result<BusinessResult<Self::Event, Self::Call, Self::Response>, Self::Error> {
        match input {
            ReservationQueryCommand::GetReservation {
                reservation_id,
                fetched,
            } => {
                let data = fetched.ok_or(ReservationQueryError::DataNotFetched)?;

                // Validate we have valid data (check if ID matches)
                if data.id != reservation_id {
                    return Err(ReservationQueryError::ReservationNotFound { reservation_id });
                }

                Ok(BusinessResult::Respond(
                    ReservationQueryResponse::Reservation(data),
                ))
            }

            ReservationQueryCommand::ListUserReservations {
                customer_id,
                requesting_user_id,
                fetched,
            } => {
                // Verify ownership (users can only see their own reservations)
                // TODO: Add admin override check
                if customer_id != requesting_user_id {
                    return Err(ReservationQueryError::AccessDenied {
                        customer_id,
                        requesting_user_id,
                    });
                }

                let data = fetched.ok_or(ReservationQueryError::DataNotFetched)?;

                Ok(BusinessResult::Respond(
                    ReservationQueryResponse::ReservationList(data),
                ))
            }
        }
    }

    fn apply(&self, _state: &mut Self::State, _event: &Self::Event) {
        // Queries are read-only, no state changes
    }

    fn feedback_input(results: Vec<Self::CallResult>) -> Self::Input {
        // No calls - Infallible means this is never called
        match results.first() {
            Some(infallible) => match *infallible {},
            None => unreachable!("ReservationQueryLogic has no external calls"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_next::FixedClock;

    fn test_clock() -> FixedClock {
        FixedClock::new(Utc::now())
    }

    #[test]
    fn get_reservation_succeeds_with_data() {
        let logic = ReservationQueryLogic;
        let clock = test_clock();

        let reservation_id = ReservationId::new();
        let customer_id = CustomerId::new();
        let event_id = EventId::new();

        let reservation = ReservationDto {
            id: reservation_id,
            event_id,
            customer_id,
            section: "VIP".to_string(),
            quantity: 2,
            status: ReservationStatus::PaymentPending,
            total_amount: Money::from_cents(10000),
            expires_at: Some(Utc::now() + chrono::Duration::minutes(5)),
            created_at: Utc::now(),
            completed_at: None,
        };

        let input = ReservationQueryCommand::GetReservation {
            reservation_id,
            fetched: Some(reservation),
        };

        let result = logic.process(input, &clock);
        assert!(result.is_ok());

        match result.unwrap() {
            BusinessResult::Respond(ReservationQueryResponse::Reservation(data)) => {
                assert_eq!(data.id, reservation_id);
                assert_eq!(data.quantity, 2);
            }
            _ => panic!("Expected Respond with Reservation"),
        }
    }

    #[test]
    fn get_reservation_fails_without_data() {
        let logic = ReservationQueryLogic;
        let clock = test_clock();

        let input = ReservationQueryCommand::GetReservation {
            reservation_id: ReservationId::new(),
            fetched: None,
        };

        let result = logic.process(input, &clock);
        assert!(matches!(
            result,
            Err(ReservationQueryError::DataNotFetched)
        ));
    }

    #[test]
    fn list_user_reservations_fails_for_wrong_user() {
        let logic = ReservationQueryLogic;
        let clock = test_clock();

        let customer_id = CustomerId::new();
        let other_user = CustomerId::new();

        let input = ReservationQueryCommand::ListUserReservations {
            customer_id,
            requesting_user_id: other_user,
            fetched: Some(ReservationListDto {
                reservations: vec![],
                total: 0,
            }),
        };

        let result = logic.process(input, &clock);
        assert!(matches!(
            result,
            Err(ReservationQueryError::AccessDenied { .. })
        ));
    }

    #[test]
    fn list_user_reservations_succeeds_for_owner() {
        let logic = ReservationQueryLogic;
        let clock = test_clock();

        let customer_id = CustomerId::new();

        let input = ReservationQueryCommand::ListUserReservations {
            customer_id,
            requesting_user_id: customer_id, // Same user
            fetched: Some(ReservationListDto {
                reservations: vec![],
                total: 0,
            }),
        };

        let result = logic.process(input, &clock);
        assert!(result.is_ok());
    }
}
