# OAuth Reducer - In-Depth Code Review

**Date**: 2025-11-08
**Component**: `auth/src/reducers/oauth.rs`
**Priority**: HIGH
**Risk Level**: HIGH
**Lines of Code**: 464

---

## Executive Summary

The OAuth Reducer implements OAuth2/OIDC authentication flows with CSRF protection and event sourcing. The implementation has **good fundamentals** (CSRF state generation, expiration checking, single-use enforcement), but has **1 critical security issue** (non-constant-time state comparison), **6 TODOs**, and **7 hardcoded values** that must be addressed before production.

**Key Strengths**:
- ✅ Cryptographically secure CSRF state generation (256 bits)
- ✅ State expiration checking (5 minutes)
- ✅ Single-use state enforcement
- ✅ Event sourcing implemented correctly
- ✅ 9 integration tests passing

**Critical Issues**:
- ❌ **SECURITY**: CSRF state comparison NOT constant-time (timing attack vulnerable)
- ❌ OAuth token storage not implemented (access_token, refresh_token)
- ❌ Provider user ID extraction is placeholder
- ❌ Hardcoded risk scores (ignoring RiskCalculator)
- ❌ Hardcoded device names and types
- ❌ Missing HTTP redirect handling

---

## 1. TODOs Found

### TODO #1: Move Risk Calculation to RiskCalculator Provider
**Location**: Line 102
**Code**:
```rust
/// TODO: Move to RiskCalculator provider (Phase 6B).
fn calculate_basic_risk(&self, _ip_address: IpAddr, _user_agent: &str) -> f32 {
    // Placeholder - just return low risk for now
    // In Phase 6B, we'll use the RiskCalculator provider
    0.1
}
```

