# Phase 6 Implementation Plan: Composable Auth

**Status**: 🚧 In Progress - Phase 6A Foundation
**Dependencies**: Phase 5 (Event-driven systems)
**Estimated Duration**: 14 weeks (extended for proper WebAuthn implementation and security hardening)

**Current Progress**:
- ✅ Core types, actions, and traits defined
- ✅ OAuth2 reducer with real effects implemented
- ✅ Mock providers for all traits (OAuth2, Email, WebAuthn, Session, User, Device, Risk)
- ✅ OAuth2 integration tests (9 tests, all passing)
- 🔄 Next: Magic link and WebAuthn reducers

---

## Overview

Implement authentication and authorization as first-class composable primitives that integrate natively with the reducer/effect architecture.

---

## Phase 6A: Foundation (Weeks 1-2)

### Core Types & Traits ✅ COMPLETED

- [x] Create `composable-rust-auth` crate
  - [x] Add to workspace
  - [x] Set up dependencies (webauthn-rs, oauth2, etc.)
  - [x] Configure lints and CI

- [x] Define core types (`auth/src/state.rs`)
  - [x] `AuthState` - Authentication state
  - [x] `Session` - User session type
  - [x] `User` - User identifier types (UserId, DeviceId)
  - [x] `OAuthState` - OAuth flow state with CSRF protection
  - [x] `OAuthProvider` - Enum (Google, GitHub, Microsoft)

- [x] Define action types (`auth/src/actions.rs`)
  - [x] `AuthAction::InitiatePasskeyLogin` - Start passkey flow
  - [x] `AuthAction::VerifyPasskey` - Verify passkey credential
  - [x] `AuthAction::SendMagicLink` - Send passwordless email
  - [x] `AuthAction::VerifyMagicLink` - Verify magic link token
  - [x] `AuthAction::InitiateOAuth` - Start OAuth flow
  - [x] `AuthAction::OAuthCallback` - Handle OAuth callback
  - [x] `AuthAction::OAuthSuccess` - OAuth token exchange success
  - [x] `AuthAction::OAuthFailed` - OAuth error handling
  - [x] `AuthAction::SessionCreated` - Session creation event
  - [x] `AuthAction::Logout`
  - [x] `AuthLevel` - Progressive authentication levels (Basic, MultiFactor, HardwareBacked)
  - [x] `DeviceTrust` - Device trust levels

- [x] Define effect types (`auth/src/effects.rs`)
  - [x] Effects now use core `Effect::Future` with async operations
  - [x] No custom effect types needed - provider calls return `Effect<AuthAction>`

- [x] Define core traits (`auth/src/providers/mod.rs`)
  - [x] `OAuth2Provider` - OAuth2/OIDC trait with RPITIT pattern
  - [x] `WebAuthnProvider` - WebAuthn/FIDO2 trait
  - [x] `EmailProvider` - Email delivery trait
  - [x] `SessionStore` - Session storage trait
  - [x] `UserRepository` - User persistence trait
  - [x] `DeviceRepository` - Device tracking trait
  - [x] `RiskCalculator` - Risk assessment trait

### OAuth2 Implementation (Simplest to start) ✅ REDUCER DONE

- [x] Implement OAuth2 reducer (`auth/src/reducers/oauth.rs`)
  - [x] Handle `InitiateOAuth` action with CSRF state generation
  - [x] Handle `OAuthCallback` action with state validation
  - [x] Handle `OAuthSuccess` action with session creation
  - [x] Handle `OAuthFailed` action with error handling
  - [x] State machine for OAuth flow (5-minute expiration)
  - [x] Generate appropriate `Effect::Future` for async operations

- [x] Mock Providers for Testing (`auth/src/mocks/`)
  - [x] `MockOAuth2Provider` - Configurable success/failure
  - [x] `MockUserRepository` - In-memory user storage
  - [x] `MockDeviceRepository` - In-memory device tracking
  - [x] `MockSessionStore` - In-memory session storage with TTL
  - [x] `MockEmailProvider` - Email delivery stub
  - [x] `MockWebAuthnProvider` - WebAuthn simulation
  - [x] `MockRiskCalculator` - Configurable risk scores

