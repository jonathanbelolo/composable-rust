# Phase 6 Implementation Plan: Composable Auth

**Status**: Planning
**Dependencies**: Phase 5 (Event-driven systems)
**Estimated Duration**: 14 weeks (extended for proper WebAuthn implementation and security hardening)

---

## Overview

Implement authentication and authorization as first-class composable primitives that integrate natively with the reducer/effect architecture.

---

## Phase 6A: Foundation (Weeks 1-2)

### Core Types & Traits

- [ ] Create `composable-rust-auth` crate
  - [ ] Add to workspace
  - [ ] Set up dependencies (webauthn-rs, oauth2, etc.)
  - [ ] Configure lints and CI

- [ ] Define core types (`auth/src/state.rs`)
  - [ ] `AuthState` - Authentication state
  - [ ] `Session` - User session type
  - [ ] `TokenPair` - Access + refresh tokens
  - [ ] `User` - Authenticated user type
  - [ ] `Challenge` - WebAuthn challenge type
  - [ ] `PublicKeyCredential` - Passkey credential

- [ ] Define action types (`auth/src/actions.rs`)
  - [ ] `AuthAction::InitiatePasskeyLogin` - Start passkey flow
  - [ ] `AuthAction::VerifyPasskey` - Verify passkey credential
  - [ ] `AuthAction::SendMagicLink` - Send passwordless email
  - [ ] `AuthAction::VerifyMagicLink` - Verify magic link token
  - [ ] `AuthAction::InitiateOAuth` - Start OAuth flow
  - [ ] `AuthAction::OAuthCallback` - Handle OAuth callback
  - [ ] `AuthAction::LoginSuccess`
  - [ ] `AuthAction::LoginFailed`
  - [ ] `AuthAction::RefreshToken`
  - [ ] `AuthAction::Logout`

- [ ] Define effect types (`auth/src/effects.rs`)
  - [ ] `AuthEffect::GenerateChallenge` - WebAuthn challenge
  - [ ] `AuthEffect::VerifyAssertion` - Verify WebAuthn assertion
  - [ ] `AuthEffect::StoreCredential` - Store passkey
  - [ ] `AuthEffect::SendEmail` - Send magic link
  - [ ] `AuthEffect::ValidateMagicToken` - Verify magic link
  - [ ] `AuthEffect::RedirectToProvider` - OAuth redirect
  - [ ] `AuthEffect::ExchangeCodeForToken` - OAuth token exchange
  - [ ] `AuthEffect::GenerateTokens`
  - [ ] `AuthEffect::RefreshTokens`
  - [ ] `AuthEffect::RevokeSession`
  - [ ] `AuthEffect::StoreSession`

- [ ] Define core traits (`auth/src/providers/mod.rs`)
  - [ ] `AuthProvider` - Generic auth provider trait
  - [ ] `PasskeyProvider` - WebAuthn/FIDO2 trait
  - [ ] `MagicLinkProvider` - Email/SMS auth trait
  - [ ] `OAuthProvider` - OAuth2 provider trait
  - [ ] `TokenGenerator` - Token generation trait
  - [ ] `SessionStore` - Session storage trait

### OAuth2 Implementation (Simplest to start)

- [ ] Implement OAuth2 provider (`auth/src/providers/oauth2.rs`)
  - [ ] Generic OAuth2 provider
  - [ ] Google provider
  - [ ] GitHub provider
  - [ ] Authorization URL generation
  - [ ] Code exchange for tokens
  - [ ] Token refresh

- [ ] Implement OAuth2 reducer (`auth/src/reducers/oauth.rs`)
  - [ ] Handle `InitiateOAuth` action
  - [ ] Handle `OAuthCallback` action
  - [ ] State machine for OAuth flow
  - [ ] Generate appropriate effects

### Magic Link Implementation

- [ ] Magic link provider (`auth/src/providers/magic_link.rs`)
  - [ ] Token generation (cryptographically secure)
  - [ ] Token storage (Redis/PostgreSQL)
  - [ ] Token expiration (5-15 minutes)
  - [ ] Email template support

- [ ] Magic link reducer (`auth/src/reducers/magic_link.rs`)
  - [ ] Handle `SendMagicLink` action
  - [ ] Handle `VerifyMagicLink` action
  - [ ] Rate limiting logic
  - [ ] Generate appropriate effects

### Axum Integration

- [ ] Middleware (`auth/src/middleware/axum.rs`)
  - [ ] `RequireAuth` layer
  - [ ] Token extraction from headers
  - [ ] Error responses (401, 403)

- [ ] Extractors (`auth/src/middleware/extractors.rs`)
  - [ ] `AuthenticatedUser` extractor
  - [ ] `OptionalAuth` extractor
  - [ ] Error handling

### WebAuthn/Passkeys Implementation (Advanced - Week 2)

- [ ] WebAuthn provider (`auth/src/providers/webauthn.rs`)
  - [ ] Challenge generation
  - [ ] Credential registration flow
  - [ ] Assertion verification
  - [ ] Credential storage
  - [ ] Device management

- [ ] Passkey reducer (`auth/src/reducers/passkey.rs`)
  - [ ] Handle `InitiatePasskeyLogin` action
  - [ ] Handle `VerifyPasskey` action
  - [ ] Handle `RegisterPasskey` action
  - [ ] Generate appropriate effects

### Testing

- [ ] Unit tests
  - [ ] OAuth2 flow state machine
  - [ ] Magic link token generation/validation
  - [ ] WebAuthn challenge/response
  - [ ] Token expiration
  - [ ] Reducer logic

- [ ] Integration tests
  - [ ] Full OAuth2 flow with mock provider
  - [ ] Magic link flow
  - [ ] WebAuthn registration and login
  - [ ] Token refresh
  - [ ] Invalid token handling

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
