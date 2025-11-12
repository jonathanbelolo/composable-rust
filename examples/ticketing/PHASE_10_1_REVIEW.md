# Phase 10.1 Architecture Review

**Date**: 2025-11-12
**Reviewer**: Claude Code
**Status**: ⚠️ CRITICAL ARCHITECTURAL ISSUE IDENTIFIED

## Executive Summary

Phase 10.1 implementation has a **critical architectural misalignment** with Composable Rust principles. We implemented authentication as **imperative HTTP handlers** instead of using the framework's **Reducer-based architecture**.

### Key Finding

The `composable-rust-auth` crate already provides a complete, Reducer-based authentication system that we should be using. Instead, we re-implemented ~2,100 lines of code that bypasses the core architecture.

---

## Architecture Violation Analysis

### What We Built ❌

```rust
// Our approach: Imperative handlers calling stores directly
async fn request_magic_link<E>(
    State(env): State<AuthEnvironment<E>>,
    Json(req): Json<MagicLinkRequest>,
) -> HandlerResult<impl IntoResponse> {
    // Direct imperative code
    env.rate_limiter.check_rate_limit(&req.email).await?;
    let user = env.user_repo.find_or_create(&req.email, None).await?;
    let token = Uuid::new_v4().to_string();
    env.token_store.store_token(&token, &user.email).await?;
    env.email_provider.send_magic_link(&user.email, &token).await?;
    Ok(...)
}
```

**Problems:**
1. No Reducer pattern - business logic in handlers
2. No State management - direct database calls
3. No Effect system - side effects executed immediately
4. No Action dispatch - no unidirectional data flow
5. Untestable at memory speed - requires Redis/PostgreSQL

### What We Should Have Built ✅

```rust
// Framework approach: Reducer-based with Action dispatch
let store = Arc::new(Store::new(
    AuthState::default(),
    AuthReducer::new(),
    environment,
));

async fn request_magic_link(
    State(store): State<AuthStore>,
    Json(req): Json<MagicLinkRequest>,
) -> Result<Json<Response>, ApiError> {
    // Dispatch action to reducer
    store.send(AuthAction::SendMagicLink {
        correlation_id: Uuid::new_v4(),
        email: req.email,
        ip_address,
        user_agent,
    }).await?;

    Ok(Json(Response { message: "Magic link sent" }))
}

// Reducer (pure function, testable at memory speed)
impl Reducer for AuthReducer {
    fn reduce(
        &self,
        state: &mut AuthState,
        action: AuthAction,
        env: &AuthEnvironment,
    ) -> SmallVec<[Effect<AuthAction>; 4]> {
        match action {
            AuthAction::SendMagicLink { email, .. } => {
                // Validate, update state, return effects
                vec![
                    Effect::Future(Box::pin(async move {
                        // Generate token, store in Redis
                        Some(AuthAction::MagicLinkSent { token, email })
                    })),
                ]
            }
            AuthAction::MagicLinkSent { token, email } => {
                // Send email effect
                vec![Effect::Future(Box::pin(async move {
                    // Send email via provider
                    None
                }))]
            }
            // ...
        }
    }
}
```

**Benefits:**
1. ✅ Pure business logic in reducer - testable at memory speed
2. ✅ State management - centralized auth state
3. ✅ Effect system - side effects as values
4. ✅ Unidirectional data flow - Action → Reducer → Effects
5. ✅ Composable - multiple reducers coordinate via actions

---

## What the Framework Provides

The `composable-rust-auth` crate includes **everything we just built**:

### Core Components

| Component | Framework Path | Our Implementation |
|-----------|---------------|-------------------|
| Actions | `auth/src/actions.rs` | `src/auth/schemas.rs` (partial) |
| Reducer | `auth/src/reducers/mod.rs` | ❌ Not implemented |
| State | `auth/src/state.rs` | ❌ Not implemented |
| Environment | `auth/src/environment.rs` | `src/auth/environment.rs` (different design) |
| Effects | `auth/src/effects.rs` | ❌ Not implemented |

### Infrastructure (Correctly Implemented ✅)