- [ ] Real OAuth2 Provider Implementation (deferred until after reducers)
  - [ ] Generic OAuth2 provider with openidconnect crate
  - [ ] Google provider configuration
  - [ ] GitHub provider configuration
  - [ ] Authorization URL generation
  - [ ] Code exchange for tokens
  - [ ] Token refresh

### OAuth2 Testing ✅ COMPLETED

- [x] Integration tests (`auth/tests/oauth_integration.rs`)
  - [x] Complete happy path (initiate → callback → success)
  - [x] CSRF state validation (reject invalid state)
  - [x] State expiration (5-minute TTL)
  - [x] Security: prior initiation required
  - [x] Error handling (OAuthFailed)
  - [x] Multi-provider support (Google, GitHub, Microsoft)
  - [x] Session metadata validation
  - [x] CSRF state uniqueness
  - [x] Session creation event
  - [x] **9 tests passing** ✅

### Magic Link Implementation 🔄 NEXT

- [ ] Magic link reducer (`auth/src/reducers/magic_link.rs`)
  - [ ] Handle `SendMagicLink` action
  - [ ] Handle `VerifyMagicLink` action
  - [ ] Handle `MagicLinkSent` event
  - [ ] Handle `MagicLinkVerified` event
  - [ ] Handle `MagicLinkFailed` event
  - [ ] Token generation with cryptographic randomness
  - [ ] Token expiration logic (5-15 minutes)
  - [ ] Rate limiting state tracking
  - [ ] Generate appropriate `Effect::Future` for async operations

- [ ] Magic link integration tests
  - [ ] Complete happy path (send → verify)
  - [ ] Token expiration
  - [ ] Invalid token rejection
  - [ ] Rate limiting (5 per hour)
  - [ ] Token single-use enforcement
  - [ ] User enumeration prevention

**Note**: Magic link provider trait already exists. Mock implementation already done. Real implementation deferred until after reducers.

### WebAuthn/Passkeys Implementation 🔄 NEXT

- [ ] Passkey reducer (`auth/src/reducers/passkey.rs`)
  - [ ] Handle `InitiatePasskeyLogin` action
  - [ ] Handle `InitiatePasskeyRegistration` action
  - [ ] Handle `VerifyPasskey` action
  - [ ] Handle `RegisterPasskey` action
  - [ ] Handle `PasskeyVerified` event
  - [ ] Handle `PasskeyRegistered` event
  - [ ] Handle `PasskeyFailed` event
  - [ ] Challenge generation and storage
  - [ ] Challenge expiration (5 minutes)
  - [ ] Generate appropriate `Effect::Future` for async operations

- [ ] Passkey integration tests
  - [ ] Registration flow (initiate → verify)
  - [ ] Login flow (challenge → verify)
  - [ ] Challenge expiration
  - [ ] Invalid assertion rejection
  - [ ] Credential storage
  - [ ] Multi-device support

**Note**: WebAuthn provider trait already exists. Mock implementation already done. Real implementation deferred until after reducers.

### Axum Integration (Deferred to Phase 6B)

- [ ] Middleware (`auth/src/middleware/axum.rs`)
  - [ ] `RequireAuth` layer
  - [ ] Token extraction from headers
  - [ ] Error responses (401, 403)

- [ ] Extractors (`auth/src/middleware/extractors.rs`)
  - [ ] `AuthenticatedUser` extractor
  - [ ] `OptionalAuth` extractor
  - [ ] Error handling

### Testing Status

- [x] OAuth2 integration tests (9 tests ✅)
  - [x] OAuth2 flow state machine
  - [x] Token expiration
  - [x] Reducer logic
  - [x] Full OAuth2 flow with mock provider
  - [x] Invalid token handling

- [ ] Magic link integration tests (upcoming)
  - [ ] Magic link token generation/validation
  - [ ] Full magic link flow

- [ ] WebAuthn integration tests (upcoming)
  - [ ] WebAuthn challenge/response
  - [ ] WebAuthn registration and login
  - [ ] Token refresh

### Documentation

- [ ] API documentation
  - [ ] Module-level docs
  - [ ] Function-level docs
  - [ ] Examples in docstrings

- [ ] User guide
  - [ ] Quickstart: OAuth2 in 5 minutes
  - [ ] Magic link setup
  - [ ] Passkey implementation guide
  - [ ] Configuration guide

---

## Phase 6B: WebAuthn & Session Management (Weeks 3-5)

