# Phase 6: Composable Authentication & Authorization

**Status**: Planning
**Start Date**: TBD
**Estimated Duration**: 14 weeks

---

## Vision Statement

Authentication and authorization in Composable Rust are not middleware layers or external concerns - they are **integral parts of the architecture**, implemented as reducers, effects, and sagas. This approach provides:

- **Composability**: From simple JWT validation to enterprise SSO
- **Testability**: Auth logic runs at memory speed with deterministic tests
- **Observability**: All auth operations emit metrics and traces
- **Type Safety**: Compile-time guarantees for permissions and roles
- **Event Sourcing**: Complete audit trail of all auth decisions

## Core Principle: Auth IS Architecture

```
Authentication = State + Actions + Effects
Authorization = Reducers + Sagas + Policies
Passwordless = Modern Cryptography + WebAuthn + OAuth2
```

### Passwordless-First Philosophy

**We do NOT store passwords.** Instead, we support:

1. **Passkeys/WebAuthn** (FIDO2) - The future of authentication
2. **Magic Links** - Email/SMS-based passwordless auth
3. **OAuth2/OIDC** - Delegate to trusted providers (Google, GitHub, etc.)
4. **Biometric Auth** - Built into passkeys/WebAuthn

**Passwords are supported ONLY for:**
- Legacy system migration (with clear deprecation path)
- Scenarios where modern auth is technically impossible
- Always with strong hashing (Argon2id) and clear warnings

### Authentication as State Management

```rust
// Authentication is just state
struct AuthState {
    session: Option<Session>,
    tokens: TokenPair,
    refresh_in_progress: bool,
    // For WebAuthn
    challenge: Option<Challenge>,
    // For magic links
    pending_verification: Option<PendingVerification>,
}

// Login is just an action
enum AuthAction {
    // Passkey/WebAuthn flow
    InitiatePasskeyLogin { username: String },
    VerifyPasskey { credential: PublicKeyCredential },
    PasskeyLoginSuccess { session: Session },

    // Magic link flow
    SendMagicLink { email: String },
    VerifyMagicLink { token: String },
    MagicLinkSuccess { session: Session },

    // OAuth2 flow
    InitiateOAuth { provider: OAuthProvider },
    OAuthCallback { code: String },
    OAuthSuccess { session: Session },

    // Common actions
    LoginFailed { error: AuthError },
    RefreshToken,
    Logout,
}

// Token validation is just an effect
enum AuthEffect {
    // WebAuthn effects
    GenerateChallenge,
    VerifyAssertion { credential: PublicKeyCredential },
    StoreCredential { credential: StoredCredential },

    // Magic link effects
    SendEmail { to: String, token: String },
    ValidateMagicToken { token: String },

    // OAuth2 effects
    RedirectToProvider { provider: OAuthProvider },
    ExchangeCodeForToken { code: String },

    // Common effects
    ValidateToken(String),
    RefreshTokens { refresh_token: String },
    RevokeSession { session_id: SessionId },
}

// Comprehensive error taxonomy
enum AuthError {
    // Authentication errors
    InvalidCredentials,
    PasskeyNotFound,
    PasskeyVerificationFailed { reason: String },
    MagicLinkExpired,
    MagicLinkInvalid,
    MagicLinkAlreadyUsed,
    OAuthCodeInvalid,
    OAuthStateInvalid,

    // Authorization errors
    InsufficientPermissions { required: Permission },
    ResourceNotFound,

    // Session errors
    SessionExpired,
    SessionNotFound,
    SessionRevoked,
    InvalidRefreshToken,

    // Rate limiting
    TooManyAttempts { retry_after: Duration },

    // WebAuthn specific
    ChallengeExpired,
    ChallengeNotFound,
    OriginMismatch,
    RpIdMismatch,

    // System errors
    DatabaseError(String),
    EmailDeliveryFailed,
    InternalError,
}
```

### Authorization as Sagas

```rust
// Permission checks are sagas that coordinate multiple sources
struct PermissionSaga;

impl Saga for PermissionSaga {
    // Check user role, resource ownership, org membership, etc.
    fn check_permission(
        user: UserId,
        resource: ResourceId,
        action: Action,
    ) -> Result<Authorized, Denied> {
        // Multi-step check coordinated as a saga
    }
}
```

---

## Design Philosophy

### 1. Progressive Complexity

Users should be able to start simple and scale up:

**Level 1: OAuth2 Delegation (Day 1)**
```rust
// Start with OAuth2 - delegate auth to Google/GitHub
let auth = OAuthAuth::new(vec![
    GoogleProvider::new(client_id, client_secret),
    GitHubProvider::new(client_id, client_secret),
]);
```

**Level 2: Magic Links (Week 1)**
```rust
// Add passwordless email-based auth
let auth = AuthStack::new()
    .oauth(oauth_providers)
    .magic_link(email_service);
```

