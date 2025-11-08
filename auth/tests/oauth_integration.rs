//! Integration tests for OAuth authentication flow.

use composable_rust_auth::{
    actions::AuthAction,
    environment::AuthEnvironment,
    mocks::{
        MockDeviceRepository, MockEmailProvider, MockOAuth2Provider, MockRiskCalculator,
        MockSessionStore, MockUserRepository, MockWebAuthnProvider,
    },
    reducers::oauth::OAuthReducer,
    state::{AuthState, OAuthProvider},
};
use composable_rust_core::reducer::Reducer;
use std::net::{IpAddr, Ipv4Addr};

/// Create a test environment with mock providers.
fn create_test_env() -> AuthEnvironment<
    MockOAuth2Provider,
    MockEmailProvider,
    MockWebAuthnProvider,
    MockSessionStore,
    MockUserRepository,
    MockDeviceRepository,
    MockRiskCalculator,
> {
    AuthEnvironment::new(
        MockOAuth2Provider::new(),
        MockEmailProvider::new(),
        MockWebAuthnProvider::new(),
        MockSessionStore::new(),
        MockUserRepository::new(),
        MockDeviceRepository::new(),
        MockRiskCalculator::new(),
    )
}

/// Create a test reducer.
fn create_test_reducer() -> OAuthReducer<
    MockOAuth2Provider,
    MockEmailProvider,
    MockWebAuthnProvider,
    MockSessionStore,
    MockUserRepository,
    MockDeviceRepository,
    MockRiskCalculator,
> {
    OAuthReducer::new("https://app.example.com".to_string())
}

