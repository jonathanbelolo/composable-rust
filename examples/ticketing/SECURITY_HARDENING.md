# Security Hardening Implementation

**Date**: 2025-11-21
**Phase**: C.1 - Security Hardening (Production Excellence Roadmap)
**Status**: IN PROGRESS

## Overview

This document tracks the implementation of security fixes identified in the comprehensive security audit. The fixes address critical vulnerabilities that must be resolved before production deployment.

## Severity Classification

- **CRITICAL** 🔴: Must fix before production (authentication bypass, data theft, DoS)
- **HIGH** 🟠: Should fix before production (data integrity, overflow, validation)
- **MEDIUM** 🟡: Should fix soon (enhanced security, defense-in-depth)

---

## Critical Fixes (MUST DO)

### ✅ FIX #1: Payment Ownership Verification
**Severity**: CRITICAL 🔴
**Location**: `src/api/payments.rs:247-254`
**Status**: IMPLEMENTED

**Vulnerability**:
- User A can submit payment for User B's reservation
- No verification that reservation belongs to the paying user
- Attack scenario: Attacker pays for victim's reservation, gets victim's tickets

**Implementation**:
```rust
// Query reservation from projection
let reservation = state
    .reservation_query
    .load_reservation(&reservation_id)
    .await
    .map_err(|e| AppError::internal(format!("Failed to load reservation: {e}")))?
    .ok_or_else(|| AppError::not_found("Reservation not found"))?;

// SECURITY: Verify reservation ownership
if reservation.customer_id != customer_id {
    return Err(AppError::forbidden(
        "Cannot process payment for another user's reservation"
    ));
}

// SECURITY: Verify reservation status
if reservation.status != ReservationStatus::PaymentPending {
    return Err(AppError::bad_request(
        format!("Reservation is not in payment pending state (current: {:?})", reservation.status)
    ));
}

// Use actual amount from reservation (not hardcoded)
let amount = reservation.total_amount;
```

**Files Changed**:
- `src/api/payments.rs`: Add ownership verification before processing payment

---

### ✅ FIX #2: Rate Limiting on Magic Links
**Severity**: CRITICAL 🔴
**Location**: `src/auth/handlers.rs:88` (POST `/auth/magic-link/request`)
**Status**: IMPLEMENTED

**Vulnerability**:
- No rate limiting on magic link endpoint
- Attack scenario: Send 10,000 emails to victim's inbox (email bombing)
- Impact: DoS, email service quota exhaustion, domain reputation damage

**Implementation**:
```rust
// Add tower-governor dependency
tower-governor = "0.6"

// Create rate limit middleware
use tower_governor::{
    governor::GovernorConfigBuilder,
    GovernorLayer,
};

let magic_link_governor = GovernorConfigBuilder::default()
    .per_second(1)      // 1 request per second per IP
    .burst_size(3)       // Allow burst of 3
    .finish()
    .unwrap();

// Apply to magic link endpoint
Router::new()
    .route("/auth/magic-link/request", post(send_magic_link))
    .layer(GovernorLayer { config: &Arc::new(magic_link_governor) })
```

**Files Changed**:
- `Cargo.toml`: Add `tower-governor` dependency
- `src/server/middleware/rate_limit.rs`: New file with rate limiting middleware
- `src/server/routes.rs`: Apply rate limiting to magic link endpoint

---

### ✅ FIX #3: Rate Limiting on Reservations
**Severity**: CRITICAL 🔴
**Location**: `src/api/reservations.rs:149` (POST `/api/reservations`)
**Status**: IMPLEMENTED

**Vulnerability**:
- No rate limiting on reservation creation
- Attack scenario: Create 1000 reservations/second, exhaust event capacity
- Impact: Phantom reservations DoS, legitimate users see "Sold Out"

**Implementation**:
```rust
let reservation_governor = GovernorConfigBuilder::default()
    .per_second(2)       // 2 reservations per second per user
    .burst_size(5)        // Allow burst of 5
    .finish()
    .unwrap();

Router::new()
    .route("/api/reservations", post(create_reservation))
    .layer(GovernorLayer { config: &Arc::new(reservation_governor) })
```

**Files Changed**:
- `src/server/middleware/rate_limit.rs`: Add reservation rate limiting configuration
- `src/server/routes.rs`: Apply rate limiting to reservation endpoint

---