**Level 3: Passkeys/WebAuthn (Week 2)**
```rust
// Add FIDO2 passkeys for maximum security
let auth = AuthStack::new()
    .oauth(oauth_providers)
    .magic_link(email_service)
    .passkeys(webauthn_config);
```

**Level 4: RBAC + Authorization (Month 1)**
```rust
// Add role-based access control
let auth = AuthStack::new()
    .passkeys(webauthn_config)
    .oauth(oauth_providers)
    .authorization(rbac_authorizer);
```

**Level 5: Enterprise SSO (Production)**
```rust
// Full OIDC + SAML with multi-tenancy
let auth = EnterpriseAuth::new()
    .oidc(oidc_providers)
    .saml(saml_config)
    .passkeys(webauthn_config)
    .multi_tenant(tenant_config);
```

### 2. Composable Building Blocks

All components should compose:

```rust
// Compose multiple auth strategies
let auth = AuthStack::new()
    .passkeys(webauthn_config)
    .magic_link(email_service)
    .oauth(oauth_providers)
    .api_key(api_key_store)  // For service-to-service auth
    .fallback(AnonymousAuth);
```

### 3. Event-Driven Audit Trail

Every auth decision is an event:

```rust
enum AuthEvent {
    UserLoggedIn { user_id: UserId, ip: IpAddr, timestamp: DateTime },
    TokenRefreshed { session_id: SessionId, timestamp: DateTime },
    PermissionGranted { user_id: UserId, resource: ResourceId, action: Action },
    PermissionDenied { user_id: UserId, resource: ResourceId, reason: String },
    SessionExpired { session_id: SessionId },
    SuspiciousActivity { user_id: UserId, reason: String },
}
```

All events are stored in the event store for compliance and forensics.

---

## Architecture Components

### Core Crate: `composable-rust-auth`

```
composable-rust-auth/
├── src/
│   ├── lib.rs                  # Public API
│   ├── state.rs                # AuthState, Session types
│   ├── actions.rs              # AuthAction enum
│   ├── effects.rs              # AuthEffect enum
│   ├── reducers/
│   │   ├── mod.rs
│   │   ├── jwt.rs              # JWT authentication reducer
│   │   ├── session.rs          # Session management reducer
│   │   ├── oauth.rs            # OAuth2 flow reducer
│   │   └── saml.rs             # SAML authentication reducer
│   ├── authorization/
│   │   ├── mod.rs
│   │   ├── rbac.rs             # Role-based access control
│   │   ├── abac.rs             # Attribute-based access control
│   │   ├── policies.rs         # Policy engine
│   │   └── permissions.rs      # Permission definitions
│   ├── providers/
│   │   ├── mod.rs
│   │   ├── jwt.rs              # JWT provider trait
│   │   ├── oidc.rs             # OpenID Connect provider
│   │   ├── saml.rs             # SAML provider
│   │   └── ldap.rs             # LDAP/Active Directory
│   ├── middleware/
│   │   ├── mod.rs
│   │   ├── axum.rs             # Axum middleware integration
│   │   └── extractors.rs       # Request extractors
│   └── stores/
│       ├── mod.rs
│       ├── session_store.rs    # Session storage trait
│       ├── redis.rs            # Redis session store
│       └── postgres.rs         # PostgreSQL session store
```

---

## Key Abstractions

### 1. Authentication Reducer

```rust
/// Core authentication reducer
pub struct AuthReducer<P: AuthProvider> {
    provider: P,
}

impl<P: AuthProvider> Reducer for AuthReducer<P> {
    type State = AuthState;
    type Action = AuthAction;
    type Environment = AuthEnvironment;

    fn reduce(
        &self,
        state: &mut AuthState,
        action: AuthAction,
        env: &AuthEnvironment,
    ) -> Vec<Effect<AuthAction>> {
        match action {
            AuthAction::Login { credentials } => {
                // Effect: Validate credentials with auth provider
                vec![Effect::Future(Box::pin(async move {
                    match self.provider.authenticate(credentials).await {
                        Ok(session) => Some(AuthAction::LoginSuccess {
                            session,
                            tokens: session.tokens.clone(),
                        }),
                        Err(e) => Some(AuthAction::LoginFailed { error: e }),
                    }
                }))]
            }

            AuthAction::LoginSuccess { session, tokens } => {
                state.session = Some(session.clone());
                state.tokens = tokens.clone();

                // Effects: Store session, emit event
                vec![
                    Effect::StoreSession(session),
                    Effect::PublishEvent(AuthEvent::UserLoggedIn {
                        user_id: session.user_id,
                        ip: session.ip_address,
                        timestamp: env.clock.now(),
                    }),
                ]
            }

            AuthAction::RefreshToken => {
                state.refresh_in_progress = true;

                vec![Effect::Future(Box::pin(async move {
                    // Call token refresh endpoint
                    self.provider.refresh_token(state.tokens.refresh_token).await
                        .map(|new_tokens| AuthAction::TokensRefreshed { tokens: new_tokens })
                        .ok()
                }))]
            }

            // ... more cases
        }
    }
}
```

