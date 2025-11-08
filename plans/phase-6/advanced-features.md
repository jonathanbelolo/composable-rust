# Phase 6 Advanced Features

**Status**: Planned for Phase 6 Implementation
**Priority**: High - Production-ready patterns

These features represent mature, proven authentication patterns that significantly enhance security and scalability without introducing experimental risk.

---

## 1. Risk-Based Adaptive Authentication

**Status**: Implement in Phase 6C (Weeks 6-8)
**Maturity**: Proven - Used by Google, Auth0, Okta, AWS
**Risk**: Low - Well-understood technology

### Problem

Not all login attempts are equal. Login from home on trusted device vs. login from new country on unknown device should require different levels of verification.

Traditional authentication is binary: logged in or logged out. Modern authentication should be contextual and adaptive.

### Solution

Risk scoring engine that dynamically adjusts authentication requirements based on context.

#### Data Model

```rust
// Add to RegisteredDevice (PostgreSQL)
struct RegisteredDevice {
    // ... existing fields

    // Risk profiling
    risk_score: f32,              // 0.0 (trusted) to 1.0 (high risk)
    usual_ip_ranges: Vec<IpRange>, // Learn user's normal locations
    usual_login_hours: Vec<Hour>,  // Learn user's patterns
    login_count: u32,              // Total logins from this device
    failed_attempts: u32,          // Failed login attempts
    last_failed_attempt: Option<DateTime>,
}

// Add to Session (Redis)
struct Session {
    // ... existing fields

    // Risk assessment
    login_risk_score: f32,         // Risk at time of login
    current_risk_level: RiskLevel, // Current risk (can change mid-session)
    requires_step_up: bool,        // Need additional verification
}

enum RiskLevel {
    Low,       // Normal login, no extra verification
    Medium,    // Require email verification
    High,      // Require passkey + email
    Critical,  // Block + alert user
}
```

#### Risk Calculator

```rust
struct RiskCalculator {
    geo_db: GeoIpDatabase,
    breach_db: BreachDatabase, // HaveIBeenPwned API
}

struct LoginContext {
    user_id: UserId,
    device_id: DeviceId,
    ip_address: IpAddr,
    timestamp: DateTime,
    device_fingerprint: DeviceFingerprint,
}

struct RiskAssessment {
    score: f32,              // 0.0 to 1.0
    level: RiskLevel,
    factors: Vec<RiskFactor>, // Why this score?
    required_challenges: Vec<ChallengeType>,
}

impl RiskCalculator {
    async fn calculate_login_risk(
        &self,
        context: &LoginContext,
    ) -> Result<RiskAssessment> {
        let mut risk = 0.0;
        let mut factors = Vec::new();

        // Factor 1: New device
        let device = postgres.get_device(context.device_id).await?;
        if device.is_none() {
            risk += 0.3;
            factors.push(RiskFactor::NewDevice);
        }

        // Factor 2: New location
        let location = self.geo_db.lookup(context.ip_address)?;
        let user_locations = postgres.get_user_locations(context.user_id).await?;
        if !user_locations.contains(&location.country) {
            risk += 0.4;
            factors.push(RiskFactor::NewCountry {
                country: location.country,
            });
        }

        // Factor 3: Impossible travel
        if let Some(last_login) = postgres.get_last_login(context.user_id).await? {
            let time_diff = context.timestamp - last_login.timestamp;
            let distance = geo_distance(
                last_login.location,
                location
            );
            let max_speed_kmh = distance / time_diff.as_hours();

            // Impossible to travel 10,000 km in 1 hour
            if max_speed_kmh > 900.0 {
                risk += 0.8;
                factors.push(RiskFactor::ImpossibleTravel {
                    distance_km: distance,
                    time_hours: time_diff.as_hours(),
                });
            }
        }

        // Factor 4: Unusual time
        let hour = context.timestamp.hour();
        let device = device.unwrap_or_default();
        if !device.usual_login_hours.contains(&hour) {
            risk += 0.2;
            factors.push(RiskFactor::UnusualTime { hour });
        }

        // Factor 5: Recent failed attempts
        if device.failed_attempts > 3
            && device.last_failed_attempt.is_some_and(|t| {
                context.timestamp - t < Duration::hours(1)
            }) {
            risk += 0.5;
            factors.push(RiskFactor::RecentFailedAttempts {
                count: device.failed_attempts,
            });
        }

        // Factor 6: Breached credentials (HaveIBeenPwned)
        let user = postgres.get_user(context.user_id).await?;
        if self.breach_db.check_email(&user.email).await? {
            risk += 0.9;
            factors.push(RiskFactor::BreachedCredentials);
        }

        // Determine risk level and required challenges
        let (level, challenges) = match risk.min(1.0) {
            r if r < 0.3 => (
                RiskLevel::Low,
                vec![ChallengeType::Passkey], // Normal flow
            ),
            r if r < 0.6 => (
                RiskLevel::Medium,
                vec![
                    ChallengeType::Passkey,
                    ChallengeType::EmailVerification,
                ],
            ),
            r if r < 0.8 => (
                RiskLevel::High,
                vec![
                    ChallengeType::Passkey,
                    ChallengeType::EmailVerification,
                    ChallengeType::RecoveryCode, // Backup method
                ],
            ),
            _ => (
                RiskLevel::Critical,
                vec![ChallengeType::Block], // Don't allow login
            ),
        };

        Ok(RiskAssessment {
            score: risk.min(1.0),
            level,
            factors,
            required_challenges: challenges,
        })
    }
}
```

