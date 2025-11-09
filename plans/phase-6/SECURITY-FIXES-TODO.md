# Phase 6: Security Audit Fixes - TODO List

**Created**: 2025-11-09
**Source**: Comprehensive security audit of auth crate
**Status**: 🔴 **14 Issues Found** (3 CRITICAL, 2 HIGH, 4 MEDIUM, 5 LOW)

---

## 🚨 CRITICAL ISSUES (Must Fix) - 3 Issues

### CRITICAL #1: Remove unwrap_or_else() in Risk Calculation (Panic Risk)

**Severity**: CRITICAL
**Impact**: Service crash if risk calculator fails
**Files Affected**: 3 files

#### Tasks:
- [ ] **magic_link.rs:438** - Fix unwrap_or_else in risk calculation
  ```rust
  // Current (line 438):
  }).await.unwrap_or_else(|_| { ... });

  // Fix to:
  }).await.ok().unwrap_or_else(|| { ... });
  ```

- [ ] **oauth.rs:482** - Fix unwrap_or_else in risk calculation
  ```rust
  // Current (line 482):
  }).await.unwrap_or_else(|_| { ... });

  // Fix to:
  }).await.ok().unwrap_or_else(|| { ... });
  ```

- [ ] **passkey.rs:812** - Fix unwrap_or_else in risk calculation
  ```rust
  // Current (line 812):
  }).await.unwrap_or_else(|_| { ... });

  // Fix to:
  }).await.ok().unwrap_or_else(|| { ... });
  ```

**Fix Strategy**: Convert `Result<T, E>` to `Option<T>` using `.ok()` before unwrapping

**Testing**: Run all reducer tests to verify fallback behavior works

**Estimated Time**: 30 minutes

---

### CRITICAL #2: Fix Memory Leak in Session Set TTL Handling

**Severity**: CRITICAL
**Impact**: Memory exhaustion (unbounded set growth)
**File**: `auth/src/stores/session_redis.rs:204`

#### Tasks:
- [ ] **Remove `.ignore()` from EXPIRE command** (line 204)
  ```rust
  // Current:
  .expire(&user_sessions_key, set_ttl_seconds)
  .ignore() // ❌ If EXPIRE fails, set grows unbounded!

  // Fix to:
  .expire(&user_sessions_key, set_ttl_seconds)
  // Let EXPIRE failures propagate
  ```

- [ ] **Update error handling** - Ensure EXPIRE failures are logged
  ```rust
  .query_async(&mut conn)
  .await
  .map_err(|e| {
      tracing::error!("Failed to create session with TTL: {}", e);
      AuthError::InternalError(format!("Session creation failed: {e}"))
  })?;
  ```

- [ ] **Alternative: Use Lua script** for atomic guarantees (if needed)
  ```lua
  -- Atomic session creation + set addition + TTL in one script
  redis.call('SETEX', session_key, ttl, session_data)
  redis.call('SADD', user_sessions_key, session_id)
  redis.call('EXPIRE', user_sessions_key, set_ttl)
  return 1
  ```

**Fix Strategy**: Don't ignore EXPIRE failures; propagate errors OR use Lua script

**Testing**:
- [ ] Test Redis failure scenarios (connection loss during pipeline)
- [ ] Verify set TTL is always set
- [ ] Monitor set sizes in production

**Estimated Time**: 2 hours (includes Lua script option)

---

### CRITICAL #3: Fix Cipher Cloning - Potential Nonce Reuse Risk

**Severity**: CRITICAL
**Impact**: Catastrophic encryption failure if nonce reused
**File**: `auth/src/stores/oauth_token_redis.rs:172-180`

#### Tasks:
- [ ] **Replace cipher field with Arc** (lines 62-63)
  ```rust
  // Current:
  pub struct RedisOAuthTokenStore {
      conn_manager: ConnectionManager,
      cipher: Aes256Gcm, // ❌ Cloning may be unsafe
  }

  // Fix to:
  use std::sync::Arc;

  pub struct RedisOAuthTokenStore {
      conn_manager: ConnectionManager,
      cipher: Arc<Aes256Gcm>, // ✅ Safe sharing
  }
  ```

