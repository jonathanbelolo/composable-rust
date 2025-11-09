//! Application state for Axum handlers.
//!
//! This module provides a generic `AppState` that can be customized per application.
//! Domain-specific stores should be added by the application using this crate.

/// Application state shared across all HTTP handlers.
///
/// This is a placeholder type. Applications should define their own state struct
/// containing their domain-specific `Store` instances.
///
/// # Examples
///
/// ```ignore
/// use axum::{extract::State, Json};
/// use composable_rust_web::AppError;
/// use composable_rust_runtime::Store;
/// use std::sync::Arc;
///
/// // Define your app-specific state
/// struct MyAppState {
///     auth_store: Arc<Store<AuthState, AuthAction, AuthEnv, AuthReducer>>,
///     orders_store: Arc<Store<OrderState, OrderAction, OrderEnv, OrderReducer>>,
/// }
///
/// async fn handler(
///     State(state): State<Arc<MyAppState>>,
/// ) -> Result<Json<Response>, AppError> {
///     // Use your stores
///     state.auth_store.send(action).await?;
///     Ok(Json(response))
/// }
/// ```
#[derive(Clone)]
pub struct AppState {
    // Placeholder - applications should define their own state
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    /// Create a new application state.
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_is_clone() {
        // Ensure AppState implements Clone (required for Axum)
        fn assert_clone<T: Clone>() {}
        assert_clone::<AppState>();
    }

    #[test]
    fn test_state_default() {
        let _ = AppState::default();
    }
}