| Component | Framework | Our Implementation | Status |
|-----------|-----------|-------------------|--------|
| Redis Session Store | `auth/src/stores/session_redis.rs` | `src/auth/stores.rs::RedisSessionStore` | ✅ Similar |
| Redis Token Store | `auth/src/stores/token_redis.rs` | `src/auth/stores.rs::RedisTokenStore` | ✅ Similar |
| Redis Challenge Store | `auth/src/stores/challenge_redis.rs` | `src/auth/stores.rs::RedisChallengeStore` | ✅ Similar |
| Redis OAuth Store | `auth/src/stores/oauth_token_redis.rs` | `src/auth/stores.rs::RedisOAuthTokenStore` | ✅ Similar |
| Redis Rate Limiter | `auth/src/stores/rate_limiter_redis.rs` | `src/auth/stores.rs::RedisRateLimiter` | ✅ Similar |
| PostgreSQL User Repo | `auth/src/stores/postgres/user.rs` | `src/auth/repositories.rs::PostgresUserRepository` | ✅ Similar |
| PostgreSQL Device Repo | `auth/src/stores/postgres/device.rs` | `src/auth/repositories.rs::PostgresDeviceRepository` | ✅ Similar |
| Console Email | `auth/src/providers/console_email.rs` | `src/auth/email.rs::ConsoleEmailProvider` | ✅ Similar |

### HTTP Layer

| Component | Framework | Our Implementation | Status |
|-----------|-----------|-------------------|--------|
| Router | `auth/src/router.rs` | `src/auth/handlers.rs::auth_routes` | ⚠️ Different pattern |
| Magic Link Handlers | `auth/src/handlers/magic_link.rs` | `src/auth/handlers.rs` | ⚠️ Imperative vs Reducer |
| OAuth Handlers | `auth/src/handlers/oauth.rs` | `src/auth/handlers.rs` | ⚠️ Imperative vs Reducer |
| Passkey Handlers | `auth/src/handlers/passkey.rs` | `src/auth/handlers.rs` (stubs) | ⚠️ Imperative vs Reducer |

---

## Framework Architecture Pattern

### The Composable Rust Way

```
┌─────────────────────────────────────────────────────────────┐
│                     HTTP Request                            │
└─────────────────────────┬───────────────────────────────────┘
                          ↓
                  ┌───────────────┐
                  │    Handler    │ (Thin: Parse request → Action)
                  └───────┬───────┘
                          ↓
                  ┌───────────────┐
                  │  Store.send() │ (Dispatch action)
                  └───────┬───────┘
                          ↓
        ┌─────────────────────────────────────┐
        │           Reducer                   │ (Pure function)
        │  (State, Action, Env) → (State, [Effect])
        └─────────────────┬───────────────────┘
                          ↓
                ┌─────────────────┐
                │  Effect Executor│ (Runtime)
                └─────────┬───────┘
                          ↓
        ┌─────────────────────────────────────┐
        │  Side Effects (DB, Email, Events)   │
        └─────────────────────────────────────┘
```

**Key Principles:**
1. **HTTP handlers are thin** - Only parse/validate, dispatch actions
2. **Business logic in reducers** - Pure functions, testable at memory speed
3. **Effects as values** - Side effects returned as descriptions
4. **Unidirectional flow** - Actions flow one direction through the system
5. **State centralized** - Single source of truth in Store

### Our Approach (Incorrect)

```
┌─────────────────────────────────────────────────────────────┐
│                     HTTP Request                            │
└─────────────────────────┬───────────────────────────────────┘
                          ↓
                  ┌───────────────┐
                  │    Handler    │ (Fat: Contains all business logic)
                  └───────┬───────┘
                          ↓
        ┌─────────────────────────────────────┐
        │  Direct Store/Repo Calls            │ (Imperative)
        │  - rate_limiter.check()             │
        │  - user_repo.find_or_create()       │
        │  - token_store.store_token()        │
        │  - email_provider.send()            │
        └─────────────────────────────────────┘
```

**Problems:**
1. ❌ Business logic in handlers (untestable at memory speed)
2. ❌ No state management
3. ❌ Side effects executed immediately (not as values)
4. ❌ No Action/Reducer pattern
5. ❌ Violates separation of concerns

---

## Detailed Code Review

### ✅ Correctly Implemented Components

#### 1. Redis Stores (stores.rs)

```rust
pub struct RedisSessionStore {
    client: ConnectionManager,  // ✅ Correct (no Arc needed)
    ttl_seconds: u64,
}

impl RedisSessionStore {
    pub async fn create_session(...) -> Result<(String, SessionInfo)> {
        let session_id = Uuid::new_v4().to_string();
        // ... implementation
        let _: () = conn.set_ex(&key, session_json, self.ttl_seconds).await?;  // ✅ Type annotation
        Ok((session_id, session_info))
    }
}
```