- [ ] **Update constructor** (line 107)
  ```rust
  // Fix:
  Ok(Self {
      conn_manager,
      cipher: Arc::new(cipher),
  })
  ```

- [ ] **Update Clone impl** (lines 172-180)
  ```rust
  // Fix:
  impl Clone for RedisOAuthTokenStore {
      fn clone(&self) -> Self {
          Self {
              conn_manager: self.conn_manager.clone(),
              cipher: Arc::clone(&self.cipher),
          }
      }
  }
  ```

- [ ] **Update encrypt() method** (line 126)
  ```rust
  // Fix (line 126):
  let ciphertext = self.cipher
      .encrypt(&nonce, plaintext)
      // ... rest unchanged
  ```

- [ ] **Update decrypt() method** (line 147)
  ```rust
  // Fix (line 147):
  let plaintext = self.cipher
      .decrypt(&nonce, ciphertext)
      // ... rest unchanged
  ```

**Fix Strategy**: Use `Arc<Aes256Gcm>` to safely share cipher across clones

**Testing**:
- [ ] Verify encryption/decryption still works
- [ ] Test concurrent token operations
- [ ] Run all oauth_token_redis tests

**Estimated Time**: 1 hour

---

## ⚠️ HIGH ISSUES (Should Fix) - 2 Issues

### HIGH #1: Add Email Normalization to Prevent Account Collision

**Severity**: HIGH
**Impact**: Account takeover via email case/whitespace variations
**File**: `auth/src/utils.rs:158-188`

#### Tasks:
- [ ] **Add email normalization function** (new function in utils.rs)
  ```rust
  /// Normalize email address for storage.
  ///
  /// Normalization:
  /// 1. Convert to lowercase (RFC 5321 says local-part is case-sensitive,
  ///    but most providers treat it case-insensitive)
  /// 2. Trim whitespace
  /// 3. Validate after normalization
  ///
  /// Returns normalized email or error if invalid.
  pub fn normalize_email(email: &str) -> Result<String> {
      let normalized = email.trim().to_lowercase();

      if !is_valid_email(&normalized) {
          return Err(AuthError::InvalidEmail(
              "Email address is invalid after normalization".to_string()
          ));
      }

      Ok(normalized)
  }
  ```

- [ ] **Add tests for normalization** (utils.rs tests)
  ```rust
  #[test]
  fn test_email_normalization() {
      assert_eq!(normalize_email("User@Example.COM").unwrap(), "user@example.com");
      assert_eq!(normalize_email("  admin@test.com  ").unwrap(), "admin@test.com");
      assert_eq!(normalize_email("Admin+Tag@Company.COM").unwrap(), "admin+tag@company.com");
  }

  #[test]
  fn test_email_normalization_rejects_invalid() {
      assert!(normalize_email("not-an-email").is_err());
      assert!(normalize_email("@example.com").is_err());
  }
  ```

- [ ] **Update magic_link reducer** (magic_link.rs:153-157)
  ```rust
  // After validation:
  let email = crate::utils::normalize_email(&email)?;
  ```

- [ ] **Update oauth reducer** (oauth.rs:277-286)
  ```rust
  // After validation:
  let email = crate::utils::normalize_email(&email)?;
  ```

- [ ] **Update database migration** (add unique constraint on normalized email)
  ```sql
  -- migration 003_email_normalization.sql
  CREATE UNIQUE INDEX idx_users_email_normalized
  ON users_projection (LOWER(TRIM(email)));
  ```

- [ ] **Document in code** that `+` tags are NOT collapsed
  ```rust
  /// Note: This function does NOT collapse `+` tags (e.g., user+tag@example.com).
  /// Gmail may treat `user@gmail.com` and `user+tag@gmail.com` as the same,
  /// but we store them separately to avoid data loss.
  ```