#### Integration with Auth Flow

```rust
// During login
async fn initiate_login(
    user_id: UserId,
    context: LoginContext,
) -> Result<LoginResponse> {
    // Calculate risk
    let risk = risk_calculator.calculate_login_risk(&context).await?;

    // Log risk assessment for security monitoring
    emit_event(AuthEvent::RiskAssessed {
        user_id,
        risk_score: risk.score,
        risk_level: risk.level,
        factors: risk.factors,
    });

    // Based on risk, require different challenges
    match risk.level {
        RiskLevel::Low => {
            // Normal passkey flow
            Ok(LoginResponse::RequirePasskey)
        }
        RiskLevel::Medium => {
            // Passkey + email verification
            send_verification_email(user_id).await?;
            Ok(LoginResponse::RequirePasskeyAndEmail)
        }
        RiskLevel::High => {
            // Maximum verification
            send_verification_email(user_id).await?;
            Ok(LoginResponse::RequireAllMethods {
                message: "Unusual login detected. Please verify via all methods."
            })
        }
        RiskLevel::Critical => {
            // Block and alert
            send_security_alert(user_id, risk.factors).await?;
            Err(AuthError::LoginBlocked {
                reason: "Suspicious activity detected. Check your email."
            })
        }
    }
}
```

### Benefits

- ✅ **Security**: Adaptive to threats, harder to bypass
- ✅ **UX**: Low friction for normal logins, high friction only when risky
- ✅ **Compliance**: Demonstrates reasonable security measures
- ✅ **Observability**: Rich risk factor data for security monitoring

### Metrics

```rust
// Prometheus metrics for risk-based auth
metrics::histogram!("auth_risk_score", "level" => "low").record(0.2);
metrics::counter!("auth_risk_factors_total", "factor" => "new_device").increment(1);
metrics::counter!("auth_critical_risk_blocked_total").increment(1);
```

### Implementation Tasks

- [ ] Risk calculator service (`auth/src/risk/calculator.rs`)
- [ ] GeoIP database integration (MaxMind GeoLite2)
- [ ] HaveIBeenPwned API integration
- [ ] Device location tracking (PostgreSQL)
- [ ] Impossible travel detection algorithm
- [ ] Risk-based challenge selection
- [ ] Security alerting for critical risk
- [ ] Metrics and observability

---

## 2. Granular Permission Caching with TTL

**Status**: Implement in Phase 6C (Weeks 6-8)
**Maturity**: Proven - Standard pattern for RBAC systems at scale
**Risk**: Low - Well-understood caching pattern

### Problem

Users with 1000+ permissions make sessions huge (100KB+). Storing all permissions in every session:
- Wastes Redis memory
- Slows down session serialization
- Makes permission updates hard (must invalidate all sessions)

Example: SaaS platform with 10,000 users × 500 permissions each = 5GB wasted in Redis.