**Priority**: HIGH
**Decision**: **Implement Now** (Same as Magic Link #5)

**Recommendation**:
Remove this method entirely and use the `RiskCalculator` provider from environment:
```rust
// In OAuthSuccess handler (line 277)
let risk = env.risk.clone();

// Inside the Future
let risk_assessment = risk.assess_login_risk(&LoginContext {
    user_id: Some(final_user_id),
    email: email_clone.clone(),
    ip_address,
    user_agent: user_agent_clone.clone(),
    device_id: Some(device_id),
    last_login_location: None,
    last_login_at: None,
}).await?;

let login_risk_score = risk_assessment.score;
```

**Impact**: All OAuth logins have the same risk score (0.1), defeating risk-based authentication.

---

### TODO #2: Return Action with Redirect URL
**Location**: Line 160
**Code**:
```rust
match oauth_provider.build_authorization_url(provider, &state_param, &redirect_uri).await {
    Ok(_auth_url) => {
        // In a real implementation, this would trigger an HTTP redirect
        // For now, we'll emit a "redirect ready" action
        // The web framework integration will handle the actual redirect
        None // TODO: Return action with redirect URL when we have HTTP effects
    }
    // ...
}
```

**Priority**: MEDIUM
**Decision**: **Defer to Phase 6B** (Web framework integration)

**Recommendation**:
Add action to `actions.rs`:
```rust
pub enum AuthAction {
    // ...

    /// OAuth redirect ready.
    OAuthRedirectReady {
        /// Authorization URL to redirect to.
        auth_url: String,
        /// OAuth provider.
        provider: OAuthProvider,
    },
}
```

Return this action instead of `None`:
```rust
Ok(auth_url) => {
    Some(AuthAction::OAuthRedirectReady {
        auth_url,
        provider,
    })
}
```

**Impact**: Currently no way for web framework to know where to redirect user. Blocks integration with Axum/other HTTP frameworks.

---

### TODO #3: Get Actual Provider User ID
**Location**: Line 329
**Code**:
```rust
events.push(AuthEvent::OAuthAccountLinked {
    user_id: final_user_id,
    provider,
    provider_user_id: format!("oauth-{}", final_user_id.0), // TODO: Get actual provider user ID
    provider_email: email_clone.clone(),
    timestamp: now,
});
```

**Priority**: HIGH
**Decision**: **Implement Now**

**Issue**: Currently using a placeholder format instead of the actual provider user ID (e.g., Google's `sub` claim).

**Recommendation**:
Update `OAuthUserInfo` struct to include `provider_user_id`:
```rust
// In providers/mod.rs
pub struct OAuthUserInfo {
    pub provider_user_id: String,  // The provider's unique user ID
    pub email: String,
    pub email_verified: bool,
    pub name: Option<String>,
    pub picture: Option<String>,
}
```

Update OAuth2Provider trait to return this in `fetch_user_info`:
```rust
async fn fetch_user_info(&self, provider: OAuthProvider, access_token: &str) -> Result<OAuthUserInfo>;
```

Use it in the reducer:
```rust
// Line 230: fetch_user_info already returns OAuthUserInfo
Ok(user_info) => {
    Some(AuthAction::OAuthSuccess {
        // ... existing fields
        provider_user_id: user_info.provider_user_id,  // Add this field
    })
}

// Line 329: Use actual provider user ID
provider_user_id: provider_user_id.clone(),  // Not placeholder
```

**Impact**: Cannot properly link OAuth accounts without actual provider user ID. User could authenticate multiple times and create duplicate accounts.

---

### TODO #4: Store OAuth Access Token and Refresh Token
**Location**: Line 402
**Code**:
```rust
// TODO: Store OAuth access_token and refresh_token
let _ = (access_token, refresh_token);
```

**Priority**: HIGH
**Decision**: **Design Decision Needed**

**Question**: Where should OAuth tokens be stored?

**Options**:

**Option A: Store in Session (Redis) - Recommended**
```rust
// Update Session struct to include OAuth tokens
pub struct Session {
    // ... existing fields
    pub oauth_access_token: Option<String>,
    pub oauth_refresh_token: Option<String>,
    pub oauth_token_expires_at: Option<DateTime<Utc>>,
}
```
- ✅ Pros: Automatic cleanup via TTL, simple to implement
- ❌ Cons: Lost when session expires, requires re-authentication

**Option B: Store in PostgreSQL (oauth_links_projection)**
```rust
// Store in oauth_links_projection table
pub struct OAuthLink {
    // ... existing fields
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}
```
- ✅ Pros: Persistent, can refresh tokens without re-authentication
- ❌ Cons: Requires encryption, more complex

**Option C: Don't Store Tokens**
```rust
// Just use tokens to fetch user info, then discard
```
- ✅ Pros: Most secure (no token storage)
- ❌ Cons: Cannot make API calls on behalf of user

**Recommendation**: **Option A** for MVP, **Option B** if you need to make API calls to OAuth providers on behalf of users.

If storing tokens, **MUST encrypt** them:
```rust
// Use aes-gcm or similar
pub trait TokenEncryption: Send + Sync {
    fn encrypt(&self, plaintext: &str) -> Result<String>;
    fn decrypt(&self, ciphertext: &str) -> Result<String>;
}
```

**Impact**: Currently OAuth tokens are discarded after fetching user info. Cannot refresh tokens or make API calls.

---

### TODO #5: Redirect to Error Page
**Location**: Line 429
**Code**:
```rust
AuthAction::OAuthFailed { error: _, error_description: _ } => {
    // Clear OAuth state
    state.oauth_state = None;

    // TODO: Redirect to error page
    smallvec![Effect::None]
}
```

**Priority**: MEDIUM
**Decision**: **Defer to Phase 6B** (Web framework integration)

**Recommendation**:
Add action or effect to signal redirect:
```rust
AuthAction::OAuthFailed { error, error_description } => {
    state.oauth_state = None;

    // Emit redirect action
    smallvec![Effect::Future(Box::pin(async move {
        Some(AuthAction::RedirectToErrorPage {
            error,
            error_description,
        })
    }))]
}
```

**Impact**: Users see no feedback when OAuth fails. Poor UX.

---

### TODO #6: Re-enable Unit Tests
**Location**: Lines 457, 462
**Code**:
```rust
#[cfg(test)]
mod tests {
    // Tests temporarily disabled - will be replaced with proper tests using mock providers
    // See TODO item: "Implement mock OAuth provider for testing"

    // TODO: Implement mock providers and re-enable tests
}
```

**Priority**: LOW
**Decision**: **Won't Fix** (misleading comment)

**Reason**: Integration tests already exist in `auth/tests/oauth_integration.rs` (9 tests passing). The comment is outdated.

**Recommendation**: Remove the misleading TODO comment and note that integration tests cover OAuth reducer functionality.

---

## 2. Hardcoded Values

### HV #1: Risk Score (0.1)
**Location**: Line 106
**Value**: `0.1`
**Solution**: Use `RiskCalculator` provider (see TODO #1)
**Priority**: HIGH

---

### HV #2: OAuth State Expiration (5 minutes)
**Location**: Line 206
**Value**: `Duration::minutes(5)`
**Solution**: Extract to configuration
**Priority**: MEDIUM

**Recommendation**:
Add to `OAuthReducer` struct:
```rust
pub struct OAuthReducer<...> {
    pub base_url: String,
    pub session_ttl_hours: i64,
    pub state_ttl_minutes: i64,  // NEW
    _phantom: std::marker::PhantomData<...>,
}

impl<...> OAuthReducer<...> {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            session_ttl_hours: 24,
            state_ttl_minutes: 5,  // Default 5 minutes
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_state_ttl(mut self, minutes: i64) -> Self {
        self.state_ttl_minutes = minutes;
        self
    }
}
```

Use in expiration check:
```rust
if age > Duration::minutes(self.state_ttl_minutes) {
    // State expired
}
```

---

### HV #3: Device Name ("Web Browser")
**Location**: Line 338
**Value**: `"Web Browser"`
**Solution**: Parse from user agent (same as Magic Link HV #2)
**Priority**: MEDIUM

---

### HV #4: Device Type ("desktop")
**Location**: Line 339
**Value**: `"desktop"`
**Solution**: Parse from user agent (same as Magic Link HV #3)
**Priority**: MEDIUM

---

### HV #5: OAuth Method String Format
**Location**: Line 350
**Value**: `format!("oauth_{}", provider.as_str())`
**Solution**: Extract to constants
**Priority**: LOW

**Recommendation**:
```rust
// In constants.rs
pub mod login_methods {
    pub const fn oauth_method(provider: &str) -> String {
        format!("oauth_{provider}")
    }

    // Or use match
    pub fn oauth_method_for_provider(provider: OAuthProvider) -> &'static str {
        match provider {
            OAuthProvider::Google => "oauth_google",
            OAuthProvider::GitHub => "oauth_github",
            OAuthProvider::Microsoft => "oauth_microsoft",
        }
    }
}
```

---

### HV #6: Base URL in Tests
**Location**: `oauth_integration.rs` line 49
**Value**: `"https://app.example.com"`
**Solution**: Use test constant or environment variable
**Priority**: LOW

---

## 3. Security Issues

### Security Issue #1: CSRF State Comparison NOT Constant-Time ❌ **CRITICAL**
**Location**: Line 192
**Severity**: **CRITICAL**

**Issue**:
```rust
if oauth_state.state_param != state_param {
    // State mismatch → CSRF attack
    state.oauth_state = None;
    return smallvec![...];
}
```

This uses Rust's default string comparison which is **NOT constant-time**. An attacker could use timing attacks to guess the CSRF state byte-by-byte.

**Fix**: Use constant-time comparison (like Magic Link does):
```rust
use constant_time_eq::constant_time_eq;

if !constant_time_eq(
    oauth_state.state_param.as_bytes(),
    state_param.as_bytes(),
) {
    // State mismatch → CSRF attack
    tracing::warn!("OAuth CSRF state validation failed");
    state.oauth_state = None;
    return smallvec![Effect::Future(Box::pin(async move {
        Some(AuthAction::OAuthFailed {
            error: "invalid_state".to_string(),
            error_description: Some("CSRF state validation failed".to_string()),
        })
    }))];
}
```

**Impact**: **CRITICAL** - Timing attack could allow attacker to guess CSRF state and hijack OAuth flow.

**Must fix before production.**

---

### Security Issue #2: Missing Email Validation
**Severity**: MEDIUM

**Issue**: No email validation in `OAuthSuccess` handler (line 263)

**Fix**: Add validation:
```rust
AuthAction::OAuthSuccess { email, ... } => {
    // Validate email format
    if !is_valid_email(&email) {
        tracing::warn!("Invalid email from OAuth provider: {}", email);
        return smallvec![Effect::Future(Box::pin(async move {
            Some(AuthAction::OAuthFailed {
                error: "invalid_email".to_string(),
                error_description: Some("Invalid email from OAuth provider".to_string()),
            })
        }))];
    }

    // ... rest of logic
}
```

**Impact**: Could allow invalid email addresses from compromised/malicious OAuth providers.

---

### Security Review Checklist

- ✅ **CSRF State Generation**: 256-bit random (line 84) ✅
- ❌ **Constant-Time Comparison**: NOT using constant_time_eq (line 192) ❌ **CRITICAL**
- ✅ **Single-Use Enforcement**: State cleared after callback (line 219) ✅
- ✅ **Expiration Checking**: 5-minute TTL enforced (line 206) ✅
- ❌ **Email Validation**: Missing ❌
- ⚠️ **Token Storage Security**: Not implemented yet ⚠️
- ⚠️ **Redirect URI Validation**: Assumed handled by OAuth2Provider ⚠️
- ✅ **State Initialization Check**: Required prior initiation (line 182) ✅

**Overall Security Score**: 5/8 (62.5%)
**Critical Issues**: 1 (constant-time comparison)

---

## 4. Event Sourcing Alignment

### Event Emission

✅ **Correct**: All four events emitted properly:
1. `UserRegistered` (conditional - only for new users)
2. `OAuthAccountLinked` (always)
3. `DeviceRegistered` (always)
4. `UserLoggedIn` (always - audit trail)

✅ **Batch Appending**: Events correctly batched (lines 358-390)

✅ **Stream ID**: Correct format: `user-{user_id}`

⚠️ **Provider User ID**: Placeholder value (see TODO #3)

### Missing Events

Consider adding:
- `OAuthAuthorizationInitiated` - for analytics/auditing
- `OAuthStateMismatch` - for security monitoring
- `OAuthStateExpired` - for analytics

---

## 5. Test Coverage

### Existing Tests (9 tests in oauth_integration.rs, all passing)

1. ✅ `test_oauth_flow_complete_happy_path` - Full flow
2. ✅ `test_oauth_callback_rejects_invalid_csrf_state` - CSRF protection
3. ✅ `test_oauth_callback_requires_prior_initiation` - State validation
4. ✅ `test_oauth_state_expires_after_5_minutes` - Expiration
5. ✅ `test_oauth_state_single_use_enforcement` - Single-use
6. ✅ `test_oauth_google_provider` - Provider-specific
7. ✅ `test_oauth_github_provider` - Provider-specific
8. ✅ `test_session_metadata_after_oauth` - Session validation
9. ✅ `test_oauth_csrf_state_uniqueness` - State uniqueness

### Missing Tests

**High Priority:**
- ❌ Timing attack test (verify constant-time comparison after fix)
- ❌ Email validation test (invalid email from provider)
- ❌ Token storage test (once implemented)
- ❌ Provider user ID extraction test (once implemented)

**Medium Priority:**
- ❌ Concurrent OAuth initiation (same user, different sessions)
- ❌ State expiration edge cases (exactly at 5 minutes)
- ❌ Token exchange failure scenarios
- ❌ UserInfo fetch failure scenarios

**Low Priority:**
- ❌ Redirect URI validation
- ❌ PKCE flow (if implementing)

---

## 6. Code Quality & Style

### Strengths
- ✅ Well-documented flow diagram in module docs
- ✅ Clear action handler separation
- ✅ Proper error handling structure
- ✅ Good use of pattern matching
- ✅ No clippy warnings

### Improvements Needed
- Extract constants (state TTL, device names)
- Add email validation
- Fix constant-time comparison
- Implement token storage
- Get actual provider user ID

---

## 7. Missing Error Handling

### Error #1: Authorization URL Generation Failure
**Location**: Line 162
**Issue**: Error discarded without logging context

**Fix**: Add more context:
```rust
Err(e) => {
    tracing::error!("Failed to generate OAuth authorization URL for {:?}: {}", provider, e);
    Some(AuthAction::OAuthFailed {
        error: "url_generation_failed".to_string(),
        error_description: Some(format!("Failed to generate authorization URL: {e}")),
    })
}
```

---

### Error #2: Event Serialization Failure
**Location**: Line 364
**Issue**: Returns action but no logging

**Fix**: Add logging:
```rust
if serialized_events.is_empty() {
    tracing::error!("Failed to serialize OAuth events for user {}", final_user_id.0);
    return Some(AuthAction::OAuthFailed { ... });
}
```

---

### Error #3: Session Creation Failure
**Location**: Line 394
**Issue**: Error logged but no additional context

**Fix**: Add user context:
```rust
if sessions.create_session(&session, Duration::hours(session_ttl_hours)).await.is_err() {
    tracing::error!("Failed to create OAuth session for user {} device {}",
        final_user_id.0, device_id.0);
    return Some(AuthAction::OAuthFailed { ... });
}
```

---

## 8. Recommendations

### Immediate (Block Production)
1. ❌ **FIX CRITICAL: Use constant-time comparison for CSRF state** (Security Issue #1)
2. ✅ **Implement actual provider user ID extraction** (TODO #3)
3. ✅ **Integrate RiskCalculator for risk scores** (TODO #1)
4. ✅ **Add email validation** (Security Issue #2)
5. ✅ **Make design decision on OAuth token storage** (TODO #4)

### Short-Term (1-2 weeks)
6. ⏱ **Extract state TTL to configuration** (HV #2)
7. ⏱ **Add device name/type parsing** (HV #3, HV #4)
8. ⏱ **Add HTTP redirect handling** (TODO #2, TODO #5)
9. ⏱ **Add missing tests** (timing attacks, email validation)
10. ⏱ **Implement token storage** (if decided yes on TODO #4)

### Long-Term (Future Enhancement)
11. 📅 **Add PKCE flow support** (more secure for mobile/SPA)
12. 📅 **Add token refresh mechanism**
13. 📅 **Add telemetry/metrics** (OAuth flows by provider, failure reasons)
14. 📅 **Add security event logging** (CSRF attempts, state mismatches)

---

## 9. Comparison with Magic Link Reducer

| Aspect | Magic Link | OAuth | Notes |
|--------|-----------|-------|-------|
| **Token Security** | Constant-time ✅ | NOT constant-time ❌ | OAuth CRITICAL issue |
| **Risk Calculation** | Hardcoded ❌ | Hardcoded ❌ | Both need fix |
| **Device Parsing** | Hardcoded ❌ | Hardcoded ❌ | Both need fix |
| **Email Validation** | Missing ❌ | Missing ❌ | Both need fix |
| **Test Coverage** | 8 tests ✅ | 9 tests ✅ | Good |
| **Event Sourcing** | Correct ✅ | Correct ✅ | Good |

---

## 10. Conclusion

The OAuth Reducer implements the OAuth2/OIDC flow correctly with proper CSRF protection, but has **1 CRITICAL security issue** (non-constant-time state comparison) that **MUST be fixed before production**.

**Critical Fixes** (Block Production):
1. ❌ Constant-time CSRF state comparison
2. ✅ Actual provider user ID extraction
3. ✅ Risk score integration
4. ✅ Email validation
5. ✅ OAuth token storage design decision

**Estimated Fix Time**: 6-8 hours

**Next Steps**:
1. **IMMEDIATELY**: Fix constant-time comparison (30 minutes)
2. Implement provider user ID extraction (2 hours)
3. Integrate RiskCalculator (1 hour)
4. Add email validation (30 minutes)
5. Make token storage decision and implement (2-3 hours)
6. Add timing attack tests (1 hour)
7. Re-review after fixes

---

**Reviewed By**: Claude Code
**Review Status**: ✅ Complete (with CRITICAL issue identified)
**Next Review**: Passkey Reducer
**BLOCKER**: Constant-time comparison MUST be fixed before production deployment