**Fix Strategy**: Normalize to lowercase + trim, add unique constraint

**Testing**:
- [ ] Test case variations (User@Example.COM → user@example.com)
- [ ] Test whitespace handling
- [ ] Test duplicate detection
- [ ] Test all reducer flows with normalized emails

**Estimated Time**: 3 hours (includes migration)

---

### HIGH #2: Implement Browser Fingerprinting (Remove TODOs)

**Severity**: HIGH
**Impact**: Less accurate risk scoring, easier attacker bypass
**Files**: `magic_link.rs:437`, `passkey.rs:811`

#### Tasks:
- [ ] **Add fingerprint to MagicLinkVerified action** (actions.rs)
  ```rust
  MagicLinkVerified {
      token: String,
      email: String,
      ip_address: IpAddr,
      user_agent: String,
      fingerprint: Option<DeviceFingerprint>, // ✅ Add
  }
  ```

- [ ] **Update magic_link reducer** (magic_link.rs:437)
  ```rust
  // Current:
  fingerprint: None, // TODO: Pass from client

  // Fix to:
  fingerprint: fingerprint_clone,
  ```

- [ ] **Add fingerprint to passkey actions** (actions.rs)
  ```rust
  CompletePasskeyLogin {
      credential_id: String,
      signature: Vec<u8>,
      authenticator_data: Vec<u8>,
      client_data_json: Vec<u8>,
      ip_address: IpAddr,
      user_agent: String,
      fingerprint: Option<DeviceFingerprint>, // ✅ Add
  }
  ```

- [ ] **Update passkey reducer** (passkey.rs:811)
  ```rust
  // Current:
  fingerprint: None, // TODO: Pass from client

  // Fix to:
  fingerprint: fingerprint_clone,
  ```

- [ ] **Update integration tests** (magic_link_integration.rs, passkey_integration.rs)
  ```rust
  // Add fingerprint to test actions
  AuthAction::VerifyMagicLink {
      token: token.clone(),
      email: test_email.clone(),
      ip_address: test_ip,
      user_agent: test_user_agent.clone(),
      fingerprint: None, // Test without fingerprint
  }
  ```

- [ ] **Document fingerprint collection** (add to README or docs)
  ```markdown
  ## Browser Fingerprinting

  The auth system supports optional browser fingerprinting for enhanced
  security and risk scoring. To enable:

  1. Use FingerprintJS or similar library on the client
  2. Pass DeviceFingerprint to all auth actions
  3. Risk calculator will use fingerprint for device recognition
  ```

**Fix Strategy**: Wire fingerprint through actions → reducers → risk calculator

**Testing**:
- [ ] Test with fingerprint present
- [ ] Test with fingerprint None (fallback behavior)
- [ ] Verify risk scoring uses fingerprint when available

**Estimated Time**: 4 hours

---

## 📋 MEDIUM ISSUES (Recommended) - 4 Issues

### MEDIUM #1: Fix Clock Skew in Idle Timeout Check

**Severity**: MEDIUM
**Impact**: Legitimate users logged out due to clock skew
**File**: `auth/src/stores/session_redis.rs:267-279`

#### Tasks:
- [ ] **Add negative duration handling** (line 267)
  ```rust
  // Current:
  let idle_duration = now.signed_duration_since(session.last_active);

  if idle_duration > idle_timeout {
      // Reject session
      return Err(AuthError::SessionExpired);
  }

  // Fix to:
  let idle_duration = now.signed_duration_since(session.last_active);

  // Handle clock skew: If last_active is in the future, treat as 0 duration
  let idle_duration = idle_duration.max(Duration::zero());

  if idle_duration > idle_timeout {
      tracing::warn!("Session idle timeout exceeded");
      return Err(AuthError::SessionExpired);
  }
  ```

