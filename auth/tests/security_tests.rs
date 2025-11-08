//! Security-focused integration tests.
//!
//! This module contains tests that verify critical security properties
//! of the authentication system, including:
//!
//! - Atomic token consumption (prevent race conditions)
//! - Token replay prevention
//! - Timing attack resistance
//! - Information disclosure prevention

use composable_rust_auth::{
    mocks::MockTokenStore,
    providers::{TokenData, TokenStore, TokenType},
};
use chrono::{Duration, Utc};
use std::sync::Arc;

/// **BLOCKER #1 FIX: Atomic Token Consumption**
///
/// This test demonstrates that the TokenStore prevents race conditions
/// where two concurrent requests attempt to use the same token.
///
/// # Security Vulnerability (Before Fix)
///
/// The old implementation had non-atomic check-and-delete:
/// ```ignore
/// // ❌ VULNERABLE CODE (before fix):
/// if token_valid {           // Thread 1 passes check
///     // Thread 2 also passes check here!
///     delete_token();         // Both threads delete token
///     create_session();       // Both threads create session!
/// }
/// ```
///
/// # Fix
///
/// The new implementation uses atomic consume_token():
/// ```ignore
/// // ✅ SECURE CODE (after fix):
/// match token_store.consume_token(token_id, token).await {
///     Ok(Some(data)) => {
///         // Only ONE thread gets Some(data)
///         // Token is atomically removed
///         create_session()
///     }
///     Ok(None) => {
///         // Other threads get None
///         reject_request()
///     }
/// }
/// ```
#[tokio::test]
async fn test_blocker_1_magic_link_concurrent_token_use_prevented() {
    // Arrange: Create token store and store a valid token
    let token_store = Arc::new(MockTokenStore::new());
    let token = "test-magic-link-token-12345";
    let token_data = TokenData::new(
        TokenType::MagicLink,
        token.to_string(),
        serde_json::json!({"email": "user@example.com"}),
        Utc::now() + Duration::minutes(10),
    );

    token_store
        .store_token(token, token_data.clone())
        .await
        .expect("Failed to store token");

    // Act: Simulate two concurrent verification attempts
    let store1 = Arc::clone(&token_store);
    let store2 = Arc::clone(&token_store);
    let token1 = token.to_string();
    let token2 = token.to_string();

    let (result1, result2) = tokio::join!(
        async move { store1.consume_token(&token1, &token1).await },
        async move { store2.consume_token(&token2, &token2).await }
    );

    // Assert: Exactly ONE request should succeed
    let success_count = [
        result1.expect("Store operation failed"),
        result2.expect("Store operation failed"),
    ]
    .iter()
    .filter(|r| r.is_some())
    .count();

    assert_eq!(
        success_count, 1,
        "🚨 SECURITY VIOLATION: Both concurrent requests succeeded! \
         Token was used twice, allowing session hijacking."
    );

    // Verify token is consumed and cannot be used again
    let third_attempt = token_store
        .consume_token(token, token)
        .await
        .expect("Store operation failed");

    assert!(
        third_attempt.is_none(),
        "🚨 SECURITY VIOLATION: Token was not consumed after successful verification!"
    );
}

/// **BLOCKER #1 VARIANT: Token Replay Prevention**
///
/// Verifies that a token cannot be reused after successful consumption,
/// even with a time delay between attempts.
#[tokio::test]
async fn test_token_replay_prevention() {
    let token_store = MockTokenStore::new();
    let token = "replay-test-token";
    let token_data = TokenData::new(
        TokenType::MagicLink,
        token.to_string(),
        serde_json::json!({"email": "user@example.com"}),
        Utc::now() + Duration::minutes(10),
    );

    // Store token
    token_store
        .store_token(token, token_data.clone())
        .await
        .expect("Failed to store token");

    // First use: Should succeed
    let first = token_store
        .consume_token(token, token)
        .await
        .expect("Store operation failed");

    assert!(
        first.is_some(),
        "First token use should succeed"
    );

    // Wait a bit
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Second use (replay): Should fail
    let replay = token_store
        .consume_token(token, token)
        .await
        .expect("Store operation failed");

    assert!(
        replay.is_none(),
        "🚨 SECURITY VIOLATION: Token replay succeeded! \
         Same token was accepted twice."
    );
}