### Challenge Storage (Redis)

- [ ] Implement challenge store (`auth/src/stores/challenge_store.rs`)
  - [ ] `ChallengeStore` trait
  - [ ] Redis implementation with TTL (~5 minutes)
  - [ ] Challenge generation (cryptographically secure)
  - [ ] Challenge validation and one-time-use enforcement
  - [ ] Automatic expiration cleanup

### WebAuthn/Passkeys Implementation (Extended Timeline)

- [ ] WebAuthn provider (`auth/src/providers/webauthn.rs`)
  - [ ] Challenge generation with Redis storage
  - [ ] Credential registration flow
  - [ ] Assertion verification with origin and rpId validation
  - [ ] Credential storage (PostgreSQL)
  - [ ] Device management
  - [ ] Public key crypto verification (performance testing)

- [ ] Passkey reducer (`auth/src/reducers/passkey.rs`)
  - [ ] Handle `InitiatePasskeyLogin` action
  - [ ] Handle `VerifyPasskey` action with timing-safe comparison
  - [ ] Handle `RegisterPasskey` action
  - [ ] Generate appropriate effects

### Persistent Device Registry (PostgreSQL)

**Critical**: Device data outlives sessions and is stored in PostgreSQL.

- [ ] Device registry schema (`auth/migrations/002_device_registry.sql`)
  - [ ] `users` table (id, email, email_verified, created_at, updated_at)
  - [ ] `registered_devices` table:
    - device_id (UUID, PK)
    - user_id (FK to users)
    - name (String, "iPhone 15 Pro")
    - device_type (Enum: Mobile, Desktop, Tablet)
    - platform (String, "iOS 17.2")
    - first_seen (DateTime)
    - last_seen (DateTime)
    - trusted (Boolean, user-marked)
    - requires_mfa (Boolean)
    - passkey_credential_id (Optional String)
    - public_key (Optional Bytes)
  - [ ] `oauth_links` table (user_id, provider, provider_user_id, email, linked_at)
  - [ ] Indexes: user_id, last_seen, passkey_credential_id

- [ ] Device store trait (`auth/src/stores/device_store.rs`)
  - [ ] `get_device()` - Retrieve device by ID
  - [ ] `create_device()` - Register new device
  - [ ] `update_device_last_seen()` - Update last active timestamp
  - [ ] `list_user_devices()` - Get all devices for a user
  - [ ] `delete_device()` - Revoke device permanently
  - [ ] `mark_device_trusted()` - User marks device as trusted
  - [ ] `link_passkey_to_device()` - Associate passkey with device

- [ ] PostgreSQL device store (`auth/src/stores/device_postgres.rs`)
  - [ ] Implement device_store trait
  - [ ] Efficient queries with indexes
  - [ ] Tests with testcontainers

### Recovery Flows

- [ ] Recovery code implementation (`auth/src/recovery/mod.rs`)
  - [ ] Generate 10 single-use recovery codes
  - [ ] Bcrypt hashing for recovery codes
  - [ ] Recovery code validation (constant-time comparison)
  - [ ] Mark recovery code as used after verification
  - [ ] Display codes only once during generation
  - [ ] Store recovery codes in PostgreSQL (persistent)

- [ ] Backup email flow (`auth/src/recovery/backup_email.rs`)
  - [ ] Backup email registration (stored in PostgreSQL)
  - [ ] Backup email verification
  - [ ] Recovery via backup email
  - [ ] Rate limiting for recovery attempts

- [ ] Device revocation (`auth/src/recovery/revocation.rs`)
  - [ ] List all registered devices from PostgreSQL
  - [ ] Revoke specific device:
    - Delete all active sessions for device (Redis)
    - Delete device record (PostgreSQL)
  - [ ] Revoke all devices (nuclear option)
  - [ ] Audit logging for all revocations

### Device Trust Levels (Progressive Trust)

**See**: `plans/phase-6/advanced-features.md` for detailed specification

- [ ] Trust level calculation (`auth/src/devices/trust.rs`)
  - [ ] `DeviceTrustLevel` enum (Unknown, Recognized, Familiar, Trusted, HighlyTrusted)
  - [ ] Calculate trust based on age, login count, user marking
  - [ ] Allow actions per trust level
  - [ ] Risk modifier for integration with risk-based auth