**Strengths:**
- ✅ Correct Redis 0.27 usage
- ✅ ConnectionManager (Clone-able, no Arc)
- ✅ Explicit type annotations for Redis operations
- ✅ TTL management
- ✅ Error handling with custom StoreError
- ✅ All Redis operations (set_ex, del, expire, incr) properly typed

#### 2. PostgreSQL Repositories (repositories.rs)

```rust
pub struct PostgresUserRepository {
    pool: PgPool,
}

impl PostgresUserRepository {
    pub async fn create_user(...) -> Result<User> {
        let user = sqlx::query_as!(User, r#"INSERT ..."#, ...)
            .fetch_one(&self.pool)
            .await?;
        // Manual conversion for UserRole
        Ok(User { role: UserRole::from_str(&user.role), ...})
    }
}
```

**Strengths:**
- ✅ sqlx::query! and query_as! macros
- ✅ Proper error handling
- ✅ User/Device CRUD operations
- ✅ UserRole enum with conversion

**Minor Issue:**
- ⚠️ Manual struct reconstruction after query_as! (could be cleaner)

#### 3. Environment (environment.rs)

```rust
pub struct AuthEnvironment<E: EmailProvider> {
    pub session_store: RedisSessionStore,
    pub token_store: RedisTokenStore,
    pub challenge_store: RedisChallengeStore,
    pub oauth_store: RedisOAuthTokenStore,
    pub rate_limiter: RedisRateLimiter,
    pub user_repo: PostgresUserRepository,
    pub device_repo: PostgresDeviceRepository,
    pub email_provider: Arc<E>,
}
```

**Strengths:**
- ✅ Generic over EmailProvider trait
- ✅ Dependency injection pattern
- ✅ Type alias for ProductionAuthEnvironment
- ✅ Configurable TTLs

**Issue:**
- ❌ Different design than framework's AuthEnvironment
- ❌ Framework uses provider traits, not concrete types

### ❌ Architecturally Incorrect Components

#### 1. Handlers (handlers.rs) - MAJOR ISSUE

```rust
async fn request_magic_link<E>(
    State(env): State<AuthEnvironment<E>>,
    Json(req): Json<MagicLinkRequest>,
) -> HandlerResult<impl IntoResponse>
where
    E: EmailProvider + Clone + 'static,
{
    // ❌ Business logic in handler
    env.rate_limiter.check_rate_limit(&req.email).await?;

    // ❌ Direct imperative calls
    let user = env.user_repo.find_or_create(&req.email, None).await?;
    let token = Uuid::new_v4().to_string();
    env.token_store.store_token(&token, &user.email).await?;

    // ❌ Side effects executed immediately
    if let Err(e) = env.email_provider.send_magic_link(&user.email, &token).await {
        error!("Failed to send magic link email: {}", e);
        return Err(AuthHandlerError::Store(...));
    }

    info!("Magic link sent to {}", user.email);
    Ok(...)
}
```

**Problems:**
1. **Business logic in handlers** - Should be in reducer
2. **Imperative execution** - Should dispatch actions
3. **No state management** - Should update AuthState
4. **Side effects immediate** - Should return Effect values
5. **Untestable at memory speed** - Requires Redis + PostgreSQL + Email

**Should Be:**
```rust
async fn request_magic_link(
    State(store): State<AuthStore>,
    Json(req): Json<MagicLinkRequest>,
    headers: HeaderMap,
) -> Result<Json<Response>, ApiError> {
    let ip_address = extract_ip(&headers);
    let user_agent = extract_user_agent(&headers);

    store.send(AuthAction::SendMagicLink {
        correlation_id: Uuid::new_v4(),
        email: req.email,
        ip_address,
        user_agent,
    }).await?;

    Ok(Json(Response {
        message: "Magic link sent to your email",
    }))
}
```

#### 2. Missing Reducer - CRITICAL

We have **NO reducer implementation**. All business logic is in handlers.