- [ ] **Add test for negative duration** (session_redis.rs tests)
  ```rust
  #[tokio::test]
  async fn test_idle_timeout_clock_skew() {
      // last_active 1 hour in the future (clock skew)
      let mut session = create_test_session();
      session.last_active = Utc::now() + Duration::hours(1);

      // Should NOT reject (treat as 0 idle time)
      let result = store.get_session(&session.session_id).await;
      assert!(result.is_ok());
  }
  ```

**Fix Strategy**: Clamp negative durations to zero

**Testing**:
- [ ] Test with future last_active
- [ ] Test with normal idle timeout
- [ ] Verify existing tests still pass

**Estimated Time**: 1 hour

---

### MEDIUM #2: Improve Projection Idempotency for Late Events

**Severity**: MEDIUM
**Impact**: Data corruption on late-arriving duplicate events
**File**: `auth/src/projection.rs:82-96`

#### Tasks:
- [ ] **Add event sequence tracking** (add to AuthEvent enum)
  ```rust
  pub struct AuthEvent {
      pub event_id: Uuid,        // Unique event ID
      pub sequence: i64,         // Sequence number
      pub timestamp: DateTime<Utc>,
      pub payload: AuthEventPayload, // Actual event data
  }
  ```

- [ ] **Update UserRegistered projection** (projection.rs:82-96)
  ```rust
  // Current:
  ON CONFLICT (user_id) DO NOTHING

  // Fix to:
  ON CONFLICT (user_id) DO UPDATE SET
      email = EXCLUDED.email,
      name = EXCLUDED.name,
      email_verified = EXCLUDED.email_verified,
      updated_at = EXCLUDED.updated_at
  WHERE users_projection.updated_at < EXCLUDED.updated_at
  ```

- [ ] **Add sequence number to projection tables** (migration)
  ```sql
  ALTER TABLE users_projection ADD COLUMN last_event_sequence BIGINT DEFAULT 0;
  ALTER TABLE devices_projection ADD COLUMN last_event_sequence BIGINT DEFAULT 0;

  CREATE INDEX idx_users_last_event_seq ON users_projection (last_event_sequence);
  CREATE INDEX idx_devices_last_event_seq ON devices_projection (last_event_sequence);
  ```

- [ ] **Update projection logic to check sequence**
  ```rust
  sqlx::query!(
      r#"
      INSERT INTO users_projection (...)
      VALUES (...)
      ON CONFLICT (user_id) DO UPDATE SET
          email = EXCLUDED.email,
          updated_at = EXCLUDED.updated_at,
          last_event_sequence = EXCLUDED.last_event_sequence
      WHERE users_projection.last_event_sequence < EXCLUDED.last_event_sequence
      "#
  )
  ```

**Fix Strategy**: Add sequence numbers to events and projections

**Note**: This is a larger refactor. Alternative: Document that replays should be in order.

**Testing**:
- [ ] Test out-of-order event replay
- [ ] Test duplicate events with different data
- [ ] Verify idempotency

**Estimated Time**: 6 hours (if implementing full sequence tracking) OR 1 hour (if documenting limitation)

**Recommendation**: Document as known limitation for Phase 6, implement in Phase 7

---

### MEDIUM #3: Add Counter Rollback Wrapping Tests

**Severity**: MEDIUM
**Impact**: Unverified edge cases in critical security algorithm
**File**: `auth/src/reducers/passkey.rs:559-571`

#### Tasks:
- [ ] **Add unit tests for counter rollback detection** (passkey.rs tests)
  ```rust
  #[test]
  fn test_counter_rollback_detection() {
      // Helper function
      fn is_rollback(stored: u32, received: u32) -> bool {
          const HALF_SPACE: u32 = u32::MAX / 2;
          if received == stored {
              true // Same counter = replay
          } else {
              let forward_diff = received.wrapping_sub(stored);
              forward_diff > HALF_SPACE
          }
      }

      // Test cases
      assert!(is_rollback(100, 100), "Same counter should be rollback");
      assert!(is_rollback(100, 50), "Backward should be rollback");
      assert!(!is_rollback(100, 101), "Forward should be valid");
      assert!(!is_rollback(100, 150), "Forward jump should be valid");

      // Wrapping cases (most important!)
      assert!(!is_rollback(u32::MAX - 5, 10), "Valid wraparound should be allowed");
      assert!(is_rollback(10, u32::MAX - 5), "Backward wraparound should be rollback");

      // Edge cases
      assert!(!is_rollback(0, u32::MAX / 2), "Forward half-space should be valid");
      assert!(is_rollback(0, (u32::MAX / 2) + 1), "Just past half should be rollback");
      assert!(!is_rollback(u32::MAX, 0), "Wraparound at boundary should be valid");
  }
  ```