### Solution

Lazy-load permissions on-demand with Redis hashes and per-permission TTL.

#### Data Model

```rust
// Remove from Session
struct Session {
    session_id: SessionId,
    user_id: UserId,
    device_id: DeviceId,

    // NO roles/permissions stored here anymore!
    // Loaded on-demand from Redis hash
}

// Separate Redis structure
// Key: permission:{user_id}
// Hash: {permission_name -> expiry_timestamp}
//
// Example:
// permission:user_123 -> {
//   "orders:read" -> 1704067200,  // Expires in 5 minutes
//   "users:write" -> 1704067800,  // Expires in 15 minutes (critical)
// }
```

#### Permission Cache

```rust
struct PermissionCache {
    redis: RedisClient,
    postgres: PostgresClient,
}

impl PermissionCache {
    /// Check if user has permission (lazy-load from Redis or PostgreSQL)
    async fn has_permission(
        &self,
        user_id: UserId,
        permission: Permission,
    ) -> Result<bool> {
        let key = format!("permission:{}", user_id);
        let field = permission.as_str();

        // Check Redis hash (O(1) operation)
        let expiry: Option<i64> = self.redis
            .hget(&key, field)
            .await?;

        match expiry {
            Some(exp) if exp > now().timestamp() => {
                // Cached and not expired
                Ok(true)
            }
            Some(_) => {
                // Expired, reload from PostgreSQL
                self.reload_permission(user_id, permission).await
            }
            None => {
                // Not cached, load from PostgreSQL
                self.reload_permission(user_id, permission).await
            }
        }
    }

    /// Reload single permission from PostgreSQL
    async fn reload_permission(
        &self,
        user_id: UserId,
        permission: Permission,
    ) -> Result<bool> {
        // Query PostgreSQL for this specific permission
        let has_it = self.postgres
            .user_has_permission(user_id, permission)
            .await?;

        if has_it {
            // Cache with TTL based on criticality
            let ttl = self.get_permission_ttl(permission);
            let expiry = now() + ttl;

            self.redis.hset(
                format!("permission:{}", user_id),
                permission.as_str(),
                expiry.timestamp()
            ).await?;

            // Set Redis key expiration (TTL for entire hash)
            self.redis.expire(
                format!("permission:{}", user_id),
                ttl.as_secs()
            ).await?;
        }

        Ok(has_it)
    }

    /// Get TTL based on permission criticality
    fn get_permission_ttl(&self, permission: Permission) -> Duration {
        match permission.criticality() {
            Criticality::Low => Duration::hours(1),    // Read permissions
            Criticality::Medium => Duration::minutes(15), // Write permissions
            Criticality::High => Duration::minutes(5),   // Admin permissions
            Criticality::Critical => Duration::minutes(1), // Delete, transfer money
        }
    }

    /// Invalidate single permission (when role changes)
    async fn invalidate_permission(
        &self,
        user_id: UserId,
        permission: Permission,
    ) -> Result<()> {
        self.redis.hdel(
            format!("permission:{}", user_id),
            permission.as_str()
        ).await?;

        Ok(())
    }

    /// Invalidate all permissions for user (when roles change significantly)
    async fn invalidate_all_permissions(
        &self,
        user_id: UserId,
    ) -> Result<()> {
        self.redis.del(format!("permission:{}", user_id)).await?;
        Ok(())
    }
}
```

#### Axum Integration

