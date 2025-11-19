# Production Roadmap: Ticketing Application

**Date Created**: 2025-11-19
**Current Status**: 7.5/10 - Production-Ready Architecture with Implementation Gaps
**Goal**: 10/10 - Deploy to production with confidence

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Current State Assessment](#current-state-assessment)
3. [Phase 13: Critical Production Blockers](#phase-13-critical-production-blockers)
4. [Phase 14: Production Hardening](#phase-14-production-hardening)
5. [Phase 15: Observability & Operations](#phase-15-observability--operations)
6. [Phase 16: Security & Compliance](#phase-16-security--compliance)
7. [Phase 17: Performance & Scale](#phase-17-performance--scale)
8. [Phase 18: Documentation & Launch](#phase-18-documentation--launch)
9. [Success Criteria](#success-criteria)
10. [Rollback Plan](#rollback-plan)

---

## Executive Summary

The ticketing application has **excellent architectural foundations** but requires **4-6 days of focused work** to reach production readiness. The codebase demonstrates professional software engineering practices with CQRS, event sourcing, and saga patterns correctly implemented.

### Timeline to Production

| Phase | Duration | Priority | Status |
|-------|----------|----------|--------|
| **Phase 13** | 1-2 days | P0 - Critical | 🔴 Blockers |
| **Phase 14** | 1 day | P0 - Critical | 🟡 Required |
| **Phase 15** | 1 day | P1 - High | 🟢 Recommended |
| **Phase 16** | 0.5 day | P1 - High | 🟢 Recommended |
| **Phase 17** | 0.5 day | P2 - Medium | ⚪ Optional |
| **Phase 18** | 0.5 day | P0 - Critical | 🔴 Blockers |

**Minimum for production**: Phases 13, 14, 18 (2.5-3.5 days)
**Recommended for production**: All phases (4-6 days)

---

## Current State Assessment

### Strengths (What's Working) ✅

1. **Architecture** (9/10)
   - CQRS/Event Sourcing flawlessly implemented
   - Saga pattern with compensation works correctly
   - Type-safe domain modeling with newtype wrappers
   - Clean separation of concerns

2. **Code Quality** (8/10)
   - Zero unsafe blocks
   - Zero panics in production code
   - Proper Result propagation
   - Comprehensive testing (15+ test files)

3. **Infrastructure** (8/10)
   - Docker Compose with health checks
   - Multi-stage Dockerfile
   - 4 PostgreSQL databases + Redis + Redpanda
   - Proper WAL archiving for event store

4. **Bootstrap Architecture** (10/10)
   - Just refactored: 814 lines → 68 lines (92% reduction)
   - Framework-level reusable components
   - Declarative ApplicationBuilder API
   - Zero code duplication

### Gaps (What's Missing) ⚠️

1. **Incomplete API Endpoints** (High Priority)
   - 8 stub handlers returning placeholder responses
   - Missing CRUD operations for events
   - Missing payment query endpoints

2. **Health Checks** (High Priority)
   - Placeholder implementation
   - Doesn't verify infrastructure health

3. **TODO Comments** (Medium Priority)
   - 17 TODO comments throughout codebase
   - Admin authorization incomplete
   - Request lifecycle tracking incomplete

4. **Production Readiness** (Medium Priority)
   - In-memory projections lose data on restart
   - No event versioning scheme
   - Simple WebSocket rate limiting
   - No comprehensive metrics/observability

---

## Phase 13: Critical Production Blockers

**Duration**: 1-2 days
**Priority**: P0 - Must complete before production
**Goal**: Eliminate all features that would cause production failures

### 13.1: Implement Health Check Endpoints

**File**: `src/server/health.rs`
**Current**: Returns 200 OK without verification
**Required**: Actual dependency health checks

#### Tasks

- [ ] **13.1.1**: Health check for PostgreSQL event store
  - File: `src/server/health.rs:67`
  - Implementation: `SELECT 1` query with timeout
  - Failure response: 503 Service Unavailable

- [ ] **13.1.2**: Health check for PostgreSQL projections DB
  - Implementation: `SELECT 1` query with timeout
  - Failure response: 503 Service Unavailable

- [ ] **13.1.3**: Health check for PostgreSQL auth DB
  - Implementation: `SELECT 1` query with timeout
  - Failure response: 503 Service Unavailable

- [ ] **13.1.4**: Health check for Redpanda event bus
  - Implementation: Producer/consumer connectivity test
  - Failure response: 503 Service Unavailable

- [ ] **13.1.5**: Health check for Redis cache
  - Implementation: PING command
  - Failure response: 503 Service Unavailable

- [ ] **13.1.6**: Aggregate health status
  - Return JSON with individual component statuses
  - Overall healthy only if all components healthy

**Success Criteria**:
- Health endpoint returns accurate status for all 5 dependencies
- Integration test verifies health checks detect actual failures
- Load balancer can use health endpoint for traffic routing

**Files Modified**:
- `src/server/health.rs`
- `tests/health_check_test.rs` (new)

---

### 13.2: Complete Event API Endpoints

**Files**: `src/api/events.rs`
**Current**: 4 stub handlers with TODO comments
**Required**: Full CRUD operations

#### Tasks

- [ ] **13.2.1**: Implement `get_event` (line 238-250)
  - Query event from event aggregate or projection
  - Verify event exists
  - Return 404 if not found
  - Add ownership verification (if needed for privacy)

- [ ] **13.2.2**: Implement `list_events` (line 251-280)
  - Query all events (or paginated)
  - Support filtering by status, date range
  - Return events as JSON array
  - Add pagination (page, limit)

- [ ] **13.2.3**: Implement `update_event` (line 291-309)
  - Verify ownership via `RequireOwnership` extractor
  - Send `UpdateEvent` action to event aggregate
  - Handle validation errors
  - Return updated event

- [ ] **13.2.4**: Implement `delete_event` (line 323-329)
  - Verify ownership via `RequireOwnership` extractor
  - Send `DeleteEvent` or `CancelEvent` action
  - Handle cascade (reservations, payments)
  - Return 204 No Content on success

- [ ] **13.2.5**: Add event aggregate support for update/delete
  - Update `EventAction` enum with UpdateEvent, DeleteEvent
  - Implement validation in reducer
  - Add events to event stream
  - Update projections

**Success Criteria**:
- All 4 event endpoints work end-to-end
- Integration tests pass for CRUD operations
- Ownership verification prevents unauthorized access
- Proper error responses (400, 403, 404)

**Files Modified**:
- `src/api/events.rs`
- `src/aggregates/event.rs`
- `tests/event_crud_test.rs`

---

### 13.3: Complete Payment API Endpoints

**Files**: `src/api/payments.rs`
**Current**: 3 stub handlers with TODO comments
**Required**: Full payment query and refund operations

#### Tasks

- [ ] **13.3.1**: Implement `get_payment` (line 247-270)
  - Query payment projection or aggregate
  - Verify ownership (user can only see their payments)
  - Return payment details
  - Return 404 if not found

- [ ] **13.3.2**: Implement `list_user_payments` (line 213-240)
  - Query all payments for authenticated user
  - Support pagination (page, limit)
  - Filter by status (pending, completed, failed, refunded)
  - Return as JSON array

- [ ] **13.3.3**: Implement `refund_payment` (line 312-330)
  - Verify ownership (user or admin)
  - Send RefundPayment action to payment aggregate
  - Handle idempotency (already refunded)
  - Trigger compensation (release seats via saga)
  - Return 200 with refunded payment

- [ ] **13.3.4**: Add payment aggregate refund support
  - Add RefundPayment action to PaymentAction enum
  - Implement refund validation in reducer
  - Emit PaymentRefunded event
  - Update payment projection

- [ ] **13.3.5**: Extend reservation saga for refund compensation
  - Listen for PaymentRefunded event
  - Trigger seat release (ReservationCancelled)
  - Update inventory projection

**Success Criteria**:
- All 3 payment endpoints work end-to-end
- Refunds properly trigger seat release
- Integration tests cover refund saga
- Ownership verification prevents unauthorized access

**Files Modified**:
- `src/api/payments.rs`
- `src/aggregates/payment.rs`
- `src/app/coordinator.rs` (saga)
- `tests/payment_refund_test.rs` (new)

---

### 13.4: Resolve All TODO Comments

**Priority**: Medium (can defer some to Phase 14)
**Current**: 17 TODO comments
**Required**: Either implement or document decision to defer

#### High Priority TODOs (Must Complete)

- [ ] **13.4.1**: `src/auth/middleware.rs:238` - Admin role checking
  - Implement `check_admin_role()` helper
  - Query user roles from auth store
  - Return 403 Forbidden if not admin

- [ ] **13.4.2**: `src/projections/payments_postgres.rs:346` - Extract customer_id
  - Parse customer_id from reservation data
  - Update payment projection schema if needed

- [ ] **13.4.3**: `src/projections/query_adapters.rs:110,151` - Implement queries
  - Complete `get_reservation_by_id()`
  - Complete `get_payment_by_id()`

#### Medium Priority TODOs (Can Defer to Phase 14)

- [ ] **13.4.4**: `src/auth/handlers.rs:211` - Device fingerprint
  - Document decision: Defer to Phase 14 (optional security)

- [ ] **13.4.5**: `tests/full_deployment_test.rs:430,447` - Request lifecycle
  - Document decision: Defer to Phase 14 (advanced feature)

- [ ] **13.4.6**: `src/request_lifecycle/store.rs:39` - Execute Delay effects
  - Document decision: Defer to Phase 14 (advanced feature)

**Success Criteria**:
- All P0 TODOs resolved (implemented or deferred with documentation)
- No TODO comments in critical paths (auth, API handlers, aggregates)
- Deferred TODOs documented in Phase 14 plan

**Files Modified**:
- Various (see TODO locations)
- `plans/phase-12/DEFERRED_TODOS.md` (new)

---

## Phase 14: Production Hardening

**Duration**: 1 day
**Priority**: P0 - Required for production
**Goal**: Ensure reliability, data integrity, and error recovery

### 14.1: Persist In-Memory Projections

**Files**: `src/projections/sales_analytics.rs`, `src/projections/customer_history.rs`
**Current**: Data lost on restart
**Required**: PostgreSQL backup storage

#### Tasks

- [ ] **14.1.1**: Create PostgreSQL schema for SalesAnalyticsProjection
  - Table: `sales_analytics`
  - Columns: event_id, total_revenue, total_tickets, tickets_by_tier (JSONB)
  - Migration: `migrations_projections/006_create_sales_analytics.sql`

- [ ] **14.1.2**: Implement dual-write for SalesAnalyticsProjection
  - Write to in-memory HashMap AND PostgreSQL
  - Read from in-memory for performance
  - Load from PostgreSQL on startup

- [ ] **14.1.3**: Create PostgreSQL schema for CustomerHistoryProjection
  - Table: `customer_history`
  - Columns: user_id, reservation_ids (JSONB), payment_ids (JSONB)
  - Migration: `migrations_projections/007_create_customer_history.sql`

- [ ] **14.1.4**: Implement dual-write for CustomerHistoryProjection
  - Write to in-memory HashMap AND PostgreSQL
  - Read from in-memory for performance
  - Load from PostgreSQL on startup

- [ ] **14.1.5**: Add startup reconstruction from PostgreSQL
  - Load all analytics into memory on bootstrap
  - Load all customer history into memory on bootstrap
  - Add progress logging for large datasets

**Success Criteria**:
- Application restart preserves all projection data
- Performance remains fast (in-memory reads)
- Integration test verifies persistence across restarts

**Files Modified**:
- `src/projections/sales_analytics.rs`
- `src/projections/customer_history.rs`
- `migrations_projections/006_create_sales_analytics.sql` (new)
- `migrations_projections/007_create_customer_history.sql` (new)
- `tests/projection_persistence_test.rs` (new)

---

### 14.2: Implement Event Versioning

**Files**: `core/src/event.rs`, all aggregates
**Current**: No version tracking, schema evolution unsafe
**Required**: Version field in all events

#### Tasks

- [ ] **14.2.1**: Add version field to SerializedEvent
  - Update `SerializedEvent` struct in core
  - Add `event_version: u32` field
  - Default to version 1 for all current events

- [ ] **14.2.2**: Update event storage schema
  - Migration: Add `event_version` column to `events` table
  - Backfill existing events with version 1
  - Make `event_version` non-nullable

- [ ] **14.2.3**: Version all event types
  - Add `#[serde(tag = "version")]` to event enums
  - Document current version as v1
  - Create versioning guide in docs/

- [ ] **14.2.4**: Implement version-aware deserialization
  - Support reading multiple versions
  - Add migration functions (v1 → v2)
  - Test backward compatibility

**Success Criteria**:
- All events stored with version number
- Can read old and new event versions
- Migration path documented for future changes

**Files Modified**:
- `core/src/event.rs`
- `postgres/src/lib.rs`
- `migrations/002_add_event_version.sql` (new)
- `docs/event-versioning.md` (new)

---

### 14.3: Implement Dead Letter Queue (DLQ)

**Files**: `src/runtime/consumer.rs`
**Current**: Failed events logged but not persisted
**Required**: DLQ table for failed events with retry mechanism

#### Tasks

- [ ] **14.3.1**: Create DLQ database schema
  - Table: `event_dead_letter_queue`
  - Columns: id, event_id, topic, payload, error, attempts, first_failed_at, last_failed_at
  - Index on topic, first_failed_at

- [ ] **14.3.2**: Implement DLQ writer in EventConsumer
  - After max retries exceeded, write to DLQ
  - Store full event data + error message
  - Track retry attempts

- [ ] **14.3.3**: Add DLQ admin API endpoints
  - GET /admin/dlq - List failed events
  - POST /admin/dlq/{id}/retry - Retry single event
  - POST /admin/dlq/retry-all - Retry all DLQ events
  - DELETE /admin/dlq/{id} - Delete (acknowledge) failed event

- [ ] **14.3.4**: Implement manual retry mechanism
  - Re-publish event to event bus
  - Track retry in DLQ table
  - Remove from DLQ on success

**Success Criteria**:
- Failed events never lost (stored in DLQ)
- Admin can retry failed events via API
- Integration test verifies DLQ workflow

**Files Modified**:
- `src/runtime/consumer.rs`
- `src/api/admin.rs` (new)
- `migrations_projections/008_create_dlq.sql` (new)
- `tests/dlq_test.rs` (new)

---

### 14.4: Enhanced Error Handling

**Files**: Various
**Current**: Basic error handling
**Required**: Comprehensive error types and recovery

#### Tasks

- [ ] **14.4.1**: Create domain-specific error types
  - `EventError`, `ReservationError`, `PaymentError`
  - Implement proper error context
  - Add error codes for client consumption

- [ ] **14.4.2**: Implement error response middleware
  - Map errors to HTTP status codes
  - Include error codes in JSON response
  - Add request ID to all error responses

- [ ] **14.4.3**: Add structured error logging
  - Log errors with full context (user_id, event_id, etc.)
  - Use tracing::error! with structured fields
  - Include stack traces for unexpected errors

- [ ] **14.4.4**: Implement graceful degradation
  - If projections fail, serve from event store
  - If analytics fail, return 503 with retry-after
  - Add circuit breakers for external dependencies

**Success Criteria**:
- All errors have structured types
- Error responses include helpful error codes
- Errors properly logged with context

**Files Modified**:
- `src/api/errors.rs` (new)
- `src/api/middleware/error_handler.rs` (new)
- All API handlers

---

## Phase 15: Observability & Operations

**Duration**: 1 day
**Priority**: P1 - Highly recommended for production
**Goal**: Enable production monitoring and troubleshooting

### 15.1: Integrate Metrics Framework

**Files**: `src/metrics.rs` (exists but not integrated)
**Current**: Metrics module exists but unused
**Required**: Metrics in all critical paths

#### Tasks

- [ ] **15.1.1**: Add request duration metrics
  - Middleware to track all HTTP requests
  - Histogram for response times
  - Label by endpoint, method, status code

- [ ] **15.1.2**: Add business metrics
  - Counter: events_created, reservations_created, payments_processed
  - Gauge: active_reservations, pending_payments
  - Histogram: saga_completion_time

- [ ] **15.1.3**: Add infrastructure metrics
  - Gauge: database_connections (event_store, projections, auth)
  - Counter: event_bus_messages_published, event_bus_messages_consumed
  - Histogram: event_processing_duration

- [ ] **15.1.4**: Add error metrics
  - Counter: errors_total (label by type, handler)
  - Counter: dlq_events_total
  - Counter: saga_compensation_total

- [ ] **15.1.5**: Expose Prometheus endpoint
  - GET /metrics - Prometheus format
  - Include all application metrics
  - Include Rust runtime metrics (tokio, memory)

**Success Criteria**:
- Prometheus can scrape /metrics endpoint
- All critical operations instrumented
- Can build Grafana dashboard from metrics

**Files Modified**:
- `src/metrics.rs`
- `src/api/middleware/metrics.rs` (new)
- All API handlers
- All aggregates
- `docker-compose.yml` (add Prometheus + Grafana)

---

### 15.2: Distributed Tracing

**Files**: Various
**Current**: Basic logging
**Required**: Structured tracing with correlation IDs

#### Tasks

- [ ] **15.2.1**: Add request ID middleware
  - Generate UUID for each request
  - Propagate in X-Request-ID header
  - Include in all log messages

- [ ] **15.2.2**: Implement trace context propagation
  - Add trace_id and span_id to all operations
  - Propagate through event bus messages
  - Link saga steps with parent trace

- [ ] **15.2.3**: Add detailed span instrumentation
  - Span for each API handler
  - Span for each reducer execution
  - Span for each database query
  - Span for each event bus publish/consume

- [ ] **15.2.4**: Configure OpenTelemetry export (optional)
  - Support for Jaeger/Zipkin
  - Configurable sampling rate
  - Environment-based configuration

**Success Criteria**:
- Can trace single request through entire system
- Can correlate logs across distributed operations
- Can visualize saga execution in tracing UI

**Files Modified**:
- `src/api/middleware/tracing.rs` (new)
- `src/runtime/consumer.rs`
- All aggregates
- `.env.example` (OpenTelemetry config)

---

### 15.3: Operational Dashboards

**Files**: Infrastructure
**Current**: No dashboards
**Required**: Grafana dashboards for monitoring

#### Tasks

- [ ] **15.3.1**: Create system health dashboard
  - Panel: HTTP request rate, latency, error rate
  - Panel: Database connection pool usage
  - Panel: Event bus lag (consumer offset vs. producer)
  - Panel: Health check status

- [ ] **15.3.2**: Create business metrics dashboard
  - Panel: Events created (time series)
  - Panel: Reservations created, completed, cancelled
  - Panel: Payments processed, revenue
  - Panel: Saga success vs. compensation rate

- [ ] **15.3.3**: Create error tracking dashboard
  - Panel: Error rate by type
  - Panel: DLQ size over time
  - Panel: Failed saga executions
  - Panel: Circuit breaker state

- [ ] **15.3.4**: Configure alerting rules
  - Alert: Health check failing > 1 minute
  - Alert: Error rate > 5% for 5 minutes
  - Alert: DLQ size > 100 events
  - Alert: Event bus lag > 1000 messages

**Success Criteria**:
- Grafana dashboards visualize all key metrics
- Alerts configured for critical failures
- On-call can troubleshoot using dashboards

**Files Created**:
- `grafana/dashboards/system-health.json`
- `grafana/dashboards/business-metrics.json`
- `grafana/dashboards/error-tracking.json`
- `grafana/alerts/production.yml`
- `docker-compose.yml` (add Grafana)

---

### 15.4: Log Aggregation

**Files**: Infrastructure
**Current**: Console logs only
**Required**: Centralized log storage and search

#### Tasks

- [ ] **15.4.1**: Add structured JSON logging
  - Switch to JSON formatter for production
  - Include all context fields (user_id, event_id, trace_id)
  - Configure log levels per module

- [ ] **15.4.2**: Set up log aggregation (optional)
  - Add Loki to docker-compose (or use cloud service)
  - Configure log shipping
  - Set retention policy (30 days)

- [ ] **15.4.3**: Create common log queries
  - Query: All errors for user_id
  - Query: All operations for trace_id
  - Query: Saga execution timeline
  - Query: Slow queries (>1s)

**Success Criteria**:
- All logs in structured JSON format
- Can search logs by user_id, trace_id, error type
- Logs retained for troubleshooting window

**Files Modified**:
- `src/bootstrap/builder.rs` (JSON log formatter)
- `.env.example` (log level config)
- `docker-compose.yml` (Loki, optional)

---

## Phase 16: Security & Compliance

**Duration**: 0.5 day
**Priority**: P1 - Highly recommended for production
**Goal**: Harden security and meet compliance requirements

### 16.1: Complete Admin Authorization

**Files**: `src/auth/middleware.rs`
**Current**: TODO comments for admin checks
**Required**: Full RBAC implementation

#### Tasks

- [ ] **16.1.1**: Implement role storage
  - Add `user_roles` table to auth database
  - Migration: Create roles table with user_id, role
  - Seed admin user (from environment variable)

- [ ] **16.1.2**: Implement `check_admin_role()` (line 238)
  - Query user roles from auth store
  - Cache roles in session/JWT claims
  - Return 403 Forbidden if not admin

- [ ] **16.1.3**: Implement admin override for profiles (line 489)
  - Allow admin to view any user profile
  - Log all admin access (audit trail)
  - Add `is_admin_override` flag to audit logs

- [ ] **16.1.4**: Protect admin endpoints
  - DLQ management: Require admin role
  - Metrics endpoint: Require admin or monitoring role
  - Health endpoint: Public (for load balancer)

**Success Criteria**:
- Only admin users can access protected endpoints
- All admin actions logged to audit trail
- Integration tests verify RBAC enforcement

**Files Modified**:
- `src/auth/middleware.rs`
- `src/auth/roles.rs` (new)
- `migrations_auth/003_create_roles.sql` (new)
- `tests/admin_authorization_test.rs`

---

### 16.2: Rate Limiting

**Files**: `src/api/middleware/rate_limit.rs` (new)
**Current**: Simple WebSocket counter
**Required**: Comprehensive per-user rate limiting

#### Tasks

- [ ] **16.2.1**: Implement per-user rate limiting
  - Use Redis for distributed rate limiting
  - Algorithm: Token bucket or sliding window
  - Limits: 100 req/min per user (configurable)

- [ ] **16.2.2**: Implement per-IP rate limiting
  - Fallback for unauthenticated requests
  - Limits: 20 req/min per IP (configurable)
  - Consider X-Forwarded-For header

- [ ] **16.2.3**: Enhance WebSocket rate limiting
  - Per-user connection limit (5 connections)
  - Per-user message rate limit (100 msg/min)
  - Graceful disconnect with error message

- [ ] **16.2.4**: Add rate limit response headers
  - X-RateLimit-Limit: Total requests allowed
  - X-RateLimit-Remaining: Requests remaining
  - X-RateLimit-Reset: Timestamp when limit resets
  - Return 429 Too Many Requests when exceeded

**Success Criteria**:
- Users cannot exceed rate limits
- Rate limits enforced across multiple instances (via Redis)
- Proper 429 responses with retry-after headers

**Files Modified**:
- `src/api/middleware/rate_limit.rs` (new)
- `src/api/websocket.rs`
- `tests/rate_limit_test.rs` (new)

---

### 16.3: Audit Logging

**Files**: `src/audit/` (new)
**Current**: Event sourcing provides implicit audit
**Required**: Explicit audit trail with user attribution

#### Tasks

- [ ] **16.3.1**: Create audit log schema
  - Table: `audit_logs`
  - Columns: id, timestamp, user_id, action, resource_type, resource_id, ip_address, user_agent, metadata (JSONB)
  - Index on user_id, timestamp, action

- [ ] **16.3.2**: Implement audit middleware
  - Log all state-changing operations (POST, PUT, DELETE)
  - Include user context (user_id, IP, user agent)
  - Include request/response summary

- [ ] **16.3.3**: Add audit queries
  - GET /admin/audit - Query audit logs
  - Filter by user_id, action, resource_type, date range
  - Paginated results

- [ ] **16.3.4**: Add compliance reports
  - User activity report (all actions by user)
  - Resource access report (who accessed what)
  - Admin action report (all admin operations)

**Success Criteria**:
- All state changes logged with user attribution
- Audit logs queryable via admin API
- Compliance reports available for export

**Files Modified**:
- `src/audit/mod.rs` (new)
- `src/audit/middleware.rs` (new)
- `src/api/admin/audit.rs` (new)
- `migrations_projections/009_create_audit_logs.sql` (new)

---

### 16.4: Security Headers

**Files**: `src/api/middleware/security.rs` (new)
**Current**: No security headers
**Required**: OWASP-recommended headers

#### Tasks

- [ ] **16.4.1**: Add security headers middleware
  - X-Content-Type-Options: nosniff
  - X-Frame-Options: DENY
  - X-XSS-Protection: 1; mode=block
  - Strict-Transport-Security: max-age=31536000; includeSubDomains
  - Content-Security-Policy: default-src 'self'
  - Referrer-Policy: strict-origin-when-cross-origin

- [ ] **16.4.2**: Configure CORS properly
  - Restrict origins to allowed domains (from config)
  - Restrict methods to needed ones
  - Restrict headers
  - Set max-age for preflight caching

- [ ] **16.4.3**: Add request size limits
  - Max request body: 1MB
  - Max header size: 8KB
  - Timeout: 30 seconds

**Success Criteria**:
- Security headers present on all responses
- CORS configured properly
- Request size limits prevent DoS

**Files Modified**:
- `src/api/middleware/security.rs` (new)
- `src/server/routes.rs`

---

## Phase 17: Performance & Scale

**Duration**: 0.5 day
**Priority**: P2 - Medium (optimize after launch)
**Goal**: Ensure system can handle production load

### 17.1: Load Testing

**Files**: `scripts/load-test/` (new)
**Current**: No load tests
**Required**: Baseline performance metrics

#### Tasks

- [ ] **17.1.1**: Write load test scenarios
  - Scenario 1: Event creation under load (100 events/s)
  - Scenario 2: Reservation flow (50 reservations/s)
  - Scenario 3: Payment processing (20 payments/s)
  - Scenario 4: Query load (500 reads/s)

- [ ] **17.1.2**: Run load tests and capture metrics
  - Use k6, Gatling, or wrk
  - Target: 99th percentile latency < 500ms
  - Target: Error rate < 0.1%
  - Target: No memory leaks over 1 hour

- [ ] **17.1.3**: Identify bottlenecks
  - Profile with perf, flamegraph
  - Check database query performance
  - Check event bus throughput
  - Check projection update lag

- [ ] **17.1.4**: Optimize based on results
  - Add database indexes if needed
  - Optimize N+1 queries
  - Batch event bus operations
  - Tune connection pool sizes

**Success Criteria**:
- System handles 100 concurrent users
- 99th percentile latency < 500ms
- No crashes or memory leaks under load

**Files Created**:
- `scripts/load-test/event-creation.js` (k6)
- `scripts/load-test/reservation-flow.js`
- `scripts/load-test/payment-processing.js`
- `docs/performance-baseline.md`

---

### 17.2: Caching Strategy

**Files**: Various
**Current**: No caching
**Required**: Strategic caching for hot paths

#### Tasks

- [ ] **17.2.1**: Cache event aggregate state (optional)
  - Use Redis to cache recent event states
  - TTL: 5 minutes
  - Invalidate on event updates

- [ ] **17.2.2**: Cache projection queries
  - Cache available seats queries (TTL: 30s)
  - Cache analytics queries (TTL: 5 minutes)
  - Use Redis or in-memory LRU cache

- [ ] **17.2.3**: Add cache-control headers
  - Static assets: 1 year
  - API responses: no-cache (or short TTL for reads)
  - Event data: must-revalidate

- [ ] **17.2.4**: Implement cache warming
  - Pre-load popular events on startup
  - Background refresh for hot data
  - Metrics on cache hit/miss rate

**Success Criteria**:
- Cache hit rate > 80% for hot queries
- Read latency reduced by 50%+
- Proper cache invalidation on writes

**Files Modified**:
- `src/cache/mod.rs` (new)
- `src/api/events.rs`
- `src/projections/query_adapters.rs`

---

### 17.3: Database Optimization

**Files**: Various
**Current**: Basic indexes
**Required**: Production-ready indexing and tuning

#### Tasks

- [ ] **17.3.1**: Add composite indexes
  - Index on (event_id, timestamp) for queries
  - Index on (user_id, status) for reservations
  - Index on (user_id, created_at) for payments
  - Index on (event_id, status) for available seats

- [ ] **17.3.2**: Analyze query plans
  - Use EXPLAIN ANALYZE for slow queries
  - Identify sequential scans
  - Add covering indexes where needed

- [ ] **17.3.3**: Tune connection pools
  - Event store: max_connections = 20
  - Projections: max_connections = 20
  - Auth: max_connections = 10
  - Monitor pool exhaustion

- [ ] **17.3.4**: Configure PostgreSQL for production
  - shared_buffers = 25% of RAM
  - effective_cache_size = 75% of RAM
  - work_mem = 16MB
  - maintenance_work_mem = 256MB
  - checkpoint_completion_target = 0.9

**Success Criteria**:
- All common queries use indexes (no seq scans)
- Connection pools never exhausted
- Database CPU < 50% under load

**Files Modified**:
- `migrations/003_add_indexes.sql` (new)
- `migrations_projections/010_add_indexes.sql` (new)
- `docker-compose.yml` (PostgreSQL config)
- `.env.example` (pool size config)

---

### 17.4: Horizontal Scalability

**Files**: Infrastructure
**Current**: Single instance
**Required**: Multi-instance capable

#### Tasks

- [ ] **17.4.1**: Verify statelessness
  - No in-memory state shared between requests
  - All state in databases or Redis
  - WebSocket connections handled by sticky sessions

- [ ] **17.4.2**: Test multi-instance deployment
  - Run 3 instances behind load balancer
  - Verify event processing not duplicated
  - Verify projections stay consistent

- [ ] **17.4.3**: Configure session affinity for WebSockets
  - Sticky sessions based on user_id or session_id
  - Graceful connection migration on instance shutdown

- [ ] **17.4.4**: Document scaling guidance
  - When to scale horizontally (CPU, memory, latency)
  - How to add instances
  - Database connection pool tuning

**Success Criteria**:
- Can run multiple instances without issues
- Load balancer distributes traffic evenly
- No data consistency issues in multi-instance setup

**Files Modified**:
- `docker-compose.yml` (multi-instance setup)
- `docs/deployment.md`
- `docs/scaling-guide.md` (new)

---

## Phase 18: Documentation & Launch

**Duration**: 0.5 day
**Priority**: P0 - Critical before production
**Goal**: Production-ready documentation and launch checklist

### 18.1: API Documentation

**Files**: `docs/api/` (new)
**Current**: Minimal documentation
**Required**: Complete API reference

#### Tasks

- [ ] **18.1.1**: Generate OpenAPI/Swagger specification
  - Use `utoipa` crate for automatic spec generation
  - Include all endpoints, request/response schemas
  - Add examples for each endpoint

- [ ] **18.1.2**: Add API usage guide
  - Authentication flow (magic link)
  - Common workflows (create event, make reservation, process payment)
  - Error handling guide
  - Rate limiting information

- [ ] **18.1.3**: Add WebSocket API documentation
  - Connection protocol
  - Message format (ProjectionUpdate)
  - Subscription management
  - Error handling

- [ ] **18.1.4**: Host interactive API docs
  - Swagger UI or ReDoc
  - Accessible at /docs endpoint
  - Include "Try it out" functionality

**Success Criteria**:
- All endpoints documented with examples
- Developers can integrate without asking questions
- OpenAPI spec passes validation

**Files Created**:
- `docs/api/openapi.yaml` (generated)
- `docs/api/usage-guide.md`
- `docs/api/websocket.md`
- `src/api/openapi.rs` (utoipa integration)

---

### 18.2: Operations Guide

**Files**: `docs/operations/` (new)
**Current**: Minimal ops documentation
**Required**: Complete runbook

#### Tasks

- [ ] **18.2.1**: Deployment guide
  - Prerequisites (Docker, PostgreSQL, Redpanda)
  - Environment variable configuration
  - Database migration process
  - Production deployment steps
  - Rollback procedure

- [ ] **18.2.2**: Monitoring guide
  - Key metrics to watch
  - Dashboard interpretation
  - Alert response procedures
  - When to page on-call

- [ ] **18.2.3**: Troubleshooting guide
  - Common issues and solutions
  - How to check health of each component
  - How to trace requests
  - How to replay failed events from DLQ

- [ ] **18.2.4**: Backup and recovery guide
  - Event store backup procedure (PostgreSQL WAL archiving)
  - Projection rebuild from events
  - Disaster recovery plan
  - RTO/RPO targets

**Success Criteria**:
- On-call engineer can deploy without help
- On-call engineer can troubleshoot common issues
- Disaster recovery procedure tested

**Files Created**:
- `docs/operations/deployment.md`
- `docs/operations/monitoring.md`
- `docs/operations/troubleshooting.md`
- `docs/operations/disaster-recovery.md`

---

### 18.3: Architecture Decision Records (ADRs)

**Files**: `docs/architecture/adr/` (new)
**Current**: No ADRs
**Required**: Document key decisions

#### Tasks

- [ ] **18.3.1**: ADR-001: Event Sourcing and CQRS
  - Why event sourcing chosen
  - Trade-offs vs. CRUD
  - When to use this pattern

- [ ] **18.3.2**: ADR-002: Saga Pattern for Multi-Aggregate Coordination
  - Why sagas vs. distributed transactions
  - Compensation strategy
  - Timeout handling

- [ ] **18.3.3**: ADR-003: Per-Request Store Pattern
  - Why not shared state
  - Privacy benefits
  - Memory efficiency

- [ ] **18.3.4**: ADR-004: In-Memory Projections with PostgreSQL Backup
  - Performance vs. persistence trade-off
  - Dual-write strategy
  - Startup reconstruction

- [ ] **18.3.5**: ADR-005: Bootstrap Refactoring to Framework API
  - Why 92% code reduction matters
  - Framework-level reusability
  - DSL readiness

**Success Criteria**:
- Key architectural decisions documented
- New developers understand design rationale
- Future changes reference ADRs

**Files Created**:
- `docs/architecture/adr/001-event-sourcing.md`
- `docs/architecture/adr/002-saga-pattern.md`
- `docs/architecture/adr/003-per-request-store.md`
- `docs/architecture/adr/004-in-memory-projections.md`
- `docs/architecture/adr/005-bootstrap-refactoring.md`

---

### 18.4: Production Launch Checklist

**Files**: `docs/LAUNCH_CHECKLIST.md` (new)
**Current**: No checklist
**Required**: Comprehensive go-live checklist

#### Tasks

- [ ] **18.4.1**: Security checklist
  - [ ] All secrets in environment variables (not committed)
  - [ ] SSL/TLS configured for all endpoints
  - [ ] CORS restricted to allowed origins
  - [ ] Rate limiting enabled
  - [ ] Admin accounts created with strong passwords
  - [ ] Security headers enabled
  - [ ] Audit logging enabled

- [ ] **18.4.2**: Infrastructure checklist
  - [ ] All databases have backups configured
  - [ ] PostgreSQL WAL archiving enabled
  - [ ] Event bus replication factor = 3
  - [ ] Redis persistence enabled (AOF or RDB)
  - [ ] All health checks passing
  - [ ] Monitoring dashboards configured
  - [ ] Alerts configured and tested

- [ ] **18.4.3**: Application checklist
  - [ ] All TODO comments resolved or deferred
  - [ ] All tests passing (unit, integration, E2E)
  - [ ] Load tests passing with acceptable latency
  - [ ] No memory leaks detected
  - [ ] DLQ configured and tested
  - [ ] Event versioning enabled
  - [ ] In-memory projections backed up to PostgreSQL

- [ ] **18.4.4**: Operational readiness checklist
  - [ ] Runbook complete and tested
  - [ ] On-call rotation scheduled
  - [ ] Incident response plan documented
  - [ ] Rollback plan tested
  - [ ] Disaster recovery plan tested
  - [ ] API documentation published
  - [ ] Monitoring dashboards accessible

- [ ] **18.4.5**: Business checklist
  - [ ] Terms of service reviewed
  - [ ] Privacy policy reviewed
  - [ ] Compliance requirements met (GDPR, PCI, etc.)
  - [ ] Customer support process defined
  - [ ] Pricing/billing configured

**Success Criteria**:
- All checklist items completed
- Sign-off from engineering, security, and business
- Go/no-go decision documented

**Files Created**:
- `docs/LAUNCH_CHECKLIST.md`
- `docs/LAUNCH_DECISION.md` (sign-off record)

---

## Success Criteria

### Overall Production Readiness

**Technical Criteria**:
- ✅ All API endpoints implemented and tested
- ✅ Health checks verify infrastructure health
- ✅ All TODO comments resolved or documented as deferred
- ✅ Event versioning enabled for safe schema evolution
- ✅ DLQ captures and allows retry of failed events
- ✅ In-memory projections backed up to PostgreSQL
- ✅ Metrics exposed for monitoring (Prometheus)
- ✅ Distributed tracing enabled (correlation IDs)
- ✅ Admin authorization implemented (RBAC)
- ✅ Rate limiting prevents abuse
- ✅ Audit logging tracks all state changes
- ✅ Security headers protect against common attacks
- ✅ Load tests pass with acceptable latency
- ✅ Multi-instance deployment tested

**Operational Criteria**:
- ✅ All databases have backup and recovery procedures
- ✅ Monitoring dashboards visualize system health
- ✅ Alerts configured for critical failures
- ✅ Runbook enables on-call troubleshooting
- ✅ API documentation complete and published
- ✅ Launch checklist completed
- ✅ Disaster recovery plan tested

**Quality Criteria**:
- ✅ Zero clippy errors
- ✅ Zero unsafe blocks
- ✅ Zero panics in production code
- ✅ All tests passing (unit, integration, E2E, load)
- ✅ Code coverage > 80%
- ✅ No security vulnerabilities detected

---

## Rollback Plan

### Pre-Deployment

1. **Tag release**: `git tag -a v1.0.0 -m "Production release"`
2. **Backup databases**: Full backup of all PostgreSQL instances
3. **Document current state**: Capture metrics baseline
4. **Test rollback**: Deploy previous version to staging, verify it works

### During Deployment

1. **Deploy behind feature flag**: Enable new features gradually
2. **Monitor metrics**: Watch dashboards for anomalies
3. **Test critical paths**: Smoke test after deployment
4. **Gradual rollout**: 10% traffic → 50% → 100%

### Rollback Triggers

Roll back immediately if:
- Health checks failing for > 2 minutes
- Error rate > 5% for > 5 minutes
- Critical feature broken (payments, reservations)
- Data corruption detected

### Rollback Procedure

1. **Stop new deployments**: Prevent further changes
2. **Revert to previous version**: Deploy tagged release
3. **Verify health checks**: Ensure system recovered
4. **Investigate issue**: Post-mortem, fix root cause
5. **Communicate**: Notify stakeholders of rollback

---

## Timeline Summary

### Minimum Viable Production (2.5-3.5 days)
- **Phase 13**: Critical Production Blockers (1-2 days)
- **Phase 14**: Production Hardening (1 day)
- **Phase 18**: Documentation & Launch (0.5 day)

### Recommended Production (4-6 days)
- **Phase 13**: Critical Production Blockers (1-2 days)
- **Phase 14**: Production Hardening (1 day)
- **Phase 15**: Observability & Operations (1 day)
- **Phase 16**: Security & Compliance (0.5 day)
- **Phase 17**: Performance & Scale (0.5 day)
- **Phase 18**: Documentation & Launch (0.5 day)

### Post-Launch Optimization (ongoing)
- Performance tuning based on production metrics
- Feature enhancements from user feedback
- Scalability improvements as traffic grows

---

## Next Steps

1. **Review this plan** with stakeholders (engineering, security, business)
2. **Prioritize phases** based on launch timeline and requirements
3. **Create task board** with all Phase 13 tasks
4. **Start Phase 13.1** (Health checks) - quick win, high impact
5. **Track progress** daily, adjust plan as needed

---

**Plan Status**: Ready for Execution
**Created**: 2025-11-19
**Last Updated**: 2025-11-19
**Owner**: Engineering Team
**Target Launch Date**: TBD based on phase completion
