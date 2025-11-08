//! Integration tests for WebAuthn/passkey authentication flow.

use composable_rust_auth::{
    actions::AuthAction,
    environment::AuthEnvironment,
    mocks::{
        MockDeviceRepository, MockEmailProvider, MockOAuth2Provider, MockRiskCalculator,
        MockSessionStore, MockUserRepository, MockWebAuthnProvider,
    },
    reducers::PasskeyReducer,
    state::{AuthState, DeviceId, UserId},
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
fn create_test_reducer() -> PasskeyReducer<
    MockOAuth2Provider,
    MockEmailProvider,
    MockWebAuthnProvider,
    MockSessionStore,
    MockUserRepository,
    MockDeviceRepository,
    MockRiskCalculator,
> {
    PasskeyReducer::new()
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_passkey_registration_flow() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let user_id = UserId::new();
    let device_name = "iPhone 15 Pro".to_string();

    // Step 1: Initiate passkey registration
    let effects = reducer.reduce(
        &mut state,
        AuthAction::InitiatePasskeyRegistration {
            user_id,
            device_name: device_name.clone(),
        },
        &env,
    );

    // Should return effect to generate challenge
    assert_eq!(effects.len(), 1);

    // Note: In a real test, we would need to:
    // 1. Execute the effect to get the challenge
    // 2. Simulate client-side navigator.credentials.create()
    // 3. Send attestation response back
    // For now, we verify the reducer handles the action
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_passkey_registration_completion() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let user_id = UserId::new();
    let device_id = DeviceId::new();

    // Simulate completing passkey registration
    let effects = reducer.reduce(
        &mut state,
        AuthAction::CompletePasskeyRegistration {
            user_id,
            device_id,
            credential_id: "mock_credential_id_123".to_string(),
            public_key: vec![1, 2, 3, 4], // Mock public key
            attestation_response: "mock_attestation_response".to_string(),
        },
        &env,
    );

    // Should return effect to verify attestation and store credential
    assert_eq!(effects.len(), 1);
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_passkey_login_initiation() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Step 1: Initiate passkey login
    let effects = reducer.reduce(
        &mut state,
        AuthAction::InitiatePasskeyLogin {
            username: "user@example.com".to_string(),
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    // Should return effect to lookup user and generate challenge
    assert_eq!(effects.len(), 1);
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_passkey_login_completion() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Simulate completing passkey login
    let effects = reducer.reduce(
        &mut state,
        AuthAction::CompletePasskeyLogin {
            credential_id: "mock_credential_id_123".to_string(),
            assertion_response: "mock_assertion_response".to_string(),
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    // Should return effect to verify assertion and create session
    assert_eq!(effects.len(), 1);
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_passkey_login_success_creates_session() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let user_id = UserId::new();
    let device_id = DeviceId::new();
    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();
    let test_email = "user@example.com".to_string();

    // Simulate successful passkey login
    let effects = reducer.reduce(
        &mut state,
        AuthAction::PasskeyLoginSuccess {
            user_id,
            device_id,
            email: test_email.clone(),
            ip_address: test_ip,
            user_agent: test_user_agent.clone(),
        },
        &env,
    );

    // Session should be created
    assert!(state.session.is_some());
    let session = state.session.as_ref().unwrap();
    assert_eq!(session.email, test_email);
    assert_eq!(session.user_id, user_id);
    assert_eq!(session.device_id, device_id);
    assert_eq!(session.ip_address, test_ip);
    assert_eq!(session.user_agent, test_user_agent);

    // Passkey logins should have very low risk score
    assert!(session.login_risk_score < 0.1);

    // Should return effect to persist session
    assert_eq!(effects.len(), 1);
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_session_contains_correct_metadata() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let user_id = UserId::new();
    let device_id = DeviceId::new();
    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)".to_string();
    let test_email = "user@example.com".to_string();

    let _ = reducer.reduce(
        &mut state,
        AuthAction::PasskeyLoginSuccess {
            user_id,
            device_id,
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

    // Verify risk score is very low (passkeys are very secure)
    assert!(session.login_risk_score >= 0.0 && session.login_risk_score < 0.1);
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_custom_webauthn_config() {
    let reducer = PasskeyReducer::with_config(
        "https://app.example.com".to_string(),
        "app.example.com".to_string(),
    );
    let env = create_test_env();
    let mut state = AuthState::default();

    let user_id = UserId::new();
    let device_id = DeviceId::new();
    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    // Should work with custom config
    let effects = reducer.reduce(
        &mut state,
        AuthAction::PasskeyLoginSuccess {
            user_id,
            device_id,
            email: "user@example.com".to_string(),
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    assert!(state.session.is_some());
    assert_eq!(effects.len(), 1);
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_passkey_security_properties() {
    let reducer = create_test_reducer();
    let env = create_test_env();
    let mut state = AuthState::default();

    let user_id = UserId::new();
    let device_id = DeviceId::new();
    let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let test_user_agent = "Mozilla/5.0 (Test)".to_string();

    let _ = reducer.reduce(
        &mut state,
        AuthAction::PasskeyLoginSuccess {
            user_id,
            device_id,
            email: "user@example.com".to_string(),
            ip_address: test_ip,
            user_agent: test_user_agent,
        },
        &env,
    );

    let session = state.session.as_ref().unwrap();

    // Passkeys provide:
    // 1. Hardware-backed security (FIDO2)
    // 2. Public key cryptography
    // 3. Phishing resistance (origin binding)
    // 4. Counter-based replay protection

    // Therefore, risk score should be very low
    assert!(session.login_risk_score == 0.05,
        "Passkey logins should have risk score of 0.05 (very secure)");
}