- [ ] Add metrics to RegisteredDevice
  - [ ] `login_count` field
  - [ ] `user_marked_trusted` field (replaces boolean `trusted`)
  - [ ] Update schema migration

- [ ] Auto-promotion suggestions (`auth/src/devices/promotion.rs`)
  - [ ] Check promotion eligibility (30 days, 20+ logins)
  - [ ] Generate promotion suggestions
  - [ ] User acceptance/rejection tracking

- [ ] Integration
  - [ ] Use trust level in risk calculation
  - [ ] UI for device management with trust levels
  - [ ] Metrics for devices by trust level

### Session Store Trait (Redis - Ephemeral)

**Critical**: Sessions are ephemeral (TTL-based) and stored in Redis. They reference devices from PostgreSQL.

- [ ] Define `SessionStore` trait (`auth/src/stores/session_store.rs`)
  - [ ] `create_session()` - Store new session (with device_id FK)
  - [ ] `get_session()` - Retrieve session
  - [ ] `update_session()` - Update session data
  - [ ] `delete_session()` - Remove session
  - [ ] `extend_session()` - Extend expiration (sliding window)
  - [ ] `list_user_sessions()` - Get all active sessions for a user
  - [ ] `delete_all_user_sessions()` - Revoke all sessions for a user

### Redis Session Implementation

- [ ] Redis session store (`auth/src/stores/session_redis.rs`)
  - [ ] Connection pooling
  - [ ] Session serialization (bincode)
  - [ ] TTL-based expiration (24 hours default, sliding)
  - [ ] Multi-device tracking (user:{user_id}:sessions set)
  - [ ] Encrypted OAuth token storage
  - [ ] Tests with testcontainers

### Session Reducer

- [ ] Session management reducer (`auth/src/reducers/session.rs`)
  - [ ] Handle session creation:
    - Check/create device in PostgreSQL
    - Create session in Redis (with device_id reference)
  - [ ] Handle session refresh (sliding expiration)
  - [ ] Handle session expiration
  - [ ] Handle logout (delete session, keep device)
  - [ ] Handle device revocation (delete sessions + device)
  - [ ] Multi-device session support

### Refresh Token Flow

- [ ] Token refresh logic
  - [ ] Refresh token validation
  - [ ] Token rotation on refresh
  - [ ] Refresh token revocation
  - [ ] Rate limiting refresh attempts

### Testing

- [ ] Unit tests
  - [ ] Session store operations
  - [ ] Session expiration logic
  - [ ] Token refresh flow

- [ ] Integration tests
  - [ ] Full session lifecycle
  - [ ] Multi-device sessions
  - [ ] Session cleanup

### Documentation

- [ ] Session management guide
  - [ ] Redis setup
  - [ ] PostgreSQL setup
  - [ ] Multi-device support
  - [ ] Session expiration strategies

---

## Phase 6C: Authorization (Weeks 6-8)

### Account Linking Strategy

- [ ] Account linking implementation (`auth/src/linking/mod.rs`)
  - [ ] Detect same email from multiple providers
  - [ ] "Ask user" strategy (default, most secure)
  - [ ] Re-authentication before linking
  - [ ] User can decline and create separate account
  - [ ] Email ownership verification for linking
  - [ ] Audit events for all account linking operations

- [ ] Alternative auto-link strategy (`auth/src/linking/auto_link.rs`)
  - [ ] Configurable per-tenant
  - [ ] Auto-link if email verified by both providers
  - [ ] Audit event for security monitoring
  - [ ] Can be disabled for stricter security

### Risk-Based Adaptive Authentication

**See**: `plans/phase-6/advanced-features.md` for detailed specification

- [ ] Risk calculator service (`auth/src/risk/calculator.rs`)
  - [ ] `RiskCalculator` with GeoIP and breach database integration
  - [ ] Calculate login risk from context (device, location, time, patterns)
  - [ ] Risk factors: new device, new location, impossible travel, unusual time, breach database
  - [ ] Risk levels: Low, Medium, High, Critical
  - [ ] Select required challenges based on risk level

- [ ] Add risk fields to data models
  - [ ] `RegisteredDevice`: risk_score, usual_ip_ranges, usual_login_hours, failed_attempts
  - [ ] `Session`: login_risk_score, current_risk_level, requires_step_up