#[tokio::test]
async fn test_oauth_flow_complete_happy_path() {
    // Setup
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Step 1: Initiate OAuth
    let effects = reducer.reduce(
        &mut state,
        AuthAction::InitiateOAuth {
            provider: OAuthProvider::Google,
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    // Should have OAuth state stored
    assert!(state.oauth_state.is_some());
    let oauth_state = state.oauth_state.as_ref().unwrap();
    assert_eq!(oauth_state.provider, OAuthProvider::Google);
    assert!(!oauth_state.state_param.is_empty());

    // Should return effect to build authorization URL
    assert_eq!(effects.len(), 1);

    // Simulate executing the effect (in real code, Store would do this)
    // For now, we just verify the state was set

    // Step 2: OAuth callback with valid state
    let valid_state = oauth_state.state_param.clone();
    let effects = reducer.reduce(
        &mut state,
        AuthAction::OAuthCallback {
            code: "test_auth_code_123".to_string(),
            state: valid_state,
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    // OAuth state should be cleared (one-time use)
    assert!(state.oauth_state.is_none());

    // Should return effect to exchange code
    assert_eq!(effects.len(), 1);

    // Step 3: OAuth success (simulating successful token exchange)
    let effects = reducer.reduce(
        &mut state,
        AuthAction::OAuthSuccess {
            email: "test@example.com".to_string(),
            name: Some("Test User".to_string()),
            provider: OAuthProvider::Google,
            access_token: "mock_access_token".to_string(),
            refresh_token: Some("mock_refresh_token".to_string()),
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    // Session should be created in state
    assert!(state.session.is_some());
    let session = state.session.as_ref().unwrap();
    assert_eq!(session.email, "test@example.com");
    assert_eq!(session.oauth_provider, Some(OAuthProvider::Google));
    assert_eq!(session.ip_address, test_ip);

    // Should return effect to create user/device/session
    assert_eq!(effects.len(), 1);
}

#[tokio::test]
async fn test_oauth_callback_rejects_invalid_csrf_state() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Step 1: Initiate OAuth
    let _ = reducer.reduce(
        &mut state,
        AuthAction::InitiateOAuth {
            provider: OAuthProvider::Google,
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    let original_state = state.oauth_state.clone();
    assert!(original_state.is_some());

    // Step 2: Try callback with INVALID state (CSRF attack simulation)
    let effects = reducer.reduce(
        &mut state,
        AuthAction::OAuthCallback {
            code: "test_auth_code_123".to_string(),
            state: "invalid_csrf_state_12345".to_string(), // Wrong state!
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    // OAuth state should be cleared (security: reject and clear)
    assert!(state.oauth_state.is_none());

    // Should return effect indicating failure
    assert_eq!(effects.len(), 1);

    // No session should be created
    assert!(state.session.is_none());
}

#[tokio::test]
async fn test_oauth_callback_requires_prior_initiation() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Try callback WITHOUT initiating OAuth first
    let effects = reducer.reduce(
        &mut state,
        AuthAction::OAuthCallback {
            code: "test_auth_code_123".to_string(),
            state: "some_state".to_string(),
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    // Should reject (no OAuth state exists)
    assert!(state.oauth_state.is_none());
    assert_eq!(effects.len(), 1);

    // No session should be created
    assert!(state.session.is_none());
}

#[tokio::test]
async fn test_oauth_state_expires_after_5_minutes() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Step 1: Initiate OAuth
    let _ = reducer.reduce(
        &mut state,
        AuthAction::InitiateOAuth {
            provider: OAuthProvider::Google,
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    let oauth_state = state.oauth_state.as_ref().unwrap();
    let valid_state = oauth_state.state_param.clone();

    // Simulate time passing by manually setting initiated_at to 6 minutes ago
    if let Some(ref mut oauth_state) = state.oauth_state {
        oauth_state.initiated_at = chrono::Utc::now() - chrono::Duration::minutes(6);
    }

    // Step 2: Try callback with expired state
    let effects = reducer.reduce(
        &mut state,
        AuthAction::OAuthCallback {
            code: "test_auth_code_123".to_string(),
            state: valid_state,
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    // OAuth state should be cleared
    assert!(state.oauth_state.is_none());

    // Should return effect indicating expiration
    assert_eq!(effects.len(), 1);

    // No session should be created
    assert!(state.session.is_none());
}

#[tokio::test]
async fn test_oauth_failed_clears_state() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Step 1: Initiate OAuth
    let _ = reducer.reduce(
        &mut state,
        AuthAction::InitiateOAuth {
            provider: OAuthProvider::Google,
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    assert!(state.oauth_state.is_some());

    // Step 2: Simulate OAuth failure
    let effects = reducer.reduce(
        &mut state,
        AuthAction::OAuthFailed {
            error: "access_denied".to_string(),
            error_description: Some("User denied access".to_string()),
        },
        &env,
    );

    // OAuth state should be cleared
    assert!(state.oauth_state.is_none());

    // Should return effect (likely redirect to error page)
    assert_eq!(effects.len(), 1);

    // No session should be created
    assert!(state.session.is_none());
}

#[tokio::test]
async fn test_multiple_oauth_providers() {
    let reducer = create_test_reducer();
    let env = create_test_env();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Test each OAuth provider
    for provider in [
        OAuthProvider::Google,
        OAuthProvider::GitHub,
        OAuthProvider::Microsoft,
    ] {
        let mut state = AuthState::default();

        let effects = reducer.reduce(
            &mut state,
            AuthAction::InitiateOAuth {
                provider,
                ip_address: test_ip,
                user_agent: test_user_agent.clone(),
            },
            &env,
        );

        // Should store correct provider
        assert!(state.oauth_state.is_some());
        assert_eq!(state.oauth_state.as_ref().unwrap().provider, provider);
        assert_eq!(effects.len(), 1);
    }
}

#[tokio::test]
async fn test_session_created_event() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Create a session first
    let _ = reducer.reduce(
        &mut state,
        AuthAction::OAuthSuccess {
            email: "test@example.com".to_string(),
            name: Some("Test User".to_string()),
            provider: OAuthProvider::Google,
            access_token: "mock_access_token".to_string(),
            refresh_token: Some("mock_refresh_token".to_string()),
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    let session = state.session.clone().unwrap();

    // Handle SessionCreated event
    let effects = reducer.reduce(
        &mut state,
        AuthAction::SessionCreated {
            session: session.clone(),
        },
        &env,
    );

    // Session should still be in state
    assert!(state.session.is_some());
    assert_eq!(state.session.as_ref().unwrap().session_id, session.session_id);

    // Should return no additional effects (final event)
    assert_eq!(effects.len(), 1); // Effect::None
}

#[tokio::test]
async fn test_csrf_state_uniqueness() {
    let reducer = create_test_reducer();
    let env = create_test_env();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Generate multiple CSRF states
    let mut states = Vec::new();
    for _ in 0..10 {
        let mut state = AuthState::default();
        let _ = reducer.reduce(
            &mut state,
            AuthAction::InitiateOAuth {
                provider: OAuthProvider::Google,
                ip_address: test_ip,
                user_agent: test_user_agent.clone(),
            },
            &env,
        );
        states.push(state.oauth_state.unwrap().state_param);
    }

    // All states should be unique
    let unique_count = states.iter().collect::<std::collections::HashSet<_>>().len();
    assert_eq!(unique_count, 10, "CSRF states should be unique");

    // All states should be non-empty
    for state in &states {
        assert!(!state.is_empty(), "CSRF state should not be empty");
        assert!(state.len() > 20, "CSRF state should be sufficiently long");
    }
}

#[tokio::test]
async fn test_session_contains_correct_metadata() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)".to_string();
    let test_email = "user@example.com".to_string();

    let _ = reducer.reduce(
        &mut state,
        AuthAction::OAuthSuccess {
            email: test_email.clone(),
            name: Some("Test User".to_string()),
            provider: OAuthProvider::GitHub,
            access_token: "mock_access_token".to_string(),
            refresh_token: Some("mock_refresh_token".to_string()),
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    let session = state.session.as_ref().unwrap();

    // Verify session metadata
    assert_eq!(session.email, test_email);
    assert_eq!(session.ip_address, test_ip);
    assert_eq!(session.user_agent, test_user_agent);
    assert_eq!(session.oauth_provider, Some(OAuthProvider::GitHub));

    // Verify session IDs are set (not nil UUIDs)
    assert_ne!(session.session_id.0, uuid::Uuid::nil());
    assert_ne!(session.user_id.0, uuid::Uuid::nil());
    assert_ne!(session.device_id.0, uuid::Uuid::nil());

    // Verify timestamps
    assert!(session.created_at <= chrono::Utc::now());
    assert!(session.last_active <= chrono::Utc::now());
    assert!(session.expires_at > chrono::Utc::now());

    // Verify TTL (should be ~24 hours)
    let ttl = session.expires_at.signed_duration_since(session.created_at);
    assert!(ttl.num_hours() >= 23 && ttl.num_hours() <= 25);

    // Verify risk score is set
    assert!(session.login_risk_score >= 0.0 && session.login_risk_score <= 1.0);
}