### 2. Authorization Trait

```rust
/// Core authorization trait - implemented as pure functions
pub trait Authorizer: Send + Sync {
    /// Check if a user is authorized for an action on a resource
    fn authorize(
        &self,
        user: &User,
        resource: &Resource,
        action: &Action,
    ) -> AuthDecision;
}

/// Authorization decision with reason
pub enum AuthDecision {
    Allow,
    Deny { reason: String },
}

/// RBAC authorizer
pub struct RbacAuthorizer {
    role_store: Arc<dyn RoleStore>,
}

impl Authorizer for RbacAuthorizer {
    fn authorize(
        &self,
        user: &User,
        resource: &Resource,
        action: &Action,
    ) -> AuthDecision {
        let roles = self.role_store.get_user_roles(user.id);
        let required_permissions = resource.required_permissions(action);

        if roles.has_any_permission(required_permissions) {
            AuthDecision::Allow
        } else {
            AuthDecision::Deny {
                reason: format!(
                    "User {} lacks permission for {} on {}",
                    user.id, action, resource.id
                ),
            }
        }
    }
}
```

### 3. Axum Integration

```rust
/// Axum middleware for authentication
pub struct RequireAuth<A: Authorizer> {
    auth_store: Arc<Store<AuthState, AuthAction, AuthEnvironment, AuthReducer>>,
    authorizer: Arc<A>,
}

/// Extract authenticated user from request
#[async_trait]
impl<S, A> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
    A: Authorizer,
{
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        // Extract token from Authorization header
        let token = extract_bearer_token(&parts.headers)?;

        // Validate token through auth store
        let session = validate_token(token).await?;

        Ok(AuthenticatedUser {
            user_id: session.user_id,
            roles: session.roles,
        })
    }
}

/// Axum route example
async fn protected_route(
    user: AuthenticatedUser,
    Path(resource_id): Path<ResourceId>,
) -> Result<Json<Resource>, StatusCode> {
    // User is already authenticated
    // Can check authorization here
    Ok(Json(get_resource(resource_id)))
}
```

### 4. Policy Engine (for complex authorization)

```rust
/// Policy-based authorization
pub struct PolicyEngine {
    policies: Vec<Box<dyn Policy>>,
}

pub trait Policy: Send + Sync {
    fn evaluate(&self, context: &AuthContext) -> AuthDecision;
}

/// Example: Time-based access policy
pub struct BusinessHoursPolicy;

impl Policy for BusinessHoursPolicy {
    fn evaluate(&self, context: &AuthContext) -> AuthDecision {
        let hour = context.timestamp.hour();
        if hour >= 9 && hour < 17 {
            AuthDecision::Allow
        } else {
            AuthDecision::Deny {
                reason: "Access only allowed during business hours (9am-5pm)".to_string(),
            }
        }
    }
}

/// Example: Resource ownership policy
pub struct OwnershipPolicy;

impl Policy for OwnershipPolicy {
    fn evaluate(&self, context: &AuthContext) -> AuthDecision {
        if context.resource.owner_id == context.user.id {
            AuthDecision::Allow
        } else {
            AuthDecision::Deny {
                reason: "Only resource owner can perform this action".to_string(),
            }
        }
    }
}
```

---

## Progressive Implementation Levels

### Level 1: JWT Authentication (Week 1)

**Goal**: Simple stateless JWT validation

```rust
// User starts here
let auth = JwtAuth::new("secret_key");

// Axum integration
let app = Router::new()
    .route("/protected", get(handler))
    .layer(RequireAuth::new(auth));
```

**Features**:
- JWT encoding/decoding
- Token expiration checks
- Axum middleware
- Basic user extraction

**NOT included**:
- Session persistence
- Token refresh
- Role management

### Level 2: Session Management (Week 2)

**Goal**: Stateful sessions with refresh tokens

```rust
let session_store = RedisSessionStore::new("redis://localhost");
let auth = SessionAuth::new("secret_key", session_store);
```

**Features**:
- Session storage (Redis, PostgreSQL)
- Refresh token flow
- Session expiration
- Multi-device support

### Level 3: RBAC (Week 3-4)

**Goal**: Role-based access control

```rust
let role_store = PostgresRoleStore::new(pool);
let auth = RbacAuth::new("secret_key", session_store, role_store);

// Check permissions in handlers
async fn delete_order(
    user: AuthenticatedUser,
    authorizer: Extension<Arc<RbacAuthorizer>>,
) -> Result<(), StatusCode> {
    authorizer.require_permission(&user, Permission::DeleteOrders)?;
    // ... delete order
}
```

**Features**:
- Role definitions
- Permission sets
- Role assignment
- Permission checks

### Level 4: OAuth2/OIDC (Week 5-6)

**Goal**: External identity providers