- [ ] Integration
  - [ ] GeoIP database (MaxMind GeoLite2)
  - [ ] HaveIBeenPwned API integration
  - [ ] Device location tracking (PostgreSQL)
  - [ ] Impossible travel detection algorithm
  - [ ] Security alerting for critical risk events
  - [ ] Metrics: risk score distribution, risk factors, blocked logins

### Granular Permission Caching

**See**: `plans/phase-6/advanced-features.md` for detailed specification

- [ ] Permission cache service (`auth/src/permissions/cache.rs`)
  - [ ] `PermissionCache` with Redis hash storage
  - [ ] Lazy-load permissions on-demand (not in session)
  - [ ] Per-permission TTL based on criticality (1 min to 1 hour)
  - [ ] Invalidation API (single permission or all)

- [ ] Remove permissions from Session
  - [ ] Sessions store only session_id, user_id, device_id
  - [ ] Permissions loaded from Redis hash on each check

- [ ] Redis hash structure
  - [ ] Key: `permission:{user_id}`
  - [ ] Hash: `{permission_name -> expiry_timestamp}`
  - [ ] TTL: Critical=1min, High=5min, Medium=15min, Low=1hour

- [ ] Integration
  - [ ] Axum middleware for permission checks
  - [ ] Batch permission loading
  - [ ] Request-level caching
  - [ ] Metrics: cache hits/misses, check duration

### Step-Up Authentication

**See**: `plans/phase-6/advanced-features.md` for detailed specification

- [ ] Elevation scope enum (`auth/src/elevation/mod.rs`)
  - [ ] `ElevationScope`: DeleteAccount, TransferMoney, ChangeEmail, ManageUsers, ViewSensitiveData
  - [ ] Duration per scope (1-10 minutes)
  - [ ] Required challenge per scope

- [ ] Add elevation fields to Session
  - [ ] `elevated_until: Option<DateTime>`
  - [ ] `elevation_scope: Option<ElevationScope>`

- [ ] Step-up challenge flow
  - [ ] Initiate step-up (generate challenge)
  - [ ] Verify step-up (validate challenge)
  - [ ] Grant elevation (update session with expiry)
  - [ ] Require elevation middleware

- [ ] Integration
  - [ ] Axum middleware: `require_elevation()`
  - [ ] Step-up challenge endpoints
  - [ ] Audit logging for all elevation grants
  - [ ] Metrics: elevation requests, grants, by scope

### Core Authorization Types

- [ ] Define authorization types (`auth/src/authorization/mod.rs`)
  - [ ] `Permission` - Permission enum/string
  - [ ] `Role` - Role type
  - [ ] `Resource` - Resource identifier
  - [ ] `Action` - Action type (read, write, delete, etc.)
  - [ ] `AuthDecision` - Allow/Deny with reason

- [ ] `Authorizer` trait (`auth/src/authorization/authorizer.rs`)
  - [ ] `authorize()` - Core authorization method
  - [ ] `check_permission()` - Simple permission check
  - [ ] `require_permission()` - Throw on deny

### RBAC Implementation

- [ ] Role store trait (`auth/src/authorization/rbac/store.rs`)
  - [ ] `get_user_roles()` - Fetch user's roles
  - [ ] `assign_role()` - Assign role to user
  - [ ] `revoke_role()` - Remove role from user
  - [ ] `get_role_permissions()` - Get permissions for role

- [ ] RBAC authorizer (`auth/src/authorization/rbac/authorizer.rs`)
  - [ ] Role-based permission checks
  - [ ] Role hierarchy support
  - [ ] Caching for performance

- [ ] PostgreSQL role store (`auth/src/authorization/rbac/postgres.rs`)
  - [ ] Schema migration (users_roles, role_permissions tables)
  - [ ] Efficient permission queries
  - [ ] Tests

### Policy Engine

- [ ] Policy trait (`auth/src/authorization/policies/mod.rs`)
  - [ ] `Policy::evaluate()` - Core policy method
  - [ ] `AuthContext` - Context for policy evaluation

- [ ] Built-in policies (`auth/src/authorization/policies/`)
  - [ ] `OwnershipPolicy` - Resource ownership check
  - [ ] `TimePolicy` - Time-based access
  - [ ] `IpWhitelistPolicy` - IP-based access
  - [ ] `OrganizationPolicy` - Multi-tenant isolation

