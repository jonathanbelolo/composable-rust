# composable-rust-auth TODO

## Completed

### ✅ COMPLETED: Sprint 1 - Production Hardening (Security Audit Fixes)

**Status**: COMPLETE (2025-01-09)
**Duration**: Sprint 1 Week 1 + Week 2 (6 tasks total)
**Security Impact**: Fixed 2 CRITICAL, 2 HIGH, 1 MEDIUM, 1 LOW severity vulnerabilities

**Week 1 Tasks (1.1-1.3)**:
1. ✅ **Task 1.1 - Session Expiration Validation** (HIGH - CVSS 7.2)
   - Added defense-in-depth expiration check in `get_session()`
   - Guards against clock skew, Redis bugs, manual TTL manipulation
   - Location: `src/stores/session_redis.rs:158-174`

2. ✅ **Task 1.2 - Atomic Session Operations** (CRITICAL - CVSS 8.9)
   - Atomic session creation with Redis pipeline (session + user set + TTL)
   - Atomic bulk deletion with Lua script (prevents orphaned sessions)
   - Session fixation prevention (rejects duplicate session IDs)
   - Location: `src/stores/session_redis.rs:76-143, 320-367`

3. ✅ **Task 1.3 - User Sessions Set TTL** (MEDIUM - CVSS 6.3)
   - Added TTL to `user:{user_id}:sessions` sets (+1 day buffer)
   - Lazy cleanup in `get_user_sessions()` (removes dead references)
   - Prevents unbounded memory growth
   - Location: `src/stores/session_redis.rs:116-130, 396-452`

**Week 2 Tasks (1.4-1.6)**:
4. ✅ **Task 1.4 - Device Repository Authorization** (CRITICAL - CVSS 9.1)
   - Redesigned DeviceRepository trait to require `user_id` parameter
   - Database-level authorization (SQL WHERE clauses)
   - Returns ResourceNotFound (not Unauthorized) to prevent information leakage
   - Location: `src/providers/device.rs`, `src/stores/postgres/device.rs`, `src/mocks/device.rs`
   - Tests: 8 comprehensive authorization tests