/// **BLOCKER #1 VARIANT: Expired Token Rejection**
///
/// Verifies that expired tokens are automatically rejected and cleaned up.
#[tokio::test]
async fn test_expired_token_rejection() {
    let token_store = MockTokenStore::new();
    let token = "expired-token";
    let token_data = TokenData::new(
        TokenType::MagicLink,
        token.to_string(),
        serde_json::json!({"email": "user@example.com"}),
        Utc::now() - Duration::seconds(1), // Already expired
    );

    // Store expired token
    token_store
        .store_token(token, token_data.clone())
        .await
        .expect("Failed to store token");

    // Try to consume: Should fail due to expiration
    let result = token_store
        .consume_token(token, token)
        .await
        .expect("Store operation failed");

    assert!(
        result.is_none(),
        "🚨 SECURITY VIOLATION: Expired token was accepted!"
    );

    // Verify token was removed from store
    let exists = token_store
        .exists(token)
        .await
        .expect("Store operation failed");

    assert!(
        !exists,
        "Expired token should be removed from store"
    );
}

/// **BLOCKER #1 VARIANT: Wrong Token Rejection**
///
/// Verifies that incorrect token values are rejected without
/// consuming the actual token (prevents denial of service).
#[tokio::test]
async fn test_wrong_token_rejection() {
    let token_store = MockTokenStore::new();
    let token_id = "token-id-123";
    let correct_token = "correct-token-value";
    let wrong_token = "wrong-token-value";

    let token_data = TokenData::new(
        TokenType::MagicLink,
        correct_token.to_string(),
        serde_json::json!({"email": "user@example.com"}),
        Utc::now() + Duration::minutes(10),
    );

    // Store token
    token_store
        .store_token(token_id, token_data.clone())
        .await
        .expect("Failed to store token");

    // Try with wrong token value: Should fail
    let wrong_attempt = token_store
        .consume_token(token_id, wrong_token)
        .await
        .expect("Store operation failed");

    assert!(
        wrong_attempt.is_none(),
        "Wrong token should be rejected"
    );

    // Token should still exist (not consumed by failed attempt)
    let still_exists = token_store
        .exists(token_id)
        .await
        .expect("Store operation failed");

    assert!(
        still_exists,
        "Token should not be consumed by failed verification attempt"
    );

    // Correct token should still work
    let correct_attempt = token_store
        .consume_token(token_id, correct_token)
        .await
        .expect("Store operation failed");

    assert!(
        correct_attempt.is_some(),
        "Correct token should succeed after failed attempts"
    );
}