- [ ] Policy combinator (`auth/src/authorization/policies/combinator.rs`)
  - [ ] `AllOf` - All policies must allow
  - [ ] `AnyOf` - At least one policy must allow
  - [ ] `Not` - Invert policy decision

### Axum Integration

- [ ] Authorization middleware (`auth/src/middleware/authorize.rs`)
  - [ ] `RequirePermission` layer
  - [ ] `RequireRole` layer
  - [ ] Resource-based authorization

- [ ] Extractors
  - [ ] `AuthorizedUser` - User with authorization check
  - [ ] `RequirePermission<P>` - Generic permission extractor

### Testing

- [ ] Unit tests
  - [ ] RBAC logic
  - [ ] Policy evaluation
  - [ ] Policy combinators

- [ ] Integration tests
  - [ ] Full authorization flow
  - [ ] Multi-policy scenarios
  - [ ] Edge cases (no permissions, no roles)

### Documentation

- [ ] Authorization guide
  - [ ] RBAC setup
  - [ ] Custom policies
  - [ ] Policy combinators
  - [ ] Best practices

---

## Phase 6D: OAuth2 & OIDC Deep Dive (Weeks 9-10)

### OAuth2 Provider

- [ ] OAuth2 state machine (`auth/src/providers/oauth2/state.rs`)
  - [ ] Authorization URL generation
  - [ ] Callback handling
  - [ ] Code exchange
  - [ ] Token refresh

- [ ] OAuth2 reducer (`auth/src/reducers/oauth.rs`)
  - [ ] Handle authorization flow as saga
  - [ ] State transitions
  - [ ] Error handling

- [ ] OAuth2 provider implementations
  - [ ] Generic OAuth2 provider
  - [ ] Google provider
  - [ ] GitHub provider
  - [ ] Microsoft provider

### OIDC Support

- [ ] OIDC discovery (`auth/src/providers/oidc/discovery.rs`)
  - [ ] Parse `.well-known/openid-configuration`
  - [ ] Cache discovery document

- [ ] OIDC provider (`auth/src/providers/oidc/provider.rs`)
  - [ ] ID token validation
  - [ ] UserInfo endpoint integration
  - [ ] Claims extraction

### Axum Integration

- [ ] OAuth2 routes (`auth/src/middleware/oauth_routes.rs`)
  - [ ] `/auth/login` - Initiate flow
  - [ ] `/auth/callback` - Handle callback
  - [ ] `/auth/logout` - Logout

### Testing

- [ ] Unit tests
  - [ ] OAuth2 state machine
  - [ ] Token exchange
  - [ ] OIDC discovery

- [ ] Integration tests with mock providers
  - [ ] Full authorization code flow
  - [ ] Token refresh
  - [ ] UserInfo retrieval

### Documentation

- [ ] OAuth2 guide
  - [ ] Setup with Google
  - [ ] Setup with GitHub
  - [ ] Custom provider configuration
  - [ ] Scopes and claims

---

## Phase 6E: Multi-Tenancy & Delegation (Week 11)

### Multi-Tenancy

- [ ] Tenant isolation (`auth/src/multi_tenant/`)
  - [ ] `TenantId` type
  - [ ] Tenant context middleware
  - [ ] Data isolation enforcement

- [ ] Tenant resolver
  - [ ] Subdomain-based resolution
  - [ ] Header-based resolution
  - [ ] Token claim-based resolution

### Delegation & Impersonation

- [ ] Impersonation support (`auth/src/delegation/`)
  - [ ] `impersonate_user()` action
  - [ ] Permission checks for impersonation
  - [ ] Audit logging for delegation
  - [ ] Revert to original user

### Testing

- [ ] Multi-tenant isolation tests
- [ ] Delegation tests
- [ ] Impersonation security tests

### Documentation

- [ ] Enterprise guide
  - [ ] Multi-tenancy patterns
  - [ ] Delegation use cases
  - [ ] Impersonation security

---

## Phase 6F: Production Hardening (Weeks 12-14)

### Security

- [ ] Security audit
  - [ ] Third-party security review
  - [ ] Penetration testing
  - [ ] Fix identified vulnerabilities