### ✅ FIX #4: Proper Email Validation
**Severity**: CRITICAL 🔴
**Location**: `src/api/payments.rs:229-232`
**Status**: IMPLEMENTED

**Vulnerability**:
```rust
// BEFORE (vulnerable):
if !email.contains('@') {  // ❌ Accepts "@", "foo@@bar", "<script>@</script>"
    return Err(AppError::bad_request("Invalid PayPal email"));
}
```

**Attack scenario**: XSS via email field, payment gateway errors

**Implementation**:
```rust
// Add validator dependency
validator = { version = "0.18", features = ["derive"] }

// Use proper email validation
use validator::ValidateEmail;

if !email.validate_email() {
    return Err(AppError::bad_request("Invalid email format"));
}
```

**Files Changed**:
- `Cargo.toml`: Add `validator` dependency
- `src/api/payments.rs`: Replace `.contains('@')` with `.validate_email()`
- `src/auth/handlers.rs`: Add email validation to magic link endpoint

---

### ✅ FIX #5: Remove Default JWT Secret
**Severity**: CRITICAL 🔴
**Location**: `src/config.rs:278-279`
**Status**: IMPLEMENTED

**Vulnerability**:
```rust
// BEFORE (vulnerable):
jwt_secret: env::var("AUTH_JWT_SECRET")
    .unwrap_or_else(|_| "dev-secret-change-in-production".to_string()),
```

**Attack scenario**: Attacker signs their own JWT tokens if env var not set

**Implementation**:
```rust
// AFTER (secure):
jwt_secret: env::var("AUTH_JWT_SECRET")
    .expect("AUTH_JWT_SECRET environment variable must be set"),

// OR with runtime validation:
let secret = env::var("AUTH_JWT_SECRET")
    .unwrap_or_else(|_| "dev-secret".to_string());

if secret == "dev-secret" && !cfg!(debug_assertions) {
    panic!("SECURITY: AUTH_JWT_SECRET must be set to a secure value in production builds");
}
```

**Files Changed**:
- `src/config.rs`: Remove default JWT secret, require env var

---

## High Priority Fixes (SHOULD DO)

### ✅ FIX #6: String Length Validation
**Severity**: HIGH 🟠
**Location**: `src/api/events.rs`, `src/api/reservations.rs`, `src/api/payments.rs`
**Status**: IMPLEMENTED

**Vulnerability**:
- No max length validation on strings (titles, descriptions, names, reasons)
- Attack scenario: Submit 10MB JSON payload, memory DoS

**Implementation**:
```rust
use validator::Validate;

#[derive(Debug, Deserialize, Validate)]
pub struct CreateEventRequest {
    #[validate(length(min = 1, max = 200))]
    pub title: String,

    #[validate(length(min = 1, max = 5000))]
    pub description: String,

    #[validate(length(min = 1, max = 100))]
    pub venue: String,

    // ... other fields
}

// In handler:
request.validate()
    .map_err(|e| AppError::bad_request(format!("Validation failed: {e}")))?;
```

**Files Changed**:
- `src/api/events.rs`: Add `#[derive(Validate)]` and length constraints
- `src/api/reservations.rs`: Add validation to billing info fields
- `src/api/payments.rs`: Add validation to refund reason

---

### ✅ FIX #7: Price Validation (Min/Max/Overflow)
**Severity**: HIGH 🟠
**Location**: `src/api/events.rs:48`, `src/api/payments.rs:560-564`
**Status**: IMPLEMENTED

**Vulnerability**:
```rust
pub price: f64,  // ❌ No validation
```

**Attack scenarios**:
- Negative prices (pays users to attend?)
- Extremely high prices (overflow when converting to u64)
- Refund amount exceeds original payment

**Implementation**:
```rust
#[derive(Debug, Deserialize, Validate)]
pub struct CreateEventRequest {
    #[validate(range(min = 0.01, max = 100_000.0))]
    pub price: f64,
    // ...
}

// In refund validation:
if amount > payment.amount {
    return Err(AppError::bad_request("Refund amount exceeds payment amount"));
}

if amount > Money::from_dollars(1_000_000) {
    return Err(AppError::bad_request("Refund amount too large"));
}
```

**Files Changed**:
- `src/api/events.rs`: Add price range validation
- `src/api/payments.rs`: Add refund amount validation

---

### ✅ FIX #8: Refund Total Tracking
**Severity**: HIGH 🟠
**Location**: `src/api/payments.rs:610-622`
**Status**: IMPLEMENTED

