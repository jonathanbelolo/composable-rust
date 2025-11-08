# Passkey Reducer - In-Depth Code Review

**Date**: 2025-11-08
**Component**: `auth/src/reducers/passkey.rs`
**Priority**: HIGH
**Risk Level**: **CRITICAL** ⚠️
**Lines of Code**: 495

---

## Executive Summary

The Passkey Reducer implements WebAuthn/FIDO2 passwordless authentication. While the overall structure is sound, this reducer has **2 CRITICAL security issues** that make it **NOT production-ready**:

1. **Challenge storage is mocked** - Using placeholder "mock_challenge_id" instead of actual challenge state management
2. **No counter rollback detection** - Counter is updated but NOT checked for replay attacks

These issues **MUST be fixed** before any production use.

**Key Strengths**:
- ✅ WebAuthn flow structure correct (registration + login)
- ✅ Origin and RP ID validation configured
- ✅ Event sourcing structure in place
- ✅ 8 integration tests passing

**Critical Issues**:
- ❌ **SECURITY CRITICAL**: Challenge storage is mocked (replay attack vulnerable)
- ❌ **SECURITY CRITICAL**: No counter rollback detection (replay attack vulnerable)
- ❌ Missing PasskeyUsed event emission
- ❌ Missing PasskeyRegistered event emission
- ❌ Hardcoded risk scores (ignoring RiskCalculator)
- ❌ Challenge TTL not actually used

---

## 1. TODOs Found

### TODO #1 & #4: Store Challenge in State (Registration & Login)
**Locations**: Lines 177, 275
**Code**:
```rust
// Line 177 (Registration)
Ok(_challenge) => {
    // Store challenge in state via an event
    // In a real implementation, this would be a separate action
    // For now, we'll just log it
    tracing::info!("Generated WebAuthn registration challenge");
    None // TODO: Return action to store challenge in state
}

// Line 275 (Login)
Ok(_challenge) => {
    tracing::info!("Generated WebAuthn authentication challenge");
    None // TODO: Return action to store challenge in state
}
```

**Priority**: **CRITICAL** ⚠️
**Decision**: **MUST Implement Immediately**

**Issue**: Challenges are generated but never stored. The reducer later tries to use "mock_challenge_id" for verification, which means **ANY challenge can be used** - this completely breaks WebAuthn security.

**WebAuthn Spec Requirement**:
> "The challenge MUST be stored on the server side and used exactly once for verification. Challenges MUST expire after a short period (typically 5 minutes)."

**Recommendation**:

**Option A: Store challenges in AuthState (in-memory, for simple cases)**
```rust
// Add to state.rs
pub struct WebAuthnChallengeState {
    pub challenge_id: String,
    pub challenge_bytes: Vec<u8>,
    pub user_id: Option<UserId>,  // For login
    pub created_at: DateTime<Utc>,
    pub challenge_type: ChallengeType,  // Registration or Authentication
}

pub enum ChallengeType {
    Registration,
    Authentication,
}

// Add to AuthState
pub struct AuthState {
    // ... existing fields
    pub webauthn_challenge: Option<WebAuthnChallengeState>,
}
```

Return action to store challenge:
```rust
Ok(challenge) => {
    Some(AuthAction::WebAuthnChallengeGenerated {
        challenge_id: challenge.id,
        challenge_bytes: challenge.bytes,
        user_id: Some(user_id),  // or None for registration
        challenge_type: ChallengeType::Registration,
    })
}
```

**Option B: Store challenges in Redis (recommended for production)**
```rust
// Create ChallengeStore trait
pub trait ChallengeStore: Send + Sync {
    async fn store_challenge(
        &self,
        challenge_id: &str,
        challenge: &WebAuthnChallenge,
        ttl: Duration,
    ) -> Result<()>;

    async fn get_challenge(
        &self,
        challenge_id: &str,
    ) -> Result<Option<WebAuthnChallenge>>;

    async fn delete_challenge(&self, challenge_id: &str) -> Result<()>;
}

// Store in Redis with TTL
impl ChallengeStore for RedisChallengeStore {
    async fn store_challenge(...) {
        // SET challenge:{id} {bytes} EX 300  (5 minutes)
    }
}
```