- [ ] **Add integration test for cloned authenticator** (passkey_integration.rs)
  ```rust
  #[tokio::test]
  async fn test_passkey_cloned_authenticator_detection() {
      // Setup: Register passkey, authenticate once (counter=101)
      // ...

      // Clone scenario: Attacker uses cloned authenticator with counter=101
      let result = reducer.reduce(
          &mut state,
          AuthAction::CompletePasskeyLogin {
              // ... same credential_id, counter=101 (duplicate)
          },
          &env,
      );

      // Should be rejected (counter didn't advance)
      // Verify no session created, no UserLoggedIn event
  }
  ```

**Fix Strategy**: Comprehensive unit tests for wrapping arithmetic

**Testing**:
- [ ] Run new tests
- [ ] Verify existing integration tests still pass
- [ ] Document the half-space algorithm in comments

**Estimated Time**: 2 hours

---

### MEDIUM #4: Improve TOCTOU Logging Clarity

**Severity**: MEDIUM (behavior is correct, logging is confusing)
**Impact**: Security monitoring may misinterpret logs
**File**: `auth/src/reducers/passkey.rs:561-660`

#### Tasks:
- [ ] **Update logging after rollback check** (line 604)
  ```rust
  // Current:
  tracing::info!("Counter rollback check passed");

  // Fix to:
  tracing::debug!(
      "Preliminary rollback check passed (final verification at CAS)",
      stored_counter = credential.counter,
      received_counter = result.counter
  );
  ```

- [ ] **Update logging after CAS success** (line 626)
  ```rust
  // Current:
  tracing::info!("Passkey counter updated: {} -> {}", ...);

  // Fix to:
  tracing::info!(
      "Counter updated atomically - authentication ALLOWED",
      credential_id = %credential_id,
      old_counter = credential.counter,
      new_counter = result.counter
  );
  ```

- [ ] **Update logging after CAS failure** (line 640)
  ```rust
  // Current:
  tracing::error!("Failed to update passkey counter (CAS conflict)");

  // Fix to:
  tracing::warn!(
      "Concurrent authentication detected - authentication REJECTED",
      credential_id = %credential_id,
      expected_counter = credential.counter,
      received_counter = result.counter,
      reason = "CAS conflict - counter already updated by another request"
  );
  ```

- [ ] **Add documentation comment** (before rollback check)
  ```rust
  // SECURITY: Two-phase counter validation
  //
  // Phase 1: Preliminary rollback check (non-atomic)
  // - Quick rejection of obvious rollback attempts
  // - TOCTOU window exists here (acceptable)
  //
  // Phase 2: Atomic CAS update (final security barrier)
  // - Guarantees counter advances exactly once
  // - Prevents concurrent authentication with same counter
  // - Even if Phase 1 passes for both, only one CAS succeeds
  ```

**Fix Strategy**: Improve logging clarity and add documentation

**Testing**:
- [ ] Review logs in test output
- [ ] Verify monitoring dashboards show correct metrics

**Estimated Time**: 1 hour

---

## 🔧 LOW ISSUES (Nice to Have) - 5 Issues

### LOW #1: Fix Test Code Clippy Warnings (expect_used)

**Severity**: LOW
**Impact**: Test code clippy warnings
**Files**: Multiple test files