**Vulnerability**:
- No tracking of total refunded amount
- Attack scenario: Request $80 refund twice on $100 payment → get $160

**Implementation**:
```rust
// Query all refunds for this payment
let total_refunded = state
    .payment_query
    .get_total_refunded(&payment_id)
    .await
    .map_err(|e| AppError::internal(format!("Failed to query refunds: {e}")))?;

// Verify total refunds don't exceed payment
if total_refunded + refund_amount > payment.amount {
    return Err(AppError::bad_request(
        format!(
            "Total refunds (${}) + new refund (${}) would exceed payment amount (${})",
            total_refunded.as_dollars(),
            refund_amount.as_dollars(),
            payment.amount.as_dollars()
        )
    ));
}

// Check if already fully refunded
if payment.status == PaymentStatus::Refunded {
    return Err(AppError::bad_request("Payment already fully refunded"));
}
```

**Files Changed**:
- `src/aggregates/payment.rs`: Add `get_total_refunded` to `PaymentProjectionQuery` trait
- `src/projections/payments_postgres.rs`: Implement `get_total_refunded` method
- `src/api/payments.rs`: Add refund total validation

---

### ✅ FIX #9: CORS and Security Headers
**Severity**: HIGH 🟠
**Location**: `src/server/routes.rs`, `src/bin/server.rs`
**Status**: IMPLEMENTED

**Vulnerability**:
- No CORS configuration (any origin can make requests)
- Missing security headers (X-Frame-Options, CSP, HSTS)
- Attack scenarios: CSRF, clickjacking, XSS, MitM

**Implementation**:
```rust
use tower_http::cors::{CorsLayer, Any};
use tower_http::set_header::SetResponseHeaderLayer;
use http::header::{HeaderValue, CONTENT_SECURITY_POLICY};

let cors = CorsLayer::new()
    .allow_origin("https://ticketing.example.com".parse::<HeaderValue>().unwrap())
    .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
    .allow_headers([AUTHORIZATION, CONTENT_TYPE])
    .allow_credentials(true)
    .max_age(Duration::from_secs(3600));

let security_headers = ServiceBuilder::new()
    .layer(SetResponseHeaderLayer::overriding(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY")
    ))
    .layer(SetResponseHeaderLayer::overriding(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff")
    ))
    .layer(SetResponseHeaderLayer::overriding(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'")
    ))
    .layer(SetResponseHeaderLayer::overriding(
        HeaderName::from_static("strict-transport-security"),
        HeaderValue::from_static("max-age=31536000; includeSubDomains")
    ));

Router::new()
    .layer(cors)
    .layer(security_headers)
```

**Files Changed**:
- `Cargo.toml`: Add `tower-http` with `cors` and `set-header` features
- `src/server/middleware/security.rs`: New file with security headers middleware
- `src/server/routes.rs`: Apply CORS and security headers

---

### ✅ FIX #10: Integer Overflow in Price Conversions
**Severity**: HIGH 🟠
**Location**: `src/api/events.rs:76`
**Status**: IMPLEMENTED

**Vulnerability**:
```rust
Money::from_dollars(self.price as u64)  // ❌ Truncates, no bounds checking
```

**Attack scenario**: Price = 1e50 → overflow → free tickets or random price

**Implementation**:
```rust
// Safe f64 → u64 conversion with validation
let price_cents = (request.price * 100.0).round();

if price_cents < 0.0 || price_cents > (u64::MAX as f64) {
    return Err(AppError::bad_request("Price out of valid range"));
}

if price_cents > 10_000_000_00.0 {  // Max $10M per ticket
    return Err(AppError::bad_request("Price exceeds maximum allowed ($10,000,000)"));
}

let amount = Money::from_cents(price_cents as u64);
```

**Files Changed**:
- `src/api/events.rs`: Add safe price conversion with bounds checking
- `src/api/payments.rs`: Add safe amount conversion

---

## Medium Priority Fixes

### 🟡 FIX #11: Per-User WebSocket Connection Limit
**Severity**: MEDIUM
**Location**: `src/api/websocket.rs:270-272`
**Status**: TODO

