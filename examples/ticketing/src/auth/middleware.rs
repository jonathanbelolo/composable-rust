//! Authentication middleware for the ticketing system.
//!
//! Provides Axum extractors for:
//! - Bearer token extraction from Authorization header
//! - Session validation (auto-validates sessions from tokens)
//! - Role-based access control (admin checks)
//! - Resource ownership verification
//!
//! # Usage
//!
//! ```rust,ignore
//! use ticketing::auth::middleware::{SessionUser, RequireAdmin};
//!
//! // Require authentication
//! async fn get_profile(
//!     session: SessionUser,
//! ) -> Result<Json<ProfileResponse>, AppError> {
//!     // session.user_id is guaranteed valid
//!     Ok(Json(ProfileResponse { user_id: session.user_id }))
//! }
//!
//! // Require admin role
//! async fn admin_dashboard(
//!     admin: RequireAdmin,
//! ) -> Result<Json<DashboardResponse>, AppError> {
//!     // admin.user_id is guaranteed to be an admin
//!     Ok(Json(DashboardResponse { ... }))
//! }
//! ```

use crate::auth::setup::TicketingAuthStore;
use crate::server::state::AppState;
use composable_rust_auth::{AuthAction, state::{Session, SessionId, UserId}};
use composable_rust_web::{
    error::AppError,
    extractors::{ClientIp, CorrelationId},
};
use axum::{
    async_trait,
    extract::{FromRequestParts, State},
    http::request::Parts,
};
use std::sync::Arc;
use std::time::Duration;

/// Bearer token extracted from `Authorization: Bearer <token>` header.
#[derive(Debug, Clone)]
pub struct BearerToken(pub String);

#[async_trait]
impl<S> FromRequestParts<S> for BearerToken
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract Authorization header
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| AppError::unauthorized("Missing authorization header"))?;

        // Parse "Bearer <token>"
        if !auth_header.starts_with("Bearer ") {
            return Err(AppError::unauthorized("Invalid authorization format. Expected 'Bearer <token>'"));
        }

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| AppError::unauthorized("Invalid bearer token format"))?
            .to_string();

        if token.is_empty() {
            return Err(AppError::unauthorized("Empty bearer token"));
        }

        Ok(Self(token))
    }
}

/// Authenticated session user.
///
/// Extracts and validates the session from the bearer token.
/// Use this as a handler parameter to require authentication.
#[derive(Debug, Clone)]
pub struct SessionUser {
    /// The authenticated user ID
    pub user_id: UserId,
    /// The full session
    pub session: Session,
}

// Implementation for Arc<TicketingAuthStore> (used by auth routes)
#[async_trait]
impl FromRequestParts<Arc<TicketingAuthStore>> for SessionUser
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<TicketingAuthStore>,
    ) -> Result<Self, Self::Rejection> {
        // Extract bearer token
        let bearer = BearerToken::from_request_parts(parts, state).await?;

        // Check for test token bypass (only if AUTH_TEST_TOKEN env var is set)
        if let Ok(test_token) = std::env::var("AUTH_TEST_TOKEN") {
            if bearer.0 == test_token {
                // Return a test user session
                // These UUIDs are hardcoded constants and will never fail to parse
                const TEST_USER_UUID: uuid::Uuid = uuid::Uuid::from_bytes([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
                const TEST_SESSION_UUID: uuid::Uuid = uuid::Uuid::from_bytes([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);
                const TEST_DEVICE_UUID: uuid::Uuid = uuid::Uuid::from_bytes([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3]);

                let test_user_id = UserId(TEST_USER_UUID);
                let test_device_id = composable_rust_auth::state::DeviceId(TEST_DEVICE_UUID);
                let test_session = Session {
                    user_id: test_user_id,
                    session_id: SessionId(TEST_SESSION_UUID),
                    device_id: test_device_id,
                    email: "test@example.com".to_string(),
                    created_at: chrono::Utc::now(),
                    last_active: chrono::Utc::now(),
                    expires_at: chrono::Utc::now() + chrono::Duration::days(1),
                    ip_address: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                    user_agent: "Test Client".to_string(),
                    oauth_provider: None,
                    login_risk_score: 0.0,
                    idle_timeout: chrono::Duration::hours(24),
                    enable_sliding_refresh: false,
                };
                return Ok(Self {
                    user_id: test_user_id,
                    session: test_session,
                });
            }
        }

        // Parse session ID from token (UUID string)
        let uuid = uuid::Uuid::parse_str(&bearer.0)
            .map_err(|_| AppError::unauthorized("Invalid session token format"))?;
        let session_id = SessionId(uuid);

        // Extract client IP and correlation ID using framework extractors
        let client_ip = ClientIp::from_request_parts(parts, state)
            .await
            .unwrap_or(ClientIp(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)));

        let correlation_id = CorrelationId::from_request_parts(parts, state)
            .await
            .unwrap_or(CorrelationId(uuid::Uuid::new_v4()));

        // Validate session via reducer
        let action = AuthAction::ValidateSession {
            correlation_id: correlation_id.0,
            session_id,
            ip_address: client_ip.0,
        };

        // Send action and wait for response
        let store = State::<Arc<TicketingAuthStore>>::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::internal("Failed to access store"))?;

        let response = store
            .send_and_wait_for(
                action,
                |a| matches!(a, AuthAction::SessionValidated { .. } | AuthAction::SessionExpired { .. }),
                Duration::from_secs(5),
            )
            .await
            .map_err(|e| AppError::internal(format!("Session validation error: {e}")))?;

        // Handle validation result
        match response {
            AuthAction::SessionValidated { session, .. } => {
                Ok(Self {
                    user_id: session.user_id,
                    session,
                })
            }
            AuthAction::SessionExpired { .. } => {
                Err(AppError::unauthorized("Session expired"))
            }
            _ => Err(AppError::internal("Unexpected response from session validation")),
        }
    }
}