#### Tasks:
- [ ] **Add `#[allow(clippy::expect_used)]` to test functions**
  ```rust
  #[tokio::test]
  #[ignore]
  #[allow(clippy::unwrap_used, clippy::expect_used)] // ✅ Add expect_used
  async fn test_redis_token_lifecycle() {
      // ...
  }
  ```

- [ ] **Apply to all test files**:
  - [ ] `stores/session_redis.rs` tests
  - [ ] `stores/challenge_redis.rs` tests
  - [ ] `stores/token_redis.rs` tests
  - [ ] `stores/oauth_token_redis.rs` tests
  - [ ] `stores/rate_limiter_redis.rs` tests
  - [ ] `stores/postgres/user.rs` tests
  - [ ] `stores/postgres/device.rs` tests

**Fix Strategy**: Add clippy allow annotation to all test functions

**Testing**: Run `cargo clippy --all-targets --all-features -- -D warnings`

**Estimated Time**: 30 minutes

---

### LOW #2: Add HKDF to Magic Link Token Generation (Defense-in-Depth)

**Severity**: LOW
**Impact**: Hardening against RNG compromise
**File**: `auth/src/reducers/magic_link.rs:125-133`

#### Tasks:
- [ ] **Add hkdf dependency** (Cargo.toml)
  ```toml
  hkdf = "0.12"
  sha2 = "0.10"
  ```

- [ ] **Update generate_token function** (optional, defense-in-depth)
  ```rust
  use hkdf::Hkdf;
  use sha2::Sha256;

  fn generate_token(&self, email: &str, timestamp: DateTime<Utc>) -> String {
      use base64::Engine;
      use rand::RngCore;

      let mut rng = rand::thread_rng();
      let mut random_bytes = [0u8; 32];
      rng.fill_bytes(&mut random_bytes);

      // Derive token using HKDF (binds to email + timestamp)
      let hkdf = Hkdf::<Sha256>::new(None, &random_bytes);
      let info = format!("magic_link:{}:{}", email, timestamp.timestamp());
      let mut token = [0u8; 32];
      hkdf.expand(info.as_bytes(), &mut token)
          .expect("HKDF expand should never fail with valid length");

      base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(token)
  }
  ```

**Fix Strategy**: Bind tokens to context using HKDF (defense-in-depth)

**Note**: Current approach is **already secure** - this is extra hardening

**Testing**:
- [ ] Verify tokens are still unique
- [ ] Verify token validation still works
- [ ] Performance test (HKDF overhead)

**Estimated Time**: 2 hours (optional)

**Recommendation**: DEFER - current approach is adequate

---

### LOW #3: Add HKDF to OAuth State Generation

**Severity**: LOW
**Impact**: Same as LOW #2
**File**: `auth/src/reducers/oauth.rs:138-142`

**Same as LOW #2 - apply to OAuth state generation**

**Recommendation**: DEFER

---

### LOW #4: Replace panic! with assert! in Tests

**Severity**: LOW
**Impact**: Clearer test failures
**Files**: Multiple test files

#### Tasks:
- [ ] **session_redis.rs:835, 838, 941, 944** - Replace panic! with assert!
  ```rust
  // Current:
  match result {
      Err(AuthError::SessionExpired) => {
          // ✅ Expected
      }
      Ok(_) => {
          panic!("Expected SessionExpired error, but got success");
      }
      Err(e) => {
          panic!("Expected SessionExpired error, but got: {:?}", e);
      }
  }

  // Fix to:
  assert!(
      matches!(result, Err(AuthError::SessionExpired)),
      "Expected SessionExpired, got: {:?}",
      result
  );
  ```

**Fix Strategy**: Use assert macros for clearer test failures

**Testing**: Run test suite, verify failures are clear

**Estimated Time**: 30 minutes

---

### LOW #5: Remove panic! from Mock User Repository

**Severity**: LOW
**Impact**: None (mock code only)
**File**: `auth/src/mocks/user.rs:364`

