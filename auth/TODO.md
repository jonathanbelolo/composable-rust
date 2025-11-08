# composable-rust-auth TODO

## Completed

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

## Medium Priority: Production Enhancements

### 🔴 CRITICAL: Atomic Counter Update Race Condition

**Status**: Not started
**Location**: `src/reducers/passkey.rs:535-560` (counter update flow)
**Estimated Effort**: 6-8 hours
**CVSS Score**: 8.7 (High)

**What's Needed**:

The passkey authentication flow has a race condition between counter verification and counter update. Two concurrent authentication attempts with the same credential could both pass verification before either updates the stored counter value.

**Vulnerability Details**:

```rust
// CURRENT FLOW (VULNERABLE):
// 1. Read credential from database (counter = 100)
// 2. Verify signature and counter (new_counter = 101)
// 3. Update credential (SET counter = 101)

// ATTACK SCENARIO:
// Request A reads credential (counter = 100)
// Request B reads credential (counter = 100)
// Request A verifies counter 101 > 100 ✓
// Request B verifies counter 101 > 100 ✓  ← SHOULD FAIL
// Request A updates counter to 101
// Request B updates counter to 101
// Both requests succeed with same counter!
```

**Implementation Requirements**:

1. **Database-Level Atomic Compare-and-Swap**:
   ```sql
   -- PostgreSQL implementation
   UPDATE passkey_credentials
   SET counter = $2,
       last_used = NOW()
   WHERE credential_id = $1
     AND counter = $2 - 1  -- Only update if counter hasn't changed
   RETURNING counter;

   -- Check rows affected:
   -- 0 rows = counter was updated by concurrent request → REJECT
   -- 1 row = success → ACCEPT
   ```

2. **UserRepository Trait Changes**:
   ```rust
   pub trait UserRepository: Send + Sync {
       // Add atomic counter update method
       async fn update_passkey_counter_atomic(
           &self,
           credential_id: &str,
           expected_old_counter: u32,
           new_counter: u32,
       ) -> Result<bool>; // Returns false if counter mismatch
   }
   ```

3. **Reducer Logic Update** (`passkey.rs:535-560`):
   ```rust
   // Replace current update flow with:
   let updated = users
       .update_passkey_counter_atomic(
           &credential_id,
           credential.counter,      // Expected old value
           result.counter,          // New value
       )
       .await?;

   if !updated {
       // Counter changed between verification and update
       // This indicates concurrent authentication attempt
       tracing::error!(
           "Counter update failed - concurrent authentication detected"
       );
       return None;
   }
   ```

4. **Alternative: Event Sourcing with Optimistic Locking**:
   ```rust
   // Use event versioning for optimistic concurrency control
   let event = AuthEvent::PasskeyUsed { ... };
   let result = event_store
       .append_events(
           stream_id,
           Some(expected_version), // ← Optimistic lock
           vec![event],
       )
       .await;

   match result {
       Ok(_) => { /* success */ },
       Err(OptimisticConcurrencyError) => { /* retry or reject */ },
   }
   ```

**Testing Requirements**:

1. **Concurrency Test**:
   ```rust
   #[tokio::test]
   async fn test_concurrent_passkey_authentication() {
       // Spawn 10 concurrent authentication attempts
       // with same credential and counter
       let handles: Vec<_> = (0..10)
           .map(|_| {
               let reducer = reducer.clone();
               tokio::spawn(async move {
                   reducer.reduce(state, action, env)
               })
           })
           .collect();

       let results = join_all(handles).await;

       // Exactly ONE should succeed
       assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
   }
   ```

2. **Load Test**: Simulate realistic concurrent authentication patterns

3. **Race Condition Test**: Use `loom` crate for formal verification

**Security Impact**:

- **Before Fix**: Cloned authenticators could authenticate concurrently without detection
- **After Fix**: Only one authentication attempt succeeds, others are rejected
- **Detection**: Failed atomic updates should trigger security monitoring

**Acceptance Criteria**:

- [ ] Add `update_passkey_counter_atomic()` to UserRepository trait
- [ ] Implement atomic update in PostgresUserRepository using compare-and-swap SQL
- [ ] Update passkey reducer to use atomic counter update
- [ ] Add concurrency test validating exactly-once semantics
- [ ] Add security logging for failed atomic updates (concurrent auth detection)
- [ ] Update InMemoryUserRepository for testing (use Mutex or atomic types)
- [ ] Document the fix in security audit notes

**Related Files**:
- `src/reducers/passkey.rs` - Reducer logic update
- `src/providers/user_repository.rs` - Trait definition
- `src/stores/user_postgres.rs` - PostgreSQL implementation
- `tests/passkey_integration.rs` - Concurrency tests

---

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