```rust
let oidc = OidcProvider::new(OidcConfig {
    issuer: "https://accounts.google.com",
    client_id: env::var("GOOGLE_CLIENT_ID")?,
    client_secret: env::var("GOOGLE_CLIENT_SECRET")?,
    redirect_uri: "http://localhost:8080/auth/callback",
});

let auth = OAuthAuth::new(oidc);
```

**Features**:
- OAuth2 authorization code flow
- OpenID Connect support
- Token introspection
- UserInfo endpoint integration

### Level 5: Enterprise SSO (Week 7-8)

**Goal**: SAML, multi-tenancy, delegation

```rust
let saml = SamlProvider::new(SamlConfig {
    idp_metadata_url: "https://sso.company.com/metadata",
    sp_entity_id: "myapp",
});

let auth = EnterpriseAuth::new()
    .oidc(oidc_provider)
    .saml(saml_provider)
    .delegation(DelegationConfig {
        allow_impersonation: true,
        require_approval: true,
    });
```

**Features**:
- SAML 2.0 support
- Multi-tenant isolation
- User impersonation/delegation
- Just-in-time provisioning

---

## Integration with Existing Architecture

### Event Sourcing Integration

All auth decisions are events:

```rust
// Auth events are stored in the event store
let auth_event_store = PostgresEventStore::new(database_url).await?;

// Every login creates an event
let event = SerializedEvent {
    event_type: "UserLoggedIn".to_string(),
    data: serde_json::to_vec(&UserLoggedInEvent {
        user_id,
        timestamp,
        ip_address,
        user_agent,
    })?,
    metadata: Some(json!({ "correlation_id": request_id })),
};

auth_event_store.append_events(
    StreamId::new(format!("user-{}", user_id)),
    None,
    vec![event],
).await?;
```

### Projection Integration

Auth state can be projected:

```rust
// Projection: Active sessions per user
struct ActiveSessionsProjection {
    sessions: HashMap<UserId, Vec<Session>>,
}

impl Projection for ActiveSessionsProjection {
    fn apply(&mut self, event: &AuthEvent) {
        match event {
            AuthEvent::UserLoggedIn { user_id, session, .. } => {
                self.sessions.entry(*user_id).or_default().push(session.clone());
            }
            AuthEvent::SessionExpired { session_id, .. } => {
                // Remove expired session
            }
            _ => {}
        }
    }
}
```

### Saga Integration

Complex auth flows as sagas:

```rust
// OAuth2 flow as a saga
struct OAuth2Saga;

impl Saga for OAuth2Saga {
    type State = OAuth2State;
    type Action = OAuth2Action;

    fn reduce(
        &self,
        state: &mut OAuth2State,
        action: OAuth2Action,
    ) -> Vec<Effect<OAuth2Action>> {
        match (state.step, action) {
            // Step 1: Redirect to provider
            (Step::Initial, OAuth2Action::InitiateLogin) => {
                vec![Effect::RedirectToProvider]
            }

            // Step 2: Handle callback
            (Step::AwaitingCallback, OAuth2Action::CallbackReceived { code }) => {
                vec![Effect::ExchangeCodeForToken { code }]
            }

            // Step 3: Fetch user info
            (Step::TokenReceived, OAuth2Action::TokenExchangeSuccess { access_token }) => {
                vec![Effect::FetchUserInfo { access_token }]
            }

            // Step 4: Create session
            (Step::UserInfoReceived, OAuth2Action::UserInfoSuccess { user }) => {
                vec![Effect::CreateSession { user }]
            }

            _ => vec![],
        }
    }
}
```

---

## Standards Support

### JWT (JSON Web Tokens)

- **Encoding**: RS256, HS256, ES256
- **Claims**: Standard claims (iss, sub, aud, exp, iat)
- **Custom Claims**: Support for application-specific claims
- **Validation**: Signature, expiration, issuer verification

### OAuth 2.0

- **Flows**: Authorization Code, Client Credentials, Device Code
- **Token Types**: Access tokens, refresh tokens
- **Endpoints**: Authorization, token, token introspection
- **PKCE**: Proof Key for Code Exchange support

### OpenID Connect

- **Discovery**: `.well-known/openid-configuration`
- **UserInfo**: User profile endpoint
- **ID Tokens**: JWT-based identity tokens
- **Scopes**: openid, profile, email

### SAML 2.0

- **SSO**: Service Provider initiated, IdP initiated
- **SLO**: Single Logout
- **Assertions**: Encrypted assertions support
- **Metadata**: SP and IdP metadata exchange

---

## Testing Strategy

### Unit Tests (Memory Speed)

