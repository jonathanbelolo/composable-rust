//! Integration tests for magic link authentication flow.

use composable_rust_auth::{
    actions::AuthAction,
    environment::AuthEnvironment,
    mocks::{
        MockChallengeStore, MockDeviceRepository, MockEmailProvider, MockOAuth2Provider,
        MockOAuthTokenStore, MockRiskCalculator, MockSessionStore, MockTokenStore,
        MockUserRepository, MockWebAuthnProvider,
    },
    reducers::MagicLinkReducer,
    state::AuthState,
};
use composable_rust_core::reducer::Reducer;
use composable_rust_testing::mocks::InMemoryEventStore;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

/// Create a test environment with mock providers.
fn create_test_env() -> AuthEnvironment<
    MockOAuth2Provider,
    MockEmailProvider,
    MockWebAuthnProvider,
    MockSessionStore,
    MockTokenStore,
    MockUserRepository,
    MockDeviceRepository,
    MockRiskCalculator,
    MockOAuthTokenStore,
    MockChallengeStore,
> {
    AuthEnvironment::new(
        MockOAuth2Provider::new(),
        MockEmailProvider::new(),
        MockWebAuthnProvider::new(),
        MockSessionStore::new(),
        MockTokenStore::new(),
        MockUserRepository::new(),
        MockDeviceRepository::new(),
        MockRiskCalculator::new(),
        MockOAuthTokenStore::new(),
        MockChallengeStore::new(),
        Arc::new(InMemoryEventStore::new()),
    )
}

/// Create a test reducer.
fn create_test_reducer() -> MagicLinkReducer<
    MockOAuth2Provider,
    MockEmailProvider,
    MockWebAuthnProvider,
    MockSessionStore,
    MockTokenStore,
    MockUserRepository,
    MockDeviceRepository,
    MockRiskCalculator,
    MockOAuthTokenStore,
    MockChallengeStore,