**Should Have:**
```rust
pub struct AuthReducer {
    magic_link: MagicLinkReducer,
    oauth: OAuthReducer,
    passkey: PasskeyReducer,
}

impl Reducer for AuthReducer {
    type State = AuthState;
    type Action = AuthAction;
    type Environment = AuthEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            AuthAction::SendMagicLink { email, .. } => {
                // Validate email
                // Generate token
                // Update state
                // Return effects
                vec![
                    Effect::Future(Box::pin(async move {
                        // Store token in Redis
                        Some(AuthAction::MagicLinkSent { token, email })
                    })),
                ]
            }
            AuthAction::MagicLinkSent { token, email } => {
                vec![
                    Effect::Future(Box::pin(async move {
                        // Send email
                        None
                    })),
                ]
            }
            // ... more actions
        }
    }
}
```

#### 3. Missing State - CRITICAL

We have **NO centralized state**. Everything is in databases/Redis.

**Should Have:**
```rust
pub struct AuthState {
    /// Active sessions (in-memory cache)
    pub sessions: HashMap<SessionId, Session>,
    /// Pending magic link tokens
    pub pending_magic_links: HashMap<String, PendingMagicLink>,
    /// OAuth flows in progress
    pub oauth_flows: HashMap<String, OAuthFlow>,
    /// Rate limiting state
    pub rate_limits: HashMap<IpAddr, RateLimitState>,
}
```

#### 4. No Action Types - CRITICAL

We have **NO action enum**. Only HTTP request/response schemas.

**Should Have:**
```rust
pub enum AuthAction {
    // Commands
    SendMagicLink { correlation_id: Uuid, email: String, ... },
    VerifyMagicLink { correlation_id: Uuid, token: String, ... },
    InitiateOAuth { correlation_id: Uuid, provider: OAuthProvider, ... },

    // Events (from effects)
    MagicLinkSent { correlation_id: Uuid, email: String, token: String },
    MagicLinkVerified { correlation_id: Uuid, user_id: UserId },
    OAuthSuccess { correlation_id: Uuid, user_id: UserId, ... },

    // System events
    SessionExpired { session_id: SessionId },
    RateLimitExceeded { ip_address: IpAddr },
}
```

---

## Impact Assessment

### Functional Impact: ✅ LOW
- Code works correctly (compiles, logic is sound)
- Infrastructure setup is correct
- Stores and repositories are functional
- HTTP endpoints would function as expected

### Architectural Impact: ❌ CRITICAL
- Violates core Composable Rust principles
- Not using the framework's reducer pattern
- Business logic untestable at memory speed
- Side effects executed immediately (not as values)
- No unidirectional data flow
- Re-implements framework functionality

### Maintenance Impact: ⚠️ MEDIUM-HIGH
- Future phases expect Reducer-based pattern
- Auth middleware (Phase 10.2) will integrate with Store
- Business logic endpoints (Phase 10.5+) use Reducer pattern
- Inconsistency across codebase
- Extra code to maintain (~2,100 lines that duplicate framework)

### Testing Impact: ❌ HIGH
- Business logic requires Redis + PostgreSQL + Email
- Cannot test at memory speed
- Integration tests only (slow, brittle)
- Framework's reducer approach enables pure unit tests

---

## Recommendations

### Option A: Refactor to Use Framework (RECOMMENDED) ✅

**Effort**: 6-8 hours
**Benefits**:
- ✅ Proper architecture alignment
- ✅ Testable at memory speed
- ✅ Consistent with future phases
- ✅ Less code to maintain
- ✅ Leverage framework features

**Steps**:
1. Keep Redis stores & PostgreSQL repos (wrap with provider traits)
2. Use `composable_rust_auth::AuthAction` enum
3. Use `composable_rust_auth::AuthReducer`
4. Use `composable_rust_auth::AuthState`
5. Use `composable_rust_runtime::Store` for dispatch
6. Use `composable_rust_auth::auth_router` for HTTP
7. Delete our custom handlers (~400 lines)

### Option B: Continue Current Approach ⚠️

**Effort**: 0 hours (keep as-is)
**Benefits**:
- ✅ Works functionally
- ✅ Infrastructure is correct

**Drawbacks**:
- ❌ Violates architecture
- ❌ Inconsistent with framework
- ❌ Harder to test
- ❌ More code to maintain
- ❌ Technical debt

**Justification Needed**:
- Why deviate from framework?
- How will this integrate with business logic reducers?
- Plan for long-term maintenance?

### Option C: Hybrid Approach ⚠️

**Effort**: 3-4 hours
**Benefits**:
- ✅ Keep working stores/repos
- ✅ Add Reducer layer on top
- ⚠️ Partial architecture alignment

