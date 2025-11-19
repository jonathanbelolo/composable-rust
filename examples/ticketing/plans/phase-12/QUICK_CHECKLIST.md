# Production Readiness - Quick Checklist

**Last Updated**: 2025-11-19
**Status**: Ready to Start Phase 13

---

## Phase 13: Critical Blockers (P0) - 1-2 days

### 13.1 Health Checks
- [ ] PostgreSQL event store health check
- [ ] PostgreSQL projections DB health check
- [ ] PostgreSQL auth DB health check
- [ ] Redpanda event bus health check
- [ ] Redis cache health check
- [ ] Aggregate health status endpoint (JSON)

### 13.2 Event API Implementation
- [ ] GET `/api/events/:id` - Get single event
- [ ] GET `/api/events` - List events (paginated)
- [ ] PUT `/api/events/:id` - Update event
- [ ] DELETE `/api/events/:id` - Delete/cancel event
- [ ] Add UpdateEvent/DeleteEvent to aggregate

### 13.3 Payment API Implementation
- [ ] GET `/api/payments/:id` - Get single payment
- [ ] GET `/api/users/:id/payments` - List user payments
- [ ] POST `/api/payments/:id/refund` - Refund payment
- [ ] Add RefundPayment to payment aggregate
- [ ] Extend reservation saga for refund compensation

### 13.4 Resolve Critical TODOs
- [ ] `auth/middleware.rs:238` - Admin role checking
- [ ] `projections/payments_postgres.rs:346` - Extract customer_id
- [ ] `projections/query_adapters.rs:110,151` - Implement queries
- [ ] Document deferred TODOs in DEFERRED_TODOS.md

---

## Phase 14: Production Hardening (P0) - 1 day

### 14.1 Persist In-Memory Projections
- [ ] PostgreSQL schema for SalesAnalyticsProjection
- [ ] PostgreSQL schema for CustomerHistoryProjection
- [ ] Dual-write implementation (in-memory + PostgreSQL)
- [ ] Startup reconstruction from PostgreSQL
- [ ] Test restart preserves data

### 14.2 Event Versioning
- [ ] Add `event_version` field to SerializedEvent
- [ ] Migration: Add `event_version` column to events table
- [ ] Version all event types (tag with v1)
- [ ] Implement version-aware deserialization
- [ ] Document versioning guide

### 14.3 Dead Letter Queue (DLQ)
- [ ] Create DLQ database schema
- [ ] Implement DLQ writer in EventConsumer
- [ ] Admin API: List DLQ events
- [ ] Admin API: Retry DLQ events
- [ ] Test DLQ workflow

### 14.4 Enhanced Error Handling
- [ ] Domain-specific error types (EventError, ReservationError, PaymentError)
- [ ] Error response middleware (with error codes)
- [ ] Structured error logging (with context)
- [ ] Graceful degradation patterns

---

## Phase 15: Observability (P1) - 1 day

### 15.1 Metrics Integration
- [ ] Request duration metrics (histogram)
- [ ] Business metrics (events_created, reservations, payments)
- [ ] Infrastructure metrics (db connections, event bus)
- [ ] Error metrics (errors_total, dlq_events)
- [ ] Expose /metrics endpoint (Prometheus format)

### 15.2 Distributed Tracing
- [ ] Request ID middleware (X-Request-ID header)
- [ ] Trace context propagation (trace_id, span_id)
- [ ] Span instrumentation (API, reducer, DB, event bus)
- [ ] OpenTelemetry export configuration (optional)

### 15.3 Operational Dashboards
- [ ] System health dashboard (Grafana)
- [ ] Business metrics dashboard
- [ ] Error tracking dashboard
- [ ] Alerting rules (health, error rate, DLQ size)

### 15.4 Log Aggregation
- [ ] Structured JSON logging for production
- [ ] Log aggregation setup (Loki, optional)
- [ ] Common log queries documented

---

## Phase 16: Security & Compliance (P1) - 0.5 day

### 16.1 Admin Authorization
- [ ] User roles table (migration)
- [ ] Implement `check_admin_role()`
- [ ] Admin override for profiles (with audit)
- [ ] Protect admin endpoints (DLQ, metrics)