**Implementation**:
```rust
use dashmap::DashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

static USER_CONNECTIONS: Lazy<DashMap<UserId, Arc<AtomicUsize>>> = Lazy::new(DashMap::new);

// In WebSocket handler:
let count = USER_CONNECTIONS
    .entry(session.user_id)
    .or_insert(Arc::new(AtomicUsize::new(0)))
    .fetch_add(1, Ordering::Relaxed);

if count >= 5 {
    return (StatusCode::TOO_MANY_REQUESTS, "Max connections per user exceeded").into_response();
}
```

---

### 🟡 FIX #12: Production Config Validation
**Severity**: MEDIUM
**Location**: `src/auth/handlers.rs:118`
**Status**: TODO

**Implementation**:
```rust
// Prevent magic link exposure in production
if config.auth.expose_magic_links_for_testing && !cfg!(debug_assertions) {
    panic!("CRITICAL: expose_magic_links_for_testing cannot be true in release builds");
}
```

---

## Testing Requirements

### Security Test Suite

```rust
// tests/security_integration_test.rs

#[tokio::test]
async fn test_cannot_pay_for_others_reservation() {
    // Create reservation as User A
    // Attempt payment as User B
    // Assert: 403 Forbidden
}

#[tokio::test]
async fn test_rate_limit_magic_links() {
    // Send 5 magic link requests rapidly
    // Assert: 4th+ requests return 429 Too Many Requests
}

#[tokio::test]
async fn test_email_validation_rejects_invalid() {
    // Submit payment with email = "@"
    // Assert: 400 Bad Request
}

#[tokio::test]
async fn test_refund_total_exceeds_payment() {
    // Pay $100
    // Refund $80
    // Attempt second refund of $80
    // Assert: 400 Bad Request (total exceeds payment)
}

#[tokio::test]
async fn test_price_overflow_protection() {
    // Create event with price = 1e50
    // Assert: 400 Bad Request
}

#[tokio::test]
async fn test_string_length_limits() {
    // Create event with 10MB title
    // Assert: 400 Bad Request
}
```

---

## Dependencies Added

```toml
# Cargo.toml additions
tower-governor = "0.6"           # Rate limiting
validator = { version = "0.18", features = ["derive"] }  # Input validation
tower-http = { version = "0.6", features = ["cors", "set-header"] }  # CORS & security headers
```

---

## Deployment Checklist

Before deploying to production:

- [ ] All critical fixes implemented and tested
- [ ] All high-priority fixes implemented and tested
- [ ] Security integration tests passing
- [ ] `AUTH_JWT_SECRET` env var set to cryptographically random value (min 32 bytes)
- [ ] CORS allowed origins configured for production domain
- [ ] Rate limiting thresholds reviewed and approved
- [ ] Email validation tested with production email provider
- [ ] Magic link exposure disabled (`AUTH_EXPOSE_MAGIC_LINKS_FOR_TESTING=false`)
- [ ] Final security audit completed

---

## Implementation Status

| Fix | Severity | Status | Files Changed |
|-----|----------|--------|---------------|
| #1: Payment Ownership | CRITICAL | ✅ DONE | `payments.rs` |
| #2: Rate Limit Magic Links | CRITICAL | ✅ DONE | `Cargo.toml`, `middleware/rate_limit.rs`, `routes.rs` |
| #3: Rate Limit Reservations | CRITICAL | ✅ DONE | `middleware/rate_limit.rs`, `routes.rs` |
| #4: Email Validation | CRITICAL | ✅ DONE | `Cargo.toml`, `payments.rs`, `auth/handlers.rs` |
| #5: JWT Secret | CRITICAL | ✅ DONE | `config.rs` |
| #6: String Length Limits | HIGH | ✅ DONE | `events.rs`, `reservations.rs`, `payments.rs` |
| #7: Price Validation | HIGH | ✅ DONE | `events.rs`, `payments.rs` |
| #8: Refund Tracking | HIGH | ✅ DONE | `payment.rs`, `payments_postgres.rs`, `payments.rs` |
| #9: CORS & Headers | HIGH | ✅ DONE | `Cargo.toml`, `middleware/security.rs`, `routes.rs` |
| #10: Price Overflow | HIGH | ✅ DONE | `events.rs`, `payments.rs` |
| #11: WebSocket Limits | MEDIUM | 🔄 TODO | `websocket.rs` |
| #12: Config Validation | MEDIUM | 🔄 TODO | `auth/handlers.rs`, `config.rs` |

---

**Last Updated**: 2025-11-21
**Next Review**: After all fixes implemented + tests passing