5. ✅ **Task 1.5 - Device Input Validation** (MEDIUM - CVSS 5.4)
   - Added `validate_device_name()` (1-255 chars, no XSS: <>"'&\0)
   - Added `validate_platform()` (1-500 chars, ASCII-only)
   - Prevents stored XSS and injection attacks
   - Location: `src/utils.rs:142-239`
   - Tests: 13 comprehensive validation tests

6. ✅ **Task 1.6 - Device Pagination** (LOW - CVSS 3.1)
   - Added pagination to `get_user_devices()` (limit + offset)
   - MAX_LIMIT=1000, DEFAULT_LIMIT=100
   - Negative offset prevention with `max(0)`
   - Location: `src/providers/device.rs`, `src/stores/postgres/device.rs`, `src/mocks/device.rs`
   - Tests: 5 comprehensive pagination tests

**Post-Sprint Audit Fixes**:
- ✅ Added concurrent session creation race test
- ✅ Documented TOCTOU acceptance in `update_session()` with full risk assessment
- Location: `src/stores/session_redis.rs:671-718, 196-214`

**Test Coverage**:
- 63 total library tests passing
- 37 new security tests added
- 25 Redis integration tests (require Redis instance)
- 0 test failures

**Security Measures Implemented**:
- Defense-in-depth expiration validation
- Atomic Redis operations (pipelines + Lua scripts)
- Session fixation prevention
- Immutable field enforcement (5 fields)
- Sliding window TTL refresh
- Database-level authorization
- XSS prevention (blocks dangerous characters)
- DoS prevention (pagination limits)
- Information leakage prevention (consistent error responses)

**Files Modified**:
- `src/stores/session_redis.rs` - Session security hardening
- `src/providers/session.rs` - Added get_user_sessions()
- `src/mocks/session.rs` - Mock implementation
- `src/providers/device.rs` - Authorization-by-design trait
- `src/stores/postgres/device.rs` - PostgreSQL authorization
- `src/mocks/device.rs` - Mock authorization + tests
- `src/utils.rs` - Input validation functions
- `src/error.rs` - Added InvalidInput variant

**Audit Result**: ✅ **APPROVED FOR PRODUCTION DEPLOYMENT**
- Overall Assessment: 5/6 PASS, 1/6 NEEDS_ENHANCEMENT (minor)
- Risk Level: **LOW** - All critical attack vectors blocked
- Confidence: **VERY HIGH** - Comprehensive testing and defense-in-depth

---

### ✅ COMPLETED: RedisTokenStore for Magic Links

**Status**: COMPLETE (2025-01-08)
**Location**: `src/stores/token_redis.rs`

Production-ready Redis-based token store with:
- ✅ Atomic GETDEL for single-use consumption
- ✅ Constant-time token comparison (timing attack prevention)
- ✅ Defense-in-depth expiration validation
- ✅ Comprehensive test suite (8 tests)
- ✅ Full documentation and examples
- ✅ Key namespacing (`auth:token:`)
- ✅ Connection pooling via `ConnectionManager`

**Usage**:
```rust
use composable_rust_auth::stores::RedisTokenStore;

let store = RedisTokenStore::new("redis://127.0.0.1:6379").await?;
```

---

## High Priority: Production Blockers

### 🔴 CRITICAL: OAuth Token Refresh Implementation

**Status**: Structure in place, implementation needed
**Location**: `src/stores/oauth_token_redis.rs:273` (`refresh_access_token()`)
**Estimated Effort**: 4-6 hours

**What's Needed**:

The `RedisOAuthTokenStore::refresh_access_token()` method currently returns a placeholder error. It needs full implementation for each OAuth provider.

**Implementation Requirements**:

1. **Provider-Specific Token Endpoints**:
   - Google: `https://oauth2.googleapis.com/token`
   - GitHub: `https://github.com/login/oauth/access_token`
   - Microsoft: `https://login.microsoftonline.com/common/oauth2/v2.0/token`

2. **Token Refresh Flow**:
   ```rust
   async fn refresh_access_token(&self, user_id: UserId, provider: OAuthProvider) -> Result<String> {
       // 1. Get current tokens
       let tokens = self.get_tokens(user_id, provider).await?.ok_or(...)?;

       // 2. Call provider's token endpoint with refresh_token
       let request = provider.build_refresh_request(&tokens.refresh_token);
       let response = http_client.post(request).await?;

       // 3. Parse new tokens from provider response
       let new_tokens = provider.parse_token_response(response)?;

       // 4. Update stored tokens (new access_token, possibly new refresh_token)
       let updated_tokens = OAuthTokenData {
           user_id,
           provider,
           access_token: new_tokens.access_token.clone(),
           refresh_token: new_tokens.refresh_token.or(tokens.refresh_token),
           expires_at: Some(Utc::now() + Duration::seconds(new_tokens.expires_in)),
           stored_at: Utc::now(),
       };
       self.store_tokens(&updated_tokens).await?;

       // 5. Return new access_token
       Ok(new_tokens.access_token)
   }
   ```

3. **Provider Client Abstraction**:
   - Create trait `OAuthProviderClient` with methods:
     - `build_refresh_request(refresh_token: &str) -> HttpRequest`
     - `parse_token_response(response: HttpResponse) -> TokenResponse`
   - Implement for each provider (Google, GitHub, Microsoft)

4. **Error Handling**:
   - Invalid refresh token → Return `AuthError::InvalidRefreshToken`
   - Expired refresh token → Return `AuthError::RefreshTokenExpired`
   - Provider API error → Return `AuthError::OAuthProviderError`
   - Network error → Retry with exponential backoff

5. **Security Considerations**:
   - ✅ Tokens already encrypted at rest (AES-256-GCM)
   - ✅ HTTPS required for provider communication (handled by reqwest)
   - ⚠️  Add rate limiting for refresh attempts (prevent abuse)
   - ⚠️  Log refresh failures for security monitoring
   - ⚠️  Invalidate tokens after N failed refresh attempts

6. **Testing**:
   - Unit tests with mock HTTP responses
   - Integration tests with OAuth provider sandboxes
   - Test token rotation (when provider returns new refresh_token)
   - Test error scenarios (expired, revoked, invalid tokens)

**Dependencies**:
- Already have `reqwest` for HTTP client
- May need provider-specific libraries:
  - `google-authz` or use `oauth2` crate directly
  - `octocrab` for GitHub (or raw OAuth2)
  - `azure_identity` for Microsoft (or raw OAuth2)

**Acceptance Criteria**:
- [ ] Implement provider-specific token refresh for Google
- [ ] Implement provider-specific token refresh for GitHub
- [ ] Implement provider-specific token refresh for Microsoft
- [ ] Add retry logic with exponential backoff
- [ ] Add rate limiting per user/provider
- [ ] Add comprehensive error handling
- [ ] Add integration tests for all providers
- [ ] Add logging for refresh attempts/failures
- [ ] Document refresh flow in provider trait docs

**Related Files**:
- `src/stores/oauth_token_redis.rs` - Main implementation
- `src/providers/oauth.rs` - May need provider client trait
- `src/reducers/oauth.rs` - Token refresh trigger logic

---

### ✅ COMPLETED: Sprint 4 - Atomic Counter Update (CVSS 8.7 Security Fix)

**Status**: COMPLETE (2025-01-10)
**Duration**: Sprint 4 (implementation already present, tests added)
**Security Impact**: Fixed CRITICAL race condition in passkey authentication

**What Was Fixed**:

The passkey authentication flow had a race condition between counter verification and counter update allowing cloned authenticators to bypass detection via concurrent authentication.

**Implementation Complete**:

1. ✅ UserRepository trait (`src/providers/user.rs:273-278`)
2. ✅ PostgreSQL implementation with SELECT FOR UPDATE (`src/stores/postgres/user.rs:466-558`)
3. ✅ Mock implementation with Mutex-protected CAS (`src/mocks/user.rs:246-273`)
4. ✅ Passkey reducer usage (`src/reducers/passkey.rs:656-695`)
5. ✅ Comprehensive test suite - 4 NEW TESTS (`src/mocks/user.rs:288-464`)
   - Exactly-once semantics (10 concurrent → 1 success, 9 fail)
   - Sequential updates
   - Stale counter detection
   - Error handling

**Test Coverage**: 105 tests passing (4 new)

**Security Impact**:
- Before: Cloned authenticators could bypass detection via concurrent auth
- After: Exactly-once semantics (database-level atomicity)
- Detection: Failed CAS triggers security alert logging

---

## Medium Priority: Production Enhancements


### Session Refresh Logic
**Status**: Not started
**Location**: Session management
**Description**: Implement sliding window session refresh (extend TTL on activity)

### Device Fingerprinting
**Status**: Not started
**Location**: Risk calculation
**Description**: Enhance device detection beyond user-agent parsing

### Passkey Credential Management
**Status**: Not started
**Location**: Passkey reducer
**Description**: Add endpoints for listing/deleting registered passkeys

---

## Low Priority: Future Improvements

### Email Templates
**Status**: Basic implementation
**Location**: Email provider
**Description**: Rich HTML email templates for magic links

### MFA Support
**Status**: Not started
**Description**: Add TOTP/SMS as second factor option

### Account Recovery
**Status**: Not started
**Description**: Secure account recovery flow with backup codes

---

## Documentation TODO

- [ ] Production deployment guide
- [ ] Security best practices guide
- [ ] OAuth provider setup instructions (client IDs, secrets, etc.)
- [ ] Redis configuration guide (persistence, clustering)
- [ ] Example integration with web frameworks (Axum, Actix, Rocket)

---

**Last Updated**: 2025-01-08
**Maintainer**: composable-rust-auth team