### 16.2 Rate Limiting
- [ ] Per-user rate limiting (Redis, 100 req/min)
- [ ] Per-IP rate limiting (20 req/min)
- [ ] Enhanced WebSocket rate limiting
- [ ] Rate limit response headers (X-RateLimit-*)

### 16.3 Audit Logging
- [ ] Audit log schema (migrations)
- [ ] Audit middleware (log all state changes)
- [ ] Admin API: Query audit logs
- [ ] Compliance reports

### 16.4 Security Headers
- [ ] Security headers middleware (X-Content-Type-Options, etc.)
- [ ] CORS configuration
- [ ] Request size limits

---

## Phase 17: Performance & Scale (P2) - 0.5 day

### 17.1 Load Testing
- [ ] Load test scenarios (k6 or Gatling)
- [ ] Run tests, capture metrics
- [ ] Identify bottlenecks (profiling)
- [ ] Optimize based on results

### 17.2 Caching Strategy
- [ ] Cache projection queries (Redis or in-memory)
- [ ] Cache-control headers
- [ ] Cache warming on startup
- [ ] Metrics: cache hit/miss rate

### 17.3 Database Optimization
- [ ] Composite indexes (events, reservations, payments)
- [ ] Analyze query plans (EXPLAIN ANALYZE)
- [ ] Tune connection pools
- [ ] Configure PostgreSQL for production

### 17.4 Horizontal Scalability
- [ ] Verify statelessness
- [ ] Test multi-instance deployment
- [ ] Session affinity for WebSockets
- [ ] Document scaling guidance

---

## Phase 18: Documentation & Launch (P0) - 0.5 day

### 18.1 API Documentation
- [ ] Generate OpenAPI/Swagger spec (utoipa)
- [ ] API usage guide
- [ ] WebSocket API documentation
- [ ] Host interactive API docs (/docs endpoint)

### 18.2 Operations Guide
- [ ] Deployment guide
- [ ] Monitoring guide
- [ ] Troubleshooting guide
- [ ] Backup and recovery guide

### 18.3 Architecture Decision Records (ADRs)
- [ ] ADR-001: Event Sourcing and CQRS
- [ ] ADR-002: Saga Pattern
- [ ] ADR-003: Per-Request Store Pattern
- [ ] ADR-004: In-Memory Projections with PostgreSQL Backup
- [ ] ADR-005: Bootstrap Refactoring

### 18.4 Launch Checklist
- [ ] Security checklist (secrets, SSL, CORS, rate limiting)
- [ ] Infrastructure checklist (backups, monitoring, alerts)
- [ ] Application checklist (tests, TODOs, DLQ, versioning)
- [ ] Operational readiness (runbook, on-call, rollback)
- [ ] Business checklist (compliance, support, billing)

---

## Pre-Launch Gate (Required for Production)

### Critical Checks
- [ ] All P0 phases complete (13, 14, 18)
- [ ] All tests passing (unit, integration, E2E)
- [ ] Health checks functional
- [ ] No critical TODOs remaining
- [ ] Backup and recovery tested
- [ ] Rollback plan tested
- [ ] Sign-off from engineering lead
- [ ] Sign-off from security team
- [ ] Sign-off from business owner

### Recommended Checks (Should Complete)
- [ ] P1 phases complete (15, 16)
- [ ] Load tests passing
- [ ] Monitoring dashboards configured
- [ ] Alerts tested and validated
- [ ] API documentation published
- [ ] On-call rotation scheduled

---

## Timeline

**Minimum (P0 only)**: 2.5-3.5 days
- Phase 13: 1-2 days
- Phase 14: 1 day
- Phase 18: 0.5 day

**Recommended (P0 + P1)**: 4-6 days
- Phase 13: 1-2 days
- Phase 14: 1 day
- Phase 15: 1 day
- Phase 16: 0.5 day
- Phase 17: 0.5 day
- Phase 18: 0.5 day

**Next Action**: Start with Phase 13.1 (Health Checks) - quick win, high impact.

---

**Status Legend**:
- [ ] Not started
- [~] In progress
- [x] Complete
- [!] Blocked