```rust
#[tokio::test]
async fn test_login_success() {
    let clock = FixedClock::new(test_time());
    let provider = MockAuthProvider::new();

    let env = AuthEnvironment {
        clock: Arc::new(clock),
        provider: Arc::new(provider),
    };

    let mut state = AuthState::default();
    let reducer = AuthReducer::new();

    let effects = reducer.reduce(
        &mut state,
        AuthAction::Login {
            credentials: Credentials {
                username: "user".to_string(),
                password: "pass".to_string(),
            },
        },
        &env,
    );

    // Assert effects generated
    assert!(matches!(effects[0], Effect::ValidateCredentials(_)));
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_full_oauth_flow() {
    // Start mock OAuth provider
    let mock_provider = MockOAuthProvider::start().await;

    // Test authorization code flow
    let auth_url = oauth_client.authorization_url();
    let code = mock_provider.authorize(auth_url).await;
    let tokens = oauth_client.exchange_code(code).await.unwrap();

    assert!(tokens.access_token.len() > 0);
}
```

### Property-Based Tests

```rust
proptest! {
    #[test]
    fn test_jwt_roundtrip(user_id in any::<UserId>()) {
        let jwt = encode_jwt(user_id, secret);
        let decoded = decode_jwt(jwt, secret).unwrap();
        prop_assert_eq!(decoded.user_id, user_id);
    }
}
```

---

## Security Considerations

### 1. Token Security

- **Secure Storage**: Never log tokens, use secure cookies
- **Short Lifetimes**: Access tokens expire in 15 minutes
- **Refresh Rotation**: Refresh tokens rotate on use
- **Revocation**: Support for token revocation lists

### 2. Session Security

- **CSRF Protection**: CSRF tokens for state-changing operations
- **Session Fixation**: Regenerate session ID on privilege escalation
- **Concurrent Sessions**: Configurable max sessions per user
- **IP Binding**: Optional IP address validation

### 3. Passwordless Security

- **WebAuthn/FIDO2**: Hardware-backed cryptographic authentication
- **Public Key Crypto**: Private keys never leave the user's device
- **Phishing Resistant**: Origin binding prevents phishing attacks (explicit origin and rpId validation)
- **Biometric Support**: Built-in fingerprint/FaceID support
- **Magic Link Security**:
  - Time-limited (5-15 minutes), single-use tokens
  - Constant-time token comparison (prevents timing attacks)
  - No user enumeration (same response whether user exists or not)
  - Cryptographically secure random tokens (256 bits entropy)
- **Rate Limiting**: tower-governor middleware
  - Magic link requests: 5 per hour per email
  - Login attempts: 10 per minute per IP
  - Any auth attempt: 100 per minute per IP
- **Challenge Storage**: Redis-backed with TTL (~5 minutes)
- **Recovery Flows**:
  - Recovery codes (10 single-use codes, bcrypt hashed)
  - Backup email verification
  - Clear revocation paths for lost devices

### 4. Account Linking Strategy

When the same email appears from multiple auth providers (e.g., Google and GitHub):

- **Default: Ask User** (most secure)
  - "We found an existing account with this email from [Provider]. Link accounts?"
  - Requires re-authentication of existing account before linking
  - User can decline and create separate account
- **Alternative: Auto-Link** (convenience over security)
  - Automatically link if email is verified by both providers
  - Emit audit event for security monitoring
  - Can be disabled per-tenant for stricter security
- **Email Ownership Verification**:
  - Primary email must be verified before linking
  - Send confirmation to both old and new provider emails

### 5. Session Management Architecture

**Critical Design Decision**: Explicit separation of ephemeral and persistent data.

#### Persistent Data (PostgreSQL)

User and device data that outlives sessions:

```rust
// Stored in PostgreSQL: Permanent user account
struct User {
    id: UserId,
    email: String,
    email_verified: bool,
    created_at: DateTime,
    updated_at: DateTime,
}

// Stored in PostgreSQL: Permanent device registry
struct RegisteredDevice {
    device_id: DeviceId,
    user_id: UserId,

    // Device metadata
    name: String,              // "iPhone 15 Pro", "Chrome on MacBook"
    device_type: DeviceType,   // Mobile, Desktop, Tablet
    platform: String,          // "iOS 17.2", "macOS 14.1"

    // Timestamps
    first_seen: DateTime,
    last_seen: DateTime,

    // Security
    trusted: bool,             // User-marked trusted device
    requires_mfa: bool,        // Require additional verification

    // WebAuthn
    passkey_credential_id: Option<String>, // If device has registered passkey
    public_key: Option<Vec<u8>>,           // WebAuthn public key
}

// Stored in PostgreSQL: OAuth provider linkage
struct OAuthLink {
    user_id: UserId,
    provider: OAuthProvider,
    provider_user_id: String,
    email: String,
    linked_at: DateTime,
}
```

**Use cases for persistent device data:**
- User views "All devices with access to your account"
- User revokes device permanently (not just current session)
- Passkey registration is permanent
- Security audit: "Which devices accessed my account in last 30 days?"
- Trust levels: "Always trust this device"

#### Ephemeral Data (Redis)

Session data that expires with the session:

```rust
// Stored in Redis: Temporary session (TTL = 24 hours)
struct Session {
    session_id: SessionId,     // Primary key
    user_id: UserId,           // FK to PostgreSQL User
    device_id: DeviceId,       // FK to PostgreSQL RegisteredDevice

    // Cached user data (avoid PostgreSQL lookup on every request)
    email: String,
    roles: Vec<Role>,
    permissions: Vec<Permission>,

    // Session metadata
    created_at: DateTime,
    last_active: DateTime,
    expires_at: DateTime,
    ip_address: IpAddr,
    user_agent: String,

    // OAuth tokens (if authenticated via OAuth)
    oauth_provider: Option<OAuthProvider>,
    oauth_access_token: Option<String>,  // Encrypted
    oauth_refresh_token: Option<String>, // Encrypted
}

// Redis key structure:
// session:{session_id} -> Session (TTL = 24 hours)
// user:{user_id}:sessions -> Set<SessionId> (for multi-device tracking)
// challenge:{challenge_id} -> Challenge (TTL = 5 minutes, WebAuthn)
```

**Use cases for ephemeral session data:**
- Fast authentication on every request (<5ms Redis lookup)
- Instant logout (delete session from Redis)
- Instant revocation (delete all user sessions)
- Cached permissions (avoid database lookup)
- OAuth token storage (encrypted, expires with session)

#### Session Lifecycle

```rust
// 1. Login: Create both device record (if new) and session
async fn create_session(user_id: UserId, device_info: DeviceInfo) -> Session {
    // Check if device is already registered
    let device = match postgres.get_device(device_info.id).await? {
        Some(existing) => {
            // Update last_seen timestamp
            postgres.update_device_last_seen(existing.id, now()).await?;
            existing
        }
        None => {
            // Register new device (permanent record)
            postgres.create_device(RegisteredDevice {
                device_id: device_info.id,
                user_id,
                name: device_info.name,
                device_type: device_info.device_type,
                first_seen: now(),
                last_seen: now(),
                trusted: false, // User can mark as trusted later
                requires_mfa: true,
                passkey_credential_id: None,
            }).await?
        }
    };

    // Create ephemeral session in Redis (24 hour TTL)
    let session_id = SessionId::generate_secure();
    let session = Session {
        session_id,
        user_id,
        device_id: device.device_id,
        roles: load_user_roles(user_id).await?,
        created_at: now(),
        expires_at: now() + Duration::hours(24),
        // ...
    };

    redis.setex(
        format!("session:{}", session_id),
        86400, // 24 hours
        session.serialize()
    ).await?;

    // Track active sessions per user (for multi-device)
    redis.sadd(
        format!("user:{}:sessions", user_id),
        session_id
    ).await?;

    session
}

// 2. Per-Request Authentication
async fn authenticate_request(session_id: SessionId) -> Result<User> {
    // Fast Redis lookup (<5ms)
    let session = redis.get(format!("session:{}", session_id)).await?
        .ok_or(AuthError::SessionExpired)?;

    // Update last active (sliding expiration)
    redis.expire(format!("session:{}", session_id), 86400).await?;

    // Also update device last_seen in PostgreSQL (async, non-blocking)
    tokio::spawn(async move {
        postgres.update_device_last_seen(session.device_id, now()).await
    });

    Ok(User {
        id: session.user_id,
        roles: session.roles,
        permissions: session.permissions,
    })
}

// 3. Logout: Delete session, device remains registered
async fn logout(session_id: SessionId) {
    let session = redis.get(format!("session:{}", session_id)).await?;

    // Remove session from Redis
    redis.del(format!("session:{}", session_id)).await?;
    redis.srem(
        format!("user:{}:sessions", session.user_id),
        session_id
    ).await?;

    // Device remains in PostgreSQL for permanent record
    // User can still see it in "My Devices"
}

// 4. Revoke Device: Delete all sessions + device record
async fn revoke_device(user_id: UserId, device_id: DeviceId) {
    // Delete all active sessions for this device
    let all_sessions = redis.smembers(format!("user:{}:sessions", user_id)).await?;
    for session_id in all_sessions {
        let session = redis.get(format!("session:{}", session_id)).await?;
        if session.device_id == device_id {
            redis.del(format!("session:{}", session_id)).await?;
            redis.srem(format!("user:{}:sessions", user_id), session_id).await?;
        }
    }

    // Delete device from PostgreSQL (permanent revocation)
    postgres.delete_device(device_id).await?;

    // Emit security audit event
    emit_event(AuthEvent::DeviceRevoked { user_id, device_id });
}
```

#### Client-Side Storage

**What goes to the client (HTTP-only cookie):**
```rust
// ONLY the session ID - nothing sensitive
Set-Cookie: session_id=<cryptographically_random_256_bit_value>;
    HttpOnly;           // No JavaScript access
    Secure;             // HTTPS only
    SameSite=Strict;    // CSRF protection
    Max-Age=86400       // 24 hours
```

**Why minimal client storage:**
- ✅ Can't tamper with roles/permissions (stored server-side)
- ✅ Can't extend session beyond expiration
- ✅ Revocation works instantly (just delete from Redis)
- ✅ Can update permissions without new cookie
- ✅ XSS can't steal sensitive data (HTTP-only)