- [ ] Rate limiting (tower-governor)
  - [ ] Magic link requests: 5 per hour per email
  - [ ] Login attempts: 10 per minute per IP
  - [ ] Any auth attempt: 100 per minute per IP
  - [ ] Configurable per-tenant limits
  - [ ] Rate limit metrics

- [ ] Security hardening
  - [ ] Constant-time token comparison (constant_time_eq crate)
  - [ ] OAuth state parameter entropy (256 bits, cryptographic random)
  - [ ] Session ID regeneration on login (prevent session fixation)
  - [ ] WebAuthn origin validation (explicit checks)
  - [ ] WebAuthn rpId validation (explicit checks)
  - [ ] No user enumeration (same response for magic links)

- [ ] CSRF protection
  - [ ] CSRF token generation
  - [ ] Validation in state-changing endpoints

### Security Testing Suite

- [ ] Timing attack tests (`auth/tests/security/timing_attacks.rs`)
  - [ ] Magic link token comparison timing
  - [ ] Recovery code validation timing
  - [ ] Property-based tests with proptest

- [ ] Replay attack tests (`auth/tests/security/replay_attacks.rs`)
  - [ ] Challenge reuse prevention
  - [ ] Magic link reuse prevention
  - [ ] OAuth state reuse prevention

- [ ] Race condition tests (`auth/tests/security/race_conditions.rs`)
  - [ ] Concurrent session creation
  - [ ] Concurrent challenge validation
  - [ ] Recovery code concurrent use

- [ ] User enumeration tests (`auth/tests/security/enumeration.rs`)
  - [ ] Magic link response uniformity
  - [ ] Login failure response uniformity
  - [ ] Timing consistency across user existence

- [ ] WebAuthn security tests (`auth/tests/security/webauthn.rs`)
  - [ ] Origin mismatch rejection
  - [ ] RpId mismatch rejection
  - [ ] Challenge expiration
  - [ ] Virtual authenticator tests (fantoccini + Chrome DevTools Protocol)

### Performance

- [ ] Caching
  - [ ] Session caching (optional in-memory LRU)
  - [ ] Permission caching
  - [ ] Token validation caching

- [ ] Benchmarks
  - [ ] Token validation performance (<1ms target)
  - [ ] Session lookup performance (<5ms Redis, <10ms PostgreSQL)
  - [ ] Authorization check performance (<10ms)
  - [ ] WebAuthn verification latency (public key crypto)
  - [ ] Magic link generation performance

### Observability

- [ ] Metrics (per-method granularity)
  - [ ] Login attempts (success/failure, per auth method)
  - [ ] Passkey-specific: verification duration, challenge generation/expiration
  - [ ] Magic link-specific: sent, expired, invalid
  - [ ] OAuth-specific: flow duration per provider
  - [ ] Active sessions
  - [ ] Authorization checks (allow/deny, per resource type)
  - [ ] Token refresh rate
  - [ ] Rate limit hits

- [ ] Distributed tracing (OpenTelemetry)
  - [ ] OAuth flow tracing across services (authorization → callback → token exchange → UserInfo → session)
  - [ ] Trace ID propagation through auth flows
  - [ ] Child spans for each flow step
  - [ ] Tracing for passkey verification
  - [ ] Tracing for magic link flow

- [ ] Alerting
  - [ ] Failed login spike (by method)
  - [ ] Unusual geographic access
  - [ ] Permission escalation attempts
  - [ ] High rate limit hits
  - [ ] Challenge expiration rate spike

### Documentation

- [ ] Complete API documentation
- [ ] Architecture diagrams
- [ ] Sequence diagrams (OAuth flow, WebAuthn flow, magic link flow)

- [ ] Security documentation (`docs/security/`)
  - [ ] `threat-model.md` - Comprehensive threat model for auth system
  - [ ] `testing.md` - Security testing plan and procedures
  - [ ] `best-practices.md` - Security best practices guide
  - [ ] `incident-response.md` - Incident response plan for security events

- [ ] User guides (`docs/guides/`)
  - [ ] `migration/from-passwords.md` - Migration guide from password-based auth
  - [ ] `accessibility.md` - Accessibility guide for auth UI components
  - [ ] `troubleshooting.md` - Troubleshooting guide

### Examples

- [ ] Simple JWT API example
- [ ] RBAC SaaS example
- [ ] OAuth2 consumer app example
- [ ] Enterprise SAML example