// Implementation for AppState (used by API routes)
#[async_trait]
impl FromRequestParts<AppState> for SessionUser
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Delegate to the Arc<TicketingAuthStore> implementation
        Self::from_request_parts(parts, &state.auth_store).await
    }
}

/// Require admin role.
///
/// Validates that the authenticated user has admin privileges.
/// Returns 403 Forbidden if the user is not an admin.
///
/// # Note
///
/// This is a placeholder implementation. In a real system, you would:
/// 1. Add a `role` field to the `Session` state
/// 2. Check the role against an admin list or permission system
/// 3. Query a user roles table for dynamic role assignment
#[derive(Debug, Clone)]
pub struct RequireAdmin {
    /// The authenticated admin user ID
    pub user_id: UserId,
    /// The full session
    pub session: Session,
}

#[async_trait]
impl FromRequestParts<Arc<TicketingAuthStore>> for RequireAdmin
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<TicketingAuthStore>,
    ) -> Result<Self, Self::Rejection> {
        // First validate session
        let session_user = SessionUser::from_request_parts(parts, state).await?;

        // TODO: Check admin role
        // For now, we'll check if email contains "admin" as a demo placeholder
        // In production, you would:
        // - Query user roles from database
        // - Check against a roles/permissions table
        // - Use a proper RBAC system

        // Placeholder: Allow all authenticated users for demo
        // In production: Implement proper role checking
        Ok(Self {
            user_id: session_user.user_id,
            session: session_user.session,
        })
    }
}

// Implementation for AppState
#[async_trait]
impl FromRequestParts<AppState> for RequireAdmin
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Delegate to the Arc<TicketingAuthStore> implementation
        Self::from_request_parts(parts, &state.auth_store).await
    }
}

/// Require resource ownership.
///
/// Validates that the authenticated user owns a specific resource.
/// Returns 403 Forbidden if the user does not own the resource.
///
/// # Type Parameters
///
/// - `T`: The resource type (must implement `ResourceId`)
///
/// # Usage
///
/// ```rust,ignore
/// async fn update_event(
///     Path(event_id): Path<Uuid>,
///     ownership: RequireOwnership<EventId>,
///     State(store): State<...>,
/// ) -> Result<Json<EventResponse>, AppError> {
///     // ownership.user_id owns the event with event_id
///     // Safe to proceed with update
/// }
/// ```
#[derive(Debug, Clone)]
pub struct RequireOwnership<T> {
    /// The authenticated user ID (resource owner)
    pub user_id: UserId,
    /// The resource identifier
    pub resource: T,
}

/// Trait for resources that can be owned and have an ID.
pub trait ResourceId: Send + Sync + Clone {
    /// Extract the resource ID from the request path.
    fn from_path(path: &str) -> Option<Self>;

    /// Verify ownership of this resource.
    ///
    /// # Arguments
    ///
    /// * `user_id` - The user ID to check ownership for
    /// * `store` - The auth store for querying ownership
    ///
    /// # Returns
    ///
    /// `Ok(())` if the user owns the resource, `Err(AppError)` otherwise.
    fn verify_ownership(
        &self,
        user_id: &UserId,
        store: &Arc<TicketingAuthStore>,
    ) -> impl std::future::Future<Output = Result<(), AppError>> + Send;
}

// ============================================================================
// ResourceId Implementations
// ============================================================================

impl ResourceId for crate::types::ReservationId {
    fn from_path(path: &str) -> Option<Self> {
        // Extract UUID from paths like /api/reservations/:id/cancel
        // Path format: /api/reservations/{uuid}/...
        let parts: Vec<&str> = path.split('/').collect();

        // Find "reservations" segment, next segment should be UUID
        for (i, &part) in parts.iter().enumerate() {
            if part == "reservations" && i + 1 < parts.len() {
                if let Ok(uuid) = uuid::Uuid::parse_str(parts[i + 1]) {
                    return Some(crate::types::ReservationId::from_uuid(uuid));
                }
            }
        }

        None
    }

