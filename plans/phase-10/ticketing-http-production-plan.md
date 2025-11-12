# Phase 10: Ticketing Example - HTTP API & Production Infrastructure

**Status**: ✅ Approved - Ready for Implementation
**Date**: 2025-11-12
**Updated**: 2025-11-12
**Objective**: Transform the ticketing example from a domain-rich event-sourced system into a production-ready HTTP API service with full authentication, PostgreSQL projections, Redis sessions, WebSocket support, and operational excellence.

**Scope**: Full authentication system (magic link, OAuth, passkey), 35+ HTTP endpoints with authorization, three separate PostgreSQL databases (event store, projections, analytics), Redis for sessions/tokens, analytics ETL service, WebSocket real-time updates with auth, and comprehensive testing.

---

## Table of Contents

1. [Current State Analysis](#current-state-analysis)
2. [Goals & Success Criteria](#goals--success-criteria)
3. [API Design](#api-design)
4. [Architecture Decisions](#architecture-decisions)
5. [Implementation Plan](#implementation-plan)
6. [Testing Strategy](#testing-strategy)
7. [Production Considerations](#production-considerations)
8. [Future Enhancements](#future-enhancements)

---

## Current State Analysis

### What We Have (✅)

The ticketing example is already a sophisticated event-sourced system with ~6,800 lines of Rust code:

**Domain Layer (Complete)**:
- ✅ 4 aggregates with full business logic:
  - `EventAggregate` - Event lifecycle management (Draft → Published → SalesOpen → SalesClosed/Cancelled)
  - `InventoryAggregate` - Concurrency-safe seat reservation (prevents double-booking)
  - `ReservationAggregate` - Saga orchestration with 5-min timeout and compensation
  - `PaymentAggregate` - Payment processing with refunds
- ✅ Complete domain types (Money, EventId, Capacity, etc.)
- ✅ Full event sourcing with append/replay
- ✅ 36 aggregate tests + 2 integration tests

**Infrastructure Layer (Complete)**:
- ✅ PostgreSQL event store for write-side persistence
- ✅ Redpanda (Kafka) event bus for pub/sub
- ✅ 3 CQRS read model projections:
  - `AvailableSeatsProjection` - Real-time seat availability
  - `SalesAnalyticsProjection` - Revenue metrics and sales data
  - `CustomerHistoryProjection` - Purchase history per customer
- ✅ `TicketingApp` coordinator with lifecycle management
- ✅ Service layer (InventoryService, ReservationService, PaymentService)

**Configuration (Complete)**:
- ✅ Environment-based config (PostgreSQL, Redpanda, server settings)
- ✅ Database migrations with sqlx
- ✅ Structured logging with tracing

### What's Missing (❌)

**Presentation Layer**:
- ❌ HTTP API endpoints (no way to interact via REST)
- ❌ WebSocket for real-time updates
- ❌ Request/response schemas (JSON serialization)
- ❌ HTTP error mapping (domain errors → status codes)
- ❌ API documentation (OpenAPI/Swagger)

**Authentication & Authorization**:
- ❌ Auth endpoints (magic link, OAuth, passkey, session)
- ❌ Redis session store (ephemeral storage with TTL)
- ❌ PostgreSQL user/device repositories (persistent storage)
- ❌ Protected endpoints (require authentication)
- ❌ WebSocket authentication
- ❌ Rate limiting with Redis

**Production Infrastructure**:
- ❌ CORS configuration
- ❌ Request logging & correlation IDs
- ❌ Metrics exposition (Prometheus)
- ❌ Graceful shutdown
- ❌ Separate PostgreSQL instance for projections
- ❌ Separate PostgreSQL instance for analytics (OLAP)
- ❌ Analytics ETL service consuming events

**Developer Experience**:
- ❌ API testing (integration tests for HTTP endpoints)
- ❌ Example client code (curl commands, HTTP client)

---

## Goals & Success Criteria

### Primary Goals

1. **RESTful HTTP API**: Expose all domain operations via well-designed REST endpoints
2. **Full Authentication**: Magic link, OAuth, passkey auth with Redis sessions
3. **Protected Endpoints**: All ticketing operations require valid session
4. **Real-time Updates**: WebSocket support with authentication for live seat availability
5. **PostgreSQL Projections**: Durable projection storage (separate from event store)
6. **Production-Ready**: Observability, error handling, graceful degradation
7. **Developer-Friendly**: Clear API docs, good error messages, easy testing

### Success Criteria

- [ ] All domain operations accessible via HTTP endpoints (~25 endpoints)
- [ ] Authentication system fully operational (magic link, OAuth, passkey)
- [ ] Redis running with session store, token store, challenge store, rate limiter
- [ ] PostgreSQL projection store (separate instance from event store)
- [ ] Protected endpoints validate sessions via Authorization header
- [ ] WebSocket authentication validates session on connection
- [ ] HTTP → Domain error mapping with appropriate status codes
- [ ] WebSocket endpoint streaming real-time seat availability changes
- [ ] Health check endpoints (liveness + readiness)
- [ ] Structured logging with correlation IDs for request tracing
- [ ] Integration tests covering auth + ticketing workflows
- [ ] README with curl examples for all endpoints (including auth)
- [ ] Zero clippy warnings

### Non-Goals (Deferred)

- OpenAPI/Swagger generation (Phase 13 - code generation)
- GraphQL support (Future)
- gRPC support (Future)
- Advanced RBAC/permissions (Phase 11 - use basic role checks for now)

---

## API Design

### Endpoint Inventory

#### 0. Authentication (composable-rust-auth)

| Method | Endpoint | Description | Request Body | Response | Auth Required |
|--------|----------|-------------|--------------|----------|---------------|
| `POST` | `/api/auth/magic-link/send` | Send magic link email | `{ "email": "user@example.com" }` | `{ "message": "Magic link sent" }` | No |
| `GET` | `/api/auth/magic-link/verify` | Verify magic link token | Query: `?token=xxx` | `SessionResponse` | No |
| `GET` | `/api/auth/oauth/:provider/authorize` | Redirect to OAuth provider | - | Redirect | No |
| `GET` | `/api/auth/oauth/:provider/callback` | Handle OAuth callback | Query: `?code=xxx&state=xxx` | `SessionResponse` | No |
| `POST` | `/api/auth/passkey/register/begin` | Begin passkey registration | `{ "email": "user@example.com" }` | `PublicKeyCredentialCreationOptions` | No |
| `POST` | `/api/auth/passkey/register/complete` | Complete passkey registration | `PublicKeyCredential` | `SessionResponse` | No |
| `POST` | `/api/auth/passkey/login/begin` | Begin passkey login | `{ "email": "user@example.com" }` | `PublicKeyCredentialRequestOptions` | No |
| `POST` | `/api/auth/passkey/login/complete` | Complete passkey login | `PublicKeyCredential` | `SessionResponse` | No |
| `GET` | `/api/auth/session` | Get session info | Header: `Authorization: Bearer <session_id>` | `SessionResponse` | Yes |
| `POST` | `/api/auth/logout` | Logout (destroy session) | Header: `Authorization: Bearer <session_id>` | `{ "message": "Logged out" }` | Yes |

**SessionResponse**:
```json
{
  "session_id": "uuid",
  "email": "user@example.com",
  "expires_at": "2025-11-12T15:30:00Z",
  "last_active": "2025-11-12T15:00:00Z"
}
```

#### 1. Event Management (Event Aggregate)

| Method | Endpoint | Description | Request Body | Response | Auth Required |
|--------|----------|-------------|--------------|----------|---------------|
| `POST` | `/api/events` | Create new event | `CreateEventRequest` | `EventResponse` | **Yes** (Admin) |
| `GET` | `/api/events/{event_id}` | Get event details | - | `EventResponse` | No |
| `GET` | `/api/events` | List all events (paginated) | Query: `?status=&page=&per_page=` | `PaginatedEventsResponse` | No |
| `POST` | `/api/events/{event_id}/publish` | Publish event | - | `EventResponse` | **Yes** (Admin) |
| `POST` | `/api/events/{event_id}/sales/open` | Open ticket sales | - | `EventResponse` | **Yes** (Admin) |
| `POST` | `/api/events/{event_id}/sales/close` | Close ticket sales | - | `EventResponse` | **Yes** (Admin) |
| `POST` | `/api/events/{event_id}/cancel` | Cancel event | `CancelEventRequest` | `EventResponse` | **Yes** (Admin) |

#### 2. Inventory/Availability (Read Models)

| Method | Endpoint | Description | Request Body | Response | Auth Required |
|--------|----------|-------------|--------------|----------|---------------|
| `GET` | `/api/events/{event_id}/availability` | Get seat availability by section | - | `AvailabilityResponse` | No |
| `GET` | `/api/events/{event_id}/sections/{section}` | Get detailed section inventory | - | `SectionInventoryResponse` | No |

#### 3. Reservations (Reservation Saga)

| Method | Endpoint | Description | Request Body | Response | Auth Required |
|--------|----------|-------------|--------------|----------|---------------|
| `POST` | `/api/reservations` | Initiate reservation (5-min timer) | `CreateReservationRequest` | `ReservationResponse` | **Yes** (Customer) |
| `GET` | `/api/reservations/{reservation_id}` | Get reservation status | - | `ReservationResponse` | **Yes** (Owner) |
| `POST` | `/api/reservations/{reservation_id}/payment` | Complete payment | `CompletePaymentRequest` | `ReservationResponse` | **Yes** (Owner) |
| `POST` | `/api/reservations/{reservation_id}/cancel` | Cancel reservation | - | `ReservationResponse` | **Yes** (Owner) |
| `GET` | `/api/reservations/{reservation_id}/history` | Get reservation event history | - | `ReservationHistoryResponse` | **Yes** (Owner/Admin) |

#### 4. Payments

| Method | Endpoint | Description | Request Body | Response | Auth Required |
|--------|----------|-------------|--------------|----------|---------------|
| `POST` | `/api/payments` | Initiate payment | `CreatePaymentRequest` | `PaymentResponse` | **Yes** (Customer) |
| `GET` | `/api/payments/{payment_id}` | Get payment status | - | `PaymentResponse` | **Yes** (Owner) |
| `POST` | `/api/payments/{payment_id}/refund` | Refund payment | `RefundPaymentRequest` | `PaymentResponse` | **Yes** (Admin) |

#### 5. Analytics (Read Models)

| Method | Endpoint | Description | Request Body | Response | Auth Required |
|--------|----------|-------------|--------------|----------|---------------|
| `GET` | `/api/events/{event_id}/analytics` | Get sales analytics | - | `SalesAnalyticsResponse` | **Yes** (Admin) |
| `GET` | `/api/customers/{customer_id}/history` | Get customer purchase history | - | `CustomerHistoryResponse` | **Yes** (Owner) |
| `GET` | `/api/customers/{customer_id}/lifetime-value` | Get customer LTV | - | `CustomerLTVResponse` | **Yes** (Owner/Admin) |

#### 6. Real-Time (WebSocket)

| Method | Endpoint | Description | Protocol | Auth Required |
|--------|----------|-------------|----------|---------------|
| `GET` | `/ws/events/{event_id}/availability` | Real-time seat updates | WebSocket JSON | **Yes** (Valid session) |

**WebSocket Authentication**: Session ID passed via query parameter: `/ws/events/{event_id}/availability?session_id=xxx`

#### 7. Operations

| Method | Endpoint | Description | Response | Auth Required |
|--------|----------|-------------|----------|---------------|
| `GET` | `/health` | Liveness check | `{"status": "ok"}` | No |
| `GET` | `/health/ready` | Readiness check (includes DB) | `HealthCheckResponse` | No |
| `GET` | `/metrics` | Prometheus metrics | Prometheus format | No |

### Request/Response Schemas

#### Event Management

```rust
// POST /api/events
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateEventRequest {
    pub name: String,
    pub venue: Venue,
    pub date: EventDate,
    pub pricing_tiers: Vec<PricingTier>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EventResponse {
    pub event_id: EventId,
    pub name: String,
    pub venue: Venue,
    pub date: EventDate,
    pub status: EventStatus,
    pub pricing_tiers: Vec<PricingTier>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PaginatedEventsResponse {
    pub events: Vec<EventResponse>,
    pub page: u32,
    pub per_page: u32,
    pub total: u64,
}
```

#### Reservations

```rust
// POST /api/reservations
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateReservationRequest {
    pub event_id: EventId,
    pub customer_id: CustomerId,
    pub section: String,
    pub quantity: u32,
    pub specific_seats: Option<Vec<SeatId>>, // For numbered seats
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReservationResponse {
    pub reservation_id: ReservationId,
    pub event_id: EventId,
    pub customer_id: CustomerId,
    pub seats: Vec<SeatAssignment>,
    pub total_amount: Money,
    pub status: ReservationStatus,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
```

#### Availability

```rust
// GET /api/events/{event_id}/availability
#[derive(Debug, Serialize, Deserialize)]
pub struct AvailabilityResponse {
    pub event_id: EventId,
    pub sections: HashMap<String, SectionAvailability>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SectionAvailability {
    pub total_capacity: u32,
    pub available: u32,
    pub reserved: u32,
    pub sold: u32,
    pub last_updated: DateTime<Utc>,
}
```

#### Analytics

```rust
// GET /api/events/{event_id}/analytics
#[derive(Debug, Serialize, Deserialize)]
pub struct SalesAnalyticsResponse {
    pub event_id: EventId,
    pub total_revenue: Money,
    pub tickets_sold: u32,
    pub average_price: Money,
    pub by_tier: HashMap<String, u32>,
    pub by_section: HashMap<String, SectionSales>,
}
```

### Error Response Format

All errors follow a consistent JSON structure:

```rust
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub code: String,           // Machine-readable error code
    pub message: String,        // Human-readable message
    pub details: Option<Value>, // Optional additional context
}
```

**Example error responses**:

```json
// 404 Not Found
{
  "code": "EVENT_NOT_FOUND",
  "message": "Event with id evt_123 not found"
}

// 409 Conflict (double-booking attempt)
{
  "code": "INSUFFICIENT_INVENTORY",
  "message": "Only 3 seats available in VIP section, requested 5",
  "details": {
    "requested": 5,
    "available": 3,
    "section": "VIP"
  }
}

// 408 Request Timeout (reservation expired)
{
  "code": "RESERVATION_EXPIRED",
  "message": "Reservation rsv_456 expired after 5 minutes",
  "details": {
    "reservation_id": "rsv_456",
    "expired_at": "2025-11-12T15:30:00Z"
  }
}
```

### Domain Error → HTTP Status Mapping

| Domain Error | HTTP Status | Error Code |
|--------------|-------------|------------|
| EventNotFound, ReservationNotFound | 404 | `{RESOURCE}_NOT_FOUND` |
| InsufficientInventory | 409 Conflict | `INSUFFICIENT_INVENTORY` |
| ReservationExpired | 408 Request Timeout | `RESERVATION_EXPIRED` |
| InvalidTransition (e.g., publish draft) | 422 Unprocessable Entity | `INVALID_STATE_TRANSITION` |
| PaymentFailed | 402 Payment Required | `PAYMENT_FAILED` |
| AlreadyExists (duplicate ID) | 409 Conflict | `RESOURCE_ALREADY_EXISTS` |
| InvalidQuantity (< 1) | 400 Bad Request | `INVALID_QUANTITY` |
| EventCancelled | 410 Gone | `EVENT_CANCELLED` |

---

## Architecture Decisions

### 1. HTTP Framework: Axum

**Decision**: Use Axum (already available via `composable-rust-web`)

**Rationale**:
- ✅ Composable Rust already has `composable-rust-web` crate with Axum
- ✅ Type-safe extractors (State, Json, Path, Query)
- ✅ Built on Tower (middleware ecosystem)
- ✅ Excellent error handling with `IntoResponse`
- ✅ Native async/await support

**Alternative considered**: Actix-web (rejected: not already in use, different ecosystem)

### 2. State Management

**Decision**: `Arc<TicketingApp>` as shared application state

**Architecture**:
```rust
#[derive(Clone)]
pub struct TicketingAppState {
    pub app: Arc<TicketingApp>,
}

// Axum handlers
async fn create_event(
    State(state): State<TicketingAppState>,
    Json(req): Json<CreateEventRequest>,
) -> Result<Json<EventResponse>, AppError> {
    // Use state.app.inventory, state.app.reservation, etc.
}
```

**Rationale**:
- TicketingApp already contains all services and projections
- Arc enables cheap cloning for Axum's state requirements
- Read-write lock on projections (`Arc<RwLock<Projection>>`) for concurrent reads

### 3. Error Handling Strategy

**Decision**: Custom `TicketingError` enum that implements `IntoResponse`

**Architecture**:
```rust
#[derive(Debug, Error)]
pub enum TicketingError {
    #[error("Event {0} not found")]
    EventNotFound(EventId),

    #[error("Insufficient inventory: requested {requested}, available {available}")]
    InsufficientInventory { requested: u32, available: u32 },

    #[error("Reservation {0} expired")]
    ReservationExpired(ReservationId),

    // ... more variants
}

impl IntoResponse for TicketingError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::EventNotFound(id) => (
                StatusCode::NOT_FOUND,
                "EVENT_NOT_FOUND",
                format!("Event {id} not found"),
            ),
            Self::InsufficientInventory { requested, available } => (
                StatusCode::CONFLICT,
                "INSUFFICIENT_INVENTORY",
                format!("Requested {requested} seats, only {available} available"),
            ),
            // ...
        };

        (status, Json(ErrorResponse { code, message })).into_response()
    }
}
```

### 4. Projection Storage: PostgreSQL (Separate Instance)

**Decision**: Use `composable-rust-projections` PostgreSQL store (separate database from event store)

**Architecture**:
```rust
// Two separate PostgreSQL connections
struct TicketingApp {
    event_store_pool: PgPool,        // events database (append-only)
    projection_store_pool: PgPool,   // projections database (read-optimized)
    // ...
}

// Query projections from PostgreSQL
async fn get_availability(
    State(state): State<TicketingAppState>,
    Path(event_id): Path<EventId>,
) -> Result<Json<AvailabilityResponse>, TicketingError> {
    let availability = state.app.projection_store
        .query_availability(&event_id)
        .await?;

    Ok(Json(AvailabilityResponse::from(availability)))
}
```

**Rationale**:
- ✅ Durable: Projections survive restarts
- ✅ Scalable: Read replicas for projections database
- ✅ Separation of concerns: Event store (write-optimized) vs projections (read-optimized)
- ✅ Production-ready: No need to rebuild projections on startup

**Database URLs**:
- Event Store: `postgresql://localhost:5432/ticketing_events`
- Projections: `postgresql://localhost:5433/ticketing_projections`

### 5. Redis for Authentication

**Decision**: Use Redis for ephemeral auth data (sessions, tokens, challenges)

**Architecture**:
```rust
struct AuthEnvironment {
    session_store: RedisSessionStore,           // Session with TTL
    token_store: RedisTokenStore,               // Magic link tokens
    challenge_store: RedisChallengeStore,       // WebAuthn challenges
    oauth_token_store: RedisOAuthTokenStore,    // OAuth tokens (encrypted)
    rate_limiter: RedisRateLimiter,             // Rate limiting counters
    // PostgreSQL for durable data
    user_repo: PostgresUserRepository,
    device_repo: PostgresDeviceRepository,
}
```

**Rationale**:
- ✅ TTL support: Sessions/tokens expire automatically
- ✅ Atomic operations: Prevent race conditions (e.g., token consumption)
- ✅ Fast: In-memory performance for hot data
- ✅ Appropriate: Sessions are ephemeral by nature

**Redis URL**: `redis://localhost:6379`

### 6. Authentication Middleware

**Decision**: Create `RequireAuth` extractor for protected endpoints

**Architecture**:
```rust
// Axum extractor that validates session
pub struct RequireAuth {
    pub session: Session,
    pub user_id: UserId,
}

#[async_trait]
impl<S> FromRequestParts<S> for RequireAuth
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // 1. Extract Authorization header
        let auth_header = parts.headers.get("Authorization")
            .ok_or(AppError::unauthorized("Missing Authorization header"))?;

        // 2. Parse Bearer token
        let session_id = parse_bearer_token(auth_header)?;

        // 3. Validate session via auth store
        let session = state.auth_store.validate_session(session_id).await?;

        Ok(RequireAuth {
            session,
            user_id: session.user_id,
        })
    }
}

// Usage in handlers
async fn create_event(
    auth: RequireAuth,  // Automatically validates session
    State(state): State<TicketingAppState>,
    Json(req): Json<CreateEventRequest>,
) -> Result<Json<EventResponse>, TicketingError> {
    // auth.user_id is guaranteed valid
    // ...
}
```

**Rationale**:
- ✅ Type-safe: Auth checked at compile time
- ✅ DRY: No boilerplate in every handler
- ✅ Composable: Can add role checks (RequireAdmin, RequireCustomer)
- ✅ Clear: Endpoint signature shows auth requirement

### 7. WebSocket Protocol with Authentication

**Decision**: Authenticate WebSocket connections via query parameter

**Connection URL**:
```
ws://localhost:8080/ws/events/{event_id}/availability?session_id={uuid}
```

**Authentication Flow**:
1. Client connects with session_id in query string
2. Server validates session before upgrade
3. If invalid: Return 401 Unauthorized (no WebSocket upgrade)
4. If valid: Upgrade to WebSocket and subscribe to event updates

**Protocol**:
```json
// Server → Client (availability update)
{
  "type": "availability_update",
  "event_id": "evt_123",
  "sections": {
    "VIP": { "available": 45, "total": 100 },
    "General": { "available": 120, "total": 500 }
  },
  "timestamp": "2025-11-12T15:30:00Z"
}

// Server → Client (error)
{
  "type": "error",
  "message": "Event evt_123 not found"
}

// Server → Client (ping/pong for keepalive)
{
  "type": "ping"
}
```

**Implementation**:
- WebSocket handler subscribes to EventBus
- Filters for seat reservation/release events for specific event_id
- Pushes updates to all connected clients for that event
- Session validation happens on connection (not per message)

### 6. Command vs Query Separation

**Write endpoints** (Commands):
- Go through aggregate services (InventoryService, ReservationService, PaymentService)
- Trigger event sourcing (append to event store)
- Publish events to Redpanda
- Return immediately (no waiting for projections)

**Read endpoints** (Queries):
- Read directly from projections (AvailableSeatsProjection, etc.)
- Eventually consistent (may lag slightly behind writes)
- Fast in-memory reads

### 7. Middleware Stack

**Planned middleware** (using Tower):
1. **Request logging** - Log all requests with correlation ID
2. **Tracing** - Distributed tracing with `tracing` crate
3. **CORS** - Allow cross-origin requests (configurable)
4. **Timeout** - 30-second timeout for all requests
5. **Compression** - Gzip response compression

**Deferred to Phase 11**:
- Authentication (JWT, API keys)
- Rate limiting (token bucket)
- Authorization (RBAC)

### 8. Analytics Database (Separate OLAP Store)

**Decision**: Add a fourth PostgreSQL database optimized for analytics queries

**Architecture**:
```text
Event-Driven Analytics Pipeline:

┌─────────────────────────────────────────────────────────────┐
│                    Operational System                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │ Event Store  │  │ Projections  │  │   Redis      │      │
│  │ (OLTP)       │  │ (OLTP)       │  │  Sessions    │      │
│  │ Port 5432    │  │ Port 5433    │  │  Port 6379   │      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
│         │                                                     │
│         └────────> Redpanda Event Bus <────────┐            │
└──────────────────────────────│──────────────────┴───────────┘
                               │
                               ▼
                    ┌──────────────────────┐
                    │   Analytics ETL      │
                    │  (Event Consumer)    │
                    └──────────────────────┘
                               │
                               ▼
                    ┌──────────────────────┐
                    │  Analytics Database  │
                    │   (OLAP-Optimized)   │
                    │     Port 5434        │
                    │                      │
                    │  - Star Schema       │
                    │  - Fact Tables       │
                    │  - Dimension Tables  │
                    │  - Aggregations      │
                    │  - Time-Series       │
                    └──────────────────────┘
```

**Why Separate Analytics Database?**

| Aspect | Operational DB (OLTP) | Analytics DB (OLAP) |
|--------|----------------------|---------------------|
| **Purpose** | Real-time operations | Business intelligence |
| **Queries** | Simple, fast (ms) | Complex, slow (seconds) |
| **Schema** | Normalized (3NF) | Denormalized (star/snowflake) |
| **Writes** | High frequency | Batch inserts |
| **Reads** | Row-oriented | Column-oriented |
| **Size** | Operational data only | Historical + aggregates |
| **Updates** | Frequent | Rare (mostly append) |
| **Indexes** | B-tree (transactional) | Bitmap, columnar |

**Separation benefits**:
- ✅ Analytics queries don't slow down operational system
- ✅ Different optimization strategies (row vs column storage)
- ✅ Can rebuild analytics DB from event stream without affecting operations
- ✅ Historical data retention (keep 5 years of analytics, 1 year operational)
- ✅ Specialized analytics tools (Metabase, Grafana, Tableau)

**Analytics Schema Design** (Star Schema for Ticketing):

```sql
-- Fact table: Ticket sales (immutable, append-only)
CREATE TABLE fact_ticket_sales (
    sale_id BIGSERIAL PRIMARY KEY,

    -- Dimension foreign keys
    event_id UUID NOT NULL,
    customer_id UUID NOT NULL,
    venue_id UUID NOT NULL,
    date_id INT NOT NULL,  -- YYYYMMDD for easy partitioning
    time_id INT NOT NULL,  -- HHMMSS

    -- Measures (numeric facts)
    quantity INT NOT NULL,
    unit_price_cents INT NOT NULL,
    total_amount_cents INT NOT NULL,
    discount_cents INT DEFAULT 0,
    fees_cents INT DEFAULT 0,

    -- Degenerate dimensions (facts that are also attributes)
    section VARCHAR(50) NOT NULL,
    pricing_tier VARCHAR(50) NOT NULL,
    payment_method VARCHAR(50) NOT NULL,

    -- Timestamps for time-series analysis
    reserved_at TIMESTAMPTZ NOT NULL,
    paid_at TIMESTAMPTZ,
    confirmed_at TIMESTAMPTZ,

    -- SCD Type 2: Track changes
    reservation_id UUID NOT NULL,
    payment_id UUID,

    -- Flags for filtering
    is_cancelled BOOLEAN DEFAULT false,
    is_refunded BOOLEAN DEFAULT false,

    CONSTRAINT fk_event FOREIGN KEY (event_id) REFERENCES dim_events(event_id),
    CONSTRAINT fk_customer FOREIGN KEY (customer_id) REFERENCES dim_customers(customer_id)
);

-- Dimension table: Events
CREATE TABLE dim_events (
    event_id UUID PRIMARY KEY,
    event_name VARCHAR(255) NOT NULL,
    venue_name VARCHAR(255) NOT NULL,
    venue_city VARCHAR(100),
    venue_country VARCHAR(100),
    event_date DATE NOT NULL,
    event_category VARCHAR(100),  -- Concert, Sports, Theater
    genre VARCHAR(100),            -- Rock, Pop, Soccer, etc.
    total_capacity INT,

    -- SCD Type 2: Slowly Changing Dimensions
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ,
    is_current BOOLEAN DEFAULT true
);

-- Dimension table: Customers
CREATE TABLE dim_customers (
    customer_id UUID PRIMARY KEY,
    email VARCHAR(255),
    signup_date DATE,
    customer_segment VARCHAR(50),  -- VIP, Regular, New
    lifetime_value_cents BIGINT,
    total_tickets_purchased INT,

    -- SCD Type 2
    valid_from TIMESTAMPTZ NOT NULL,
    valid_to TIMESTAMPTZ,
    is_current BOOLEAN DEFAULT true
);

-- Dimension table: Calendar (for time-series)
CREATE TABLE dim_calendar (
    date_id INT PRIMARY KEY,  -- YYYYMMDD
    date DATE NOT NULL,
    year INT,
    quarter INT,
    month INT,
    week INT,
    day_of_week INT,
    day_name VARCHAR(10),
    is_weekend BOOLEAN,
    is_holiday BOOLEAN,
    holiday_name VARCHAR(100)
);

-- Pre-aggregated table for fast dashboards
CREATE MATERIALIZED VIEW mv_daily_sales AS
SELECT
    date_id,
    event_id,
    COUNT(*) as tickets_sold,
    SUM(total_amount_cents) as revenue_cents,
    AVG(unit_price_cents) as avg_price_cents,
    COUNT(DISTINCT customer_id) as unique_customers
FROM fact_ticket_sales
WHERE NOT is_cancelled AND NOT is_refunded
GROUP BY date_id, event_id;

-- Indexes for common query patterns
CREATE INDEX idx_sales_event_date ON fact_ticket_sales(event_id, date_id);
CREATE INDEX idx_sales_customer ON fact_ticket_sales(customer_id, date_id);
CREATE INDEX idx_sales_time ON fact_ticket_sales(reserved_at);
CREATE INDEX idx_sales_section ON fact_ticket_sales(section, event_id);
```

**Event Consumption Pattern**:

```rust
// Analytics ETL service (separate binary)
pub struct AnalyticsETL {
    event_bus: Arc<RedpandaEventBus>,
    analytics_db: PgPool,  // Port 5434 - analytics database
}

impl AnalyticsETL {
    pub async fn run(&self) -> Result<()> {
        // Subscribe to ALL ticketing events
        let mut stream = self.event_bus.subscribe(vec![
            "ticketing.events",
            "ticketing.inventory",
            "ticketing.reservations",
            "ticketing.payments",
        ]).await?;

        while let Some(event) = stream.next().await {
            match event {
                TicketingEvent::ReservationCompleted {
                    reservation_id,
                    event_id,
                    customer_id,
                    seats,
                    total_amount,
                    completed_at,
                } => {
                    // Insert into fact table
                    sqlx::query!(
                        r#"
                        INSERT INTO fact_ticket_sales (
                            reservation_id, event_id, customer_id,
                            date_id, time_id,
                            quantity, total_amount_cents,
                            section, reserved_at, confirmed_at
                        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                        "#,
                        reservation_id,
                        event_id,
                        customer_id,
                        completed_at.format("%Y%m%d").parse::<i32>()?,
                        completed_at.format("%H%M%S").parse::<i32>()?,
                        seats.len() as i32,
                        total_amount.cents(),
                        seats[0].section,
                        seats[0].reserved_at,
                        completed_at,
                    )
                    .execute(&self.analytics_db)
                    .await?;

                    // Update customer dimension (lifetime value)
                    self.update_customer_lifetime_value(customer_id).await?;
                }

                TicketingEvent::EventCreated { event_id, name, venue, date, .. } => {
                    // Insert/update dimension table
                    sqlx::query!(
                        r#"
                        INSERT INTO dim_events (
                            event_id, event_name, venue_name,
                            event_date, valid_from, is_current
                        ) VALUES ($1, $2, $3, $4, NOW(), true)
                        ON CONFLICT (event_id) WHERE is_current
                        DO UPDATE SET
                            event_name = EXCLUDED.event_name,
                            valid_from = NOW()
                        "#,
                        event_id,
                        name,
                        venue.name,
                        date,
                    )
                    .execute(&self.analytics_db)
                    .await?;
                }

                // ... handle other events
                _ => {}
            }
        }

        Ok(())
    }
}
```

**Example Analytics Queries Enabled**:

```sql
-- 1. Sales performance over time (time-series)
SELECT
    c.date,
    c.day_name,
    COUNT(*) as tickets_sold,
    SUM(f.total_amount_cents) / 100.0 as revenue,
    AVG(f.unit_price_cents) / 100.0 as avg_price
FROM fact_ticket_sales f
JOIN dim_calendar c ON f.date_id = c.date_id
WHERE f.date_id >= 20250101
  AND NOT f.is_cancelled
GROUP BY c.date, c.day_name
ORDER BY c.date;

-- 2. Top-selling events by revenue
SELECT
    e.event_name,
    e.venue_name,
    COUNT(*) as tickets_sold,
    SUM(f.total_amount_cents) / 100.0 as revenue,
    e.total_capacity,
    (COUNT(*) * 100.0 / e.total_capacity) as sell_through_pct
FROM fact_ticket_sales f
JOIN dim_events e ON f.event_id = e.event_id
WHERE e.is_current
  AND NOT f.is_cancelled
GROUP BY e.event_name, e.venue_name, e.total_capacity
ORDER BY revenue DESC
LIMIT 10;

-- 3. Customer segmentation by lifetime value
SELECT
    c.customer_segment,
    COUNT(DISTINCT c.customer_id) as customers,
    AVG(c.lifetime_value_cents) / 100.0 as avg_ltv,
    SUM(c.lifetime_value_cents) / 100.0 as total_ltv,
    AVG(c.total_tickets_purchased) as avg_tickets
FROM dim_customers c
WHERE c.is_current
GROUP BY c.customer_segment
ORDER BY avg_ltv DESC;

-- 4. Section popularity by event type
SELECT
    e.event_category,
    f.section,
    COUNT(*) as tickets_sold,
    SUM(f.total_amount_cents) / 100.0 as revenue,
    AVG(f.unit_price_cents) / 100.0 as avg_price
FROM fact_ticket_sales f
JOIN dim_events e ON f.event_id = e.event_id
WHERE NOT f.is_cancelled
GROUP BY e.event_category, f.section
ORDER BY e.event_category, revenue DESC;

-- 5. Weekend vs weekday sales
SELECT
    c.is_weekend,
    COUNT(*) as tickets_sold,
    SUM(f.total_amount_cents) / 100.0 as revenue,
    AVG(f.unit_price_cents) / 100.0 as avg_price
FROM fact_ticket_sales f
JOIN dim_calendar c ON f.date_id = c.date_id
GROUP BY c.is_weekend;

-- 6. Cohort analysis: Customer retention
SELECT
    DATE_TRUNC('month', c.signup_date) as cohort_month,
    DATE_TRUNC('month', f.reserved_at) as purchase_month,
    COUNT(DISTINCT f.customer_id) as active_customers,
    SUM(f.total_amount_cents) / 100.0 as revenue
FROM fact_ticket_sales f
JOIN dim_customers c ON f.customer_id = c.customer_id
WHERE c.is_current
  AND NOT f.is_cancelled
GROUP BY cohort_month, purchase_month
ORDER BY cohort_month, purchase_month;
```

**Technology Options**:

| Database | Use Case | Pros | Cons |
|----------|----------|------|------|
| **PostgreSQL** | General analytics | Battle-tested, good for most use cases, familiar | Not optimized for massive data |
| **PostgreSQL + Citus** | Distributed analytics | Scales horizontally, sharding | More complex setup |
| **TimescaleDB** | Time-series analytics | Excellent for time-series, compression | PostgreSQL extension |
| **ClickHouse** | High-volume OLAP | 100x faster than PostgreSQL, columnar | New stack, steeper learning curve |
| **DuckDB** | Embedded analytics | In-process, zero setup, fast | Not suitable for concurrent users |

**Recommendation for Phase 10**:
- Use **PostgreSQL** (port 5434) for simplicity
- Star schema with fact/dimension tables
- Separate ETL consumer service
- Deferred: ClickHouse migration in Phase 11 if scale requires

**Implementation Scope**:

**Phase 10 (Basic Analytics)**:
- [ ] Add PostgreSQL analytics database (port 5434)
- [ ] Create star schema migrations (fact + dimension tables)
- [ ] Implement analytics ETL service (event consumer)
- [ ] Populate fact_ticket_sales from reservation events
- [ ] Add basic analytics queries to API

**Phase 11 (Advanced Analytics)**:
- [ ] Pre-aggregated materialized views
- [ ] Customer cohort analysis
- [ ] Time-series forecasting
- [ ] ClickHouse migration (if needed for scale)
- [ ] BI tool integration (Metabase, Grafana)

**Benefits of Event-Driven Analytics**:
1. ✅ **Decoupled**: Analytics doesn't impact operational performance
2. ✅ **Replayable**: Rebuild analytics DB from event stream anytime
3. ✅ **Flexible**: Add new analytics without touching operational code
4. ✅ **Historical**: Keep years of data for trend analysis
5. ✅ **Real-time**: Near real-time analytics (lag depends on consumer speed)
6. ✅ **Auditable**: Complete history of all business events

---

## Implementation Plan

### Phase 10.0: Infrastructure & Production Hardening (3-4 hours)

**Goal**: Set up complete production infrastructure with all databases, Redis, and critical operational requirements

**Docker Compose Infrastructure**:
- [ ] Create `docker-compose.yml` with 6 services:
  - PostgreSQL Event Store (port 5432, database: `ticketing_events`)
  - PostgreSQL Projections (port 5433, database: `ticketing_projections`)
  - PostgreSQL Analytics (port 5434, database: `ticketing_analytics`)
  - PostgreSQL Auth (port 5435, database: `ticketing_auth`)
  - Redis (port 6379, with persistence enabled)
  - Redpanda (ports 9092, 9644, 8082)
- [ ] Configure health checks for all services
- [ ] Add volume mounts for data persistence
- [ ] Create `.env.example` with all connection strings

**PostgreSQL Setup**:
- [ ] Create migrations directories for each database:
  - `migrations_events/` - Event store schema (events table with version field)
  - `migrations_projections/` - Projections schema (available_seats, sales_analytics, customer_history)
  - `migrations_analytics/` - Star schema (fact_ticket_sales, dim_events, dim_customers, dim_calendar)
  - `migrations_auth/` - Auth schema (users, devices, sessions, magic_link_tokens)
- [ ] **Event Versioning**: Add `event_version` field to events table:
```sql
CREATE TABLE events (
    id BIGSERIAL PRIMARY KEY,
    aggregate_id UUID NOT NULL,
    aggregate_type VARCHAR(255) NOT NULL,
    event_type VARCHAR(255) NOT NULL,
    event_version INT NOT NULL DEFAULT 1,  -- ← Critical for schema evolution
    event_data BYTEA NOT NULL,
    metadata JSONB,
    sequence_number BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(aggregate_id, sequence_number)
);
CREATE INDEX idx_events_aggregate ON events(aggregate_id, sequence_number);
CREATE INDEX idx_events_type_version ON events(event_type, event_version);
```
- [ ] **Optimistic Concurrency**: Verify `composable-rust-postgres` uses sequence numbers correctly
- [ ] Document connection pool sizing:
  - Event Store: 20 connections (high write volume)
  - Projections: 10 connections (moderate read volume)
  - Analytics: 5 connections (batch operations)
  - Auth: 10 connections (session lookups)

**PostgreSQL Backup Strategy**:
- [ ] Enable WAL archiving in `docker-compose.yml`:
```yaml
postgres_events:
  environment:
    - POSTGRES_INITDB_ARGS=-c wal_level=replica -c archive_mode=on
  volumes:
    - ./backups/wal:/var/lib/postgresql/wal_archive
```
- [ ] Create `scripts/backup-postgres.sh`:
```bash
#!/bin/bash
# Full backup of all PostgreSQL databases
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
for DB in events projections analytics auth; do
  docker exec ticketing_postgres_${DB} pg_dump -U postgres ticketing_${DB} \
    | gzip > "./backups/${DB}_${TIMESTAMP}.sql.gz"
done
```
- [ ] Create `scripts/restore-postgres.sh` for point-in-time recovery
- [ ] Document backup schedule (daily full, continuous WAL archiving)
- [ ] Document retention policy (30 days full backups, 7 days WAL)

**Redis Configuration**:
- [ ] Enable Redis persistence (RDB + AOF) in `docker-compose.yml`:
```yaml
redis:
  image: redis:7-alpine
  command: >
    redis-server
    --appendonly yes
    --appendfsync everysec
    --save 900 1
    --save 300 10
    --save 60 10000
  volumes:
    - redis_data:/data
  healthcheck:
    test: ["CMD", "redis-cli", "ping"]
    interval: 10s
    timeout: 3s
    retries: 3
```
- [ ] Document Redis persistence modes:
  - **RDB**: Snapshots every 60s if 10,000 keys changed (sessions bulk)
  - **AOF**: Append-only file with `everysec` fsync (durability vs performance)
- [ ] Create `scripts/backup-redis.sh`:
```bash
#!/bin/bash
docker exec ticketing_redis redis-cli BGSAVE
docker cp ticketing_redis:/data/dump.rdb ./backups/redis_$(date +%Y%m%d_%H%M%S).rdb
```
- [ ] Document Redis HA strategy (Sentinel for production, single instance for demo)

**Admin User Bootstrap**:
- [ ] Create `scripts/bootstrap-admin.sh`:
```bash
#!/bin/bash
# Create initial admin user for system access
ADMIN_EMAIL="${ADMIN_EMAIL:-admin@ticketing.example.com}"
cargo run --bin bootstrap-admin -- --email "$ADMIN_EMAIL" --role admin
```
- [ ] Implement `bin/bootstrap_admin.rs`:
  - Generate secure password
  - Create user in auth database
  - Assign admin role
  - Output credentials to stdout (one-time display)
- [ ] Document admin user creation in README

**Connection String Management**:
- [ ] Create `src/config.rs` with environment variable loading:
```rust
pub struct DatabaseConfig {
    pub event_store_url: String,      // DATABASE_URL_EVENTS
    pub projection_store_url: String,  // DATABASE_URL_PROJECTIONS
    pub analytics_url: String,         // DATABASE_URL_ANALYTICS
    pub auth_url: String,              // DATABASE_URL_AUTH
}

pub struct RedisConfig {
    pub url: String,                   // REDIS_URL
    pub max_connections: u32,          // REDIS_MAX_CONNECTIONS (default: 10)
}
```
- [ ] Add `.env.example`:
```bash
# Event Store
DATABASE_URL_EVENTS=postgresql://postgres:password@localhost:5432/ticketing_events

# Projections
DATABASE_URL_PROJECTIONS=postgresql://postgres:password@localhost:5433/ticketing_projections

# Analytics
DATABASE_URL_ANALYTICS=postgresql://postgres:password@localhost:5434/ticketing_analytics

# Auth
DATABASE_URL_AUTH=postgresql://postgres:password@localhost:5435/ticketing_auth

# Redis
REDIS_URL=redis://localhost:6379
REDIS_MAX_CONNECTIONS=10

# Redpanda
REDPANDA_BROKERS=localhost:9092

# Server
HTTP_PORT=8080
LOG_LEVEL=info

# WebSocket
WS_MAX_CONNECTIONS=50000  # Max concurrent WebSocket connections (adjust based on available memory)

# Performance
DB_POOL_SIZE_EVENTS=20
DB_POOL_SIZE_PROJECTIONS=10
DB_POOL_SIZE_ANALYTICS=5
DB_POOL_SIZE_AUTH=10
```

**Service Orchestration**:
- [ ] Create `scripts/wait-for-services.sh`:
```bash
#!/bin/bash
# Wait for all infrastructure services to be healthy
services=("postgres_events:5432" "postgres_projections:5433"
          "postgres_analytics:5434" "postgres_auth:5435"
          "redis:6379" "redpanda:9092")

for service in "${services[@]}"; do
  IFS=':' read -r name port <<< "$service"
  echo "Waiting for $name on port $port..."
  timeout 60s bash -c "until nc -z localhost $port; do sleep 1; done"
done
```
- [ ] Update `README.md` with startup sequence:
```bash
# 1. Start infrastructure
docker-compose up -d

# 2. Wait for services
./scripts/wait-for-services.sh

# 3. Run migrations (all databases)
DATABASE_URL=$DATABASE_URL_EVENTS sqlx migrate run --source migrations_events
DATABASE_URL=$DATABASE_URL_PROJECTIONS sqlx migrate run --source migrations_projections
DATABASE_URL=$DATABASE_URL_ANALYTICS sqlx migrate run --source migrations_analytics
DATABASE_URL=$DATABASE_URL_AUTH sqlx migrate run --source migrations_auth

# 4. Bootstrap admin user
./scripts/bootstrap-admin.sh

# 5. Start services
cargo run --bin server
cargo run --bin analytics_etl
```

**Health Checks**:
- [ ] Implement comprehensive health check at `/health/ready`:
```rust
pub async fn health_ready(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HealthStatus>, StatusCode> {
    let checks = vec![
        check_postgres(&state.event_store_pool, "event_store").await,
        check_postgres(&state.projection_store_pool, "projections").await,
        check_postgres(&state.analytics_pool, "analytics").await,
        check_postgres(&state.auth_pool, "auth").await,
        check_redis(&state.redis_client).await,
        check_redpanda(&state.event_bus).await,
    ];

    let all_healthy = checks.iter().all(|c| c.healthy);
    let status = if all_healthy { "healthy" } else { "degraded" };

    Ok(Json(HealthStatus { status, checks }))
}
```

**Verification**:
- [ ] Run `docker-compose up -d` and verify all 6 services start
- [ ] Run `./scripts/wait-for-services.sh` successfully
- [ ] Run all migrations without errors
- [ ] Verify event versioning works with composable-rust-postgres
- [ ] Create admin user with bootstrap script
- [ ] Test Redis persistence (restart container, verify data persists)
- [ ] Test PostgreSQL backup script
- [ ] Access `/health/ready` endpoint and see all checks pass

**Deliverable**: Complete production infrastructure with backup/restore, persistence, and operational tooling

---

### Phase 10.1: Authentication Setup (4-5 hours)

**Goal**: Set up complete authentication system with Redis and PostgreSQL

**Dependencies**: Phase 10.0 (infrastructure must be running)

**Authentication Foundation**:
- [ ] Add dependencies to `Cargo.toml`:
  - `composable-rust-auth` (magic link, OAuth, passkeys)
  - `redis` crate for session/token storage
  - `argon2` for password hashing (if supporting passwords)
- [ ] Create `src/auth/` module structure:
  - `mod.rs` - Module exports
  - `environment.rs` - `AuthEnvironment` struct with Redis + PostgreSQL
  - `repositories.rs` - User and device repositories
  - `handlers.rs` - Auth endpoint handlers
  - `schemas.rs` - Request/response types
  - `email.rs` - Email provider (console for demo)

**Redis Store Implementation**:
- [ ] Implement `RedisSessionStore`:
```rust
pub struct RedisSessionStore {
    client: redis::Client,
}

impl SessionStore for RedisSessionStore {
    async fn create_session(&self, user_id: UserId) -> Result<Session> {
        let session_id = Uuid::new_v4();
        let key = format!("session:{}", session_id);
        let ttl = Duration::from_secs(24 * 60 * 60); // 24 hours

        self.client.set_ex(key, user_id, ttl).await?;
        Ok(Session { id: session_id, user_id, expires_at: ... })
    }

    async fn get_session(&self, session_id: Uuid) -> Result<Option<Session>> {
        let key = format!("session:{}", session_id);
        self.client.get(key).await
    }

    async fn delete_session(&self, session_id: Uuid) -> Result<()> {
        self.client.del(format!("session:{}", session_id)).await
    }
}
```
- [ ] Implement `RedisTokenStore` (magic link tokens with 15-minute TTL)
- [ ] Implement `RedisChallengeStore` (WebAuthn challenges with 5-minute TTL)
- [ ] Implement `RedisOAuthTokenStore` (OAuth tokens with encryption)
- [ ] Implement `RedisRateLimiter` (per-IP rate limiting):
```rust
pub struct RedisRateLimiter {
    client: redis::Client,
    max_requests: u32,  // 100 requests
    window: Duration,   // per 15 minutes
}

impl RateLimiter for RedisRateLimiter {
    async fn check(&self, ip: IpAddr) -> Result<bool> {
        let key = format!("ratelimit:{}", ip);
        let count: u32 = self.client.incr(key).await?;

        if count == 1 {
            self.client.expire(key, self.window).await?;
        }

        Ok(count <= self.max_requests)
    }
}
```

**PostgreSQL Repositories**:
- [ ] Implement `PostgresUserRepository`:
```rust
pub struct PostgresUserRepository {
    pool: PgPool,
}

impl UserRepository for PostgresUserRepository {
    async fn create_user(&self, email: String, role: Role) -> Result<User> {
        sqlx::query_as!(
            User,
            r#"
            INSERT INTO users (id, email, role, created_at)
            VALUES ($1, $2, $3, NOW())
            RETURNING *
            "#,
            Uuid::new_v4(),
            email,
            role as _
        )
        .fetch_one(&self.pool)
        .await
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<User>> { ... }
    async fn find_by_id(&self, id: UserId) -> Result<Option<User>> { ... }
}
```
- [ ] Implement `PostgresDeviceRepository` (for passkeys/WebAuthn)

**Auth Environment**:
- [ ] Create `AuthEnvironment` struct:
```rust
pub struct AuthEnvironment {
    // Redis stores (ephemeral data)
    pub session_store: RedisSessionStore,
    pub token_store: RedisTokenStore,
    pub challenge_store: RedisChallengeStore,
    pub oauth_token_store: RedisOAuthTokenStore,
    pub rate_limiter: RedisRateLimiter,

    // PostgreSQL repositories (durable data)
    pub user_repo: PostgresUserRepository,
    pub device_repo: PostgresDeviceRepository,

    // Email provider
    pub email_provider: ConsoleEmailProvider,  // Prints to console for demo

    // Clock for time-based operations
    pub clock: SystemClock,
}
```

**Authentication Reducers**:
- [ ] Implement `AuthReducer` for user authentication logic:
```rust
pub enum AuthAction {
    // Magic Link Flow
    RequestMagicLink { email: String },
    MagicLinkSent { email: String, token: String, expires_at: DateTime<Utc> },
    VerifyMagicLink { token: String },
    MagicLinkVerified { user_id: UserId, session_id: Uuid },

    // OAuth Flow
    InitiateOAuth { provider: OAuthProvider },
    OAuthCallback { code: String, state: String },
    OAuthCompleted { user_id: UserId, session_id: Uuid },

    // Passkey Flow
    InitiatePasskeyRegistration { user_id: UserId, device_name: String },
    PasskeyChallengeSent { challenge: String, user_id: UserId },
    VerifyPasskeyRegistration { credential: PublicKeyCredential },
    PasskeyRegistered { user_id: UserId, device_id: Uuid },

    // Session Management
    InvalidateSession { session_id: Uuid },
    SessionInvalidated { session_id: Uuid },
}

pub struct AuthState {
    pub active_sessions: HashMap<Uuid, Session>,
    pub pending_challenges: HashMap<Uuid, Challenge>,
}

impl Reducer for AuthReducer {
    type State = AuthState;
    type Action = AuthAction;
    type Environment = AuthEnvironment;

    fn reduce(&self, state: &mut AuthState, action: AuthAction, env: &AuthEnvironment)
        -> Vec<Effect<AuthAction>>
    {
        match action {
            AuthAction::RequestMagicLink { email } => {
                // Check rate limit
                // Generate token
                // Store in Redis with 15-min TTL
                // Send email
                vec![
                    Effect::Future(Box::pin(async move {
                        env.rate_limiter.check(ip).await?;
                        let token = generate_token();
                        env.token_store.store(token, email, Duration::minutes(15)).await?;
                        env.email_provider.send_magic_link(email, token).await?;
                        Some(AuthAction::MagicLinkSent { email, token, expires_at })
                    }))
                ]
            }
            AuthAction::VerifyMagicLink { token } => {
                // Verify token exists in Redis
                // Create session
                // Store session in Redis
                vec![...]
            }
            // ... other actions
        }
    }
}
```

**Testing**:
- [ ] Test Redis session creation and retrieval
- [ ] Test session expiry (verify TTL works)
- [ ] Test magic link token flow
- [ ] Test rate limiting (verify 429 after limit)
- [ ] Test user repository CRUD operations
- [ ] Test complete auth flow (magic link → session → authenticated request)

**Deliverable**: Fully functional authentication system with Redis sessions and PostgreSQL user storage

---

### Phase 10.2: Auth Middleware & Protected Endpoints (2-3 hours)

**Goal**: Implement authentication middleware and role-based authorization

**Dependencies**: Phase 10.1 (auth system must be operational)

**Authentication Extractors**:
- [ ] Implement `RequireAuth` extractor:
```rust
pub struct RequireAuth {
    pub session: Session,
    pub user_id: UserId,
    pub user: User,
}

#[async_trait]
impl<S> FromRequestParts<S> for RequireAuth
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        // 1. Extract Authorization header
        let auth_header = parts.headers.get("Authorization")
            .ok_or(AuthError::MissingToken)?;

        // 2. Parse Bearer token
        let token = auth_header.to_str()
            .map_err(|_| AuthError::InvalidToken)?
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidToken)?;

        // 3. Parse session ID from token
        let session_id = Uuid::parse_str(token)
            .map_err(|_| AuthError::InvalidToken)?;

        // 4. Validate session via Redis
        let session = state.auth_env.session_store
            .get_session(session_id).await?
            .ok_or(AuthError::SessionExpired)?;

        // 5. Load user from PostgreSQL
        let user = state.auth_env.user_repo
            .find_by_id(session.user_id).await?
            .ok_or(AuthError::UserNotFound)?;

        Ok(RequireAuth {
            session,
            user_id: user.id,
            user,
        })
    }
}
```

- [ ] Implement `RequireAdmin` extractor:
```rust
pub struct RequireAdmin {
    pub auth: RequireAuth,
}

#[async_trait]
impl<S> FromRequestParts<S> for RequireAdmin {
    async fn from_request_parts(...) -> Result<Self, Self::Rejection> {
        let auth = RequireAuth::from_request_parts(parts, state).await?;

        if auth.user.role != Role::Admin {
            return Err(AuthError::Forbidden);
        }

        Ok(RequireAdmin { auth })
    }
}
```

- [ ] Implement `RequireOwnership` extractor for resource-level authorization:
```rust
// Example: Users can only access their own reservations
pub struct RequireOwnership<T> {
    pub auth: RequireAuth,
    pub resource: T,
}

impl RequireOwnership<Reservation> {
    pub async fn verify(
        auth: RequireAuth,
        reservation_id: Uuid,
        state: &AppState,
    ) -> Result<Self, AuthError> {
        let reservation = state.load_reservation(reservation_id).await?;

        if reservation.customer_id != auth.user_id && auth.user.role != Role::Admin {
            return Err(AuthError::Forbidden);
        }

        Ok(RequireOwnership { auth, resource: reservation })
    }
}
```

**Auth HTTP Endpoints**:
- [ ] Implement auth handlers in `src/auth/handlers.rs`:
  - `POST /api/auth/magic-link/request` - Request magic link (public, rate-limited)
  - `POST /api/auth/magic-link/verify` - Verify magic link token (public)
  - `POST /api/auth/oauth/initiate` - Start OAuth flow (public)
  - `GET /api/auth/oauth/callback` - OAuth callback (public)
  - `POST /api/auth/passkey/register/initiate` - Start passkey registration (authenticated)
  - `POST /api/auth/passkey/register/verify` - Complete passkey registration (authenticated)
  - `POST /api/auth/passkey/authenticate/initiate` - Start passkey auth (public)
  - `POST /api/auth/passkey/authenticate/verify` - Complete passkey auth (public)
  - `POST /api/auth/logout` - Invalidate session (authenticated)
  - `GET /api/auth/me` - Get current user (authenticated)

**Request/Response Schemas**:
```rust
#[derive(Deserialize)]
pub struct MagicLinkRequest {
    pub email: String,
}

#[derive(Serialize)]
pub struct MagicLinkResponse {
    pub message: String,  // "Magic link sent to email@example.com"
    pub expires_in_seconds: u64,
}

#[derive(Deserialize)]
pub struct VerifyMagicLinkRequest {
    pub token: String,
}

#[derive(Serialize)]
pub struct AuthResponse {
    pub session_token: String,  // UUID to use in Authorization: Bearer header
    pub user: UserResponse,
    pub expires_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub role: String,  // "admin" | "customer" | "event_owner"
    pub created_at: DateTime<Utc>,
}
```

**Rate Limiting Middleware**:
- [ ] Implement rate limiting for auth endpoints:
```rust
pub async fn rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if !state.auth_env.rate_limiter.check(addr.ip()).await? {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    Ok(next.run(req).await)
}
```
- [ ] Apply rate limiting to all `/api/auth/*` endpoints (100 req/15min per IP)

**Error Handling**:
- [ ] Implement `AuthError` enum:
```rust
pub enum AuthError {
    MissingToken,
    InvalidToken,
    SessionExpired,
    UserNotFound,
    Forbidden,
    RateLimitExceeded,
    InvalidCredentials,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AuthError::MissingToken => (StatusCode::UNAUTHORIZED, "Missing authorization token"),
            AuthError::InvalidToken => (StatusCode::UNAUTHORIZED, "Invalid token"),
            AuthError::SessionExpired => (StatusCode::UNAUTHORIZED, "Session expired"),
            AuthError::UserNotFound => (StatusCode::UNAUTHORIZED, "User not found"),
            AuthError::Forbidden => (StatusCode::FORBIDDEN, "Insufficient permissions"),
            AuthError::RateLimitExceeded => (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded"),
            AuthError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "Invalid credentials"),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}
```

**Integration with Main App**:
- [ ] Update `AppState` to include `AuthEnvironment`:
```rust
pub struct AppState {
    pub ticketing_store: Arc<Store<TicketingState, TicketingAction, TicketingEnv, TicketingReducer>>,
    pub auth_env: Arc<AuthEnvironment>,
    pub event_store_pool: PgPool,
    pub projection_store_pool: PgPool,
    pub analytics_pool: PgPool,
    pub auth_pool: PgPool,
}
```

**Testing**:
- [ ] Test `RequireAuth` extractor with valid/invalid/expired tokens
- [ ] Test `RequireAdmin` rejects non-admin users
- [ ] Test `RequireOwnership` verifies resource ownership
- [ ] Test rate limiting blocks after limit
- [ ] Test complete flow: register → get token → authenticated request
- [ ] Test WebSocket authentication (token in query param)

**Deliverable**: Complete authentication middleware with role-based authorization and rate limiting

---

### Phase 10.3: PostgreSQL Projections (3-4 hours)

**Goal**: Set up separate PostgreSQL database for CQRS read models

**Dependencies**: Phase 10.0 (projection database must be running on port 5433)

**Projection Schema Migrations** (`migrations_projections/`):
- [ ] Create `001_available_seats_projection.sql`:
```sql
CREATE TABLE available_seats (
    event_id UUID PRIMARY KEY,
    event_name VARCHAR(255) NOT NULL,
    total_capacity INT NOT NULL,

    -- Section-level availability
    vip_total INT NOT NULL,
    vip_available INT NOT NULL,
    vip_reserved INT NOT NULL,
    vip_sold INT NOT NULL,

    standard_total INT NOT NULL,
    standard_available INT NOT NULL,
    standard_reserved INT NOT NULL,
    standard_sold INT NOT NULL,

    general_total INT NOT NULL,
    general_available INT NOT NULL,
    general_reserved INT NOT NULL,
    general_sold INT NOT NULL,

    -- Event status
    sales_open BOOLEAN NOT NULL DEFAULT false,
    event_status VARCHAR(50) NOT NULL,  -- 'draft', 'published', 'cancelled'

    last_updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_available_seats_sales_open ON available_seats(sales_open)
WHERE sales_open = true;
```

- [ ] Create `002_sales_analytics_projection.sql`:
```sql
CREATE TABLE sales_analytics (
    event_id UUID PRIMARY KEY,
    event_name VARCHAR(255) NOT NULL,

    -- Revenue metrics
    total_revenue_cents BIGINT NOT NULL DEFAULT 0,
    vip_revenue_cents BIGINT NOT NULL DEFAULT 0,
    standard_revenue_cents BIGINT NOT NULL DEFAULT 0,
    general_revenue_cents BIGINT NOT NULL DEFAULT 0,

    -- Sales volume
    total_tickets_sold INT NOT NULL DEFAULT 0,
    vip_sold INT NOT NULL DEFAULT 0,
    standard_sold INT NOT NULL DEFAULT 0,
    general_sold INT NOT NULL DEFAULT 0,

    -- Performance metrics
    capacity_utilization_percent DECIMAL(5,2) NOT NULL DEFAULT 0.00,
    average_ticket_price_cents INT NOT NULL DEFAULT 0,

    -- Time metrics
    first_sale_at TIMESTAMPTZ,
    last_sale_at TIMESTAMPTZ,

    last_updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

- [ ] Create `003_customer_history_projection.sql`:
```sql
CREATE TABLE customer_history (
    customer_id UUID NOT NULL,
    event_id UUID NOT NULL,
    reservation_id UUID NOT NULL,

    -- Purchase details
    section VARCHAR(50) NOT NULL,
    quantity INT NOT NULL,
    total_paid_cents BIGINT NOT NULL,

    -- Status
    status VARCHAR(50) NOT NULL,  -- 'reserved', 'confirmed', 'cancelled', 'expired'

    -- Timestamps
    reserved_at TIMESTAMPTZ NOT NULL,
    confirmed_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,

    PRIMARY KEY (customer_id, reservation_id)
);

CREATE INDEX idx_customer_history_customer ON customer_history(customer_id, reserved_at DESC);
CREATE INDEX idx_customer_history_event ON customer_history(event_id);
CREATE INDEX idx_customer_history_status ON customer_history(status);
```

- [ ] Create `004_customer_lifetime_value.sql`:
```sql
CREATE TABLE customer_lifetime_value (
    customer_id UUID PRIMARY KEY,

    -- Aggregated metrics
    total_purchases INT NOT NULL DEFAULT 0,
    total_spent_cents BIGINT NOT NULL DEFAULT 0,
    total_cancelled INT NOT NULL DEFAULT 0,

    -- Engagement metrics
    events_attended INT NOT NULL DEFAULT 0,
    favorite_section VARCHAR(50),

    -- Time metrics
    first_purchase_at TIMESTAMPTZ,
    last_purchase_at TIMESTAMPTZ,

    last_updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

**Projection Update Service** (`bin/projection_updater.rs`):
- [ ] Create projection updater binary:
```rust
pub struct ProjectionUpdater {
    event_bus: Arc<RedpandaEventBus>,
    projection_pool: PgPool,
}

impl ProjectionUpdater {
    pub async fn run(&self) -> Result<()> {
        // Subscribe to all ticketing events from Redpanda
        let mut stream = self.event_bus
            .subscribe("ticketing.events", "projection-updater-group")
            .await?;

        while let Some(event) = stream.next().await {
            match event.event_type.as_str() {
                // Inventory events → Update available_seats
                "SeatsReserved" => self.handle_seats_reserved(event).await?,
                "SeatsReleased" => self.handle_seats_released(event).await?,
                "SeatsConfirmed" => self.handle_seats_confirmed(event).await?,

                // Payment events → Update sales_analytics
                "PaymentCompleted" => self.handle_payment_completed(event).await?,

                // Reservation events → Update customer_history
                "ReservationCreated" => self.handle_reservation_created(event).await?,
                "ReservationConfirmed" => self.handle_reservation_confirmed(event).await?,
                "ReservationCancelled" => self.handle_reservation_cancelled(event).await?,
                "ReservationExpired" => self.handle_reservation_expired(event).await?,

                _ => {
                    tracing::debug!(event_type = %event.event_type, "Ignoring event");
                }
            }

            // Commit offset after successful processing
            stream.commit().await?;
        }

        Ok(())
    }

    async fn handle_seats_reserved(&self, event: SerializedEvent) -> Result<()> {
        let data: SeatsReservedData = bincode::deserialize(&event.event_data)?;

        sqlx::query!(
            r#"
            UPDATE available_seats
            SET vip_available = vip_available - $1,
                vip_reserved = vip_reserved + $1,
                last_updated_at = NOW()
            WHERE event_id = $2
            "#,
            data.quantity,
            data.event_id
        )
        .execute(&self.projection_pool)
        .await?;

        Ok(())
    }

    // ... other event handlers
}
```

- [ ] Implement idempotency using event sequence numbers:
```sql
-- Add sequence tracking table
CREATE TABLE projection_checkpoints (
    aggregate_id UUID NOT NULL,
    projection_name VARCHAR(100) NOT NULL,
    last_sequence_number BIGINT NOT NULL,
    last_updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (aggregate_id, projection_name)
);

-- Update projections atomically with checkpoint
BEGIN;
UPDATE available_seats SET ... WHERE event_id = $1;
INSERT INTO projection_checkpoints (aggregate_id, projection_name, last_sequence_number)
VALUES ($1, 'available_seats', $2)
ON CONFLICT (aggregate_id, projection_name)
DO UPDATE SET last_sequence_number = GREATEST(projection_checkpoints.last_sequence_number, $2);
COMMIT;
```

**Projection Query Service**:
- [ ] Create `src/projections/queries.rs`:
```rust
pub struct ProjectionQueries {
    pool: PgPool,
}

impl ProjectionQueries {
    pub async fn get_availability(&self, event_id: Uuid) -> Result<AvailableSeats> {
        sqlx::query_as!(
            AvailableSeats,
            r#"
            SELECT * FROM available_seats
            WHERE event_id = $1
            "#,
            event_id
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn get_sales_analytics(&self, event_id: Uuid) -> Result<SalesAnalytics> {
        sqlx::query_as!(
            SalesAnalytics,
            r#"
            SELECT * FROM sales_analytics
            WHERE event_id = $1
            "#,
            event_id
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn get_customer_history(
        &self,
        customer_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<CustomerPurchase>> {
        sqlx::query_as!(
            CustomerPurchase,
            r#"
            SELECT * FROM customer_history
            WHERE customer_id = $1
            ORDER BY reserved_at DESC
            LIMIT $2 OFFSET $3
            "#,
            customer_id,
            limit,
            offset
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_customer_ltv(&self, customer_id: Uuid) -> Result<CustomerLTV> {
        sqlx::query_as!(
            CustomerLTV,
            r#"
            SELECT * FROM customer_lifetime_value
            WHERE customer_id = $1
            "#,
            customer_id
        )
        .fetch_one(&self.pool)
        .await
    }
}
```

**Testing**:
- [ ] Test projection updater consumes events from Redpanda
- [ ] Test idempotency (replaying same event doesn't double-update)
- [ ] Test projection queries return correct data
- [ ] Test projection rebuild (replay all events to rebuild projections)
- [ ] Integration test: Create event → Reserve seats → Verify projection updated

**Deliverable**: Fully operational CQRS projection system with separate read database

---

### Phase 10.4: HTTP Foundation (2-3 hours)

**Goal**: Basic HTTP server with health checks and state management

- [x] Add `composable-rust-web` dependency to `Cargo.toml`
- [ ] Create `src/http/` module structure:
  - `mod.rs` - Module exports
  - `state.rs` - `TicketingAppState` wrapper
  - `error.rs` - `TicketingError` enum + `IntoResponse` impl
  - `routes.rs` - Router factory function
  - `schemas.rs` - Request/response types
- [ ] Implement `TicketingAppState` wrapping `Arc<TicketingApp>`
- [ ] Implement `TicketingError` with full domain error mapping
- [ ] Create `/health` and `/health/ready` endpoints
- [ ] Wire up basic router with health endpoints
- [ ] Update `bin/server.rs` to run HTTP server on port 8080
- [ ] Test: `curl http://localhost:8080/health` returns 200

**Deliverable**: HTTP server running with working health checks

---

### Phase 10.5: Event Management Endpoints (2-3 hours)

**Goal**: Full CRUD for events (Event Aggregate)

**Dependencies**: Phase 10.2 (auth middleware), Phase 10.4 (HTTP foundation)

- [ ] Define request/response schemas (`CreateEventRequest`, `EventResponse`, etc.)
- [ ] Implement `POST /api/events` - Create event
- [ ] Implement `GET /api/events/{event_id}` - Get event by ID
- [ ] Implement `GET /api/events` - List events with pagination
- [ ] Implement `POST /api/events/{event_id}/publish` - Publish event
- [ ] Implement `POST /api/events/{event_id}/sales/open` - Open sales
- [ ] Implement `POST /api/events/{event_id}/sales/close` - Close sales
- [ ] Implement `POST /api/events/{event_id}/cancel` - Cancel event
- [ ] Add error handling for not found, invalid transitions
- [ ] Manual testing with curl

**Deliverable**: Complete event management API

---

### Phase 10.6: Availability & Inventory Endpoints (1-2 hours)

**Goal**: Read-only queries for seat availability

**Dependencies**: Phase 10.3 (projections), Phase 10.4 (HTTP foundation)

- [ ] Implement `GET /api/events/{event_id}/availability` - All sections
- [ ] Implement `GET /api/events/{event_id}/sections/{section}` - Single section
- [ ] Map projection data to response schemas
- [ ] Handle missing data (event not found)
- [ ] Manual testing with curl

**Deliverable**: Availability query API

---

### Phase 10.7: Reservation Endpoints (3-4 hours)

**Goal**: Full reservation saga via HTTP

**Dependencies**: Phase 10.2 (auth middleware), Phase 10.4 (HTTP foundation)

- [ ] Define schemas (`CreateReservationRequest`, `ReservationResponse`, etc.)
- [ ] Implement `POST /api/reservations` - Initiate reservation
  - Validate event exists and sales open
  - Check availability before reserving
  - Return reservation with expiry time
- [ ] Implement `GET /api/reservations/{reservation_id}` - Get status
- [ ] Implement `POST /api/reservations/{reservation_id}/payment` - Complete payment
- [ ] Implement `POST /api/reservations/{reservation_id}/cancel` - Cancel
- [ ] Implement `GET /api/reservations/{reservation_id}/history` - Event history
- [ ] Error handling:
  - Insufficient inventory → 409
  - Expired reservation → 408
  - Invalid transitions → 422
- [ ] Test complete flow: reserve → pay → confirm

**Deliverable**: Full reservation API with compensation

---

### Phase 10.8: Payment Endpoints (1-2 hours)

**Goal**: Payment operations

**Dependencies**: Phase 10.2 (auth middleware), Phase 10.4 (HTTP foundation)

- [ ] Implement `POST /api/payments` - Initiate payment
- [ ] Implement `GET /api/payments/{payment_id}` - Get payment status
- [ ] Implement `POST /api/payments/{payment_id}/refund` - Refund payment
- [ ] Error handling for payment failures
- [ ] Manual testing

**Deliverable**: Payment API

---

### Phase 10.9: Analytics Endpoints (1-2 hours)

**Goal**: Read models for reporting

**Dependencies**: Phase 10.2 (auth middleware), Phase 10.4 (HTTP foundation)

- [ ] Implement `GET /api/events/{event_id}/analytics` - Sales analytics
- [ ] Implement `GET /api/customers/{customer_id}/history` - Purchase history
- [ ] Implement `GET /api/customers/{customer_id}/lifetime-value` - LTV
- [ ] Map projection data to schemas
- [ ] Manual testing

**Deliverable**: Analytics query API

---

### Phase 10.10: WebSocket Real-Time Updates (2-3 hours)

**Goal**: Real-time seat availability via WebSocket with authentication

**Dependencies**: Phase 10.1 (auth system), Phase 10.4 (HTTP foundation)

- [ ] Create WebSocket handler in `src/http/websocket.rs`
- [ ] **WebSocket Authentication**:
```rust
// Extract session token from query parameter
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    // Extract token from ?token=<session_id>
    let token = params.get("token")
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Validate session via Redis
    let session_id = Uuid::parse_str(token)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let session = state.auth_env.session_store
        .get_session(session_id).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Upgrade to WebSocket with authenticated user context
    ws.on_upgrade(move |socket| handle_socket(socket, session, state))
}
```
- [ ] **Connection Limits**: Implement configurable max concurrent WebSocket connections:
```rust
pub struct ConnectionLimiter {
    current_connections: Arc<AtomicUsize>,
    max_connections: usize,  // Configurable via env: WS_MAX_CONNECTIONS (default: 50,000)
}

impl ConnectionLimiter {
    pub fn try_acquire(&self) -> Option<ConnectionGuard> {
        let current = self.current_connections.fetch_add(1, Ordering::SeqCst);
        if current >= self.max_connections {
            self.current_connections.fetch_sub(1, Ordering::SeqCst);
            None
        } else {
            Some(ConnectionGuard { limiter: self.current_connections.clone() })
        }
    }
}

// RAII guard to automatically decrement on drop
pub struct ConnectionGuard {
    limiter: Arc<AtomicUsize>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.limiter.fetch_sub(1, Ordering::SeqCst);
    }
}
```

**Note**: With Rust's async runtime (tokio), we should easily handle 10,000-50,000+ concurrent WebSocket connections per server on modern hardware. The C10K problem is long solved, and tokio's efficiency means we're primarily limited by memory (each connection ~4-8KB overhead). On a 16GB server, 100,000+ connections is feasible. Set the limit conservatively based on your hardware, but expect high throughput.
- [ ] Implement subscription to EventBus for reservation events
- [ ] Filter events by event_id (clients subscribe to specific events)
- [ ] Push availability updates on:
  - SeatsReserved → decrement available
  - SeatsReleased → increment available
  - SeatsConfirmed → update sold
- [ ] Handle client disconnect/reconnect (graceful cleanup)
- [ ] Emit `websocket_connections_active` metric on connect/disconnect
- [ ] Test authentication (verify invalid tokens rejected)
- [ ] Test connection limits (verify 429 after configured limit)
- [ ] Load test WebSocket connections (aim for 10,000+ concurrent)
- [ ] Test with WebSocket client (websocat or browser)

**Deliverable**: Authenticated WebSocket with connection limits and live seat updates

### Phase 10.11: Integration Tests (3-4 hours)

**Goal**: Automated HTTP API testing with authentication

**Dependencies**: All above phases (comprehensive end-to-end testing)

- [ ] Create `tests/http_api_test.rs`
- [ ] Test setup: Start server with test containers (PostgreSQL + Redpanda)
- [ ] Happy path tests:
  - Create event → Publish → Open sales
  - Reserve seats → Complete payment → Verify sold
  - Check availability after reservation
  - Cancel reservation → Verify seats released
- [ ] Error case tests:
  - Reserve non-existent event → 404
  - Reserve more seats than available → 409
  - Payment after expiry → 408
- [ ] Projection consistency tests:
  - Verify projections update after events
- [ ] WebSocket tests (if feasible)

**Deliverable**: Comprehensive integration test suite

---

### Phase 10.12: Documentation & Polish (2-3 hours)

**Goal**: Developer experience and production readiness documentation

**Dependencies**: All above phases

- [ ] Create `examples/ticketing/README.md` with:
  - System architecture diagram (all 4 databases, Redis, Redpanda)
  - Complete API endpoint list (35+ endpoints)
  - curl examples for all endpoints
  - Authentication flow examples
  - WebSocket client example with token authentication
  - Environment setup instructions
  - Production deployment checklist
- [ ] **HTTPS/TLS Documentation**: Add section to README on TLS termination:
```markdown
## Production TLS Setup

### Option 1: Reverse Proxy (Recommended)
Use nginx/Caddy for TLS termination:
- nginx handles HTTPS → HTTP to localhost:8080
- Automatic certificate renewal with Let's Encrypt
- Load balancing and rate limiting

### Option 2: Native Axum TLS
Use `axum-server` with rustls:
```rust
let tls_config = RustlsConfig::from_pem_file("cert.pem", "key.pem").await?;
axum_server::bind_rustls(addr, tls_config)
    .serve(app.into_make_service())
    .await?;
```

### WebSocket over TLS
- Use `wss://` instead of `ws://`
- Token still passed via query parameter
```
- [ ] **Secrets Management Documentation**: Add section on managing sensitive data:
```markdown
## Secrets Management

### Development
- Use `.env` file (never commit to git)
- Copy `.env.example` to `.env` and fill in values

### Production
- Use environment variables (Kubernetes secrets, AWS SSM, etc.)
- PostgreSQL passwords: 32+ character random strings
- Redis password: 32+ character random string
- OAuth client secrets: From provider (GitHub, Google, etc.)
- Session encryption key: 32-byte random key

### Key Rotation
1. Generate new key
2. Add to secondary slot
3. Update .env: `SESSION_KEY_NEW=<new-key>`
4. Deploy (validates with both keys)
5. After 24h, rotate: `SESSION_KEY_PRIMARY=<new-key>`
```
- [ ] **Graceful Shutdown**: Add shutdown handler to `bin/server.rs`:
```rust
#[tokio::main]
async fn main() -> Result<()> {
    // ... setup

    let server = axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_signal());

    info!("Server started on {}", addr);
    server.await?;

    info!("Server shutting down gracefully...");
    // Allow in-flight requests to complete (30 second timeout)
    // WebSocket connections will be closed

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received");
}
```
- [ ] **CORS Configuration**: Add CORS middleware for browser clients:
```rust
use tower_http::cors::{CorsLayer, Any};

let cors = CorsLayer::new()
    .allow_origin(Any)  // For demo; restrict to specific origins in production
    .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
    .allow_headers([AUTHORIZATION, CONTENT_TYPE])
    .expose_headers([CONTENT_LENGTH, CONTENT_TYPE])
    .max_age(Duration::from_secs(3600));

let app = Router::new()
    .route("/api/*", ...)
    .layer(cors);
```
- [ ] Add OpenTelemetry tracing setup to README
- [ ] Add inline API documentation (doc comments on handlers)
- [ ] Document rate limiting configuration (100 req/15min per IP for auth)
- [ ] Add examples of monitoring queries (PostgreSQL slow query log, Redis INFO)
- [ ] Run `cargo clippy --all-targets` - fix all warnings
- [ ] Run `cargo fmt --all`
- [ ] Run `cargo test --all`
- [ ] Update top-level `CLAUDE.md` with ticketing example reference

**Deliverable**: Production-ready, documented API with comprehensive operational guidance

---

### Phase 10.13: Analytics Database (Optional) (4-6 hours)

**Goal**: Separate OLAP database with star schema for business intelligence

**Infrastructure**:
- [ ] Add PostgreSQL analytics database to Docker Compose (port 5434)
- [ ] Create analytics database migrations directory (`migrations_analytics/`)
- [ ] Implement star schema:
  - `fact_ticket_sales` table (append-only fact table)
  - `dim_events` table (event dimension with SCD Type 2)
  - `dim_customers` table (customer dimension with SCD Type 2)
  - `dim_calendar` table (time dimension for time-series analysis)
  - Indexes for common query patterns

**Analytics ETL Service**:
- [ ] Create `bin/analytics_etl.rs` binary
- [ ] Implement `AnalyticsETL` struct:
  - Subscribe to all ticketing events from Redpanda
  - Transform events into fact table inserts
  - Update dimension tables (SCD Type 2 pattern)
  - Handle event replay for backfilling
- [ ] Event handlers:
  - `ReservationCompleted` → Insert into `fact_ticket_sales`
  - `EventCreated` → Upsert `dim_events`
  - `CustomerRegistered` → Upsert `dim_customers`
  - `PaymentCompleted` → Update customer lifetime value

**Analytics Queries**:
- [ ] Implement example queries module (`src/analytics/queries.rs`):
  - Sales performance over time (time-series)
  - Top-selling events by revenue
  - Customer segmentation by lifetime value
  - Section popularity by event type
  - Weekend vs weekday sales
  - Customer cohort retention analysis

**HTTP Endpoints (Optional)**:
- [ ] `GET /api/analytics/sales/time-series` - Time-series sales data
- [ ] `GET /api/analytics/events/top-sellers` - Top events by revenue
- [ ] `GET /api/analytics/customers/segments` - Customer segmentation
- [ ] `GET /api/analytics/sections/popularity` - Section performance

**Testing**:
- [ ] Test ETL service consumes events correctly
- [ ] Test fact table inserts with valid data
- [ ] Test dimension table updates (SCD Type 2)
- [ ] Test analytics queries return correct aggregations
- [ ] Test analytics DB can be rebuilt from event stream

**Documentation**:
- [ ] Document star schema design
- [ ] Add example SQL queries to README
- [ ] Document ETL service architecture
- [ ] Add instructions for connecting BI tools (Metabase, Grafana)

**Deliverable**: Production-ready analytics database with ETL service and example queries

**Why Optional?**:
- Adds significant complexity (separate DB, ETL service)
- May not be needed for initial demo/MVP
- Can be added later without affecting operational system
- Recommended for Phase 11 if time is constrained

**Why Include in Phase 10?**:
- Showcases complete event-driven architecture
- Demonstrates OLTP vs OLAP separation
- Shows power of event sourcing (rebuild analytics anytime)
- Provides real business value (BI dashboards, reports)

### Phase 10.14: Metrics & Observability (3-4 hours)

**Goal**: Production-grade observability with Prometheus metrics and structured logging

**Metrics Infrastructure**:
- [ ] Add `prometheus` and `metrics-exporter-prometheus` crates to `Cargo.toml`
- [ ] Create `src/observability/metrics.rs`:
  - Initialize Prometheus registry
  - Define metric collectors (counters, histograms, gauges)
  - Export `/metrics` endpoint (Prometheus scrape format)
- [ ] Add `tower-http` tracing middleware for automatic HTTP metrics

**HTTP Metrics** (RED Method):
```rust
// Request Rate
http_requests_total{method="POST", path="/api/reservations", status="200"}

// Error Rate
http_requests_total{method="POST", path="/api/reservations", status="500"}

// Duration (latency)
http_request_duration_seconds{method="POST", path="/api/reservations", le="0.1"}
http_request_duration_seconds_sum
http_request_duration_seconds_count
```

**Database Metrics**:
- [ ] Connection pool metrics per database:
  ```rust
  db_connections_active{pool="event_store"}
  db_connections_idle{pool="event_store"}
  db_connections_max{pool="event_store"}
  db_query_duration_seconds{pool="event_store", query="append_events"}
  ```

**Redis Metrics**:
- [ ] Session operations:
  ```rust
  redis_commands_total{command="GET"}
  redis_commands_total{command="SET"}
  redis_command_duration_seconds{command="GET"}
  redis_connections_active
  ```

**Redpanda/EventBus Metrics**:
- [ ] Consumer lag and throughput:
  ```rust
  consumer_lag{topic="ticketing.reservations", partition="0"}
  consumer_offset{topic="ticketing.reservations", partition="0"}
  consumer_messages_processed_total{topic="ticketing.reservations"}
  consumer_errors_total{topic="ticketing.reservations"}
  ```

**WebSocket Metrics**:
- [ ] Real-time connection tracking:
  ```rust
  websocket_connections_active
  websocket_connections_total
  websocket_messages_sent_total
  websocket_messages_received_total
  websocket_connection_duration_seconds
  ```

**Business Metrics**:
- [ ] Domain event tracking:
  ```rust
  reservations_created_total
  reservations_completed_total
  reservations_expired_total
  reservations_cancelled_total
  payments_succeeded_total
  payments_failed_total
  revenue_total_cents
  ```

**OpenTelemetry Tracing** (Distributed Tracing):
- [ ] Add `tracing-opentelemetry` and `opentelemetry-jaeger` crates
- [ ] Initialize OpenTelemetry tracer:
  ```rust
  let tracer = opentelemetry_jaeger::new_pipeline()
      .with_service_name("ticketing-api")
      .install_batch(Tokio)?;

  tracing_subscriber::registry()
      .with(tracing_opentelemetry::layer().with_tracer(tracer))
      .init();
  ```
- [ ] Trace context propagation across services via Redpanda events
- [ ] Add trace IDs to all log messages (correlation)

**Structured Logging Enhancements**:
- [ ] Add correlation IDs to all requests (extract from header or generate)
- [ ] Log format standardization:
  ```json
  {
    "timestamp": "2025-11-12T15:30:00Z",
    "level": "INFO",
    "message": "Reservation created",
    "correlation_id": "abc-123",
    "trace_id": "xyz-789",
    "user_id": "user-456",
    "event_id": "evt-999",
    "duration_ms": 45
  }
  ```
- [ ] Log all auth events (login, logout, failed attempts)
- [ ] Log all business events (reservations, payments)
- [ ] Log all errors with full context

**Health Check Endpoint** (Enhanced):
- [ ] Update `/health/ready` to check ALL dependencies:
  ```rust
  async fn health_check(State(app): State<TicketingAppState>) -> Json<HealthResponse> {
      let mut checks = HashMap::new();

      // Check PostgreSQL event store
      checks.insert("event_store", app.event_store.ping().await.is_ok());

      // Check PostgreSQL projections
      checks.insert("projection_store", app.projection_store.ping().await.is_ok());

      // Check PostgreSQL analytics
      checks.insert("analytics_store", app.analytics_store.ping().await.is_ok());

      // Check Redis
      checks.insert("redis", app.redis.ping().await.is_ok());

      // Check Redpanda (try to fetch metadata)
      checks.insert("redpanda", app.event_bus.health_check().await.is_ok());

      let all_healthy = checks.values().all(|&v| v);

      Json(HealthResponse {
          status: if all_healthy { "healthy" } else { "unhealthy" },
          checks,
      })
  }
  ```

**Grafana Dashboard** (Optional):
- [ ] Provide example Grafana dashboard JSON:
  - HTTP request rate and latency
  - Database connection pool usage
  - Consumer lag over time
  - Error rate by endpoint
  - Business metrics (reservations, revenue)

**Testing**:
- [ ] Test metrics endpoint returns Prometheus format
- [ ] Test metrics update when operations happen
- [ ] Test health check fails when service is down
- [ ] Test trace IDs propagate through logs

**Documentation**:
- [ ] Document metrics available at `/metrics`
- [ ] Provide example Prometheus scrape config
- [ ] Document recommended alert thresholds
- [ ] Add Grafana dashboard import instructions

**Deliverable**: Complete observability stack ready for production monitoring

**Why Critical**:
- Cannot run production without knowing system health
- Metrics enable proactive incident detection
- Tracing enables debugging across distributed services
- Foundation for alerting and SLA monitoring

---

### Phase 10.15: Operational Runbook (2-3 hours)

**Goal**: Comprehensive operational guide for production support

**Create `RUNBOOK.md` in `examples/ticketing/`**:

**Section 1: Architecture Overview**
- [ ] System architecture diagram
- [ ] Service dependencies
- [ ] Data flow diagrams
- [ ] Port allocation reference

**Section 2: Deployment**
- [ ] Prerequisites (Docker, Rust, environment variables)
- [ ] Initial deployment steps:
  ```bash
  # 1. Start infrastructure
  docker-compose up -d

  # 2. Wait for services to be healthy
  ./scripts/wait-for-services.sh

  # 3. Run migrations
  DATABASE_URL="..." sqlx migrate run

  # 4. Bootstrap admin user
  cargo run --bin bootstrap-admin -- --email admin@example.com

  # 5. Start API server
  cargo run --bin server

  # 6. Start analytics ETL
  cargo run --bin analytics_etl
  ```
- [ ] Verify deployment checklist

**Section 3: Monitoring**
- [ ] Key metrics to watch:
  - HTTP error rate < 1%
  - P95 latency < 500ms
  - Consumer lag < 1000 messages
  - Database connection pool < 80% capacity
  - Redis memory < 80%
- [ ] Grafana dashboard URLs
- [ ] Log aggregation (where to find logs)
- [ ] Alert notification channels

**Section 4: Common Issues & Solutions**

| Issue | Symptoms | Diagnosis | Solution |
|-------|----------|-----------|----------|
| **High consumer lag** | Projections stale, analytics delayed | Check consumer lag metrics | Restart ETL service, or scale horizontally |
| **Redis out of memory** | Auth failures, session errors | Check Redis memory metrics | Scale up Redis, enable eviction policy |
| **Database connection pool exhausted** | Timeout errors, slow requests | Check pool metrics | Increase pool size, fix connection leaks |
| **PostgreSQL disk full** | Write failures, crashes | `df -h`, check disk usage | Archive old events, expand disk |
| **High HTTP latency** | Slow responses | Check P95 latency metrics | Check DB query performance, add indexes |
| **WebSocket disconnections** | Clients lose connection | Check connection metrics | Check network, increase timeout |

**Section 5: Emergency Procedures**

**Complete Outage**:
```bash
# 1. Check all services are running
docker-compose ps

# 2. Check service health
curl http://localhost:8080/health/ready

# 3. Restart services in order
docker-compose restart redis
docker-compose restart postgres-events
docker-compose restart postgres-projections
docker-compose restart postgres-analytics
docker-compose restart redpanda
cargo run --bin server
cargo run --bin analytics_etl

# 4. Verify recovery
./scripts/smoke-test.sh
```

**Data Corruption**:
```bash
# 1. Stop all writes
docker-compose stop server

# 2. Identify corruption (check logs, query DB)

# 3. Restore from backup
./scripts/restore-backup.sh --date 2025-11-12

# 4. Replay events to rebuild projections
cargo run --bin rebuild-projections

# 5. Resume operations
docker-compose start server
```

**Security Incident**:
```bash
# 1. Rotate all credentials
./scripts/rotate-credentials.sh

# 2. Invalidate all sessions
redis-cli FLUSHDB

# 3. Review audit logs
./scripts/audit-review.sh --since 1h

# 4. Notify affected users
```

**Section 6: Maintenance Tasks**

**Daily**:
- [ ] Check Grafana dashboards
- [ ] Review error logs
- [ ] Check consumer lag
- [ ] Verify backups completed

**Weekly**:
- [ ] Review security alerts
- [ ] Check database growth trends
- [ ] Review slow query logs
- [ ] Update dependencies (security patches)

**Monthly**:
- [ ] Capacity planning review
- [ ] Archive old events (if needed)
- [ ] Review and update alerts
- [ ] Disaster recovery drill

**Section 7: Backup & Restore**

**Backup Strategy**:
```bash
# PostgreSQL continuous WAL archiving
# Config in postgresql.conf:
archive_mode = on
archive_command = 'aws s3 cp %p s3://backups/wal/%f'
wal_level = replica

# Daily base backups (cron job)
0 2 * * * pg_basebackup -D /backup -Ft -z -P

# Retention: 7 daily, 4 weekly, 12 monthly
```

**Restore Procedure**:
```bash
# Point-in-time recovery
# 1. Stop PostgreSQL
systemctl stop postgresql

# 2. Clear data directory
rm -rf /var/lib/postgresql/data/*

# 3. Restore base backup
tar -xzf backup.tar.gz -C /var/lib/postgresql/data/

# 4. Create recovery.conf
restore_command = 'aws s3 cp s3://backups/wal/%f %p'
recovery_target_time = '2025-11-12 15:30:00'

# 5. Start PostgreSQL (auto-recovery)
systemctl start postgresql

# 6. Rebuild projections from events
cargo run --bin rebuild-projections --from-event 0
```

**Section 8: Scaling Guidance**

**Horizontal Scaling**:
- HTTP API: Stateless, scale to N instances behind load balancer
- Analytics ETL: Consumer group, scale to N instances (Redpanda handles partitioning)
- Projections: Shared PostgreSQL, no changes needed
- Sessions: Shared Redis, no changes needed

**Vertical Scaling**:
- PostgreSQL: Increase CPU/memory, tune connection pools
- Redis: Increase memory, enable persistence
- Redpanda: Increase disk, tune retention

**Section 9: Performance Tuning**

**PostgreSQL**:
```sql
-- Connection pooling
max_connections = 200

-- Memory
shared_buffers = 2GB
effective_cache_size = 6GB
work_mem = 50MB

-- Query performance
random_page_cost = 1.1  -- For SSD
effective_io_concurrency = 200
```

**Redis**:
```conf
maxmemory 2gb
maxmemory-policy allkeys-lru
save 900 1
save 300 10
save 60 10000
```

**Section 10: Contact Information**
- [ ] On-call engineer: [contact info]
- [ ] Escalation path
- [ ] External dependencies (AWS, Redpanda support)

**Deliverable**: Complete operational runbook ready for production support team

**Why Critical**:
- First responders need guidance during incidents
- Reduces MTTR (mean time to recovery)
- Prevents costly mistakes during emergencies
- Enables junior engineers to support production

---

### Phase 10.16: Load Testing & Performance Validation (2-3 hours)

**Goal**: Validate system performance under load and establish capacity limits

**Load Testing Tool**: Use `wrk` or `k6` for HTTP load testing

**Install Load Testing Tools**:
```bash
# wrk (simple, fast)
brew install wrk  # macOS
# or
sudo apt install wrk  # Linux

# k6 (more features, better reporting)
brew install k6
```

**Test Scenarios**:

**1. Baseline Performance Test** (Single User):
```bash
# Measure baseline latency
curl -w "@curl-format.txt" -o /dev/null -s http://localhost:8080/api/events

# curl-format.txt:
time_namelookup:  %{time_namelookup}\n
time_connect:  %{time_connect}\n
time_starttransfer:  %{time_starttransfer}\n
time_total:  %{time_total}\n
```

**Expected** (single server, development laptop):
- GET requests: < 10ms p95, < 5ms p50
- POST requests: < 100ms p95, < 50ms p50

**Note**: These are conservative baselines. With Rust + tokio + optimized PostgreSQL queries, we should see significantly better performance in practice.

**2. Read-Heavy Load** (Availability Queries):
```bash
# 100 concurrent users, 30 seconds
wrk -t4 -c100 -d30s http://localhost:8080/api/events/123/availability

# Conservative targets (single server):
# Requests/sec: > 10,000 (expect 50,000+)
# P95 latency: < 50ms
# P99 latency: < 100ms
# Error rate: 0%

# Rationale:
# - Simple SELECT from indexed projection table
# - Rust JSON serialization is extremely fast (~1-2μs)
# - PostgreSQL can handle 10,000+ simple SELECTs/sec
# - Axum overhead is minimal (<100μs per request)
# - We should be CPU-bound, not I/O-bound on reads
```

**Performance Note**: Given our architecture:
- **Projections**: Pre-computed, indexed, denormalized read models
- **Rust**: Zero-cost abstractions, no GC pauses
- **tokio**: Highly efficient async runtime
- **PostgreSQL**: Connection pooling, query optimization

We expect **read-heavy** workloads to achieve **50,000-100,000+ req/sec** on production hardware (16-core server). The 10,000 req/sec target is a conservative baseline for a development laptop.

**3. Write-Heavy Load** (Reservations):
```lua
-- reservation-load.lua for wrk
request = function()
   local body = '{"event_id":"evt-123","customer_id":"cust-456","section":"VIP","quantity":2}'
   return wrk.format("POST", "/api/reservations",
      {["Content-Type"]="application/json", ["Authorization"]="Bearer <token>"},
      body)
end
```

```bash
wrk -t4 -c50 -d30s -s reservation-load.lua http://localhost:8080
```

**Conservative targets** (writes involve event sourcing + Redpanda + projections):
- Requests/sec: > 500 (expect 2,000-5,000+)
- P95 latency: < 200ms
- P99 latency: < 500ms
- Error rate: < 0.1%
- No race conditions (check for double-booking in DB)

**Rationale**:
- Event appends to PostgreSQL: ~1-2ms (indexed, sequential writes)
- Redpanda publish: ~2-5ms (async, batched)
- Optimistic concurrency check: ~1ms (sequence number validation)
- Even with full event sourcing overhead, should handle 1,000s of writes/sec
- Limited by PostgreSQL write throughput (~10,000 TPS on good hardware)

**4. Mixed Load** (Realistic Traffic):
```javascript
// k6-mixed-load.js
import http from 'k6/http';
import { check, sleep } from 'k6';

export let options = {
  stages: [
    { duration: '1m', target: 50 },   // Ramp up
    { duration: '3m', target: 100 },  // Steady state
    { duration: '1m', target: 200 },  // Spike
    { duration: '1m', target: 0 },    // Ramp down
  ],
  thresholds: {
    http_req_duration: ['p(95)<500'], // 95% < 500ms
    http_req_failed: ['rate<0.01'],   // Error rate < 1%
  },
};

export default function() {
  // 70% reads
  if (Math.random() < 0.7) {
    let res = http.get('http://localhost:8080/api/events');
    check(res, { 'status 200': (r) => r.status === 200 });
  }
  // 30% writes
  else {
    let res = http.post('http://localhost:8080/api/reservations',
      JSON.stringify({ event_id: 'evt-123', ... }),
      { headers: { 'Content-Type': 'application/json' } }
    );
    check(res, { 'status 200 or 201': (r) => [200,201].includes(r.status) });
  }

  sleep(1);
}
```

```bash
k6 run k6-mixed-load.js
```

**5. Concurrent Reservation Test** (Race Condition Validation):
```bash
# Simulate 100 users trying to reserve the last 10 seats simultaneously
# Expected: Exactly 10 succeed, 90 get "insufficient inventory" error

./scripts/concurrent-reservation-test.sh \
  --event-id evt-123 \
  --section VIP \
  --concurrent-users 100 \
  --available-seats 10

# Verify:
# - Exactly 10 reservations created
# - No double-booking (query DB)
# - All failures got 409 Conflict
```

**6. WebSocket Load Test**:
```bash
# Simulate 1000 concurrent WebSocket connections
# Send availability update events, measure broadcast latency

node websocket-load-test.js \
  --connections 1000 \
  --duration 60s \
  --event-id evt-123

# Expected:
# - All connections established
# - Broadcast latency < 100ms p95
# - No dropped messages
# - Server memory stable
```

**7. Database Load Test**:
```sql
-- Simulate heavy write load to event store
-- Insert 10,000 events in parallel

BEGIN;
INSERT INTO events (aggregate_id, event_type, data, ...)
SELECT
  uuid_generate_v4(),
  'EventCreated',
  ...
FROM generate_series(1, 10000);
COMMIT;

-- Measure: Write throughput, lock contention, replication lag
```

**8. Redis Load Test**:
```bash
# Test Redis session store under load
redis-benchmark -h localhost -p 6379 \
  -c 100 \
  -n 100000 \
  -t get,set \
  --csv

# Expected:
# - > 50,000 operations/sec
# - Latency < 1ms p95
```

**9. Consumer Lag Test**:
```bash
# Publish 10,000 events to Redpanda rapidly
# Measure how quickly projection/analytics ETL catches up

./scripts/publish-burst.sh --events 10000

# Monitor consumer lag metrics:
curl http://localhost:8080/metrics | grep consumer_lag

# Expected:
# - Lag < 1000 messages within 10 seconds
# - No consumer crashes
# - Projections eventually consistent
```

**Performance Baseline Documentation**:
- [ ] Create `PERFORMANCE.md` with **actual measured results** (template below shows expected ranges):

```markdown
# Performance Baseline

**Test Hardware**: [Record actual hardware specs: CPU cores, RAM, SSD type, etc.]
**Test Date**: [Date]
**Load Testing Tool**: wrk 4.2.0 / k6 0.48.0

## HTTP API Performance

### Read Endpoints (Projection Queries)
- **Throughput**: [MEASURED] req/sec (expect 10,000-50,000+ on production hardware)
- **Latency**:
  - P50: [MEASURED] ms (expect < 5ms)
  - P95: [MEASURED] ms (expect < 50ms)
  - P99: [MEASURED] ms (expect < 100ms)
- **Concurrent users**: [MEASURED] sustained (expect 5,000-10,000+)

### Write Endpoints (Event Sourcing)
- **Throughput**: [MEASURED] req/sec (expect 500-5,000+ depending on hardware)
- **Latency**:
  - P50: [MEASURED] ms (expect < 50ms)
  - P95: [MEASURED] ms (expect < 200ms)
  - P99: [MEASURED] ms (expect < 500ms)

### WebSocket Connections
- **Max concurrent**: [MEASURED] connections (expect 10,000-50,000+ per server)
- **Memory per connection**: [MEASURED] KB
- **CPU overhead**: [MEASURED]% at max connections

## Database Performance

### PostgreSQL Event Store
- **Write throughput**: [MEASURED] events/sec (bottleneck: disk write speed)
- **Event append latency**: [MEASURED] ms (expect 1-5ms with SSD)
- **Connection pool**: [MEASURED] max, [MEASURED] avg active

### PostgreSQL Projections
- **Query throughput**: [MEASURED] queries/sec (expect 10,000-50,000+ simple SELECTs)
- **Query latency**: [MEASURED] ms (expect < 5ms for indexed queries)
- **Connection pool**: [MEASURED] max, [MEASURED] avg active

## Redis
- Session operations: 60,000 ops/sec
- Memory usage: 500MB at 10,000 sessions

## Redpanda
- Event throughput: [MEASURED] events/sec (expect 5,000-50,000+ depending on configuration)
- Consumer lag: [MEASURED] ms p95 (expect < 100ms)

## Capacity Limits (Empirically Determined)
- **Max concurrent reservations**: [MEASURED]/sec (bottleneck: PostgreSQL write throughput)
- **Max WebSocket connections**: [MEASURED] (expect 10,000-50,000+ per server, configurable)
- **Database growth rate**: [MEASURED] MB/day at [MEASURED] events/sec
- **Memory footprint**: [MEASURED] GB at steady state

## Performance Philosophy

**🎯 Key Point**: The numbers above are **expectations based on architecture**, not hard limits. Phase 10.16's goal is to **empirically discover actual performance** through load testing.

**Why we expect high performance**:
1. **Rust + tokio**: Zero-cost abstractions, no GC pauses, highly efficient async runtime
2. **CQRS projections**: Pre-computed read models eliminate complex queries
3. **Event sourcing**: Sequential append-only writes are PostgreSQL's sweet spot
4. **Optimized data structures**: Indexed queries, connection pooling, batching

**What we'll measure**:
- Actual throughput on your hardware (not theoretical maximums)
- Real latency distributions (P50, P95, P99, P999)
- Bottlenecks (CPU, memory, disk I/O, network)
- Breaking points (where does performance degrade?)
- Concurrency limits (race conditions, deadlocks)

**The results may surprise us** (hopefully in a good way) - Rust is **fast**. 🚀
```

**Bottleneck Analysis**:
- [ ] Identify bottlenecks from load tests (CPU-bound vs I/O-bound)
- [ ] Document optimization opportunities (if any)
- [ ] Set realistic SLA targets based on **measured** performance
- [ ] Compare actual results vs expectations (validate architecture assumptions)

**Load Test Automation**:
- [ ] Add load tests to CI/CD (optional, for regression testing)
- [ ] Or document manual load test procedure for pre-release validation
- [ ] Run before each major release to catch performance regressions

**Deliverable**: Performance baseline documented with **actual measured data**, system limits empirically validated, capacity planning guidance

**Why Critical**:
- Must know **actual** system capacity before production (not guesses)
- Validate no race conditions under concurrent load (concurrent reservation test)
- Establish data-driven SLA targets (e.g., "99.9% uptime, P95 < 100ms")
- Prevents surprises during traffic spikes
- **Proves** the architecture delivers on performance promises

**Success Criteria**:
- [ ] Zero race conditions detected (no double-booking)
- [ ] Performance meets or exceeds expectations
- [ ] Bottlenecks identified and documented
- [ ] Capacity limits known with confidence
- [ ] PERFORMANCE.md created with reproducible test procedures

---

### Phase 10.17: Web Frontend (Composable Svelte) - Integrated Deployment (15-20 hours)

**Goal**: Build production-ready web frontend using Composable Svelte with integrated single-deployment architecture

**Architecture Decision**: **Single deployment unit** where Rust backend serves the built Svelte frontend (not separate deployments)

**File Structure**:
```
examples/ticketing/
├── Cargo.toml                 # Rust backend
├── src/                       # Rust source
│   ├── main.rs               # Updated to serve static files
│   ├── api/                  # REST endpoints
│   ├── websocket.rs
│   └── ...
├── web/                       # Svelte frontend SOURCE
│   ├── src/
│   │   ├── app/
│   │   │   ├── app.reducer.ts
│   │   │   ├── app.types.ts
│   │   │   └── App.svelte
│   │   ├── features/
│   │   │   ├── tickets/
│   │   │   │   ├── ticket-list.reducer.ts
│   │   │   │   ├── ticket-list.types.ts
│   │   │   │   ├── TicketList.svelte
│   │   │   │   ├── TicketDetail.svelte
│   │   │   │   └── TicketForm.svelte
│   │   │   ├── events/
│   │   │   │   ├── event-list.reducer.ts
│   │   │   │   ├── EventList.svelte
│   │   │   │   └── EventDetail.svelte
│   │   │   ├── reservations/
│   │   │   │   ├── reservation.reducer.ts
│   │   │   │   ├── ReservationList.svelte
│   │   │   │   └── CheckoutFlow.svelte
│   │   │   └── auth/
│   │   │       ├── auth.reducer.ts
│   │   │       ├── Login.svelte
│   │   │       └── Register.svelte
│   │   ├── api/
│   │   │   ├── client.ts           # APIClient configuration
│   │   │   ├── websocket.ts        # WebSocket setup
│   │   │   └── types.ts            # API types (matches Rust)
│   │   ├── lib/
│   │   │   ├── stores.ts           # Global stores
│   │   │   └── utils.ts
│   │   └── server/
│   │       └── index.ts            # Fastify SSR server (optional)
│   ├── static/                     # Static assets
│   ├── package.json
│   ├── vite.config.ts
│   ├── svelte.config.js
│   └── tsconfig.json
├── static/                    # BUILT frontend assets (from web/dist)
│   ├── index.html
│   ├── assets/
│   │   ├── index-[hash].js
│   │   └── index-[hash].css
│   └── favicon.ico
├── Dockerfile                 # Multi-stage build (frontend + backend)
├── docker-compose.yml         # Updated to include single ticketing service
├── fly.toml                   # Single Fly.io app configuration
└── README.md                  # Updated with frontend dev instructions
```

**Implementation Steps**:

**Step 1: Update Rust Backend to Serve Static Files** (1 hour)
```rust
// src/main.rs
use axum::{Router, routing::get};
use tower_http::services::ServeDir;

#[tokio::main]
async fn main() -> Result<()> {
    let app = Router::new()
        // API routes (all under /api prefix)
        .route("/api/tickets", get(list_tickets))
        .route("/api/tickets/:id", get(get_ticket))
        .route("/api/events", get(list_events))
        .route("/api/reservations", post(create_reservation))
        .route("/ws", get(websocket_handler))

        // Health checks
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))

        // Metrics
        .route("/metrics", get(metrics_handler))

        // Serve built frontend (must be last)
        // Development: Run `cd web && npm run dev` separately
        // Production: Serve from static/
        .nest_service("/", ServeDir::new("static").not_found_service(
            ServeFile::new("static/index.html")  // SPA fallback
        ))

        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

**Cargo.toml additions**:
```toml
[dependencies]
tower-http = { version = "0.6", features = ["fs", "cors"] }
```

**Step 2: Initialize Composable Svelte Frontend** (1 hour)
```bash
cd examples/ticketing
npm create vite@latest web -- --template svelte-ts
cd web
npm install
npm install @composable-svelte/core
npm install -D @sveltejs/vite-plugin-svelte
```

**Step 3: Configure Vite with Backend Proxy** (30 min)
```typescript
// web/vite.config.ts
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],

  server: {
    port: 5173,
    proxy: {
      // Proxy API requests to Rust backend
      '/api': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
      '/ws': {
        target: 'ws://localhost:8080',
        ws: true,
      },
      '/health': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
    },
  },

  build: {
    outDir: '../static',  // Build directly into Rust's static dir
    emptyOutDir: true,
    sourcemap: false,  // Disable in production
  },
});
```

**Step 4: Create API Client and Types** (2 hours)
```typescript
// web/src/api/types.ts
// TypeScript types matching Rust API

export interface Ticket {
  id: string;
  event_id: string;
  customer_id: string;
  section: string;
  row: string;
  seat: number;
  price_cents: number;
  status: 'reserved' | 'confirmed' | 'cancelled';
  reserved_at: string;
  confirmed_at?: string;
}

export interface Event {
  id: string;
  name: string;
  venue: string;
  date: string;
  total_capacity: number;
  sections: Section[];
  status: 'draft' | 'published' | 'sales_open' | 'sales_closed' | 'cancelled';
}

export interface Section {
  name: string;
  rows: number;
  seats_per_row: number;
  price_cents: number;
  available: number;
}

export interface CreateReservationRequest {
  event_id: string;
  customer_id: string;
  section: string;
  quantity: number;
  specific_seats?: { row: string; seat: number }[];
}

export interface CreateReservationResponse {
  reservation_id: string;
  ticket_ids: string[];
  expires_at: string;
}
```

```typescript
// web/src/api/client.ts
import { createAPIClient } from '@composable-svelte/core/api';
import type { APIClient } from '@composable-svelte/core/api';

export const api: APIClient = createAPIClient({
  baseURL: import.meta.env.PROD ? '' : 'http://localhost:8080',  // Prod: same origin
  timeout: 30000,

  interceptors: {
    request: (config) => {
      // Add auth token from localStorage
      const token = localStorage.getItem('auth_token');
      if (token) {
        config.headers = {
          ...config.headers,
          Authorization: `Bearer ${token}`,
        };
      }
      return config;
    },

    response: (response) => response,

    error: (error) => {
      if (error.response?.status === 401) {
        // Redirect to login
        localStorage.removeItem('auth_token');
        window.location.href = '/login';
      }
      throw error;
    },
  },
});
```

```typescript
// web/src/api/websocket.ts
import { createWebSocketClient } from '@composable-svelte/core/websocket';
import type { WebSocketClient } from '@composable-svelte/core/websocket';

export const ws: WebSocketClient = createWebSocketClient({
  url: import.meta.env.PROD
    ? `${window.location.protocol === 'https:' ? 'wss:' : 'ws:'}//${window.location.host}/ws`
    : 'ws://localhost:8080/ws',

  reconnect: {
    enabled: true,
    maxAttempts: 10,
    delay: 1000,
    maxDelay: 30000,
  },

  heartbeat: {
    enabled: true,
    interval: 30000,
    timeout: 5000,
  },

  onConnect: () => {
    console.log('WebSocket connected');
  },

  onDisconnect: () => {
    console.log('WebSocket disconnected');
  },

  onError: (error) => {
    console.error('WebSocket error:', error);
  },
});
```

**Step 5: Implement Ticket Feature** (3-4 hours)
```typescript
// web/src/features/tickets/ticket-list.types.ts
import type { Ticket } from '../../api/types';

export interface TicketListState {
  tickets: Ticket[];
  loading: boolean;
  error: string | null;
  selectedTicket: Ticket | null;
}

export type TicketListAction =
  | { type: 'LoadTickets' }
  | { type: 'TicketsLoaded'; tickets: Ticket[] }
  | { type: 'LoadFailed'; error: string }
  | { type: 'SelectTicket'; ticket: Ticket }
  | { type: 'DeselectTicket' }
  | { type: 'TicketUpdated'; ticket: Ticket };  // From WebSocket
```

```typescript
// web/src/features/tickets/ticket-list.reducer.ts
import { Effect } from '@composable-svelte/core';
import type { EffectType } from '@composable-svelte/core';
import type { APIClient } from '@composable-svelte/core/api';
import type { TicketListState, TicketListAction } from './ticket-list.types';

interface Dependencies {
  api: APIClient;
}

export function ticketListReducer(
  state: TicketListState,
  action: TicketListAction,
  deps: Dependencies
): [TicketListState, EffectType<TicketListAction>] {
  switch (action.type) {
    case 'LoadTickets':
      return [
        { ...state, loading: true, error: null },
        Effect.api(
          deps.api,
          { endpoint: '/api/tickets', method: 'GET' },
          (tickets) => ({ type: 'TicketsLoaded', tickets }),
          (error) => ({ type: 'LoadFailed', error: error.message })
        ),
      ];

    case 'TicketsLoaded':
      return [
        { ...state, tickets: action.tickets, loading: false },
        Effect.none(),
      ];

    case 'LoadFailed':
      return [
        { ...state, loading: false, error: action.error },
        Effect.none(),
      ];

    case 'SelectTicket':
      return [
        { ...state, selectedTicket: action.ticket },
        Effect.none(),
      ];

    case 'DeselectTicket':
      return [
        { ...state, selectedTicket: null },
        Effect.none(),
      ];

    case 'TicketUpdated':  // Real-time update from WebSocket
      return [
        {
          ...state,
          tickets: state.tickets.map((t) =>
            t.id === action.ticket.id ? action.ticket : t
          ),
          selectedTicket:
            state.selectedTicket?.id === action.ticket.id
              ? action.ticket
              : state.selectedTicket,
        },
        Effect.none(),
      ];

    default:
      return [state, Effect.none()];
  }
}
```

```svelte
<!-- web/src/features/tickets/TicketList.svelte -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { createStore } from '@composable-svelte/core';
  import { ticketListReducer } from './ticket-list.reducer';
  import { api } from '../../api/client';
  import { ws } from '../../api/websocket';
  import type { TicketListState } from './ticket-list.types';

  const initialState: TicketListState = {
    tickets: [],
    loading: false,
    error: null,
    selectedTicket: null,
  };

  const store = createStore({
    initialState,
    reducer: ticketListReducer,
    dependencies: { api },
  });

  // Subscribe to WebSocket updates
  onMount(() => {
    ws.subscribe((message) => {
      if (message.type === 'event' && message.action.type === 'TicketUpdated') {
        store.dispatch(message.action);
      }
    });

    // Load tickets on mount
    store.dispatch({ type: 'LoadTickets' });
  });

  $: state = $store;  // Svelte 5 reactive state
</script>

<div class="ticket-list">
  <h2>My Tickets</h2>

  {#if state.loading}
    <p>Loading tickets...</p>
  {:else if state.error}
    <p class="error">{state.error}</p>
  {:else if state.tickets.length === 0}
    <p>No tickets found.</p>
  {:else}
    <ul>
      {#each state.tickets as ticket (ticket.id)}
        <li
          class:selected={state.selectedTicket?.id === ticket.id}
          on:click={() => store.dispatch({ type: 'SelectTicket', ticket })}
        >
          <div class="ticket-info">
            <h3>Event: {ticket.event_id}</h3>
            <p>Section: {ticket.section}, Row: {ticket.row}, Seat: {ticket.seat}</p>
            <p>Status: <span class="status-{ticket.status}">{ticket.status}</span></p>
            <p>Price: ${(ticket.price_cents / 100).toFixed(2)}</p>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .ticket-list {
    padding: 1rem;
  }

  ul {
    list-style: none;
    padding: 0;
  }

  li {
    border: 1px solid #ccc;
    padding: 1rem;
    margin-bottom: 0.5rem;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  li:hover,
  li.selected {
    background-color: #f0f0f0;
  }

  .error {
    color: red;
  }

  .status-reserved {
    color: orange;
  }

  .status-confirmed {
    color: green;
  }

  .status-cancelled {
    color: red;
  }
</style>
```

**Step 6: Implement Event and Reservation Features** (3-4 hours)
- Similar pattern to tickets
- EventList, EventDetail components
- ReservationFlow (multi-step checkout)
- Payment integration UI

**Step 7: Implement Authentication Flow** (2-3 hours)
```typescript
// web/src/features/auth/auth.reducer.ts
// Handle login, logout, session management
// Store token in localStorage
// Redirect on auth failures
```

**Step 8: Update Docker Compose** (1 hour)
```yaml
# docker-compose.yml
services:
  postgres-events:
    # ... (existing)

  postgres-projections:
    # ... (existing)

  postgres-analytics:
    # ... (existing)

  postgres-auth:
    # ... (existing)

  redis:
    # ... (existing)

  redpanda:
    # ... (existing)

  ticketing:
    build:
      context: .
      dockerfile: Dockerfile
    ports:
      - "8080:8080"  # Single port for API + frontend
    environment:
      DATABASE_URL_EVENTS: postgres://postgres:password@postgres-events:5432/ticketing_events
      DATABASE_URL_PROJECTIONS: postgres://postgres:password@postgres-projections:5433/ticketing_projections
      DATABASE_URL_ANALYTICS: postgres://postgres:password@postgres-analytics:5434/ticketing_analytics
      DATABASE_URL_AUTH: postgres://postgres:password@postgres-auth:5435/ticketing_auth
      REDIS_URL: redis://redis:6379
      REDPANDA_BROKERS: redpanda:9092
      RUST_LOG: info
    depends_on:
      - postgres-events
      - postgres-projections
      - postgres-analytics
      - postgres-auth
      - redis
      - redpanda

volumes:
  postgres_events_data:
  postgres_projections_data:
  postgres_analytics_data:
  postgres_auth_data:
  redis_data:
  redpanda_data:
```

**Step 9: Create Multi-Stage Dockerfile** (2 hours)
```dockerfile
# Dockerfile

# ============================================
# Stage 1: Build Frontend
# ============================================
FROM node:20-alpine AS frontend-builder

WORKDIR /web

# Install dependencies (cached layer)
COPY web/package*.json ./
RUN npm ci --prefer-offline --no-audit

# Copy source and build
COPY web/ ./
RUN npm run build
# Output: /web/dist/ → will be copied to static/

# ============================================
# Stage 2: Build Backend
# ============================================
FROM rust:1.85-bookworm AS backend-builder

WORKDIR /app

# Install dependencies (cached layer)
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src

# Copy source
COPY src/ ./src/
COPY migrations/ ./migrations/

# Copy built frontend into static/
COPY --from=frontend-builder /web/dist ./static

# Build backend (release mode)
RUN cargo build --release

# ============================================
# Stage 3: Runtime
# ============================================
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && \
    apt-get install -y ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=backend-builder /app/target/release/ticketing /usr/local/bin/ticketing

# Copy static files
COPY --from=backend-builder /app/static /app/static

# Copy migrations (for runtime migration)
COPY --from=backend-builder /app/migrations /app/migrations

# Expose port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:8080/health/ready || exit 1

# Run
CMD ["ticketing"]
```

**Step 10: Update Documentation** (1 hour)
```markdown
# README.md

## Development

### Backend Development
```bash
# Terminal 1: Start dependencies (PostgreSQL, Redis, Redpanda)
docker-compose up postgres-events postgres-projections redis redpanda

# Terminal 2: Run backend
cargo run
```

### Frontend Development
```bash
# Terminal 3: Run Vite dev server (hot reload)
cd web && npm run dev
```

Visit:
- Frontend: http://localhost:5173
- API: http://localhost:8080/api
- WebSocket: ws://localhost:8080/ws

### Production Build
```bash
# Build everything
docker-compose build

# Run production
docker-compose up

# Visit: http://localhost:8080 (frontend + API on same port)
```

## Deployment

### Fly.io Production Deployment
```bash
fly deploy
```

The frontend is built at Docker build time and served by the Rust backend in production.
```

**Step 11: Create Fly.io Configuration** (1 hour)
```toml
# fly.toml
app = "ticketing-production"
primary_region = "sjc"

[build]
  dockerfile = "Dockerfile"

[env]
  RUST_LOG = "info"
  PORT = "8080"

[[services]]
  internal_port = 8080
  protocol = "tcp"

  [[services.ports]]
    handlers = ["http"]
    port = 80

  [[services.ports]]
    handlers = ["tls", "http"]
    port = 443

  [[services.http_checks]]
    interval = "10s"
    timeout = "2s"
    grace_period = "5s"
    method = "GET"
    path = "/health/ready"

# PostgreSQL (use Fly Postgres)
[[statics]]
  guest_path = "/app/static"
  url_prefix = "/"
```

**Step 12: Testing** (2 hours)
- [ ] Manual testing of all UI flows
- [ ] WebSocket real-time updates working
- [ ] Authentication flow working
- [ ] Mobile responsive design
- [ ] Browser testing (Chrome, Firefox, Safari)
- [ ] Docker Compose end-to-end test

**Deliverables**:
- [ ] Full Svelte frontend application
- [ ] Type-safe API integration (TypeScript types match Rust)
- [ ] Real-time WebSocket updates working
- [ ] Single deployment unit (Dockerfile + docker-compose.yml)
- [ ] Production-ready Fly.io configuration
- [ ] Development documentation (README)
- [ ] No separate deployment coordination needed

**Development Workflow**:
```bash
# Development: Hot reload for both backend and frontend
# Terminal 1
cargo watch -x run

# Terminal 2
cd web && npm run dev

# Terminal 3
docker-compose up postgres redis redpanda

# Visit: http://localhost:5173 (Vite proxies API to :8080)
```

**Production Workflow**:
```bash
# Single command: Everything up
docker-compose up

# Or deploy to Fly.io
fly deploy

# Visit: http://localhost:8080 (or https://ticketing-production.fly.dev)
```

**Success Criteria**:
- [ ] Frontend application built and integrated
- [ ] All CRUD operations working through UI
- [ ] Real-time updates via WebSocket functional
- [ ] Authentication flow complete (login, logout, session management)
- [ ] Responsive design (mobile, tablet, desktop)
- [ ] Single Docker Compose command starts everything
- [ ] Single Fly.io deployment for production
- [ ] Documentation complete with development/production instructions

**Why This Architecture?**:
1. **Simplicity**: One deployment, one URL, one bill
2. **Version Coordination**: Frontend/backend always in sync
3. **Performance**: Axum serves static files efficiently (zero-copy with tower-http)
4. **Developer Experience**: `docker-compose up` → everything works
5. **Production-Ready**: Single Fly.io app, simple scaling
6. **Cost-Effective**: One app instance, not two
7. **No CORS Issues**: Same-origin requests in production

---

## Testing Strategy

### Unit Tests (Existing)

- ✅ 36 aggregate tests already exist
- ✅ 2 integration tests with testcontainers
- No additional unit tests needed (domain logic already tested)

### HTTP Integration Tests (New)

**Test Structure**:
```rust
#[tokio::test]
async fn test_full_reservation_flow() {
    // Setup: Start test server with PostgreSQL + Redpanda
    let app = setup_test_app().await;
    let client = TestClient::new(app);

    // 1. Create event
    let event = client.post("/api/events")
        .json(&create_event_request())
        .send().await.unwrap();
    assert_eq!(event.status(), 200);
    let event_id = event.json::<EventResponse>().event_id;

    // 2. Publish event and open sales
    client.post(&format!("/api/events/{event_id}/publish")).send().await.unwrap();
    client.post(&format!("/api/events/{event_id}/sales/open")).send().await.unwrap();

    // 3. Check availability
    let availability = client.get(&format!("/api/events/{event_id}/availability"))
        .send().await.unwrap()
        .json::<AvailabilityResponse>();
    assert_eq!(availability.sections["VIP"].available, 100);

    // 4. Reserve seats
    let reservation = client.post("/api/reservations")
        .json(&CreateReservationRequest {
            event_id,
            customer_id: CustomerId::new(),
            section: "VIP".to_string(),
            quantity: 5,
            specific_seats: None,
        })
        .send().await.unwrap()
        .json::<ReservationResponse>();

    // 5. Verify availability decreased
    let availability = client.get(&format!("/api/events/{event_id}/availability"))
        .send().await.unwrap()
        .json::<AvailabilityResponse>();
    assert_eq!(availability.sections["VIP"].available, 95);

    // 6. Complete payment
    let payment = client.post(&format!("/api/reservations/{}/payment", reservation.reservation_id))
        .json(&CompletePaymentRequest { payment_id: PaymentId::new() })
        .send().await.unwrap();
    assert_eq!(payment.status(), 200);

    // 7. Verify seats confirmed (sold)
    let analytics = client.get(&format!("/api/events/{event_id}/analytics"))
        .send().await.unwrap()
        .json::<SalesAnalyticsResponse>();
    assert_eq!(analytics.tickets_sold, 5);
}
```

**Test Coverage**:
- [ ] Happy path: Create event → Reserve → Pay → Confirm
- [ ] Timeout path: Reserve → Wait 5min → Verify released
- [ ] Cancellation path: Reserve → Cancel → Verify released
- [ ] Double-booking prevention: Concurrent reservation attempts
- [ ] Error cases: 404, 409, 408, 422 responses
- [ ] Projection consistency: Verify read models update

### Manual Testing

**Tools**:
- `curl` - Command-line HTTP client
- `httpie` - User-friendly HTTP client
- `websocat` - WebSocket client

**Test checklist** (examples/ticketing/TESTING.md):
- [ ] Create event
- [ ] List events
- [ ] Publish event
- [ ] Check availability
- [ ] Reserve seats (verify 5-min expiry)
- [ ] Complete payment
- [ ] Check analytics
- [ ] Cancel reservation
- [ ] WebSocket subscription

---

## Production Considerations

### Observability

**Logging**:
- ✅ Already using `tracing` crate
- [ ] Add structured logging for HTTP requests (correlation IDs)
- [ ] Log all command executions with context
- [ ] Log projection updates

**Metrics** (Future - Phase 11):
- HTTP request rate (by endpoint)
- Request latency (p50, p95, p99)
- Error rate (by status code)
- Reservation conversion rate (reserved → confirmed)
- Seat utilization per event

**Tracing**:
- [ ] Distributed tracing with OpenTelemetry (optional)
- [ ] Trace reservation saga across multiple aggregates

### Performance

**Bottlenecks**:
- Read queries are in-memory (fast)
- Write commands involve PostgreSQL (moderate)
- Event bus publishing is async (non-blocking)

**Optimization opportunities** (Phase 12):
- Connection pooling (already configured in TicketingApp)
- Batch event publishing
- Projection caching (already in-memory)
- Read replicas for projections (PostgreSQL-backed projections)

### Reliability

**Error Handling**:
- Graceful degradation: If event bus fails, writes still succeed to PostgreSQL
- Projection lag: Read models may be slightly behind writes (eventual consistency)
- Idempotency: Aggregate commands should be idempotent (event sourcing ensures this)

**Graceful Shutdown**:
- [ ] Drain in-flight HTTP requests (30-second timeout)
- [ ] Close EventBus subscription gracefully
- [ ] Close PostgreSQL connection pool

**Monitoring**:
- Health check endpoints for Kubernetes liveness/readiness probes
- DLQ monitoring (if effects fail)

### Security (Implemented in Phase 10)

**Implemented**:
- ✅ **Authentication**: Full auth system (magic link, OAuth, passkey)
- ✅ **Session management**: Redis-backed sessions with TTL
- ✅ **Authorization**: Role-based access control (Admin vs Customer)
- ✅ **Ownership validation**: Users can only access their own resources
- ✅ **Rate limiting**: Redis-based rate limiter (via composable-rust-auth)
- ✅ **Input validation**: Quantity > 0, valid UUIDs, session validation
- ✅ **Secure session storage**: Redis with automatic expiry
- ✅ **Password-less auth**: WebAuthn (phishing-resistant) and magic links

**Security boundaries**:
- Auth required for all write operations (create, update, delete)
- Public read endpoints: Event list, availability (no auth needed)
- Ownership checks: Users can only view/modify their own reservations/payments
- Admin-only endpoints: Event management, analytics, refunds
- WebSocket auth: Session validation before upgrade

**Still deferred**:
- HTTPS/TLS (assume reverse proxy/load balancer handles this)
- Advanced RBAC (fine-grained permissions beyond admin/customer)
- Audit logging (track who accessed what, when)
- IP-based blocking/allowlisting

### Scalability (Production-Ready)

**Phase 10 design**:
- ✅ PostgreSQL-backed projections (shared state across instances)
- ✅ Stateless HTTP handlers (horizontal scaling ready)
- ✅ Redis for sessions (shared across instances)
- ✅ Separate event store and projection databases

**Ready for horizontal scaling**:
- Multiple HTTP server instances can share:
  - Event store (PostgreSQL write-side)
  - Projection store (PostgreSQL read-side)
  - Redis sessions/tokens
  - Redpanda event bus

**Phase 11 improvements**:
- Load balancer (Nginx, HAProxy, AWS ALB)
- Read replicas for projection database
- Redis cluster (high availability)
- Connection pooling tuning for multiple instances

---

## Future Enhancements

### Phase 11: Advanced Production Features
- Audit logging (track all resource access with timestamps)
- Advanced RBAC (fine-grained permissions beyond admin/customer)
- IP-based rate limiting and blocking
- Circuit breakers for external dependencies
- Retry policies with exponential backoff
- Load balancer setup (Nginx, HAProxy, AWS ALB)
- Read replicas for projection database

### Phase 12: Developer Experience
- OpenAPI/Swagger documentation generation
- Client SDKs (TypeScript, Python, Go)
- Postman collection
- Interactive API explorer

### Phase 14: Advanced Features
- Ticket transfer (change ownership)
- Waitlist management (notify when seats available)
- Dynamic pricing (adjust based on demand)
- Multi-event bundles (buy tickets for series)
- Seat selection UI (interactive seating chart)

---

## Decisions Made (User Input)

All key architectural decisions have been finalized based on user feedback:

1. **Projection Durability**: ✅ **DECIDED**
   - **Decision**: Use PostgreSQL for projections (separate database from event store)
   - **Rationale**: Production-ready, durable, supports horizontal scaling
   - **Implementation**: Phase 10.3 - Separate PostgreSQL instance on port 5433

2. **Authentication**: ✅ **DECIDED**
   - **Decision**: Full authentication in Phase 10 (not deferred)
   - **Methods**: Magic link, OAuth, Passkey (WebAuthn)
   - **Storage**: Redis for sessions/tokens, PostgreSQL for users/devices
   - **Implementation**: Phases 10.0-10.2

3. **WebSocket Authentication**: ✅ **DECIDED**
   - **Decision**: Validate session before WebSocket upgrade
   - **Method**: Session ID in query parameter (`?session_id=xxx`)
   - **Rationale**: Simple, secure, works with WebSocket protocol
   - **Implementation**: Phase 10.10

4. **Error Response Details**: ✅ **DECIDED**
   - **Decision**: Detailed error messages (this is a demo)
   - **Format**: JSON with `code`, `message`, optional `details`
   - **Rationale**: Developer-friendly, helps with debugging and learning
   - **Implementation**: Phase 10.4 (error handling module)

5. **Event Bus**: ✅ **DECIDED**
   - **Decision**: Redpanda for event bus (as originally planned)
   - **Rationale**: Kafka-compatible, self-hostable, production-ready
   - **Implementation**: Already in use, no changes needed

6. **Pagination Strategy**: 📝 **DEFERRED**
   - **Decision**: Use offset/limit for Phase 10 (simpler)
   - **Future**: Cursor-based pagination in Phase 11 if needed
   - **Rationale**: Offset/limit is sufficient for demo and initial production use

---

## Timeline & Effort Estimate

| Phase | Estimated Time | Priority | Dependencies |
|-------|----------------|----------|--------------|
| 10.0 - Infrastructure Setup | 3-4 hours | P0 (blocking) | None |
| 10.1 - Authentication Setup | 4-5 hours | P0 (blocking) | 10.0 |
| 10.2 - Auth Middleware | 2-3 hours | P0 (blocking) | 10.1 |
| 10.3 - PostgreSQL Projections | 3-4 hours | P0 (blocking) | 10.0 |
| 10.4 - HTTP Foundation | 2-3 hours | P0 (blocking) | 10.1, 10.2 |
| 10.5 - Event Management | 2-3 hours | P0 (blocking) | 10.2, 10.4 |
| 10.6 - Availability Endpoints | 1-2 hours | P0 (blocking) | 10.3, 10.4 |
| 10.7 - Reservation Endpoints | 3-4 hours | P0 (blocking) | 10.2, 10.4 |
| 10.8 - Payment Endpoints | 1-2 hours | P0 (blocking) | 10.2, 10.4 |
| 10.9 - Analytics Endpoints | 1-2 hours | P1 (nice to have) | 10.2, 10.4 |
| 10.10 - WebSocket Real-Time | 2-3 hours | P1 (nice to have) | 10.1, 10.4 |
| 10.11 - Integration Tests | 3-4 hours | P0 (blocking) | All above |
| 10.12 - Documentation & Polish | 2-3 hours | P0 (blocking) | All above |
| 10.13 - Analytics Database (Optional) | 4-6 hours | P2 (optional) | 10.0, 10.1 |
| 10.14 - Metrics & Observability | 3-4 hours | P0 (blocking) | 10.4 |
| 10.15 - Operational Runbook | 2-3 hours | P0 (blocking) | All above |
| 10.16 - Load Testing | 2-3 hours | P0 (blocking) | All above |
| 10.17 - Web Frontend (Composable Svelte) | 15-20 hours | P1 (optional) | 10.4, 10.16 |
| **Total (Core Backend)** | **42-55 hours** | | |
| **Total (with Analytics)** | **46-61 hours** | | |
| **Total (Backend Production-Ready)** | **50-66 hours** | | |
| **Total (Full-Stack with Frontend)** | **65-86 hours** | | |

**Critical path**: 10.0 → 10.1 → 10.2 → 10.4 → 10.5 → 10.7 → 10.8 → 10.11 → 10.14 → 10.15 → 10.16 (~32-42 hours)

**Work breakdown**:
- **Infrastructure & Auth**: 9-12 hours (Phases 10.0-10.2)
  - Docker Compose setup (6 services)
  - Event versioning, backup strategies
  - Authentication system (Redis + PostgreSQL)
  - Rate limiting and middleware
- **HTTP API Implementation**: 10-14 hours (Phases 10.3-10.9)
  - CQRS projections setup
  - 35+ HTTP endpoints with authorization
  - WebSocket real-time updates
- **Testing & Quality**: 5-7 hours (Phases 10.10-10.11)
  - Integration tests with testcontainers
  - WebSocket authentication tests
- **Production Hardening**: 10-13 hours (Phases 10.12, 10.14-10.16)
  - Comprehensive documentation (TLS, secrets, graceful shutdown, CORS)
  - Prometheus metrics and OpenTelemetry tracing
  - Operational runbook (10 sections)
  - Load testing and performance validation
- **Analytics (Optional)**: 4-6 hours (Phase 10.13)
  - Star schema OLAP database
  - ETL service for business intelligence
- **Web Frontend (Optional)**: 15-20 hours (Phase 10.17)
  - Composable Svelte integration
  - TypeScript API client
  - Real-time WebSocket UI
  - Multi-stage Docker build
  - Single deployment unit

**Estimated calendar time**:
- **Core backend**: 5-7 days of focused work
- **Backend production-ready**: 6-9 days of focused work
- **Full-stack with frontend**: 8-11 days of focused work
- **Part-time backend**: 3-4 weeks
- **Part-time full-stack**: 4-6 weeks

**Production Readiness Milestones**:
1. ✅ **Day 1-2**: Infrastructure + Auth (Phases 10.0-10.2) - ~9-12 hours
2. ✅ **Day 3-4**: HTTP API Implementation (Phases 10.3-10.9) - ~10-14 hours
3. ✅ **Day 5**: Testing (Phases 10.10-10.11) - ~5-7 hours
4. ✅ **Day 6-7**: Production Hardening (Phases 10.12, 10.14-10.16) - ~10-13 hours
5. 🔹 **Optional**: Analytics (Phase 10.13) - ~4-6 hours
6. 🔹 **Optional**: Web Frontend (Phase 10.17) - ~15-20 hours (Days 8-10 if included)

---

## Success Metrics

After Phase 10 completion:

**Authentication**:
- [ ] Full auth system operational (magic link, OAuth, passkey)
- [ ] Redis storing sessions with TTL
- [ ] PostgreSQL storing users and devices
- [ ] Session validation working in protected endpoints
- [ ] WebSocket authentication validating sessions

**HTTP API**:
- [ ] 35+ HTTP endpoints operational (10 auth + 25 ticketing)
- [ ] All write operations require authentication
- [ ] Ownership validation working (users can only access their resources)
- [ ] Admin vs customer role separation working
- [ ] Public read endpoints accessible without auth

**Infrastructure**:
- [ ] PostgreSQL event store running (port 5432)
- [ ] PostgreSQL projection store running (port 5433)
- [ ] PostgreSQL analytics store running (port 5434)
- [ ] Redis running (port 6379)
- [ ] Redpanda running (port 9092)
- [ ] Projections persisting to PostgreSQL and surviving restarts
- [ ] Analytics ETL service consuming events and populating star schema
- [ ] Fact and dimension tables populated with historical data

**Quality**:
- [ ] Zero clippy warnings
- [ ] All integration tests passing (auth + ticketing flows)
- [ ] Health checks returning correct status for all 6 dependencies
- [ ] Structured logging with correlation IDs
- [ ] Error messages detailed and developer-friendly
- [ ] Input validation on all endpoints
- [ ] CORS configured for browser clients
- [ ] Graceful shutdown implemented (30s timeout)

**Observability** (Phase 10.14):
- [ ] Prometheus metrics endpoint at `/metrics`
- [ ] HTTP metrics (RED method): requests/sec, error rate, latency
- [ ] Database metrics: connection pool usage (4 pools)
- [ ] Redis metrics: command counts, hit rate
- [ ] Redpanda metrics: consumer lag monitoring
- [ ] WebSocket metrics: active connections (configurable limit, default 50,000)
- [ ] Business metrics: reservations_created_total, revenue_total_cents
- [ ] OpenTelemetry tracing configured (Jaeger)
- [ ] Distributed trace IDs in logs
- [ ] Key metrics dashboards defined

**Operational Readiness** (Phase 10.15):
- [ ] Complete RUNBOOK.md with 10 sections
- [ ] Common issues table with solutions (6+ scenarios)
- [ ] Emergency procedures documented (outage, corruption, security)
- [ ] Backup/restore procedures documented and tested
- [ ] PostgreSQL WAL archiving configured
- [ ] Redis persistence (RDB + AOF) configured
- [ ] Contact information and escalation paths defined
- [ ] Monitoring thresholds documented (error rate < 1%, P95 < 500ms, lag < 1000)
- [ ] Performance tuning parameters documented

**Performance** (Phase 10.16):
- [ ] Load testing completed with wrk/k6
- [ ] Baseline performance documented (conservative targets for development laptop):
  - **Read endpoints**: > 10,000 req/sec (expect 50,000+ on production hardware), P95 < 50ms
  - **Write endpoints**: > 500 req/sec (expect 2,000-5,000+), P95 < 200ms
  - **Concurrent users**: 5,000 sustained, 10,000 peak
  - **Max concurrent reservations**: 1,000-5,000/sec (bottleneck: PostgreSQL write throughput)
  - **Max WebSocket connections**: 10,000-50,000 per server (configurable, memory-limited)
- [ ] Concurrent reservation test validates no double-booking (race condition testing)
- [ ] Consumer lag under load < 1,000 messages
- [ ] Database connection pools properly sized (empirically determined)
- [ ] PERFORMANCE.md created with actual measured capacity limits

**Note**: These are **conservative baselines**. The goal of Phase 10.16 is to **empirically discover** the actual limits through load testing. We've architected this system for high performance:
- Rust's zero-cost abstractions and tokio's efficiency
- Optimized CQRS read models (pre-computed projections)
- Event sourcing with sequential writes (append-only)
- We expect to **significantly exceed** these targets on production hardware

**Security**:
- [ ] Rate limiting on all auth endpoints (100 req/15min per IP)
- [ ] Session tokens validated on every request
- [ ] Ownership checks prevent unauthorized access
- [ ] Admin-only endpoints reject non-admin users
- [ ] Secrets management documented (development vs production)
- [ ] TLS/HTTPS setup documented
- [ ] SQL injection prevention (parameterized queries)
- [ ] XSS prevention (proper JSON serialization)

**Documentation**:
- [ ] README with Docker Compose setup
- [ ] curl examples for all common workflows
- [ ] Authentication guide (how to login and use sessions)
- [ ] API endpoint reference (35+ endpoints)
- [ ] TLS/HTTPS setup guide (nginx reverse proxy)
- [ ] Secrets management guide (key rotation)
- [ ] Graceful shutdown implementation
- [ ] CORS configuration examples
- [ ] OpenTelemetry setup instructions
- [ ] Monitoring and alerting guide

**Web Frontend** (Phase 10.17 - Optional):
- [ ] Composable Svelte application integrated
- [ ] TypeScript types match Rust API (type-safe)
- [ ] All CRUD operations working through UI
- [ ] Real-time WebSocket updates functional
- [ ] Authentication flow complete (login, logout, session management)
- [ ] Responsive design (mobile, tablet, desktop)
- [ ] Multi-stage Dockerfile builds frontend + backend
- [ ] Single deployment unit (one docker-compose up)
- [ ] Vite dev server proxies API to backend (development workflow)
- [ ] Frontend documentation (development + production setup)

**Definition of Done** (Functional - Backend API):
Developer can:
1. Register/login using magic link, OAuth, or passkey
2. Create events as admin (with admin token)
3. Make reservations as customer (with customer token)
4. Complete payment flow
5. See real-time updates via WebSocket (with session token)
6. Query analytics as admin
7. All with proper authorization and error handling

**Definition of Done** (Functional - with Web Frontend, Phase 10.17):
End user can:
1. Register/login through web UI (magic link, OAuth, passkey)
2. Browse events and view availability
3. Make reservations through interactive seat selection
4. Complete checkout and payment flow
5. View their tickets in real-time (updates via WebSocket)
6. Admin can manage events through web dashboard
7. Mobile-responsive experience on all devices

**Definition of Done** (Production):
Operations team can:
1. Deploy system using Docker Compose
2. Monitor system health via `/health/ready` endpoint
3. View metrics in Prometheus at `/metrics`
4. Trace requests through distributed tracing (Jaeger)
5. Respond to incidents using RUNBOOK.md
6. Restore from backup after data corruption
7. Validate system performance via load tests
8. Scale system using documented capacity limits

---

## Next Steps

1. ✅ **Plan approved** - All architecture decisions finalized
2. ✅ **Scope confirmed** - Phase 10 includes full auth + HTTP API + PostgreSQL projections + production hardening
3. 🚀 **Begin implementation** - Start with Phase 10.0 (Infrastructure Setup)
4. **Iterate** - Adjust implementation based on discoveries during development
5. **Production checklist** - Verify all observability, operational readiness, and performance criteria

**Implementation order** (16 phases):
1. **Phase 10.0** (3-4h): Infrastructure setup (Docker Compose, event versioning, backups, Redis persistence)
2. **Phase 10.1** (4-5h): Authentication system (Redis sessions, PostgreSQL users, rate limiting)
3. **Phase 10.2** (2-3h): Auth middleware (RequireAuth, RequireAdmin, RequireOwnership)
4. **Phase 10.3** (3-4h): PostgreSQL projections (separate DB, ETL service, idempotency)
5. **Phase 10.4** (2-3h): HTTP foundation (Axum server, health checks, state management)
6. **Phase 10.5** (2-3h): Event management endpoints (CRUD with auth)
7. **Phase 10.6** (1-2h): Availability endpoints (projection queries)
8. **Phase 10.7** (3-4h): Reservation endpoints (saga with auth)
9. **Phase 10.8** (1-2h): Payment endpoints
10. **Phase 10.9** (1-2h): Analytics endpoints
11. **Phase 10.10** (2-3h): WebSocket with authentication and connection limits
12. **Phase 10.11** (3-4h): Integration tests (testcontainers)
13. **Phase 10.12** (2-3h): Documentation (TLS, secrets, graceful shutdown, CORS)
14. **Phase 10.13** (4-6h): Analytics database (OLAP star schema) - **OPTIONAL**
15. **Phase 10.14** (3-4h): Metrics & observability (Prometheus, OpenTelemetry)
16. **Phase 10.15** (2-3h): Operational runbook (10 sections, emergency procedures)
17. **Phase 10.16** (2-3h): Load testing & performance validation
18. **Phase 10.17** (15-20h): Web frontend (Composable Svelte) - integrated deployment

---

**Plan Status**: ✅ **Approved - Ready for Production Implementation**

**Key Decisions**:
- ✅ Four PostgreSQL databases (events, projections, analytics, auth)
- ✅ Full authentication in Phase 10 (magic link, OAuth, passkeys)
- ✅ Redis for sessions/tokens with persistence (RDB + AOF)
- ✅ WebSocket with session validation and connection limits (50,000 default, configurable)
- ✅ Detailed error messages (demo-friendly)
- ✅ Redpanda for event bus
- ✅ Analytics ETL with star schema (OLAP-optimized)
- ✅ **Production hardening**: Metrics, runbook, load testing
- ✅ **Event versioning**: Schema evolution support
- ✅ **Backup strategies**: PostgreSQL WAL + Redis persistence
- ✅ **Observability**: Prometheus + OpenTelemetry + structured logging
- ✅ **Operational excellence**: Complete runbook, emergency procedures
- ✅ **Web Frontend** (Phase 10.17): Composable Svelte with integrated deployment (single Docker build)

**Infrastructure Summary**:
- 4x PostgreSQL (ports 5432, 5433, 5434, 5435) - Events, Projections, Analytics, Auth
- 1x Redis (port 6379) - Sessions, tokens, rate limiting
- 1x Redpanda (port 9092) - Event bus
- **Total**: 6 services in Docker Compose

**Production Capabilities**:
- ✅ **Authentication**: Magic link, OAuth 2.0, WebAuthn passkeys
- ✅ **Authorization**: Role-based (Admin, Customer, Owner) + resource ownership
- ✅ **Observability**: RED metrics, distributed tracing, structured logging
- ✅ **Resilience**: Rate limiting, graceful shutdown, configurable connection limits
- ✅ **Operations**: Complete runbook, backup/restore, monitoring thresholds
- ✅ **Performance**: Load tested to empirically determine limits (expect 10,000-50,000+ req/sec reads, 2,000-5,000+ req/sec writes on production hardware)
- ✅ **Scalability**: High-performance Rust + tokio (10,000-50,000+ concurrent WebSocket connections per server)
- ✅ **Security**: TLS/HTTPS support, secrets management, SQL injection prevention

**Estimated Timeline**:
- **Core system** (without analytics): 42-55 hours (5-7 days focused work)
- **With analytics** (Phase 10.13): 46-61 hours (6-8 days focused work)
- **Full production-ready** (all phases): 50-66 hours (6-9 days focused work)
- **Part-time**: 3-4 weeks

**What Makes This Production-Ready**:
1. **Complete observability**: Can see what's happening (metrics, tracing, logs)
2. **Operational runbook**: Team knows how to respond to incidents
3. **Performance validated**: Know exact capacity limits via load testing
4. **Backup/restore**: Can recover from disasters
5. **Security hardened**: Rate limiting, auth, TLS, secrets management
6. **Graceful degradation**: Health checks, graceful shutdown, error handling