```rust
// Permission check in handler
async fn delete_order(
    user: AuthenticatedUser,
    Extension(perms): Extension<Arc<PermissionCache>>,
    Path(order_id): Path<OrderId>,
) -> Result<StatusCode> {
    // Lazy-load permission (cached if already checked)
    if !perms.has_permission(user.id, Permission::DeleteOrders).await? {
        return Err(AuthError::InsufficientPermissions);
    }

    // Proceed with deletion
    delete_order_logic(order_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

### Benefits

- ✅ **Memory**: 100KB session → 1KB session (100x reduction)
- ✅ **Scalability**: 10,000 users × 1KB = 10MB vs. 1GB
- ✅ **Fine-grained invalidation**: Revoke one permission without affecting others
- ✅ **TTL-based security**: Critical permissions expire quickly
- ✅ **Performance**: Redis HGET is O(1), same as session lookup

### Trade-offs

- ⚠️ **Complexity**: More Redis operations (session + permission hash)
- ⚠️ **Latency**: Permission check = 2 Redis calls (session + permission)
  - Mitigation: Batch permission checks, request-level caching

### Metrics

```rust
metrics::counter!("permission_cache_hits_total").increment(1);
metrics::counter!("permission_cache_misses_total").increment(1);
metrics::histogram!("permission_check_duration_seconds").record(0.002);
```

### Implementation Tasks

- [ ] Permission cache service (`auth/src/permissions/cache.rs`)
- [ ] Redis hash-based storage
- [ ] TTL configuration per permission criticality
- [ ] Invalidation API
- [ ] Axum middleware integration
- [ ] Batch permission loading
- [ ] Request-level caching
- [ ] Metrics and observability

---

## 3. Step-Up Authentication for Sensitive Actions

**Status**: Implement in Phase 6C (Weeks 6-8)
**Maturity**: Proven - Standard in banking, finance, healthcare
**Risk**: Low - Widely adopted pattern

### Problem

Being logged in doesn't mean you should be able to:
- Delete your account
- Transfer $10,000
- Change your email
- Revoke all devices

Users expect additional verification for high-risk actions, even if already authenticated.

### Solution

Time-bound elevation tokens that grant temporary permission for sensitive actions.

#### Data Model

```rust
// Add to Session
struct Session {
    // ... existing fields

    // Elevation state
    elevated_until: Option<DateTime>,
    elevation_scope: Option<ElevationScope>,
}

enum ElevationScope {
    DeleteAccount,       // Can delete account for next 5 minutes
    TransferMoney,       // Can transfer money for next 2 minutes
    ChangeEmail,         // Can change email for next 1 minute
    ManageUsers,         // Can add/remove users for next 10 minutes
    ViewSensitiveData,   // Can view SSN, payment methods for next 5 minutes
}

impl ElevationScope {
    fn duration(&self) -> Duration {
        match self {
            Self::DeleteAccount => Duration::minutes(5),
            Self::TransferMoney => Duration::minutes(2),
            Self::ChangeEmail => Duration::minutes(1),
            Self::ManageUsers => Duration::minutes(10),
            Self::ViewSensitiveData => Duration::minutes(5),
        }
    }

    fn required_challenge(&self) -> ChallengeType {
        match self {
            Self::DeleteAccount => ChallengeType::Passkey, // Maximum security
            Self::TransferMoney => ChallengeType::Passkey,
            Self::ChangeEmail => ChallengeType::Passkey,
            Self::ManageUsers => ChallengeType::Passkey,
            Self::ViewSensitiveData => ChallengeType::EmailVerification, // Lighter
        }
    }
}
```

#### Step-Up Middleware

```rust
/// Require elevation before executing handler
async fn require_elevation(
    session: &Session,
    scope: ElevationScope,
) -> Result<()> {
    match (&session.elevated_until, &session.elevation_scope) {
        (Some(until), Some(current_scope))
            if until > &now() && current_scope == &scope => {
            // Already elevated for this scope
            Ok(())
        }
        _ => {
            // Require fresh authentication
            Err(AuthError::ElevationRequired {
                scope,
                challenge: scope.required_challenge(),
                message: format!(
                    "Please verify your identity to {}",
                    scope.action_description()
                ),
            })
        }
    }
}