#### Performance Targets

- Session lookup (Redis): **<5ms**
- Device lookup (PostgreSQL): **<10ms** (only during login/device management)
- Per-request auth: **<5ms** (Redis only, PostgreSQL cached)
- Session creation: **<20ms** (Redis + PostgreSQL)
- Optional LRU cache for hot sessions: **<100μs**

#### Store Pattern Integration

- Session middleware extracts session_id from cookie
- Loads `Session` from Redis (<5ms)
- Reconstructs `AuthState` with cached roles/permissions
- Store operates on reconstructed state
- Session updates persisted back to Redis
- Device updates persisted to PostgreSQL (async, non-blocking)

### 6. Audit Logging

- **Comprehensive**: All auth decisions logged
- **Immutable**: Events stored in event store
- **Retention**: Configurable retention policies
- **Compliance**: GDPR, SOC2, HIPAA support

---

## Metrics & Observability

### Authentication Metrics

```rust
// Prometheus metrics with per-method granularity
metrics::counter!("auth_login_attempts_total",
    "method" => "passkey",
    "result" => "success"
).increment(1);
metrics::counter!("auth_login_attempts_total",
    "method" => "magic_link",
    "result" => "failure"
).increment(1);
metrics::counter!("auth_login_attempts_total",
    "method" => "oauth",
    "provider" => "google",
    "result" => "success"
).increment(1);

metrics::histogram!("auth_login_duration_seconds",
    "method" => "passkey"
).record(duration.as_secs_f64());

metrics::gauge!("auth_active_sessions").set(session_count as f64);

// WebAuthn-specific metrics
metrics::histogram!("auth_passkey_verification_duration_seconds").record(duration.as_secs_f64());
metrics::counter!("auth_challenge_generated_total").increment(1);
metrics::counter!("auth_challenge_expired_total").increment(1);

// Magic link metrics
metrics::counter!("auth_magic_link_sent_total").increment(1);
metrics::counter!("auth_magic_link_expired_total").increment(1);

// OAuth metrics
metrics::histogram!("auth_oauth_flow_duration_seconds",
    "provider" => "google"
).record(duration.as_secs_f64());
```

### Authorization Metrics

```rust
metrics::counter!("auth_authorization_checks_total",
    "result" => "allow",
    "resource_type" => "order"
).increment(1);
metrics::counter!("auth_authorization_checks_total",
    "result" => "deny",
    "reason" => "insufficient_permissions"
).increment(1);
metrics::histogram!("auth_authorization_duration_seconds").record(duration.as_secs_f64());
```

### Distributed Tracing

```rust
use tracing::{instrument, Span};

#[instrument(skip(self), fields(
    auth.method = ?method,
    auth.user_id = tracing::field::Empty,
))]
async fn authenticate(&self, method: AuthMethod) -> Result<Session> {
    let span = Span::current();

    // OAuth flow spans multiple services
    match method {
        AuthMethod::OAuth { provider } => {
            // Trace ID propagated across:
            // 1. Authorization redirect
            // 2. Provider callback
            // 3. Token exchange
            // 4. UserInfo fetch
            // 5. Session creation

            span.record("auth.provider", &provider.name());
            span.record("auth.flow_step", "initiate");

            // Each step creates child span
            let auth_url = self.generate_auth_url().await?;

            span.record("auth.flow_step", "callback");
            // ...
        }
        _ => {}
    }

    Ok(session)
}

### Alerting

- **Failed Login Attempts**: Alert on spike in failed logins
- **Session Anomalies**: Unusual geographic locations
- **Permission Escalation**: Attempts to access unauthorized resources
- **Token Expiration**: Alert on high token refresh failures

---

## Migration Strategy

### Existing Systems

```rust
// Gradual migration from existing auth
let auth = HybridAuth::new()
    .primary(NewAuthSystem::new())
    .fallback(LegacyAuthSystem::new())
    .migration_deadline(DateTime::from_str("2025-12-31")?);