#### Tasks:
- [ ] **Replace panic! with proper error return** (line 364)
  ```rust
  // Current:
  Err(_) => panic!("Unexpected error during concurrent update"),

  // Fix to:
  Err(e) => {
      tracing::error!("Concurrent update error in mock: {}", e);
      return Err(AuthError::InternalError(
          "Mock concurrent update failed".to_string()
      ));
  }
  ```

**Fix Strategy**: Return error instead of panic

**Testing**: Run mock tests

**Estimated Time**: 15 minutes

---

## 📊 Summary

| Priority | Count | Estimated Time |
|----------|-------|----------------|
| CRITICAL | 3 | 3.5 hours |
| HIGH | 2 | 7 hours |
| MEDIUM | 4 | 10 hours (or 4 hours if deferring projection refactor) |
| LOW | 5 | 2 hours (or 0 if deferring HKDF) |
| **TOTAL** | **14** | **22.5 hours** (or **14.5 hours minimal**) |

---

## 🎯 Recommended Fix Order

### Phase 1: CRITICAL Only (3.5 hours)
1. Fix unwrap_or_else in risk calculation (30 min)
2. Fix memory leak in session set TTL (2 hours)
3. Fix cipher cloning with Arc (1 hour)

**Result**: Production-safe (no crashes, no memory leaks, no crypto failures)

---

### Phase 2: HIGH Priority (7 hours)
4. Add email normalization (3 hours)
5. Implement browser fingerprinting (4 hours)

**Result**: Security-hardened (no account collisions, better risk scoring)

---

### Phase 3: MEDIUM Priority (4 hours - deferred items)
6. Fix clock skew handling (1 hour)
7. Add counter rollback tests (2 hours)
8. Improve TOCTOU logging (1 hour)
9. ~~Projection idempotency~~ (DEFER to Phase 7 - document limitation)

**Result**: Robust edge case handling

---

### Phase 4: LOW Priority (30 min - skip HKDF)
10. Fix test clippy warnings (30 min)
11. ~~HKDF hardening~~ (DEFER - not needed)
12. ~~Replace panic in tests~~ (DEFER - nice to have)

**Result**: Clean code quality

---

## 🚀 Deployment Strategy

### Minimal Viable Security (CRITICAL only)
- **Fix**: Issues #1-3
- **Time**: 3.5 hours
- **Result**: Production-safe (no crashes/leaks)
- **Risk**: Email collision possible, risk scoring suboptimal

### Recommended Production (CRITICAL + HIGH)
- **Fix**: Issues #1-5
- **Time**: 10.5 hours (~1.5 days)
- **Result**: Security-hardened
- **Risk**: Edge cases not fully tested

### Comprehensive Fix (CRITICAL + HIGH + MEDIUM)
- **Fix**: Issues #1-9 (defer projection refactor)
- **Time**: 14.5 hours (~2 days)
- **Result**: Production-grade, robust
- **Risk**: None significant

---

## 📋 Tracking Progress

Update this section as fixes are completed:

- [ ] CRITICAL #1: unwrap_or_else fixes (3 files)
- [ ] CRITICAL #2: Memory leak fix (session_redis.rs)
- [ ] CRITICAL #3: Cipher cloning fix (oauth_token_redis.rs)
- [ ] HIGH #1: Email normalization (utils.rs + reducers + migration)
- [ ] HIGH #2: Browser fingerprinting (actions + reducers + tests)
- [ ] MEDIUM #1: Clock skew handling (session_redis.rs)
- [ ] MEDIUM #2: ~~Projection idempotency~~ (DEFERRED to Phase 7)
- [ ] MEDIUM #3: Counter rollback tests (passkey.rs)
- [ ] MEDIUM #4: TOCTOU logging (passkey.rs)
- [ ] LOW #1: Test clippy warnings
- [ ] LOW #2-5: ~~HKDF, panic! fixes~~ (DEFERRED)

---

**Created**: 2025-11-09
**Status**: Ready to begin fixes
**Next**: Start with CRITICAL #1 (unwrap_or_else)
