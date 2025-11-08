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
  - [ ] Set up dependencies (jsonwebtoken, argon2, etc.)
  - [ ] Configure lints and CI

- [ ] Define core types (`auth/src/state.rs`)
  - [ ] `AuthState` - Authentication state
  - [ ] `Session` - User session type
  - [ ] `TokenPair` - Access + refresh tokens
  - [ ] `Credentials` - Username/password
  - [ ] `User` - Authenticated user type

- [ ] Define action types (`auth/src/actions.rs`)
  - [ ] `AuthAction::Login`
  - [ ] `AuthAction::LoginSuccess`
  - [ ] `AuthAction::LoginFailed`
  - [ ] `AuthAction::RefreshToken`
  - [ ] `AuthAction::Logout`
  - [ ] `AuthAction::ValidateToken`

- [ ] Define effect types (`auth/src/effects.rs`)
  - [ ] `AuthEffect::ValidateCredentials`
  - [ ] `AuthEffect::GenerateTokens`
  - [ ] `AuthEffect::RefreshTokens`
  - [ ] `AuthEffect::RevokeSession`
  - [ ] `AuthEffect::StoreSession`

- [ ] Define core traits (`auth/src/providers/mod.rs`)
  - [ ] `AuthProvider` - Generic auth provider trait
  - [ ] `PasswordHasher` - Password hashing trait
  - [ ] `TokenGenerator` - Token generation trait
  - [ ] `SessionStore` - Session storage trait

### JWT Implementation

- [ ] Implement JWT provider (`auth/src/providers/jwt.rs`)
  - [ ] `JwtProvider::encode()` - Create JWT
  - [ ] `JwtProvider::decode()` - Validate JWT
  - [ ] Support RS256, HS256, ES256
  - [ ] Custom claims support

- [ ] Implement JWT reducer (`auth/src/reducers/jwt.rs`)
  - [ ] Handle `Login` action
  - [ ] Handle `RefreshToken` action
  - [ ] Handle `Logout` action
  - [ ] Generate appropriate effects

- [ ] Password hashing (`auth/src/password.rs`)
  - [ ] Argon2id implementation
  - [ ] Secure salt generation
  - [ ] Optional pepper support

### Axum Integration

- [ ] Middleware (`auth/src/middleware/axum.rs`)
  - [ ] `RequireAuth` layer
  - [ ] Token extraction from headers
  - [ ] Error responses (401, 403)

- [ ] Extractors (`auth/src/middleware/extractors.rs`)
  - [ ] `AuthenticatedUser` extractor
  - [ ] `OptionalAuth` extractor
  - [ ] Error handling

### Testing

- [ ] Unit tests
  - [ ] JWT encoding/decoding
  - [ ] Password hashing
  - [ ] Token expiration
  - [ ] Reducer logic

- [ ] Integration tests
  - [ ] Full login flow
  - [ ] Token refresh
  - [ ] Invalid token handling

### Documentation

- [ ] API documentation
  - [ ] Module-level docs
  - [ ] Function-level docs
  - [ ] Examples in docstrings

- [ ] User guide
  - [ ] Quickstart: JWT in 5 minutes
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

- `jsonwebtoken` - JWT encoding/decoding
- `argon2` - Password hashing
- `oauth2` - OAuth2 flows
- `openidconnect` - OIDC support
- `samael` - SAML support
- `axum` - Web framework
- `tower` - Middleware
- `redis` - Session storage
- `sqlx` - Database access

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

- [ ] Multi-factor authentication (MFA)
  - TOTP (Time-based OTP)
  - SMS verification
  - Hardware keys (WebAuthn)

- [ ] Passwordless authentication
  - Magic links
  - Passkeys (WebAuthn)

- [ ] Advanced rate limiting
  - Adaptive rate limiting
  - Distributed rate limiting

- [ ] API key management
  - API key generation
  - Scoped permissions
  - Usage tracking

- [ ] Audit log export
  - SIEM integration
  - Custom formatters
  - Retention policies

---

## Notes

- Phase 6 focuses on **composable auth primitives**
- Every auth component is a reducer, effect, or saga
- Full integration with event sourcing architecture
- Progressive complexity: simple to enterprise
- Production-ready from day 1