---

## Success Criteria

### Functionality

- [ ] JWT authentication works out of the box
- [ ] Session management with Redis/PostgreSQL
- [ ] RBAC with custom roles and permissions
- [ ] OAuth2/OIDC with major providers
- [ ] SAML SSO support
- [ ] Multi-tenancy isolation

### Quality

- [ ] >95% test coverage
- [ ] All examples build and run
- [ ] CI/CD passes all checks
- [ ] Security audit complete

### Performance

- [ ] Token validation <1ms (cached)
- [ ] Session lookup <5ms (Redis)
- [ ] Authorization check <10ms
- [ ] Full OAuth flow <500ms

### Documentation

- [ ] API docs 100% complete
- [ ] User guide published
- [ ] 4+ working examples
- [ ] Video tutorial

---

## Dependencies

### Internal

- `composable-rust-core` - Core traits
- `composable-rust-runtime` - Store, effects
- `composable-rust-postgres` - Session storage

### External

- `webauthn-rs` - WebAuthn/FIDO2 implementation
- `oauth2` - OAuth2 flows
- `openidconnect` - OIDC support
- `axum` - Web framework
- `tower` - Middleware
- `tower-http` - HTTP middleware (CORS, compression, etc.)
- `tower-governor` - Rate limiting middleware
- `redis` - Session and challenge storage
- `sqlx` - Database access
- `lettre` - Email sending (for magic links)
- `rand` - Cryptographic random numbers
- `base64` - Encoding support
- `constant_time_eq` - Timing-safe comparison for tokens
- `proptest` - Property-based testing for crypto operations
- `wiremock` - Mock HTTP servers for OAuth testing
- `fantoccini` - Browser automation for WebAuthn testing

---

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| SAML complexity | High | Start with OAuth2, defer SAML to later |
| Security vulnerabilities | Critical | Third-party security audit |
| Performance bottlenecks | Medium | Early benchmarking, profiling |
| Breaking API changes | Medium | Extensive testing, semver |

---

## Future Enhancements (Phase 7)

### SAML Support (Deferred from Phase 6)

- [ ] SAML provider (`auth/src/providers/saml/`)
  - [ ] SP metadata generation
  - [ ] Assertion parsing
  - [ ] Signature validation
  - [ ] Encryption support

- [ ] SAML reducer
  - [ ] SP-initiated SSO
  - [ ] IdP-initiated SSO
  - [ ] Single Logout (SLO)

- [ ] SAML testing and documentation

**Rationale**: SAML is enterprise-focused and complex. Phase 6 prioritizes modern passwordless auth (OAuth2, WebAuthn, magic links) which covers 90% of use cases. SAML can be added in Phase 7 once the core passwordless foundation is solid.

### Additional MFA Options

- [ ] TOTP (Time-based OTP) for backup auth
  - Note: Passkeys already provide MFA (something you have + biometric)
  - TOTP useful as fallback for users without passkey-capable devices
- [ ] SMS verification (with security warnings about SIM swapping)
- [ ] Email-based 2FA

### Passkey Advanced Features

- [ ] Cross-device passkey syncing (Apple/Google/1Password ecosystem integration)
- [ ] Passkey backup/recovery flows
- [ ] Device attestation verification
- [ ] Conditional UI (autofill)

### Advanced Rate Limiting

- [ ] Adaptive rate limiting (adjust limits based on threat level)
- [ ] Distributed rate limiting (shared state across instances)
- [ ] Per-user limits (in addition to per-IP)

### API Key Management

- [ ] API key generation
- [ ] Scoped permissions
- [ ] Usage tracking
- [ ] Rotation policies

### Audit Log Export

- [ ] SIEM integration (Splunk, DataDog, etc.)
- [ ] Custom formatters
- [ ] Retention policies
- [ ] Compliance reports (GDPR, SOC2, HIPAA)

### Social Auth Providers

- [ ] Apple Sign-In
- [ ] Twitter/X auth
- [ ] LinkedIn auth
- [ ] Discord auth

---

## Notes

- Phase 6 focuses on **composable auth primitives**
- Every auth component is a reducer, effect, or saga
- Full integration with event sourcing architecture
- Progressive complexity: simple to enterprise
- Production-ready from day 1