**Impact**: **CRITICAL** - Without proper challenge storage, the WebAuthn flow is completely insecure. Anyone can forge authentication responses.

---

### TODO #2 & #5: Get Challenge from State
**Locations**: Lines 197, 294
**Code**:
```rust
// Line 197 (Registration verification)
let challenge_id = "mock_challenge_id".to_string(); // TODO: Get from state

// Line 294 (Login verification)
let challenge_id = "mock_challenge_id".to_string(); // TODO: Get from state
```

**Priority**: **CRITICAL** ⚠️
**Decision**: **MUST Implement Immediately** (depends on TODO #1)

**Recommendation**:

If using **Option A** (AuthState):
```rust
// Get challenge from state
let Some(webauthn_challenge) = &state.webauthn_challenge else {
    tracing::warn!("No WebAuthn challenge found in state");
    return smallvec![Effect::None];
};

// Verify challenge hasn't expired (5 minutes)
if Utc::now() - webauthn_challenge.created_at > Duration::minutes(5) {
    tracing::warn!("WebAuthn challenge expired");
    state.webauthn_challenge = None;
    return smallvec![Effect::None];
}

// Verify challenge type matches
if webauthn_challenge.challenge_type != ChallengeType::Registration {
    tracing::warn!("Wrong challenge type");
    state.webauthn_challenge = None;
    return smallvec![Effect::None];
}

let challenge_id = webauthn_challenge.challenge_id.clone();
let challenge_bytes = webauthn_challenge.challenge_bytes.clone();

// Clear challenge (single-use)
state.webauthn_challenge = None;

// Use challenge_id and challenge_bytes for verification
```

If using **Option B** (Redis):
```rust
// Get challenge from Redis via provider
let challenge_store = env.challenge_store.clone();
let challenge = match challenge_store.get_challenge(&challenge_id).await {
    Ok(Some(c)) => c,
    Ok(None) => {
        tracing::warn!("Challenge not found or expired");
        return smallvec![Effect::None];
    }
    Err(e) => {
        tracing::error!("Failed to get challenge: {}", e);
        return smallvec![Effect::None];
    }
};

// Delete challenge (single-use)
let _ = challenge_store.delete_challenge(&challenge_id).await;
```

**Impact**: **CRITICAL** - Using mock challenge ID means verification will always fail with real WebAuthn providers.

---

### TODO #3: Return Success Event
**Location**: Line 225
**Code**:
```rust
match users.create_passkey_credential(&credential).await {
    Ok(()) => {
        tracing::info!("Passkey registered successfully");
        None // TODO: Return success event
    }
    Err(_) => None,
}
```

**Priority**: HIGH
**Decision**: **Implement Now**

**Recommendation**:
Emit `PasskeyRegistered` event:
```rust
Ok(()) => {
    Some(AuthAction::PasskeyRegistered {
        user_id,
        device_id,
        credential_id: result.credential_id,
    })
}
```

Add to events.rs:
```rust
pub enum AuthEvent {
    // ... existing events

    /// Passkey credential registered.
    PasskeyRegistered {
        user_id: UserId,
        device_id: DeviceId,
        credential_id: String,
        timestamp: DateTime<Utc>,
    },
}
```

**Impact**: No audit trail of passkey registrations. Cannot track when/where passkeys were registered.

---

## 2. Hardcoded Values

### HV #1: Mock Challenge ID ❌ **CRITICAL**
**Locations**: Lines 197, 294
**Value**: `"mock_challenge_id"`
**Solution**: Implement proper challenge storage (see TODOs #1, #2, #4, #5)
**Priority**: **CRITICAL**

---

### HV #2: Risk Score (0.05)
**Locations**: Lines 377, 412, 439
**Value**: `0.05`
**Solution**: Use `RiskCalculator` provider
**Priority**: HIGH

**Recommendation**: Same as Magic Link and OAuth - use RiskCalculator:
```rust
// Calculate risk for passkey auth (should still be very low)
let risk_assessment = risk.assess_login_risk(&LoginContext {
    user_id: Some(user_id),
    email: email.clone(),
    ip_address,
    user_agent: user_agent.clone(),
    device_id: Some(device_id),
    last_login_location: None,
    last_login_at: None,
}).await?;

// Passkeys are hardware-backed, so risk should be very low
// But RiskCalculator can still detect anomalies (impossible travel, etc.)
let login_risk_score = risk_assessment.score.min(0.1);  // Cap at 0.1
```

---

### HV #3: Session Duration (24 hours)
**Locations**: Lines 373, 435, 449
**Value**: `chrono::Duration::hours(24)`
**Solution**: Extract to configuration
**Priority**: MEDIUM

**Recommendation**:
```rust
pub struct PasskeyReducer<...> {
    _challenge_ttl_minutes: i64,
    expected_origin: String,
    expected_rp_id: String,
    session_ttl_hours: i64,  // NEW
    _phantom: std::marker::PhantomData<...>,
}
```

---

### HV #4: Passkey Method String
**Location**: Line 408
**Value**: `"passkey"`
**Solution**: Extract to constants (same as Magic Link and OAuth)
**Priority**: LOW

---

### HV #5: Default Origin and RP ID (localhost)
**Locations**: Lines 95-96
**Values**: `"http://localhost:3000"`, `"localhost"`
**Solution**: Already configurable via `with_config()` ✅
**Priority**: LOW (already handled)

**Note**: These defaults are reasonable for development. Production apps must use `with_config()`.

---

## 3. Security Issues

### Security Issue #1: Mock Challenge Storage ❌ **CRITICAL**
**Locations**: Lines 197, 294
**Severity**: **CRITICAL**

**Issue**: Challenges are not actually stored or retrieved. Using a hardcoded mock value.

**Attack Vector**:
1. Attacker can replay old authentication responses
2. Attacker can use responses from different users
3. No challenge expiration enforcement
4. Single-use challenge constraint not enforced

**Fix**: Implement proper challenge storage (see TODO #1, #2, #4, #5)

**Must fix before ANY production use.**

---

### Security Issue #2: No Counter Rollback Detection ❌ **CRITICAL**
**Locations**: Lines 327-332
**Severity**: **CRITICAL**

**Current Code**:
```rust
// Update counter
if let Err(_) = users
    .update_passkey_counter(&credential_id, result.counter)
    .await
{
    tracing::error!("Failed to update passkey counter");
}
```

**Issue**: Counter is updated but **NOT checked for rollback**. This defeats the entire purpose of the signature counter.

**WebAuthn Spec Requirement**:
> "The signature counter MUST be checked. If the new counter value is less than or equal to the stored counter value, authentication MUST fail (indicating a cloned authenticator)."

**Attack Scenario**:
1. Attacker clones a physical FIDO2 key
2. Uses both the original and cloned key
3. Since we don't check for counter rollback, both keys work
4. Attacker can authenticate even after key is "revoked"

**Fix**: Add counter rollback check BEFORE accepting authentication:
```rust
// Get current counter from database
let credential = match users.get_passkey_credential(&credential_id).await {
    Ok(c) => c,
    Err(_) => {
        tracing::warn!("Credential not found: {}", credential_id);
        return None;
    }
};

// Verify assertion
let result = match webauthn.verify_authentication(...).await {
    Ok(r) => r,
    Err(_) => {
        tracing::warn!("Passkey verification failed");
        return None;
    }
};

// CRITICAL: Check for counter rollback (replay attack detection)
if result.counter <= credential.counter {
    tracing::error!(
        "Counter rollback detected! Stored: {}, Received: {}. Possible cloned authenticator.",
        credential.counter,
        result.counter
    );

    // Emit security event
    // TODO: Emit PasskeyCounterRollbackDetected event

    // REJECT the authentication
    return None;
}

// Counter is valid - proceed with update
if let Err(e) = users.update_passkey_counter(&credential_id, result.counter).await {
    tracing::error!("Failed to update passkey counter: {}", e);
    // This is a serious error - counter update failed
    // Should we still allow authentication? Probably not for max security.
    return None;
}
```

**Impact**: **CRITICAL** - Cloned authenticators can be used. Replay attacks possible.

**Must fix before production.**

---

### Security Issue #3: Missing PasskeyUsed Event
**Location**: Line 391 (comment mentions it but it's not emitted)
**Severity**: HIGH

**Issue**: Comment says "PasskeyUsed event is emitted in CompletePasskeyLogin" but it's not actually emitted anywhere.

**Purpose of PasskeyUsed Event**:
- Track counter updates for projection
- Audit trail of passkey usage
- Detect unusual usage patterns
- Counter history for forensics

**Fix**: Add PasskeyUsed event:
```rust
// In CompletePasskeyLogin, after counter check passes
events.push(AuthEvent::PasskeyUsed {
    user_id: result.user_id,
    device_id: result.device_id,
    credential_id: credential_id.clone(),
    counter: result.counter,
    ip_address,
    timestamp: now,
});
```

Add to events.rs:
```rust
pub enum AuthEvent {
    // ... existing events

    /// Passkey was used for authentication.
    PasskeyUsed {
        user_id: UserId,
        device_id: DeviceId,
        credential_id: String,
        counter: u32,
        ip_address: IpAddr,
        timestamp: DateTime<Utc>,
    },
}
```

**Impact**: No counter history in event store. Cannot reconstruct counter state from events. Breaks event sourcing model.

---

### Security Issue #4: Challenge TTL Not Enforced
**Location**: Lines 76-77, 94
**Severity**: MEDIUM

**Issue**: `_challenge_ttl_minutes` field is declared but **never actually used**:
```rust
_challenge_ttl_minutes: i64,  // Underscore prefix means "unused"
```

Comment on line 76 says "Currently unused - will be used when challenge state management is implemented."

**Fix**: When implementing challenge storage (TODO #1), use this field:
```rust
// Remove underscore prefix
challenge_ttl_minutes: i64,

// Use in challenge expiration check
if Utc::now() - challenge.created_at > Duration::minutes(self.challenge_ttl_minutes) {
    tracing::warn!("Challenge expired");
    return None;
}
```

**Impact**: Challenges could theoretically live forever (once storage is implemented). 5-minute limit not enforced.

---

### Security Issue #5: Direct CRUD in PasskeyCredential Storage
**Location**: Line 222
**Severity**: LOW (architectural violation, not security)

**Issue**:
```rust
match users.create_passkey_credential(&credential).await {
```

This violates the event-sourcing architecture - directly writing to database instead of emitting event.

**Fix**: Emit `PasskeyRegistered` event instead (see TODO #3).

---

### Security Review Checklist

- ❌ **Challenge Storage**: Mock implementation ❌ **CRITICAL**
- ❌ **Challenge Single-Use**: Not enforced ❌ **CRITICAL**
- ❌ **Challenge Expiration**: TTL field unused ❌
- ❌ **Counter Rollback Detection**: Not implemented ❌ **CRITICAL**
- ✅ **Origin Validation**: Configured ✅
- ✅ **RP ID Validation**: Configured ✅
- ⚠️ **Public Key Verification**: Assumed handled by WebAuthnProvider ⚠️
- ❌ **PasskeyUsed Event**: Not emitted ❌
- ❌ **PasskeyRegistered Event**: Not emitted ❌

**Overall Security Score**: 2/9 (22%)
**Critical Issues**: 3 (challenge storage, single-use, counter rollback)

---

## 4. Event Sourcing Alignment

### Missing Events

**CRITICAL**:
1. ❌ `PasskeyRegistered` - Should be emitted when passkey is created
2. ❌ `PasskeyUsed` - Should be emitted on each authentication (with counter)

**Currently Emitted** (in PasskeyLoginSuccess):
1. ✅ `DeviceAccessed` - For device trust calculation
2. ✅ `UserLoggedIn` - For audit trail

**Issue**: Without `PasskeyRegistered` and `PasskeyUsed` events, the passkey credential state cannot be reconstructed from the event store. This breaks the event sourcing architecture.

**Fix**: Emit proper events:
```rust
// In CompletePasskeyRegistration
events.push(AuthEvent::PasskeyRegistered {
    user_id,
    device_id,
    credential_id,
    public_key: credential.public_key.clone(),
    counter: credential.counter,
    timestamp: Utc::now(),
});

// In CompletePasskeyLogin (after counter check)
events.push(AuthEvent::PasskeyUsed {
    user_id,
    device_id,
    credential_id,
    counter: result.counter,
    ip_address,
    timestamp: Utc::now(),
});
```

---

## 5. Test Coverage

### Existing Tests (8 tests in passkey_integration.rs, all passing)

1. ✅ `test_passkey_registration_flow` - Registration initiation
2. ✅ `test_passkey_registration_completion` - Registration completion
3. ✅ `test_passkey_login_initiation` - Login initiation
4. ✅ `test_passkey_login_completion` - Login completion
5. ✅ `test_passkey_login_success_creates_session` - Session creation
6. ✅ `test_session_metadata_after_passkey_login` - Session metadata
7. ✅ `test_passkey_custom_webauthn_config` - Custom config
8. ✅ `test_passkey_security_properties` - Security validation

**Note**: Tests pass because they don't actually execute the async effects or verify WebAuthn properly. They just verify the reducer structure.

### Missing Tests

**CRITICAL Priority:**
- ❌ Counter rollback detection test (MUST have)
- ❌ Challenge expiration test
- ❌ Challenge single-use enforcement test
- ❌ Challenge storage/retrieval test

**High Priority:**
- ❌ PasskeyUsed event emission test
- ❌ PasskeyRegistered event emission test
- ❌ Origin validation test (wrong origin should fail)
- ❌ RP ID validation test (wrong RP ID should fail)

**Medium Priority:**
- ❌ Concurrent authentication attempts (same credential)
- ❌ Multiple passkeys for same user
- ❌ Credential not found handling
- ❌ User not found handling

---

## 6. Code Quality & Style

### Strengths
- ✅ Excellent module documentation with flow diagrams
- ✅ Clear separation of registration vs login flows
- ✅ Proper async/await patterns
- ✅ Good error logging
- ✅ Configurable origin and RP ID

### Improvements Needed
- Remove underscore prefix from `challenge_ttl_minutes` when implementing
- Extract risk score calculation to RiskCalculator
- Extract session TTL to configuration
- Implement proper challenge storage
- Add counter rollback detection
- Emit missing events

---

## 7. Comparison with Other Reducers

| Aspect | Magic Link | OAuth | Passkey |
|--------|-----------|-------|---------|
| **Token/Challenge Security** | Constant-time ✅ | NOT constant-time ❌ | Mock storage ❌ **CRITICAL** |
| **Storage Completeness** | In-memory ✅ | State ✅ | Mock ❌ **CRITICAL** |
| **Single-Use Enforcement** | Yes ✅ | Yes ✅ | Not enforced ❌ |
| **Expiration Checking** | Yes ✅ | Yes ✅ | Not implemented ❌ |
| **Replay Protection** | Yes ✅ | Yes ✅ | NO ❌ **CRITICAL** |
| **Risk Calculation** | Hardcoded ❌ | Hardcoded ❌ | Hardcoded ❌ |
| **Event Sourcing** | Correct ✅ | Correct ✅ | Incomplete ❌ |
| **Test Coverage** | 8 tests ✅ | 9 tests ✅ | 8 tests ✅ |

**Passkey is the LEAST complete** of the three reducers.

---

## 8. Recommendations

### IMMEDIATE (BLOCKS ALL USE)
1. ❌ **CRITICAL: Implement challenge storage** (TODO #1, #2, #4, #5)
2. ❌ **CRITICAL: Implement counter rollback detection** (Security Issue #2)
3. ❌ **CRITICAL: Remove mock challenge ID** (HV #1)

### High Priority (Before Production)
4. ✅ **Emit PasskeyUsed event** (Security Issue #3)
5. ✅ **Emit PasskeyRegistered event** (TODO #3)
6. ✅ **Use challenge_ttl_minutes field** (Security Issue #4)
7. ✅ **Integrate RiskCalculator** (HV #2)
8. ✅ **Add critical security tests** (counter rollback, challenge expiration)

### Medium Priority (1-2 weeks)
9. ⏱ **Extract session TTL to config** (HV #3)
10. ⏱ **Add origin/RP ID validation tests**
11. ⏱ **Add security event for counter rollback**
12. ⏱ **Improve error handling** (return specific errors, not just None)

### Long-Term (Future Enhancement)
13. 📅 **Add device attestation verification**
14. 📅 **Add conditional UI support** (autofill)
15. 📅 **Add passkey backup/recovery**
16. 📅 **Add telemetry** (passkey usage, counter rollbacks)

---

## 9. Detailed Implementation Plan for Critical Fixes

### Fix #1: Challenge Storage (Redis-based)

**Step 1**: Create ChallengeStore trait
```rust
// auth/src/providers/challenge.rs
pub trait ChallengeStore: Send + Sync {
    async fn store_challenge(
        &self,
        challenge_id: &str,
        user_id: Option<UserId>,
        challenge_bytes: Vec<u8>,
        challenge_type: ChallengeType,
        ttl: Duration,
    ) -> Result<()>;

    async fn get_and_delete_challenge(
        &self,
        challenge_id: &str,
    ) -> Result<Option<WebAuthnChallenge>>;
}

pub struct WebAuthnChallenge {
    pub user_id: Option<UserId>,
    pub challenge_bytes: Vec<u8>,
    pub challenge_type: ChallengeType,
    pub created_at: DateTime<Utc>,
}
```

**Step 2**: Implement Redis storage
```rust
// auth/src/stores/challenge_redis.rs
pub struct RedisChallengeStore {
    pool: deadpool_redis::Pool,
}

impl ChallengeStore for RedisChallengeStore {
    async fn store_challenge(...) -> Result<()> {
        let key = format!("webauthn:challenge:{}", challenge_id);
        let value = serde_json::to_string(&WebAuthnChallenge { ... })?;

        let mut conn = self.pool.get().await?;
        conn.set_ex(&key, value, ttl.num_seconds() as usize).await?;
        Ok(())
    }

    async fn get_and_delete_challenge(...) -> Result<Option<WebAuthnChallenge>> {
        let key = format!("webauthn:challenge:{}", challenge_id);

        let mut conn = self.pool.get().await?;

        // Get and delete in one operation (GETDEL command)
        let value: Option<String> = conn.get_del(&key).await?;

        match value {
            Some(v) => Ok(Some(serde_json::from_str(&v)?)),
            None => Ok(None),
        }
    }
}
```

**Step 3**: Update reducer to use challenge store
```rust
// In InitiatePasskeyLogin
Ok(challenge) => {
    // Store challenge in Redis
    env.challenge_store.store_challenge(
        &challenge.id,
        Some(user.user_id),
        challenge.bytes,
        ChallengeType::Authentication,
        Duration::minutes(self.challenge_ttl_minutes),
    ).await?;

    Some(AuthAction::WebAuthnChallengeReady {
        challenge_id: challenge.id,
        challenge_options: challenge.options,  // Send to client
    })
}

// In CompletePasskeyLogin
let challenge = match env.challenge_store
    .get_and_delete_challenge(&challenge_id)
    .await?
{
    Some(c) => c,
    None => {
        tracing::warn!("Challenge not found or expired");
        return None;
    }
};

// Verify challenge hasn't been tampered with
let result = webauthn.verify_authentication(
    &challenge.challenge_bytes,  // Use actual challenge bytes
    &assertion_response,
    &credential,
    &origin,
    &rp_id,
).await?;
```

---

### Fix #2: Counter Rollback Detection

```rust
// In CompletePasskeyLogin (lines 298-333)
let credential = match users.get_passkey_credential(&credential_id).await {
    Ok(c) => c,
    Err(_) => {
        tracing::warn!("Credential not found: {}", credential_id);
        return None;
    }
};

// Get challenge from store (after implementing Fix #1)
let challenge = match env.challenge_store.get_and_delete_challenge(&challenge_id).await {
    Ok(Some(c)) => c,
    Ok(None) => {
        tracing::warn!("Challenge not found or expired");
        return None;
    }
    Err(e) => {
        tracing::error!("Failed to retrieve challenge: {}", e);
        return None;
    }
};

// Verify assertion
let result = match webauthn.verify_authentication(
    &challenge.challenge_bytes,
    &assertion_response,
    &credential,
    &origin,
    &rp_id,
).await {
    Ok(r) => r,
    Err(e) => {
        tracing::warn!("Passkey verification failed: {}", e);
        return None;
    }
};

// ═══════════════════════════════════════════════════════════════
// CRITICAL: Counter Rollback Detection (Replay Attack Prevention)
// ═══════════════════════════════════════════════════════════════
if result.counter <= credential.counter {
    tracing::error!(
        "🚨 SECURITY ALERT: Counter rollback detected!\n\
         Credential ID: {}\n\
         Stored counter: {}\n\
         Received counter: {}\n\
         This indicates a CLONED AUTHENTICATOR or REPLAY ATTACK.",
        credential_id,
        credential.counter,
        result.counter
    );

    // Emit security event for monitoring
    // TODO: Add CounterRollbackDetected event

    // REJECT authentication
    return None;
}

// Counter is valid - update it
match users.update_passkey_counter(&credential_id, result.counter).await {
    Ok(()) => {
        tracing::info!("Passkey counter updated: {} -> {}", credential.counter, result.counter);
    }
    Err(e) => {
        tracing::error!("CRITICAL: Failed to update passkey counter: {}", e);
        // Counter update failure is serious - don't allow auth
        return None;
    }
}

// Continue with login success...
```

---

## 10. Conclusion

The Passkey Reducer has **3 CRITICAL security issues** that make it **COMPLETELY UNSUITABLE for production use**:

1. ❌ Mock challenge storage (replay attack vulnerable)
2. ❌ No challenge single-use enforcement
3. ❌ No counter rollback detection (cloned authenticator detection)

**These issues MUST be fixed before ANY use**, even in development/testing with real WebAuthn devices.

**Blockers for Production**:
1. Implement challenge storage (Redis or in-memory)
2. Implement counter rollback detection
3. Emit PasskeyUsed and PasskeyRegistered events
4. Add critical security tests

**Estimated Fix Time**: 12-16 hours

**Risk Assessment**: **CRITICAL** ⚠️

**Recommendation**:
- **DO NOT use this reducer with real WebAuthn devices** until critical fixes are implemented
- **DO NOT deploy to production** under any circumstances
- Focus on Magic Link and OAuth flows for MVP
- Defer WebAuthn to Phase 6B with proper implementation

---

**Reviewed By**: Claude Code
**Review Status**: ✅ Complete (with CRITICAL blockers identified)
**Next Review**: Events & Projection System
**BLOCKERS**: 3 critical security issues MUST be fixed before use