**Steps**:
1. Keep infrastructure as-is
2. Add thin Reducer layer that calls our stores
3. Add Action/State types
4. Refactor handlers to dispatch actions

**Drawbacks**:
- ⚠️ Still not using framework fully
- ⚠️ Duplicates some framework code
- ⚠️ Half-measures rarely work well

---

## Framework Usage Examples

### How to Use `composable-rust-auth` Properly

#### 1. Setup Environment with Provider Traits

```rust
use composable_rust_auth::{
    AuthEnvironment, AuthReducer, AuthState,
    providers::*,
};

// Wrap our implementations with framework traits
struct RedisSessionStoreAdapter(RedisSessionStore);
impl SessionStore for RedisSessionStoreAdapter { /* ... */ }

struct PostgresUserRepoAdapter(PostgresUserRepository);
impl UserRepository for PostgresUserRepoAdapter { /* ... */ }

// Build environment
let environment = AuthEnvironment {
    oauth: GoogleOAuthProvider::new(config),
    email: ConsoleEmailProvider::new(base_url),
    webauthn: WebAuthnProvider::new(config),
    session_store: RedisSessionStoreAdapter(session_store),
    token_store: RedisTokenStoreAdapter(token_store),
    user_repo: PostgresUserRepoAdapter(user_repo),
    device_repo: PostgresDeviceRepoAdapter(device_repo),
    risk_calculator: DefaultRiskCalculator,
    oauth_token_store: RedisOAuthTokenStoreAdapter(oauth_store),
    challenge_store: RedisChallengeStoreAdapter(challenge_store),
    rate_limiter: RedisRateLimiterAdapter(rate_limiter),
};
```

#### 2. Create Store with Reducer

```rust
use composable_rust_runtime::Store;
use std::sync::Arc;

let store = Arc::new(Store::new(
    AuthState::default(),
    AuthReducer::new(),
    environment,
));
```

#### 3. Mount Router

```rust
use composable_rust_auth::auth_router;
use axum::Router;

let app = Router::new()
    .nest("/api/v1/auth", auth_router(store.clone()))
    .layer(TraceLayer::new_for_http());
```

#### 4. Test Reducers at Memory Speed

```rust
#[test]
fn test_send_magic_link() {
    let mut state = AuthState::default();
    let reducer = AuthReducer::new();
    let env = MockEnvironment::new();  // Mock, no I/O

    let effects = reducer.reduce(
        &mut state,
        AuthAction::SendMagicLink {
            correlation_id: Uuid::new_v4(),
            email: "test@example.com".to_string(),
            ip_address: "127.0.0.1".parse().unwrap(),
            user_agent: "test".to_string(),
        },
        &env,
    );

    // Assert state changes
    assert!(state.pending_magic_links.contains_key("test@example.com"));

    // Assert effects returned
    assert_eq!(effects.len(), 1);
    assert!(matches!(effects[0], Effect::Future(_)));
}
```

---

## Code Quality Assessment

### Positives ✅

1. **Redis Implementation**: Correct usage of redis 0.27
   - ConnectionManager (Clone-able)
   - Explicit type annotations
   - TTL management
   - Rate limiting logic

2. **PostgreSQL Implementation**: Proper sqlx usage
   - Query macros
   - Error handling
   - CRUD operations

3. **Type Safety**: Custom error types
   - StoreError enum
   - RepositoryError enum
   - AuthHandlerError enum

4. **Documentation**: Function documentation
   - /// comments
   - # Arguments sections
   - # Errors sections

5. **Modern Rust**: Edition 2024 features
   - async fn in traits (EmailProvider)
   - Proper error propagation

### Negatives ❌

1. **Architecture**: Not using Reducer pattern
2. **Code Duplication**: Re-implemented framework functionality
3. **Testability**: Requires I/O for all tests
4. **Separation of Concerns**: Business logic in handlers
5. **Framework Integration**: Not using existing auth crate

---

## Migration Path

### Phase 1: Understand Framework (1 hour)

**Read**:
- `auth/src/actions.rs` - Action enum structure
- `auth/src/reducers/mod.rs` - Reducer pattern
- `auth/src/state.rs` - State structure
- `auth/src/router.rs` - Router composition
- `.claude/skills/composable-rust-web/SKILL.md` - Auth section

### Phase 2: Implement Provider Traits (3-4 hours)