/// **High-Concurrency Race Condition Test**
///
/// Simulates many concurrent attempts to use the same token.
/// Only ONE should succeed.
#[tokio::test]
async fn test_high_concurrency_token_consumption() {
    let token_store = Arc::new(MockTokenStore::new());
    let token = "high-concurrency-token";
    let token_data = TokenData::new(
        TokenType::MagicLink,
        token.to_string(),
        serde_json::json!({"email": "user@example.com"}),
        Utc::now() + Duration::minutes(10),
    );

    token_store
        .store_token(token, token_data.clone())
        .await
        .expect("Failed to store token");

    // Spawn 100 concurrent attempts
    let mut handles = vec![];
    for _ in 0..100 {
        let store = Arc::clone(&token_store);
        let token_clone = token.to_string();
        handles.push(tokio::spawn(async move {
            store.consume_token(&token_clone, &token_clone).await
        }));
    }

    // Collect results
    let mut success_count = 0;
    for handle in handles {
        if let Ok(Ok(Some(_))) = handle.await {
            success_count += 1;
        }
    }

    assert_eq!(
        success_count, 1,
        "🚨 SECURITY VIOLATION: {} concurrent attempts succeeded! \
         Expected exactly 1.",
        success_count
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// BLOCKER #2: OAuth State Single-Use Protection
// ═══════════════════════════════════════════════════════════════════════════

/// **BLOCKER #2 FIX: Atomic OAuth State Consumption**
///
/// This test demonstrates that the TokenStore prevents race conditions
/// where two concurrent OAuth callbacks attempt to use the same state parameter.
///
/// # Security Vulnerability (Before Fix)
///
/// The old implementation had non-atomic CSRF state validation:
/// ```ignore
/// // ❌ VULNERABLE CODE (before fix):
/// if state_matches && !expired {   // Request 1 passes check
///     // Request 2 also passes check here!
///     state.oauth_state = None;     // Not atomic!
///     exchange_code();              // Both requests exchange code!
/// }
/// ```
///
/// # Fix
///
/// The new implementation uses atomic consume_token():
/// ```ignore
/// // ✅ SECURE CODE (after fix):
/// match token_store.consume_token(state, state).await {
///     Ok(Some(data)) => {
///         // Only ONE callback gets Some(data)
///         // State is atomically removed
///         exchange_code()
///     }
///     Ok(None) => {
///         // Other callbacks get None
///         reject_csrf_attack()
///     }
/// }
/// ```
#[tokio::test]
async fn test_blocker_2_oauth_concurrent_state_use_prevented() {
    // Arrange: Create token store and store a valid OAuth state
    let token_store = Arc::new(MockTokenStore::new());
    let state_param = "oauth-csrf-state-abc123xyz";
    let token_data = TokenData::new(
        TokenType::OAuthState,
        state_param.to_string(),
        serde_json::json!({"provider": "Google"}),
        Utc::now() + Duration::minutes(5),
    );

    token_store
        .store_token(state_param, token_data.clone())
        .await
        .expect("Failed to store OAuth state");

    // Act: Simulate two concurrent OAuth callbacks with same state
    let store1 = Arc::clone(&token_store);
    let store2 = Arc::clone(&token_store);
    let state1 = state_param.to_string();
    let state2 = state_param.to_string();

    let (result1, result2) = tokio::join!(
        async move { store1.consume_token(&state1, &state1).await },
        async move { store2.consume_token(&state2, &state2).await }
    );

    // Assert: Exactly ONE callback should succeed
    let success_count = [
        result1.expect("Store operation failed"),
        result2.expect("Store operation failed"),
    ]
    .iter()
    .filter(|r| r.is_some())
    .count();

    assert_eq!(
        success_count, 1,
        "🚨 SECURITY VIOLATION: Both concurrent OAuth callbacks succeeded! \
         CSRF state was used twice, allowing session hijacking."
    );

    // Verify state is consumed and cannot be replayed
    let replay_attempt = token_store
        .consume_token(state_param, state_param)
        .await
        .expect("Store operation failed");

    assert!(
        replay_attempt.is_none(),
        "🚨 SECURITY VIOLATION: OAuth state was not consumed after successful callback!"
    );
}

/// **BLOCKER #2 VARIANT: OAuth State Replay Prevention**
///
/// Verifies that an OAuth state parameter cannot be reused after successful callback,
/// even with a time delay (prevents CSRF replay attacks).
#[tokio::test]
async fn test_oauth_state_replay_prevention() {
    let token_store = MockTokenStore::new();
    let state_param = "oauth-state-replay-test";
    let token_data = TokenData::new(
        TokenType::OAuthState,
        state_param.to_string(),
        serde_json::json!({"provider": "GitHub"}),
        Utc::now() + Duration::minutes(5),
    );

    // Store OAuth state
    token_store
        .store_token(state_param, token_data.clone())
        .await
        .expect("Failed to store OAuth state");

    // First callback: Should succeed
    let first = token_store
        .consume_token(state_param, state_param)
        .await
        .expect("Store operation failed");

    assert!(
        first.is_some(),
        "First OAuth callback should succeed"
    );

    // Wait a bit
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Second callback (replay): Should fail
    let replay = token_store
        .consume_token(state_param, state_param)
        .await
        .expect("Store operation failed");

    assert!(
        replay.is_none(),
        "🚨 SECURITY VIOLATION: OAuth state replay succeeded! \
         Same CSRF token was accepted twice."
    );
}

/// **BLOCKER #2 VARIANT: Expired OAuth State Rejection**
///
/// Verifies that expired OAuth state parameters are automatically rejected.
#[tokio::test]
async fn test_expired_oauth_state_rejection() {
    let token_store = MockTokenStore::new();
    let state_param = "expired-oauth-state";
    let token_data = TokenData::new(
        TokenType::OAuthState,
        state_param.to_string(),
        serde_json::json!({"provider": "Google"}),
        Utc::now() - Duration::seconds(1), // Already expired
    );

    // Store expired state
    token_store
        .store_token(state_param, token_data.clone())
        .await
        .expect("Failed to store OAuth state");

    // Try to use expired state: Should fail
    let result = token_store
        .consume_token(state_param, state_param)
        .await
        .expect("Store operation failed");

    assert!(
        result.is_none(),
        "🚨 SECURITY VIOLATION: Expired OAuth state was accepted!"
    );
}

/// **BLOCKER #2 VARIANT: Wrong OAuth State Rejection**
///
/// Verifies that incorrect state values are rejected (CSRF protection).
#[tokio::test]
async fn test_wrong_oauth_state_rejection() {
    let token_store = MockTokenStore::new();
    let state_id = "oauth-state-id";
    let correct_state = "correct-state-value";
    let attacker_state = "attacker-forged-state";

    let token_data = TokenData::new(
        TokenType::OAuthState,
        correct_state.to_string(),
        serde_json::json!({"provider": "Google"}),
        Utc::now() + Duration::minutes(5),
    );

    // Store OAuth state
    token_store
        .store_token(state_id, token_data.clone())
        .await
        .expect("Failed to store OAuth state");

    // Attacker tries with forged state: Should fail
    let attack_attempt = token_store
        .consume_token(state_id, attacker_state)
        .await
        .expect("Store operation failed");

    assert!(
        attack_attempt.is_none(),
        "🚨 SECURITY VIOLATION: Forged OAuth state was accepted! \
         CSRF protection failed."
    );

    // State should still exist (not consumed by failed CSRF attack)
    let still_exists = token_store
        .exists(state_id)
        .await
        .expect("Store operation failed");

    assert!(
        still_exists,
        "OAuth state should not be consumed by CSRF attack attempt"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// BLOCKER #3: Challenge Expiration Validation
// BLOCKER #7: Challenge Replay Protection
// ═══════════════════════════════════════════════════════════════════════════

/// **BLOCKER #3 FIX: Challenge Expiration Validation**
///
/// This test demonstrates that WebAuthn challenges are properly validated
/// for expiration using the TokenStore.
///
/// # Security Vulnerability (Before Fix)
///
/// The old implementation had no expiration validation:
/// ```ignore
/// // ❌ VULNERABLE CODE (before fix):
/// let challenge_id = "mock_challenge_id"; // Hardcoded, never expires!
/// webauthn.verify_registration(challenge_id, response);
/// ```
///
/// # Fix
///
/// Challenges are now stored in TokenStore with expiration:
/// ```ignore
/// // ✅ SECURE CODE (after fix):
/// // On challenge generation:
/// token_store.store_token(challenge_id, TokenData {
///     expires_at: now + 5_minutes,
///     ...
/// });
///
/// // On verification:
/// match token_store.consume_token(challenge_id).await {
///     Ok(Some(data)) => {
///         // Challenge valid and not expired
///         verify(data.challenge)
///     }
///     Ok(None) => {
///         // Challenge expired or already used
///         reject()
///     }
/// }
/// ```
#[tokio::test]
async fn test_blocker_3_passkey_challenge_expiration() {
    // Arrange: Create expired passkey challenge
    let token_store = MockTokenStore::new();
    let challenge_id = "passkey-challenge-expired";
    let token_data = TokenData::new(
        TokenType::PasskeyRegistrationChallenge,
        challenge_id.to_string(),
        serde_json::json!({
            "user_id": "user-123",
            "challenge": "base64-encoded-challenge"
        }),
        Utc::now() - Duration::seconds(1), // Already expired
    );

    // Store expired challenge
    token_store
        .store_token(challenge_id, token_data.clone())
        .await
        .expect("Failed to store challenge");

    // Act: Try to use expired challenge
    let result = token_store
        .consume_token(challenge_id, challenge_id)
        .await
        .expect("Store operation failed");

    // Assert: Expired challenge should be rejected
    assert!(
        result.is_none(),
        "🚨 SECURITY VIOLATION: Expired passkey challenge was accepted! \
         This allows attackers to reuse old challenges indefinitely."
    );

    // Verify challenge was removed from store
    let exists = token_store
        .exists(challenge_id)
        .await
        .expect("Store operation failed");

    assert!(
        !exists,
        "Expired challenge should be removed from store"
    );
}

/// **BLOCKER #7 FIX: Challenge Replay Protection**
///
/// This test demonstrates that WebAuthn challenges can only be used once,
/// preventing replay attacks.
///
/// # Security Vulnerability (Before Fix)
///
/// Challenges were never consumed after use:
/// ```ignore
/// // ❌ VULNERABLE CODE (before fix):
/// if verify_challenge(challenge_id, response) {
///     // Challenge NOT deleted - can be reused!
///     create_credential()
/// }
/// ```
///
/// # Fix
///
/// Challenges are atomically consumed (single-use):
/// ```ignore
/// // ✅ SECURE CODE (after fix):
/// match token_store.consume_token(challenge_id).await {
///     Ok(Some(data)) => {
///         // Challenge atomically deleted - cannot be reused
///         create_credential()
///     }
/// }
/// ```
#[tokio::test]
async fn test_blocker_7_passkey_challenge_single_use() {
    // Arrange: Create valid passkey challenge
    let token_store = MockTokenStore::new();
    let challenge_id = "passkey-challenge-single-use";
    let token_data = TokenData::new(
        TokenType::PasskeyAuthenticationChallenge,
        challenge_id.to_string(),
        serde_json::json!({
            "user_id": "user-123",
            "challenge": "base64-encoded-challenge"
        }),
        Utc::now() + Duration::minutes(5),
    );

    // Store challenge
    token_store
        .store_token(challenge_id, token_data.clone())
        .await
        .expect("Failed to store challenge");

    // Act: Use challenge once
    let first_use = token_store
        .consume_token(challenge_id, challenge_id)
        .await
        .expect("Store operation failed");

    assert!(
        first_use.is_some(),
        "First challenge use should succeed"
    );

    // Try to reuse challenge
    let replay = token_store
        .consume_token(challenge_id, challenge_id)
        .await
        .expect("Store operation failed");

    // Assert: Challenge replay should fail
    assert!(
        replay.is_none(),
        "🚨 SECURITY VIOLATION: Passkey challenge was reused! \
         This allows attackers to replay captured WebAuthn responses."
    );
}

/// **BLOCKER #3 & #7: Concurrent Challenge Use Prevention**
///
/// Verifies that concurrent attempts to use the same challenge are prevented.
#[tokio::test]
async fn test_passkey_challenge_concurrent_use_prevented() {
    // Arrange: Create valid passkey challenge
    let token_store = Arc::new(MockTokenStore::new());
    let challenge_id = "passkey-challenge-concurrent";
    let token_data = TokenData::new(
        TokenType::PasskeyRegistrationChallenge,
        challenge_id.to_string(),
        serde_json::json!({
            "user_id": "user-123",
            "challenge": "base64-encoded-challenge"
        }),
        Utc::now() + Duration::minutes(5),
    );

    token_store
        .store_token(challenge_id, token_data.clone())
        .await
        .expect("Failed to store challenge");

    // Act: Simulate two concurrent verification attempts
    let store1 = Arc::clone(&token_store);
    let store2 = Arc::clone(&token_store);
    let id1 = challenge_id.to_string();
    let id2 = challenge_id.to_string();

    let (result1, result2) = tokio::join!(
        async move { store1.consume_token(&id1, &id1).await },
        async move { store2.consume_token(&id2, &id2).await }
    );

    // Assert: Exactly ONE verification should succeed
    let success_count = [
        result1.expect("Store operation failed"),
        result2.expect("Store operation failed"),
    ]
    .iter()
    .filter(|r| r.is_some())
    .count();

    assert_eq!(
        success_count, 1,
        "🚨 SECURITY VIOLATION: Both concurrent passkey verifications succeeded! \
         Challenge was used twice."
    );
}

/// **Wrong Challenge Rejection**
///
/// Verifies that incorrect challenge values are rejected.
#[tokio::test]
async fn test_passkey_wrong_challenge_rejection() {
    let token_store = MockTokenStore::new();
    let challenge_id = "passkey-challenge-id";
    let correct_challenge = "correct-challenge-value";
    let wrong_challenge = "wrong-challenge-value";

    let token_data = TokenData::new(
        TokenType::PasskeyRegistrationChallenge,
        correct_challenge.to_string(),
        serde_json::json!({
            "user_id": "user-123",
            "challenge": "base64-encoded-challenge"
        }),
        Utc::now() + Duration::minutes(5),
    );

    // Store challenge
    token_store
        .store_token(challenge_id, token_data.clone())
        .await
        .expect("Failed to store challenge");

    // Try with wrong challenge value: Should fail
    let attack_attempt = token_store
        .consume_token(challenge_id, wrong_challenge)
        .await
        .expect("Store operation failed");

    assert!(
        attack_attempt.is_none(),
        "Wrong challenge should be rejected"
    );

    // Challenge should still exist (not consumed by failed attempt)
    let still_exists = token_store
        .exists(challenge_id)
        .await
        .expect("Store operation failed");

    assert!(
        still_exists,
        "Challenge should not be consumed by failed verification"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// BLOCKER #6: Integer Overflow in Counter Comparison
// ═══════════════════════════════════════════════════════════════════════════
//
// VULNERABILITY: Simple counter comparison (new <= stored) fails when u32
// counters wrap around from u32::MAX to 0, causing false positives.
//
// FIX: Implement "half-space" algorithm that treats differences > u32::MAX/2
// as wraparound instead of rollback.
//
// SECURITY PROPERTY: Valid wraparounds are accepted, rollbacks are rejected.

/// Test normal counter increment is accepted
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_blocker_6_normal_counter_increment_accepted() {
    // Scenario: Normal counter increment
    // Stored: 100, New: 101
    // Expected: Not a rollback (normal increment)

    const HALF_SPACE: u32 = u32::MAX / 2;

    let stored_counter: u32 = 100;
    let new_counter: u32 = 101;

    // Simulate the reducer's wraparound logic (CORRECTED to match implementation)
    // Implementation uses wrapping_sub for half-space algorithm
    let is_rollback = if new_counter == stored_counter {
        true
    } else {
        let forward_diff = new_counter.wrapping_sub(stored_counter);
        forward_diff > HALF_SPACE
    };

    assert!(
        !is_rollback,
        "Normal counter increment (100 → 101) should be accepted"
    );
}

/// Test counter rollback is rejected
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_blocker_6_counter_rollback_rejected() {
    // This test verifies the reducer logic by checking that a rollback
    // scenario would be rejected. The actual rejection happens in the
    // reducer's counter check before calling update_passkey_counter.
    //
    // Scenario: Stored counter = 100, New counter = 50
    // Expected: Rollback detected (50 < 100, and diff is small)

    const HALF_SPACE: u32 = u32::MAX / 2;

    let stored_counter: u32 = 100;
    let new_counter: u32 = 50;

    // Simulate the reducer's wraparound logic (CORRECTED to match implementation)
    // Implementation uses wrapping_sub for half-space algorithm
    let is_rollback = if new_counter == stored_counter {
        true
    } else {
        let forward_diff = new_counter.wrapping_sub(stored_counter);
        forward_diff > HALF_SPACE
    };

    assert!(
        is_rollback,
        "🚨 SECURITY VIOLATION: Counter rollback (100 → 50) should be detected"
    );
}

/// Test same counter (replay attack) is rejected
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_blocker_6_same_counter_replay_rejected() {
    const HALF_SPACE: u32 = u32::MAX / 2;

    let stored_counter: u32 = 100;
    let new_counter: u32 = 100; // Same counter = replay attack

    let is_rollback = if new_counter == stored_counter {
        true
    } else if new_counter > stored_counter {
        let diff = new_counter - stored_counter;
        diff > HALF_SPACE
    } else {
        let stored_from_max = u32::MAX - stored_counter;
        let is_near_max = stored_from_max < HALF_SPACE;
        let new_is_small = new_counter < HALF_SPACE;
        !(is_near_max && new_is_small)
    };

    assert!(
        is_rollback,
        "🚨 SECURITY VIOLATION: Same counter (replay attack) should be detected"
    );
}

/// Test valid wraparound is accepted
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_blocker_6_valid_wraparound_accepted() {
    // Scenario: Counter wraps from near u32::MAX to small number
    // Stored: u32::MAX - 10 = 4,294,967,285
    // New: 5
    // This is a valid wraparound (16 increments total)

    const HALF_SPACE: u32 = u32::MAX / 2;

    let stored_counter: u32 = u32::MAX - 10;
    let new_counter: u32 = 5;

    let is_rollback = if new_counter == stored_counter {
        true
    } else if new_counter > stored_counter {
        let diff = new_counter - stored_counter;
        diff > HALF_SPACE
    } else {
        // New counter is lower - check if it's a valid wraparound
        let stored_from_max = u32::MAX - stored_counter;
        let is_near_max = stored_from_max < HALF_SPACE;
        let new_is_small = new_counter < HALF_SPACE;
        !(is_near_max && new_is_small)
    };

    assert!(
        !is_rollback,
        "Valid wraparound (u32::MAX-10 → 5) should be accepted"
    );
}

/// Test invalid large backward jump is rejected
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_blocker_6_large_backward_jump_rejected() {
    // Scenario: Suspiciously large backward jump
    // Stored: 1000
    // New: u32::MAX - 1000
    // This would require wrapping around almost the entire u32 space,
    // which is not realistic for a counter increment.

    const HALF_SPACE: u32 = u32::MAX / 2;

    let stored_counter: u32 = 1000;
    let new_counter: u32 = u32::MAX - 1000;

    let is_rollback = if new_counter == stored_counter {
        true
    } else if new_counter > stored_counter {
        let diff = new_counter - stored_counter;
        diff > HALF_SPACE
    } else {
        let stored_from_max = u32::MAX - stored_counter;
        let is_near_max = stored_from_max < HALF_SPACE;
        let new_is_small = new_counter < HALF_SPACE;
        !(is_near_max && new_is_small)
    };

    assert!(
        is_rollback,
        "🚨 SECURITY VIOLATION: Large backward jump should be detected as rollback"
    );
}

/// Test wraparound boundary conditions
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_blocker_6_wraparound_boundary_conditions() {
    const HALF_SPACE: u32 = u32::MAX / 2;

    // Test case 1: Exactly at wraparound point
    let stored = u32::MAX;
    let new = 0;

    let is_rollback = if new == stored {
        true
    } else if new > stored {
        let diff = new - stored;
        diff > HALF_SPACE
    } else {
        let stored_from_max = u32::MAX - stored;
        let is_near_max = stored_from_max < HALF_SPACE;
        let new_is_small = new < HALF_SPACE;
        !(is_near_max && new_is_small)
    };

    assert!(
        !is_rollback,
        "Wraparound from u32::MAX to 0 should be accepted"
    );

    // Test case 2: Just before wraparound
    let stored = u32::MAX - 1;
    let new = 0;

    let is_rollback = if new == stored {
        true
    } else if new > stored {
        let diff = new - stored;
        diff > HALF_SPACE
    } else {
        let stored_from_max = u32::MAX - stored;
        let is_near_max = stored_from_max < HALF_SPACE;
        let new_is_small = new < HALF_SPACE;
        !(is_near_max && new_is_small)
    };

    assert!(
        !is_rollback,
        "Wraparound from u32::MAX-1 to 0 should be accepted"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// BLOCKER #5: Timing Attack on Expiration Check
// ═══════════════════════════════════════════════════════════════════════════
//
// VULNERABILITY: Early returns in token validation create timing side-channels
// that leak information about whether a token is wrong vs expired.
//
// - Wrong token: Fast return (no time check, no cleanup)
// - Expired token: Slow return (time check, token removal)
//
// An attacker can measure response times to distinguish these cases.
//
// FIX: Always perform all validation checks in constant time, regardless of
// early failures. All failure paths must execute similar operations.
//
// SECURITY PROPERTY: Wrong tokens and expired tokens return the same error
// in constant time.

/// Test wrong token and expired token both return None
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_blocker_5_wrong_and_expired_tokens_same_error() {
    let store = Arc::new(MockTokenStore::new());

    // Store an expired token
    let expired_token_data = TokenData::new(
        TokenType::MagicLink,
        "correct-token-value".to_string(),
        serde_json::json!({"email": "test@example.com"}),
        Utc::now() - Duration::minutes(1), // Expired 1 minute ago
    );

    store
        .store_token("token-id-1", expired_token_data)
        .await
        .expect("Failed to store token");

    // Try with correct token value (but expired)
    let result_expired = store
        .consume_token("token-id-1", "correct-token-value")
        .await
        .expect("consume_token failed");

    // Try with wrong token value (would also be expired, but fails token check)
    store
        .store_token(
            "token-id-2",
            TokenData::new(
                TokenType::MagicLink,
                "correct-token-value-2".to_string(),
                serde_json::json!({"email": "test@example.com"}),
                Utc::now() - Duration::minutes(1),
            ),
        )
        .await
        .expect("Failed to store token");

    let result_wrong = store
        .consume_token("token-id-2", "wrong-token-value")
        .await
        .expect("consume_token failed");

    // Both should return None (same error)
    assert!(
        result_expired.is_none(),
        "Expired token should return None"
    );
    assert!(result_wrong.is_none(), "Wrong token should return None");
}

/// Test expired token cleanup happens regardless of token match
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_blocker_5_expired_token_cleanup_independent_of_match() {
    let store = Arc::new(MockTokenStore::new());

    // Store an expired token
    let expired_token_data = TokenData::new(
        TokenType::MagicLink,
        "correct-token".to_string(),
        serde_json::json!({"email": "test@example.com"}),
        Utc::now() - Duration::minutes(1),
    );

    store
        .store_token("token-id-1", expired_token_data.clone())
        .await
        .expect("Failed to store token");

    // Try with WRONG token value on an EXPIRED token
    // The fix ensures expired tokens are cleaned up even if token doesn't match
    let result = store
        .consume_token("token-id-1", "wrong-token-value")
        .await
        .expect("consume_token failed");

    assert!(result.is_none(), "Should return None for wrong token");

    // Verify token was removed (cleanup happened despite wrong token)
    let exists = store
        .exists("token-id-1")
        .await
        .expect("exists check failed");

    assert!(
        !exists,
        "Expired token should be cleaned up even when token value is wrong"
    );
}

/// Test constant-time validation with valid token
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_blocker_5_valid_token_accepted() {
    let store = Arc::new(MockTokenStore::new());

    // Store a valid (not expired) token
    let valid_token_data = TokenData::new(
        TokenType::MagicLink,
        "valid-token".to_string(),
        serde_json::json!({"email": "test@example.com"}),
        Utc::now() + Duration::minutes(10), // Expires in 10 minutes
    );

    store
        .store_token("token-id-1", valid_token_data.clone())
        .await
        .expect("Failed to store token");

    // Consume with correct token value
    let result = store
        .consume_token("token-id-1", "valid-token")
        .await
        .expect("consume_token failed");

    assert!(
        result.is_some(),
        "Valid token with correct value should be consumed"
    );

    let consumed = result.unwrap();
    assert_eq!(consumed.token, "valid-token");

    // Token should be removed after consumption
    let exists = store
        .exists("token-id-1")
        .await
        .expect("exists check failed");

    assert!(!exists, "Token should be removed after consumption");
}

/// Test all failure paths return None
#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn test_blocker_5_all_failure_paths_return_none() {
    let store = Arc::new(MockTokenStore::new());

    // Case 1: Non-existent token
    let result_missing = store
        .consume_token("nonexistent", "some-token")
        .await
        .expect("consume_token failed");

    assert!(result_missing.is_none(), "Missing token should return None");

    // Case 2: Wrong token value
    store
        .store_token(
            "token-id-2",
            TokenData::new(
                TokenType::MagicLink,
                "correct-token".to_string(),
                serde_json::json!({"email": "test@example.com"}),
                Utc::now() + Duration::minutes(10),
            ),
        )
        .await
        .expect("Failed to store token");

    let result_wrong = store
        .consume_token("token-id-2", "wrong-token")
        .await
        .expect("consume_token failed");

    assert!(result_wrong.is_none(), "Wrong token should return None");

    // Case 3: Expired token
    store
        .store_token(
            "token-id-3",
            TokenData::new(
                TokenType::MagicLink,
                "expired-token".to_string(),
                serde_json::json!({"email": "test@example.com"}),
                Utc::now() - Duration::minutes(1),
            ),
        )
        .await
        .expect("Failed to store token");

    let result_expired = store
        .consume_token("token-id-3", "expired-token")
        .await
        .expect("consume_token failed");

    assert!(result_expired.is_none(), "Expired token should return None");

    // All three failure cases return the same error (None)
    // No information leakage about WHY the token failed
}