```

### Data Migration

```rust
// Migrate existing users
async fn migrate_users(
    legacy_db: &LegacyDatabase,
    new_auth: &AuthStore,
) -> Result<(), MigrationError> {
    let users = legacy_db.get_all_users().await?;

    for user in users {
        // Create new auth records
        new_auth.create_user(User {
            id: user.id,
            email: user.email,
            // Password hashes can be migrated directly if using same algorithm
            password_hash: user.password_hash,
        }).await?;
    }

    Ok(())
}
```

---

## Documentation & Examples

### User Guide

- **Quickstart**: JWT auth in 5 minutes
- **Cookbook**: Common patterns (API keys, OAuth, RBAC)
- **Best Practices**: Security guidelines
- **Troubleshooting**: Common issues and solutions

### API Documentation

- Full rustdoc coverage
- Examples for every public API
- Architecture diagrams
- Sequence diagrams for flows

### Example Applications

1. **Simple API**: JWT authentication only
2. **Multi-tenant SaaS**: RBAC with organization isolation
3. **Consumer App**: OAuth2 with Google/GitHub
4. **Enterprise**: SAML SSO with LDAP integration

---

## Success Metrics

### Adoption

- **Week 1**: Simple JWT auth works out of the box
- **Month 1**: RBAC example deployed in production
- **Quarter 1**: Enterprise SSO case study published

### Performance

- **Auth overhead**: <1ms for cached token validation
- **Session lookup**: <5ms for Redis session store
- **Full OAuth flow**: <500ms end-to-end

### Quality

- **Test coverage**: >95% for auth core
- **Security audits**: Annual third-party audit
- **CVE response**: Patch within 24 hours

---

## Timeline

### Phase 6A: Foundation (Weeks 1-2)

- Core traits and types
- OAuth2 authentication (simplest to start)
- Magic link implementation
- Axum middleware
- Basic tests

### Phase 6B: WebAuthn & Session Management (Weeks 3-5)

- WebAuthn/Passkeys implementation (given proper time)
- Challenge storage (Redis with TTL)
- Session stores (Redis, PostgreSQL)
- Token refresh flow
- Multi-device support
- Recovery flows (recovery codes, backup email)

### Phase 6C: Authorization (Weeks 6-8)

- RBAC implementation
- Policy engine
- Permission checks
- Account linking strategy implementation

### Phase 6D: OAuth/OIDC Deep Dive (Weeks 9-10)

- Advanced OAuth2 flows (device code, PKCE)
- OIDC integration
- Provider implementations (Google, GitHub, Microsoft)
- UserInfo endpoint integration

### Phase 6E: Multi-Tenancy & Delegation (Week 11)

- Multi-tenancy isolation
- Delegation/impersonation
- Tenant resolver strategies

### Phase 6F: Production Hardening (Weeks 12-14)

- Security audit (third-party review)
- Security testing suite (timing attacks, replay attacks)
- Rate limiting implementation (tower-governor)
- Performance optimization
- Documentation (threat model, security testing plan, migration guide, accessibility guide, incident response plan)
- Examples

**Note**: SAML support deferred to Phase 7 to focus on modern passwordless authentication first.

---

## Open Questions

### Resolved

1. ✅ **WebAuthn Implementation**: Use `webauthn-rs` crate (battle-tested, well-maintained)
2. ✅ **Rate Limiting**: tower-governor middleware (separate, composable)
3. ✅ **Backup Auth**: Recovery codes + backup email (specified in Phase 6B)
4. ✅ **Challenge Storage**: Redis with TTL (specified in security section)
5. ✅ **Account Linking**: Default to "ask user" strategy (security-first)
6. ✅ **Session State**: Per-request reconstruction from session token (specified in security section)

### Still Open

1. **Passkey Sync**: Support for passkey syncing across devices (Apple/Google/1Password ecosystem)?
   - May require provider-specific integration
   - Research needed for cross-platform compatibility
2. **MFA Beyond Passkeys**: Include TOTP/SMS 2FA in Phase 6 or defer to Phase 7?
   - Passkeys ARE MFA (something you have + biometric)
   - TOTP might be redundant
   - Decision: Defer TOTP to Phase 7 unless user requests
3. **Audit Export**: Support for exporting auth logs to external SIEM systems?
   - Could leverage existing event store exports
   - Decision: Phase 7 (nice-to-have, not critical)
4. **Magic Link Delivery**: Support SMS in addition to email?
   - SMS has security concerns (SIM swapping)
   - Decision: Email-only in Phase 6, SMS in Phase 7 with warnings
5. **API Keys**: Include API key management or separate module?
   - Important for service-to-service auth
   - Decision: Phase 7 (separate concern from user auth)
6. **WebAuthn Testing**: Virtual authenticator strategy?
   - Chrome DevTools Protocol supports virtual authenticators
   - Decision: Use fantoccini for browser automation tests

---

## Related Work

- **Swift TCA Auth**: No equivalent - Composable Architecture is UI-focused
- **Rust Auth Crates**:
  - `oauth2`: Good OAuth2 library, but not composable architecture
  - `jsonwebtoken`: JWT only, no session management
  - `tower-http::auth`: Middleware only, not full auth system

**Unique Value**: First auth system fully integrated with reducer/effect architecture

---

## Conclusion

Phase 6 will establish Composable Rust as a **complete application framework** by providing:

1. ✅ **Auth as Architecture**: Not middleware, but core domain logic
2. ✅ **Progressive Complexity**: Start simple, scale to enterprise
3. ✅ **Full Standards Support**: JWT, OAuth2, OIDC, SAML
4. ✅ **Production-Ready**: Security, observability, testing
5. ✅ **Composable**: Mix and match auth strategies

This positions Composable Rust for **enterprise adoption** while maintaining the simplicity that makes it accessible to individual developers.