/// Grant elevation after successful challenge
async fn grant_elevation(
    session_id: SessionId,
    scope: ElevationScope,
) -> Result<()> {
    let duration = scope.duration();
    let elevated_until = now() + duration;

    // Update session in Redis
    redis.hset(
        format!("session:{}", session_id),
        "elevated_until",
        elevated_until.timestamp()
    ).await?;

    redis.hset(
        format!("session:{}", session_id),
        "elevation_scope",
        scope.as_str()
    ).await?;

    // Emit audit event
    emit_event(AuthEvent::ElevationGranted {
        session_id,
        scope,
        duration,
    });

    Ok(())
}
```

#### Axum Handler Example

```rust
/// Delete account requires elevation
async fn delete_account(
    user: AuthenticatedUser,
    session: Extension<Session>,
) -> Result<StatusCode> {
    // Check elevation
    require_elevation(&session, ElevationScope::DeleteAccount).await?;

    // Proceed with deletion (user has been re-authenticated)
    delete_user_account(user.id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Step-up challenge endpoint
async fn initiate_step_up(
    session_id: SessionId,
    scope: ElevationScope,
) -> Result<StepUpChallenge> {
    // Generate challenge based on required method
    let challenge = match scope.required_challenge() {
        ChallengeType::Passkey => {
            generate_webauthn_challenge(session_id).await?
        }
        ChallengeType::EmailVerification => {
            send_verification_code_email(session_id).await?
        }
    };

    Ok(StepUpChallenge {
        scope,
        challenge_type: scope.required_challenge(),
        challenge_id: challenge.id,
        expires_at: now() + Duration::minutes(5),
    })
}

/// Verify step-up challenge
async fn verify_step_up(
    session_id: SessionId,
    scope: ElevationScope,
    response: ChallengeResponse,
) -> Result<()> {
    // Verify challenge
    verify_challenge(response).await?;

    // Grant elevation
    grant_elevation(session_id, scope).await?;

    Ok(())
}
```

### Benefits

- ✅ **Security**: High-risk actions always require fresh authentication
- ✅ **UX**: Users feel safer, expected behavior
- ✅ **Compliance**: Meets regulatory requirements (PCI-DSS, SOC2)
- ✅ **Audit trail**: Every elevation is logged

### Metrics

```rust
metrics::counter!("elevation_required_total", "scope" => "delete_account").increment(1);
metrics::counter!("elevation_granted_total", "scope" => "delete_account").increment(1);
metrics::histogram!("elevation_duration_seconds").record(120.0);
```

### Implementation Tasks

- [ ] Elevation scope enum (`auth/src/elevation/mod.rs`)
- [ ] Step-up challenge generation
- [ ] Step-up verification flow
- [ ] Elevation middleware
- [ ] Redis session elevation state
- [ ] Audit logging
- [ ] Metrics and observability

---

## 4. Device Trust Levels (Progressive Trust)

**Status**: Implement in Phase 6B (Weeks 3-5)
**Maturity**: Proven - Used by Google, Microsoft, Apple
**Risk**: Low - Natural evolution of binary trust

### Problem

Current device model is binary: trusted or not trusted. This is too coarse.

Real-world scenarios:
- iPhone I've used daily for 3 years
- Laptop I used once last month
- New phone I just bought
- Borrowed computer at a friend's house

These should have different trust levels and unlock different capabilities.

### Solution

Progressive trust levels that increase over time and usage.

#### Data Model

```rust
// Replace boolean in RegisteredDevice
struct RegisteredDevice {
    // ... existing fields

    // Remove: trusted: bool

    // Add: Trust level (calculated)
    trust_level: DeviceTrustLevel, // Not stored, calculated on-demand

    // Metrics for calculating trust
    login_count: u32,
    first_seen: DateTime,
    last_seen: DateTime,
    user_marked_trusted: bool, // User explicitly trusts this device
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DeviceTrustLevel {
    Unknown,        // Never seen before (0 logins)
    Recognized,     // Seen 1-5 times, < 30 days old
    Familiar,       // Seen 5+ times, 30-90 days old
    Trusted,        // User explicitly marked as trusted
    HighlyTrusted,  // Trusted + has passkey + used 100+ times
}

impl DeviceTrustLevel {
    fn calculate(device: &RegisteredDevice) -> Self {
        // User explicitly trusted it
        if device.user_marked_trusted {
            if device.passkey_credential_id.is_some()
                && device.login_count > 100 {
                return Self::HighlyTrusted;
            }
            return Self::Trusted;
        }

        // Calculate based on usage
        let age = now() - device.first_seen;
        let login_count = device.login_count;

        match (age.num_days(), login_count) {
            (_, 0) => Self::Unknown,
            (0..=30, 1..=5) => Self::Recognized,
            (31..=90, 5..) => Self::Familiar,
            (91.., 20..) => Self::Familiar, // Could auto-suggest trust
            _ => Self::Recognized,
        }
    }

    /// What actions are allowed at this trust level?
    fn allowed_actions(&self) -> Vec<Action> {
        match self {
            Self::Unknown => vec![
                Action::ViewPublicData,
                Action::Login, // But requires extra verification
            ],
            Self::Recognized => vec![
                Action::ViewPublicData,
                Action::ViewPrivateData,
                Action::Login,
            ],
            Self::Familiar => vec![
                Action::ViewPublicData,
                Action::ViewPrivateData,
                Action::Login,
                Action::EditProfile,
                Action::SmallTransactions, // < $100
            ],
            Self::Trusted => vec![
                Action::All,
                // Except HighSecurity actions
            ],
            Self::HighlyTrusted => vec![
                Action::All, // Including high-security actions
            ],
        }
    }

    /// Integrate with risk-based auth
    fn risk_modifier(&self) -> f32 {
        match self {
            Self::Unknown => 0.4,        // +0.4 to risk score
            Self::Recognized => 0.2,     // +0.2 to risk score
            Self::Familiar => 0.0,       // No change
            Self::Trusted => -0.2,       // -0.2 from risk score
            Self::HighlyTrusted => -0.4, // -0.4 from risk score
        }
    }
}
```

#### Auto-Promotion Suggestions

```rust
/// Suggest device promotion after sufficient usage
async fn check_promotion_eligibility(
    device: &RegisteredDevice,
) -> Option<PromotionSuggestion> {
    let age = now() - device.first_seen;
    let trust_level = DeviceTrustLevel::calculate(device);

    match trust_level {
        DeviceTrustLevel::Familiar
            if age > Duration::days(30)
                && device.login_count > 20 => {
            // Suggest promotion to Trusted
            Some(PromotionSuggestion {
                device_id: device.device_id,
                current_level: DeviceTrustLevel::Familiar,
                suggested_level: DeviceTrustLevel::Trusted,
                reason: format!(
                    "You've been using this device for {} days with {} logins. \
                     Trust it for faster authentication?",
                    age.num_days(),
                    device.login_count
                ),
            })
        }
        _ => None,
    }
}
```

#### Integration with Risk-Based Auth

```rust
impl RiskCalculator {
    async fn calculate_login_risk(
        &self,
        context: &LoginContext,
    ) -> Result<RiskAssessment> {
        let mut risk = 0.0;

        // ... other risk factors

        // Factor: Device trust level
        let device = postgres.get_device(context.device_id).await?;
        if let Some(device) = device {
            let trust_level = DeviceTrustLevel::calculate(&device);
            risk += trust_level.risk_modifier();
            factors.push(RiskFactor::DeviceTrustLevel(trust_level));
        } else {
            // Unknown device
            risk += 0.4;
        }

        // ...
    }
}
```

### Benefits

- ✅ **UX**: Progressive trust = less friction over time
- ✅ **Security**: Unknown devices have high friction, trusted devices low
- ✅ **Auto-learning**: System learns trusted devices automatically
- ✅ **User control**: User can always explicitly trust/untrust

### Metrics

```rust
metrics::gauge!("devices_by_trust_level", "level" => "highly_trusted").set(1250.0);
metrics::counter!("device_promotion_suggestions_total").increment(1);
metrics::counter!("device_promotions_accepted_total").increment(1);
```

### Implementation Tasks

- [ ] Trust level calculation (`auth/src/devices/trust.rs`)
- [ ] Auto-promotion suggestions
- [ ] Integration with risk-based auth
- [ ] UI for device management ("My Devices")
- [ ] Metrics and observability

---

## Summary

These four features are **production-ready** and should be implemented in Phase 6:

| Feature | Phase | Priority | Complexity | Impact |
|---------|-------|----------|------------|--------|
| Risk-based auth | 6C | High | Medium | ✅✅✅ Security + UX |
| Lazy permissions | 6C | High | Low | ✅✅✅ Scalability |
| Step-up auth | 6C | High | Low | ✅✅✅ Security + Compliance |
| Trust levels | 6B | Medium | Low | ✅✅ UX + Security |

**Total effort**: ~2-3 weeks additional (within 14-week timeline)
**ROI**: Massive - these features are standard in modern auth systems
