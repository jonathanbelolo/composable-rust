//! Seat availability query endpoints.
//!
//! Provides read-only queries against the available seats projection:
//! - GET /api/events/:id/availability - Get availability for all sections
//! - GET /api/events/:id/sections/:section/availability - Get availability for specific section

use crate::server::state::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use composable_rust_web::error::AppError;
use serde::Serialize;
use uuid::Uuid;

// ============================================================================
// Response Types
// ============================================================================

/// Seat availability for a single section.
#[derive(Debug, Serialize)]
pub struct SectionAvailability {
    /// Section identifier
    pub section: String,
    /// Total capacity
    pub total_capacity: i32,
    /// Currently reserved seats (pending payment)
    pub reserved: i32,
    /// Sold seats (payment confirmed)
    pub sold: i32,
    /// Available seats (total - reserved - sold)
    pub available: i32,
}

/// Response for event availability query.
#[derive(Debug, Serialize)]
pub struct EventAvailabilityResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Availability by section
    pub sections: Vec<SectionAvailability>,
    /// Total available across all sections
    pub total_available: i32,
}

// ============================================================================
// Handlers
// ============================================================================

/// Get seat availability for all sections of an event.
///
/// Public endpoint - queries the available seats projection.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/events/550e8400-e29b-41d4-a716-446655440000/availability
/// ```
///
/// Response:
/// ```json
/// {
///   "event_id": "550e8400-e29b-41d4-a716-446655440000",
///   "sections": [
///     {
///       "section": "VIP",
///       "total_capacity": 100,
///       "reserved": 10,
///       "sold": 50,
///       "available": 40
///     },
///     {
///       "section": "General",
///       "total_capacity": 500,
///       "reserved": 50,
///       "sold": 200,
///       "available": 250
///     }
///   ],
///   "total_available": 290
/// }
/// ```
pub async fn get_event_availability(
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<EventAvailabilityResponse>, AppError> {
    // Query projection for all sections
    let sections = state
        .available_seats_projection
        .get_all_sections(&crate::types::EventId::from_uuid(event_id))
        .await
        .map_err(|e| AppError::internal(format!("Failed to query availability: {e}")))?;

    // If no sections found, event doesn't exist or has no inventory
    if sections.is_empty() {
        return Err(AppError::not_found("Event", event_id));
    }

    // Convert to response format
    let response_sections: Vec<SectionAvailability> = sections
        .into_iter()
        .map(|s| SectionAvailability {
            section: s.section,
            total_capacity: s.total_capacity,
            reserved: s.reserved,
            sold: s.sold,
            available: s.available,
        })
        .collect();

    // Calculate total available
    let total_available: i32 = response_sections.iter().map(|s| s.available).sum();

    Ok(Json(EventAvailabilityResponse {
        event_id,
        sections: response_sections,
        total_available,
    }))
}

/// Get seat availability for a specific section.
///
/// Public endpoint - queries the available seats projection.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/events/550e8400-e29b-41d4-a716-446655440000/sections/VIP/availability
/// ```
///
/// Response:
/// ```json
/// {
///   "section": "VIP",
///   "total_capacity": 100,
///   "reserved": 10,
///   "sold": 50,
///   "available": 40
/// }
/// ```
pub async fn get_section_availability(
    Path((event_id, section)): Path<(Uuid, String)>,
    State(state): State<AppState>,
) -> Result<Json<SectionAvailability>, AppError> {
    // Query projection for specific section
    let section_data = state
        .available_seats_projection
        .get_availability(&crate::types::EventId::from_uuid(event_id), &section)
        .await
        .map_err(|e| AppError::internal(format!("Failed to query availability: {e}")))?
        .ok_or_else(|| {
            AppError::not_found("Section", format!("{event_id}/{section}"))
        })?;

    // Convert tuple to response format
    let (total_capacity, reserved, sold, available) = section_data;

    #[allow(clippy::cast_possible_wrap)] // Counts fit in i32 range
    Ok(Json(SectionAvailability {
        section,
        total_capacity: total_capacity as i32,
        reserved: reserved as i32,
        sold: sold as i32,
        available: available as i32,
    }))
}

/// Get total available seats across all sections.
///
/// Public endpoint - efficient aggregation query.
///
/// # Example
///
/// ```bash
/// curl http://localhost:8080/api/events/550e8400-e29b-41d4-a716-446655440000/total-available
/// ```
///
/// Response:
/// ```json
/// {
///   "event_id": "550e8400-e29b-41d4-a716-446655440000",
///   "total_available": 290
/// }
/// ```
#[derive(Debug, Serialize)]
pub struct TotalAvailableResponse {
    /// Event ID
    pub event_id: Uuid,
    /// Total available seats
    pub total_available: i32,
}

pub async fn get_total_available(
    Path(event_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<TotalAvailableResponse>, AppError> {
    let total = state
        .available_seats_projection
        .get_total_available(&crate::types::EventId::from_uuid(event_id))
        .await
        .map_err(|e| AppError::internal(format!("Failed to query total availability: {e}")))?;

    #[allow(clippy::cast_possible_wrap)] // Counts fit in i32 range
    Ok(Json(TotalAvailableResponse {
        event_id,
        total_available: total as i32,
    }))
}