**Wrap existing implementations**:
```rust
// examples/ticketing/src/auth/providers.rs

use composable_rust_auth::providers::*;

pub struct RedisSessionStoreProvider(pub RedisSessionStore);

impl SessionStore for RedisSessionStoreProvider {
    async fn create_session(&self, user_id: UserId) -> Result<Session> {
        let (session_id, session_info) = self.0.create_session(
            user_id,
            session_info.email,
            session_info.role,
        ).await?;

        Ok(Session {
            id: SessionId(session_id),
            user_id,
            // ... convert types
        })
    }

    // Implement remaining methods
}

// Repeat for:
// - TokenStore
// - UserRepository
// - DeviceRepository
// - etc.
```

### Phase 3: Use Framework Components (2-3 hours)

**Replace custom handlers**:
1. Delete `src/auth/handlers.rs` (400 lines)
2. Delete `src/auth/schemas.rs` (150 lines) - use framework's types
3. Keep `src/auth/stores.rs` - wrapped by providers
4. Keep `src/auth/repositories.rs` - wrapped by providers
5. Add `src/auth/providers.rs` - trait implementations

**Update main.rs**:
```rust
use composable_rust_auth::{auth_router, AuthReducer, AuthState};
use composable_rust_runtime::Store;

let auth_env = build_auth_environment().await?;
let auth_store = Arc::new(Store::new(
    AuthState::default(),
    AuthReducer::new(),
    auth_env,
));

let app = Router::new()
    .nest("/api/v1/auth", auth_router(auth_store));
```

### Phase 4: Update Tests (1 hour)

**Add reducer tests**:
```rust
#[test]
fn test_magic_link_flow() {
    let mut state = AuthState::default();
    let reducer = AuthReducer::new();
    let env = build_mock_env();

    // Send magic link
    let effects = reducer.reduce(&mut state,
        AuthAction::SendMagicLink { /* ... */ }, &env);

    assert!(effects.len() > 0);
    // ... more assertions
}
```

---

## Conclusion

### Summary

Phase 10.1 implementation has achieved:
- ✅ **Infrastructure**: Docker, databases, migrations (correct)
- ✅ **Storage Layer**: Redis stores, PostgreSQL repos (correct)
- ⚠️ **HTTP Layer**: Works but wrong pattern (imperative vs reducer)
- ❌ **Architecture**: Violates Composable Rust principles (critical)

### Severity: CRITICAL

This is not a cosmetic issue. The core architecture of Composable Rust is:
```
Action → Reducer → (State, Effects) → Effect Execution → More Actions
```

We bypassed this entirely. This will cause:
1. Integration issues with business logic reducers (Phases 10.5+)
2. Testing difficulties (no memory-speed tests)
3. Maintenance burden (extra code to maintain)
4. Architectural inconsistency across codebase

### Recommendation: REFACTOR

**Estimated effort**: 6-8 hours
**Impact**: High value, corrects critical issue
**Urgency**: Before Phase 10.2 (auth middleware expects Store pattern)

The good news: Our Redis/PostgreSQL implementations are solid. We just need to:
1. Wrap them with framework provider traits
2. Use framework's AuthAction/AuthReducer/AuthState
3. Use framework's auth_router
4. Delete custom handlers

This brings us into full compliance with the architecture while preserving the working infrastructure.

---

## Questions for Discussion

1. **Should we refactor to use the framework properly?**
   - Option A: Full refactor (recommended)
   - Option B: Continue as-is (with justification)
   - Option C: Hybrid approach

2. **Why did we build custom handlers instead of using the framework?**
   - Misunderstanding of the architecture?
   - Plan document unclear?
   - Intentional deviation?

3. **How will current approach integrate with business logic reducers?**
   - Event/Inventory/Reservation reducers use Reducer pattern
   - Auth middleware (Phase 10.2) expects Store
   - Need consistency across codebase

4. **What's the long-term vision for authentication?**
   - If we keep current approach, how do we maintain consistency?
   - If we refactor, when?

---

## References

- `composable-rust-auth/src/lib.rs` - Framework auth module
- `composable-rust-auth/src/actions.rs` - Action enum
- `composable-rust-auth/src/reducers/mod.rs` - Reducer implementation
- `composable-rust-auth/src/router.rs` - HTTP router
- `.claude/skills/composable-rust-architecture/SKILL.md` - Architecture principles
- `.claude/skills/composable-rust-web/SKILL.md` - Web integration patterns