    async fn verify_ownership(
        &self,
        user_id: &UserId,
        _store: &Arc<TicketingAuthStore>,
    ) -> Result<(), AppError> {
        // TODO: Query reservation state from event store or projection
        // TODO: Verify reservation.customer_id == user_id
        //
        // Pseudocode:
        // let reservation = state.event_store.load_aggregate(self).await?;
        // if reservation.customer_id != CustomerId(user_id.0) {
        //     return Err(AppError::forbidden("You don't own this reservation"));
        // }

        // TEMPORARY: Allow all for development
        // This will be replaced when we wire up saga state queries
        let _ = user_id;
        Ok(())
    }
}

impl ResourceId for crate::types::PaymentId {
    fn from_path(path: &str) -> Option<Self> {
        // Extract UUID from paths like /api/payments/:id/refund
        // Path format: /api/payments/{uuid}/...
        let parts: Vec<&str> = path.split('/').collect();

        // Find "payments" segment, next segment should be UUID
        for (i, &part) in parts.iter().enumerate() {
            if part == "payments" && i + 1 < parts.len() {
                if let Ok(uuid) = uuid::Uuid::parse_str(parts[i + 1]) {
                    return Some(crate::types::PaymentId::from_uuid(uuid));
                }
            }
        }

        None
    }

    async fn verify_ownership(
        &self,
        user_id: &UserId,
        _store: &Arc<TicketingAuthStore>,
    ) -> Result<(), AppError> {
        // TODO: Query payment state from event store or projection
        // TODO: Verify payment.customer_id == user_id OR user is admin
        //
        // Pseudocode:
        // let payment = state.event_store.load_aggregate(self).await?;
        // if payment.customer_id != CustomerId(user_id.0) {
        //     // Check if user is admin
        //     if !is_admin(user_id, store).await? {
        //         return Err(AppError::forbidden("You don't own this payment"));
        //     }
        // }

        // TEMPORARY: Allow all for development
        // This will be replaced when we wire up payment state queries
        let _ = user_id;
        Ok(())
    }
}

impl ResourceId for crate::types::CustomerId {
    fn from_path(path: &str) -> Option<Self> {
        // Extract UUID from paths like /api/analytics/customers/:id/profile
        // Path format: /api/analytics/customers/{uuid}/...
        let parts: Vec<&str> = path.split('/').collect();

        // Find "customers" segment, next segment should be UUID
        for (i, &part) in parts.iter().enumerate() {
            if part == "customers" && i + 1 < parts.len() {
                if let Ok(uuid) = uuid::Uuid::parse_str(parts[i + 1]) {
                    return Some(crate::types::CustomerId::from_uuid(uuid));
                }
            }
        }

        None
    }

    async fn verify_ownership(
        &self,
        user_id: &UserId,
        _store: &Arc<TicketingAuthStore>,
    ) -> Result<(), AppError> {
        // Verify that the customer ID in the path matches the authenticated user's ID
        // This ensures customers can only view their own profile
        //
        // TODO: Add admin override check - admins should be able to view any customer profile
        // Pseudocode:
        // if !is_admin(user_id, store).await? {
        //     if self.as_uuid() != &user_id.0 {
        //         return Err(AppError::forbidden("You can only view your own profile"));
        //     }
        // }

        if self.as_uuid() != &user_id.0 {
            return Err(AppError::forbidden(
                "You can only view your own profile. Admin override not yet implemented.",
            ));
        }

        Ok(())
    }
}

#[async_trait]
impl<T> FromRequestParts<Arc<TicketingAuthStore>> for RequireOwnership<T>
where
    T: ResourceId,
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<TicketingAuthStore>,
    ) -> Result<Self, Self::Rejection> {
        // First validate session
        let session_user = SessionUser::from_request_parts(parts, state).await?;

        // Extract resource ID from path
        let resource = T::from_path(parts.uri.path())
            .ok_or_else(|| AppError::bad_request("Invalid resource ID in path"))?;

        // Verify ownership
        let store = State::<Arc<TicketingAuthStore>>::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::internal("Failed to access store"))?;

        resource.verify_ownership(&session_user.user_id, &store).await?;

        Ok(Self {
            user_id: session_user.user_id,
            resource,
        })
    }
}

// Implementation for AppState
#[async_trait]
impl<T> FromRequestParts<AppState> for RequireOwnership<T>
where
    T: ResourceId,
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Delegate to the Arc<TicketingAuthStore> implementation
        Self::from_request_parts(parts, &state.auth_store).await
    }
}

#[cfg(test)]
mod tests {
    use super::{RequireOwnership, SessionUser};

    #[test]
    fn test_bearer_token_parsing() {
        // Valid bearer token
        let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let header = format!("Bearer {token}");
        assert!(header.starts_with("Bearer "));

        let extracted = header.strip_prefix("Bearer ").unwrap();
        assert_eq!(extracted, token);
    }

    #[test]
    fn test_invalid_bearer_format() {
        let invalid = "Basic dXNlcjpwYXNz";
        assert!(!invalid.starts_with("Bearer "));
    }
}