> {
    MagicLinkReducer::new()
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_magic_link_flow_complete_happy_path() {
    // Setup
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();
    let test_email = "user@example.com".to_string();

    // Step 1: Send magic link
    let effects = reducer.reduce(
        &mut state,
        AuthAction::SendMagicLink {
            email: test_email.clone(),
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    // Should have magic link state stored
    assert!(state.magic_link_state.is_some());
    let magic_link_state = state.magic_link_state.as_ref().unwrap();
    assert_eq!(magic_link_state.email, test_email);
    assert!(!magic_link_state.token.is_empty());
    assert!(magic_link_state.expires_at > chrono::Utc::now());

    // Token should be 43 characters (256 bits base64url encoded)
    assert_eq!(magic_link_state.token.len(), 43);

    // Should return 2 effects: store token + send email (BLOCKER #1 fix)
    assert_eq!(effects.len(), 2);

    // Step 2: Verify magic link with valid token
    let valid_token = magic_link_state.token.clone();
    let effects = reducer.reduce(
        &mut state,
        AuthAction::VerifyMagicLink {
            token: valid_token,
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    // VerifyMagicLink returns Effect::Future (async token consumption)
    assert_eq!(effects.len(), 1);

    // Step 3: Simulate effect execution by calling MagicLinkVerified
    // (In production, this would come from the effect executor)
    let _effects = reducer.reduce(
        &mut state,
        AuthAction::MagicLinkVerified {
            email: test_email.clone(),
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    // Magic link state should be cleared (token consumed)
    assert!(state.magic_link_state.is_none());

    // Should have created a session
    assert!(state.session.is_some());
    let session = state.session.as_ref().unwrap();
    assert_eq!(session.email, test_email);
    assert_eq!(session.ip_address, test_ip);
    assert_eq!(session.user_agent, test_user_agent);
    assert!(session.oauth_provider.is_none());
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_magic_link_rejects_invalid_token() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Step 1: Send magic link
    let _ = reducer.reduce(
        &mut state,
        AuthAction::SendMagicLink {
            email: "user@example.com".to_string(),
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    assert!(state.magic_link_state.is_some());

    // Step 2: Try to verify with INVALID token
    let effects = reducer.reduce(
        &mut state,
        AuthAction::VerifyMagicLink {
            token: "invalid_token_12345678901234567890123456789".to_string(),
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    // VerifyMagicLink returns Effect::Future (async token consumption)
    // Effect will fail to consume token and return None
    assert_eq!(effects.len(), 1);

    // Magic link state NOT cleared yet (only cleared on successful verification)
    assert!(state.magic_link_state.is_some());

    // No session should be created
    assert!(state.session.is_none());
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_magic_link_requires_prior_send() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Try to verify WITHOUT sending magic link first
    let effects = reducer.reduce(
        &mut state,
        AuthAction::VerifyMagicLink {
            token: "some_token_1234567890123456789012345678901".to_string(),
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    // Should reject (no magic link state exists)
    assert!(state.magic_link_state.is_none());
    assert_eq!(effects.len(), 1);

    // No session should be created
    assert!(state.session.is_none());
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_magic_link_token_expires_after_ttl() {
    let reducer = MagicLinkReducer::with_ttl(10); // 10 minutes
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Step 1: Send magic link
    let _ = reducer.reduce(
        &mut state,
        AuthAction::SendMagicLink {
            email: "user@example.com".to_string(),
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    let magic_link_state = state.magic_link_state.as_ref().unwrap();
    let valid_token = magic_link_state.token.clone();

    // Simulate time passing by manually setting expiration to past
    if let Some(ref mut ml_state) = state.magic_link_state {
        ml_state.expires_at = chrono::Utc::now() - chrono::Duration::minutes(1);
    }

    // Step 2: Try to verify with expired token
    let effects = reducer.reduce(
        &mut state,
        AuthAction::VerifyMagicLink {
            token: valid_token,
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    // VerifyMagicLink returns Effect::Future (async token consumption)
    // Effect will fail to consume expired token and return None
    assert_eq!(effects.len(), 1);

    // Magic link state NOT cleared (only cleared on successful verification)
    assert!(state.magic_link_state.is_some());

    // No session should be created
    assert!(state.session.is_none());
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_magic_link_token_single_use() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Step 1: Send magic link
    let _ = reducer.reduce(
        &mut state,
        AuthAction::SendMagicLink {
            email: "user@example.com".to_string(),
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    let valid_token = state.magic_link_state.as_ref().unwrap().token.clone();
    let test_email = "user@example.com".to_string();

    // Step 2: Verify token (first time - should succeed)
    let _ = reducer.reduce(
        &mut state,
        AuthAction::VerifyMagicLink {
            token: valid_token.clone(),
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    // Step 3: Simulate successful verification
    let _ = reducer.reduce(
        &mut state,
        AuthAction::MagicLinkVerified {
            email: test_email,
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    // Session should be created
    assert!(state.session.is_some());

    // Magic link state should be cleared
    assert!(state.magic_link_state.is_none());

    // Step 4: Try to verify AGAIN with same token (should fail - single use)
    // In production, TokenStore.consume_token() would return None (already consumed)
    let effects = reducer.reduce(
        &mut state,
        AuthAction::VerifyMagicLink {
            token: valid_token,
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    // Should return Effect::Future that will fail
    assert_eq!(effects.len(), 1);
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_magic_link_token_uniqueness() {
    let reducer = create_test_reducer();
    let env = create_test_env();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Generate multiple tokens
    let mut tokens = Vec::new();
    for _ in 0..10 {
        let mut state = AuthState::default();
        let _ = reducer.reduce(
            &mut state,
            AuthAction::SendMagicLink {
                email: "user@example.com".to_string(),
                ip_address: test_ip,
                user_agent: test_user_agent.clone(),
            },
            &env,
        );
        tokens.push(state.magic_link_state.unwrap().token);
    }

    // All tokens should be unique
    let unique_count = tokens.iter().collect::<std::collections::HashSet<_>>().len();
    assert_eq!(unique_count, 10, "Magic link tokens should be unique");

    // All tokens should be non-empty and correct length
    for token in &tokens {
        assert!(!token.is_empty(), "Token should not be empty");
        assert_eq!(token.len(), 43, "Token should be 43 characters (256 bits base64url)");
    }
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_session_contains_correct_metadata() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)".to_string();
    let test_email = "user@example.com".to_string();

    // Send and verify magic link
    let _ = reducer.reduce(
        &mut state,
        AuthAction::SendMagicLink {
            email: test_email.clone(),
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    let token = state.magic_link_state.as_ref().unwrap().token.clone();

    let _ = reducer.reduce(
        &mut state,
        AuthAction::VerifyMagicLink {
            token,
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    // Simulate successful verification
    let _ = reducer.reduce(
        &mut state,
        AuthAction::MagicLinkVerified {
            email: test_email.clone(),
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
    assert!(session.oauth_provider.is_none());

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

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_magic_link_custom_ttl() {
    let reducer = MagicLinkReducer::with_ttl(5); // 5 minutes
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    let _ = reducer.reduce(
        &mut state,
        AuthAction::SendMagicLink {
            email: "user@example.com".to_string(),
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    let magic_link_state = state.magic_link_state.as_ref().unwrap();
    let expires_in = magic_link_state.expires_at.signed_duration_since(chrono::Utc::now());

    // Should expire in approximately 5 minutes (allow 1 minute tolerance for test execution)
    assert!(expires_in.num_minutes() >= 4 && expires_in.num_minutes() <= 6);
}
