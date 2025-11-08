# Phase 6 Implementation Plan: Composable Auth

**Status**: Planning
**Dependencies**: Phase 5 (Event-driven systems)
**Estimated Duration**: 12 weeks

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

## Phase 6B: Session Management (Weeks 3-4)

### Session Store Trait

- [ ] Define `SessionStore` trait (`auth/src/stores/mod.rs`)
  - [ ] `create_session()` - Store new session
  - [ ] `get_session()` - Retrieve session
  - [ ] `update_session()` - Update session data
  - [ ] `delete_session()` - Remove session
  - [ ] `extend_session()` - Extend expiration

### Redis Implementation

- [ ] Redis session store (`auth/src/stores/redis.rs`)
  - [ ] Connection pooling
  - [ ] Session serialization
  - [ ] TTL-based expiration
  - [ ] Tests with testcontainers

### PostgreSQL Implementation

- [ ] PostgreSQL session store (`auth/src/stores/postgres.rs`)
  - [ ] Schema migration
  - [ ] Index on session_id
  - [ ] Index on user_id
  - [ ] Cleanup job for expired sessions
  - [ ] Tests with testcontainers

### Session Reducer

- [ ] Session management reducer (`auth/src/reducers/session.rs`)
  - [ ] Handle session creation
  - [ ] Handle session refresh
  - [ ] Handle session expiration
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

## Phase 6C: Authorization (Weeks 5-6)

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

## Phase 6D: OAuth2 & OIDC (Weeks 7-8)

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

## Phase 6E: Enterprise Features (Weeks 9-10)

### SAML Support

- [ ] SAML provider (`auth/src/providers/saml/`)
  - [ ] SP metadata generation
  - [ ] Assertion parsing
  - [ ] Signature validation
  - [ ] Encryption support

- [ ] SAML reducer
  - [ ] SP-initiated SSO
  - [ ] IdP-initiated SSO
  - [ ] Single Logout (SLO)

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

- [ ] SAML integration tests
- [ ] Multi-tenant isolation tests
- [ ] Delegation tests

### Documentation

- [ ] Enterprise guide
  - [ ] SAML setup
  - [ ] Multi-tenancy patterns
  - [ ] Delegation use cases

---

## Phase 6F: Production Hardening (Weeks 11-12)

### Security

- [ ] Security audit
  - [ ] Third-party security review
  - [ ] Penetration testing
  - [ ] Fix identified vulnerabilities

- [ ] Rate limiting
  - [ ] Login attempt rate limiting
  - [ ] Token refresh rate limiting
  - [ ] Configurable limits

- [ ] CSRF protection
  - [ ] CSRF token generation
  - [ ] Validation in state-changing endpoints

### Performance

- [ ] Caching
  - [ ] Session caching
  - [ ] Permission caching
  - [ ] Token validation caching

- [ ] Benchmarks
  - [ ] Token validation performance
  - [ ] Session lookup performance
  - [ ] Authorization check performance

### Observability

- [ ] Metrics
  - [ ] Login attempts (success/failure)
  - [ ] Active sessions
  - [ ] Authorization checks
  - [ ] Token refresh rate

- [ ] Tracing
  - [ ] Auth flow traces
  - [ ] OAuth flow traces
  - [ ] Error traces

- [ ] Alerting
  - [ ] Failed login spike
  - [ ] Unusual geographic access
  - [ ] Permission escalation attempts

### Documentation

- [ ] Complete API documentation
- [ ] Architecture diagrams
- [ ] Sequence diagrams
- [ ] Security best practices guide
- [ ] Migration guide
- [ ] Troubleshooting guide

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
- `samael` - SAML support
- `axum` - Web framework
- `tower` - Middleware
- `tower-http` - HTTP middleware (CORS, compression, etc.)
- `redis` - Session storage
- `sqlx` - Database access
- `lettre` - Email sending (for magic links)
- `rand` - Cryptographic random numbers
- `base64` - Encoding support

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

- [ ] Additional MFA options (beyond WebAuthn)
  - TOTP (Time-based OTP) for backup auth
  - SMS verification (with warnings about security)
  - Email-based 2FA

- [ ] Passkey advanced features
  - Cross-device passkey syncing
  - Passkey backup/recovery flows
  - Device attestation verification
  - Conditional UI (autofill)

- [ ] Advanced rate limiting
  - Adaptive rate limiting
  - Distributed rate limiting
  - Per-user limits

- [ ] API key management
  - API key generation
  - Scoped permissions
  - Usage tracking
  - Rotation policies

- [ ] Audit log export
  - SIEM integration (Splunk, DataDog, etc.)
  - Custom formatters
  - Retention policies
  - Compliance reports (GDPR, SOC2, HIPAA)

- [ ] Social auth providers
  - Apple Sign-In
  - Twitter/X auth
  - LinkedIn auth
  - Discord auth

---

## Notes

- Phase 6 focuses on **composable auth primitives**
- Every auth component is a reducer, effect, or saga
- Full integration with event sourcing architecture
- Progressive complexity: simple to enterprise
- Production-ready from day 1
