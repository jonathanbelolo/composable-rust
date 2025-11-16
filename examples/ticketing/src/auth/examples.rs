//! Examples of using authentication middleware in handlers.
//!
//! This module demonstrates how to use the authentication extractors
//! in Axum handlers for protected routes.

#![allow(dead_code)] // Example code
#![allow(missing_docs)] // Example code
#![allow(clippy::missing_errors_doc, clippy::unused_async)] // Example code
#![allow(clippy::double_ended_iterator_last)] // Example code

use crate::auth::middleware::{SessionUser, RequireAdmin, RequireOwnership, ResourceId};
use crate::auth::setup::TicketingAuthStore;
use composable_rust_auth::state::UserId;
use composable_rust_web::error::AppError;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ============================================================================
// Example 1: Public endpoint (no authentication required)
// ============================================================================

#[derive(Serialize)]
pub struct HealthResponse {
    status: String,
}

/// Public health check endpoint - no authentication required.
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

// ============================================================================
// Example 2: Authenticated endpoint (requires valid session)
// ============================================================================

#[derive(Serialize)]
pub struct ProfileResponse {
    user_id: UserId,
    message: String,
}

/// Get user profile - requires authentication.
///
/// The `SessionUser` extractor automatically:
/// 1. Extracts bearer token from `Authorization` header
/// 2. Validates the session via the auth reducer
/// 3. Returns 401 Unauthorized if invalid
pub async fn get_profile(
    session: SessionUser,
) -> Result<Json<ProfileResponse>, AppError> {
    Ok(Json(ProfileResponse {
        user_id: session.user_id,
        message: "Profile retrieved successfully".to_string(),
    }))
}

// ============================================================================
// Example 3: Admin-only endpoint
// ============================================================================

#[derive(Serialize)]
pub struct AdminDashboardResponse {
    total_users: usize,
    total_events: usize,
}

/// Admin dashboard - requires admin role.
///
/// The `RequireAdmin` extractor:
/// 1. Validates session (like `SessionUser`)
/// 2. Checks admin role (placeholder for demo)
/// 3. Returns 403 Forbidden if not admin
pub async fn admin_dashboard(
    _admin: RequireAdmin,
    State(_store): State<Arc<TicketingAuthStore>>,
) -> Result<Json<AdminDashboardResponse>, AppError> {
    // In production, you would query actual stats from the database
    Ok(Json(AdminDashboardResponse {
        total_users: 100,
        total_events: 50,
    }))
}

// ============================================================================
// Example 4: Resource ownership verification
// ============================================================================

/// Event resource ID for ownership verification.
#[derive(Debug, Clone)]
pub struct EventId(pub Uuid);

impl ResourceId for EventId {
    fn from_path(path: &str) -> Option<Self> {
        // Extract UUID from path like "/api/v1/events/{event_id}"
        path.split('/')
            .last()
            .and_then(|s| Uuid::parse_str(s).ok())
            .map(Self)
    }

    async fn verify_ownership(
        &self,
        user_id: &UserId,
        _store: &Arc<TicketingAuthStore>,
    ) -> Result<(), AppError> {
        // In production, you would:
        // 1. Query the database to get the event's owner_id
        // 2. Compare owner_id with user_id
        // 3. Return Err(AppError::forbidden()) if mismatch

        // Placeholder: Allow all for demo
        let _ = user_id;
        Ok(())
    }
}

#[derive(Deserialize)]
pub struct UpdateEventRequest {
    title: String,
    description: String,
}

#[derive(Serialize)]
pub struct UpdateEventResponse {
    event_id: Uuid,
    message: String,
}

/// Update event - requires ownershipof the event.
///
/// The `RequireOwnership<EventId>` extractor:
/// 1. Validates session
/// 2. Extracts `event_id` from path
/// 3. Verifies the user owns this event
/// 4. Returns 403 Forbidden if not owner
pub async fn update_event(
    Path(event_id): Path<Uuid>,
    ownership: RequireOwnership<EventId>,
    Json(request): Json<UpdateEventRequest>,
) -> Result<(StatusCode, Json<UpdateEventResponse>), AppError> {
    // At this point, we know:
    // - User is authenticated (session is valid)
    // - User owns this event (ownership verified)

    let _ = ownership; // ownership.user_id is the authenticated owner
    let _ = request; // request.title, request.description

    // Perform the update...
    Ok((
        StatusCode::OK,
        Json(UpdateEventResponse {
            event_id,
            message: "Event updated successfully".to_string(),
        }),
    ))
}

// ============================================================================
// Example 5: Combining multiple extractors
// ============================================================================

#[derive(Serialize)]
pub struct AdminEventResponse {
    event_id: Uuid,
    owner_id: UserId,
    can_delete: bool,
}

/// Admin view of event - requires both admin role and ownership verification.
///
/// This demonstrates using multiple extractors in the same handler.
pub async fn admin_view_event(
    _admin: RequireAdmin,
    Path(event_id): Path<Uuid>,
    ownership: RequireOwnership<EventId>,
) -> Result<Json<AdminEventResponse>, AppError> {
    // Both admin check and ownership verification passed
    Ok(Json(AdminEventResponse {
        event_id,
        owner_id: ownership.user_id,
        can_delete: true,
    }))
}

// ============================================================================
// Example 6: Custom authorization logic
// ============================================================================

#[derive(Serialize)]
pub struct SensitiveDataResponse {
    data: String,
}

/// Get sensitive data - custom authorization logic.
///
/// Sometimes you need custom logic beyond the standard extractors.
/// Use `SessionUser` for authentication, then add custom checks.
pub async fn get_sensitive_data(
    session: SessionUser,
    State(_store): State<Arc<TicketingAuthStore>>,
) -> Result<Json<SensitiveDataResponse>, AppError> {
    // Custom authorization logic
    // Example: Check session freshness
    let now = chrono::Utc::now();
    let session_age = now - session.session.created_at;
    if session_age > chrono::Duration::hours(24) {
        return Err(AppError::forbidden("Session too old - please re-authenticate"));
    }

    // Example: Check if session is close to expiration
    let time_until_expiry = session.session.expires_at - now;
    if time_until_expiry < chrono::Duration::minutes(5) {
        return Err(AppError::forbidden("Session expiring soon - please refresh"));
    }

    Ok(Json(SensitiveDataResponse {
        data: "Sensitive information".to_string(),
    }))
}
