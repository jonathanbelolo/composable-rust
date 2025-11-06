# Phase 4: Production Hardening - TODO List

**Goal**: Make the framework production-ready with observability, advanced error handling, and performance optimization.

**Duration**: 1.5-2 weeks

**Status**: 🔄 **IN PLANNING** (2025-11-06)

**Philosophy**: Production systems need more than correct code—they need observability, resilience, and performance. This phase hardens the framework for real-world deployment at scale.

---

## Prerequisites

Before starting Phase 4:
- [x] Phase 1 complete (Core abstractions)
- [x] Phase 2 complete (Event sourcing with PostgreSQL)
- [x] Phase 3 complete (Sagas & coordination with Redpanda)
- [x] All 87 tests passing
- [ ] Review production deployment requirements
- [ ] Review observability best practices (OpenTelemetry)
- [ ] Review error handling patterns (circuit breakers, retries)

---

## Strategic Context: Production-Ready Framework

From the roadmap:

**Goal**: Handle 1000 commands/sec sustained load with full observability.

**Key Requirements**:
1. **Observability**: See what's happening in production
2. **Resilience**: Handle failures gracefully
3. **Performance**: Meet latency and throughput targets
4. **Operations**: Easy to deploy and monitor

**Investment**: ~2 weeks to add production features
**Return**: Framework ready for real-world deployment, battle-tested reference implementation

---

## 1. Observability Infrastructure

### 1.1 Tracing Integration

**Scope**: Add `tracing` instrumentation throughout the framework

```rust
// Reducer execution
#[instrument(skip(self, state, env), fields(action = ?action))]
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
    // ...
}

// Effect execution
#[instrument(skip(effect), fields(effect_type = %effect.type_name()))]
async fn execute_effect<A>(effect: Effect<A>) -> Option<A> {
    // ...
}
```

**Tasks**:
- [ ] Add `tracing` dependency to core crates
- [ ] Instrument Store::send() with span creation
- [ ] Instrument effect execution (all variants)
- [ ] Instrument EventStore operations (append, load)
- [ ] Instrument EventBus operations (publish, subscribe)
- [ ] Add span context propagation (parent → child spans)
- [ ] Document tracing setup in examples

**Success Criteria**:
- Can trace a command through: Store → Reducer → Effects → EventStore → EventBus → Subscriber
- Spans include timing, action types, effect types
- Errors captured in spans with context

---

### 1.2 Metrics Collection

**Scope**: Expose metrics for monitoring

**Metrics to Track**:
- Command rate (commands/sec)
- Effect execution time (histogram)
- Event store latency (append, load)
- Event bus latency (publish, subscribe)
- Error rates by type
- Saga state distribution

**Tasks**:
- [ ] Add `metrics` crate dependency
- [ ] Define core metrics (counter, histogram, gauge)
- [ ] Add metrics to Store (command rate, state size)
- [ ] Add metrics to effect executor (execution time by type)
- [ ] Add metrics to EventStore (operation latency)
- [ ] Add metrics to EventBus (publish/subscribe rates)
- [ ] Export Prometheus metrics endpoint
- [ ] Document metrics in `docs/observability.md`

**Success Criteria**:
- Can query command rate, latency percentiles (p50, p95, p99)
- Can monitor error rates and types
- Metrics exportable to Prometheus

---

### 1.3 OpenTelemetry Support

**Scope**: Optional OpenTelemetry integration

**Tasks**:
- [ ] Add `opentelemetry` feature flag
- [ ] Integrate with tracing-opentelemetry
- [ ] Configure OTLP exporter (traces, metrics)
- [ ] Document OpenTelemetry setup
- [ ] Add example with Jaeger/Tempo

**Success Criteria**:
- Can export traces to Jaeger/Tempo
- Can export metrics to Prometheus via OTLP
- Feature flag works (compiles with/without)

---

## 2. Advanced Error Handling

### 2.1 Retry Policies

**Scope**: Automatic retries for transient failures

**Strategy**:
```rust
pub struct RetryPolicy {
    max_attempts: u32,           // Default: 5
    initial_delay: Duration,     // Default: 1s
    max_delay: Duration,         // Default: 32s
    backoff_multiplier: f64,     // Default: 2.0 (exponential)
}
```

**Tasks**:
- [ ] Define RetryPolicy in `core/src/error_handling.rs`
- [ ] Implement exponential backoff with jitter
- [ ] Add retry logic to effect executor
- [ ] Configure per effect type (EventStore, EventBus, Future)
- [ ] Track retry attempts in metrics
- [ ] Document retry behavior
- [ ] Add tests for retry scenarios

**Success Criteria**:
- Transient failures retry automatically (up to 5 times)
- Exponential backoff prevents thundering herd
- Permanent failures skip retries

---

### 2.2 Circuit Breaker Pattern

**Scope**: Prevent cascading failures

**Strategy**:
```rust
pub struct CircuitBreaker {
    failure_threshold: f64,      // 0.5 = 50% error rate
    sample_window: usize,        // 10 requests
    timeout: Duration,           // 30s before retry
    state: CircuitState,         // Closed, Open, HalfOpen
}
```

**Tasks**:
- [ ] Implement CircuitBreaker in `runtime/src/circuit_breaker.rs`
- [ ] Add to effect executor (per dependency)
- [ ] Track state transitions in metrics
- [ ] Add circuit breaker for EventStore
- [ ] Add circuit breaker for EventBus
- [ ] Document circuit breaker behavior
- [ ] Add tests for state transitions

**Success Criteria**:
- Circuit opens after 50% failures over 10 requests
- Circuit remains open for 30s
- Circuit half-opens to test recovery
- Metrics show circuit state changes

---

### 2.3 Dead Letter Queue (DLQ)

**Scope**: Handle permanently failed events

**Strategy**:
- Failed events (after max retries) → DLQ topic/table
- Separate monitoring for DLQ
- Manual reprocessing workflow

**Tasks**:
- [ ] Define DLQ interface in `core/src/event_bus.rs`
- [ ] Implement DLQ for EventBus (Redpanda DLQ topic)
- [ ] Implement DLQ for EventStore (failed_events table)
- [ ] Add DLQ metrics (count, age)
- [ ] Document DLQ monitoring
- [ ] Add reprocessing tool/script
- [ ] Add tests for DLQ flow

**Success Criteria**:
- Failed events land in DLQ after max retries
- DLQ visible in metrics/monitoring
- Can manually reprocess DLQ events

---

## 3. Performance Optimization

### 3.1 Effect Batching

**Scope**: Batch multiple effects for efficiency

**Example**:
```rust
// Instead of:
for event in events {
    event_store.append(event).await?;
}

// Batch as:
event_store.append_batch(events).await?;
```

**Tasks**:
- [ ] Add append_batch() to EventStore trait
- [ ] Implement batching in PostgresEventStore
- [ ] Add publish_batch() to EventBus trait
- [ ] Implement batching in RedpandaEventBus
- [ ] Update effect executor to batch compatible effects
- [ ] Benchmark batching improvements
- [ ] Document batching behavior

**Success Criteria**:
- Batching reduces latency by 30%+ for bulk operations
- No correctness regressions

---

### 3.2 SmallVec for Effects

**Scope**: Reduce allocations for small effect lists

**Rationale**: Most reducers return 0-3 effects. SmallVec avoids heap allocation.

**Tasks**:
- [ ] Add `smallvec` dependency
- [ ] Change `Vec<Effect>` → `SmallVec<[Effect; 4]>`
- [ ] Update all reducer return types
- [ ] Benchmark allocation improvements
- [ ] Verify no performance regression
- [ ] Document SmallVec usage

**Success Criteria**:
- Reduces allocations for typical reducers
- No API breakage (SmallVec derefs to slice)

---

### 3.3 Profiling & Benchmarking

**Scope**: Identify and fix performance bottlenecks

**Performance Targets** (from architecture.md):
- Reducer execution: < 1μs for typical state machines (Counter, Order)
- Effect execution overhead: < 100μs (excluding actual I/O)
- Event replay throughput: 10,000+ events/second
- End-to-end command latency: < 10ms (excluding external service calls)

**Tasks**:
- [ ] Add `criterion` benchmarks for:
  - [ ] Reducer execution (Counter, Order, Saga) - target < 1μs
  - [ ] Effect execution (all variants) - target < 100μs overhead
  - [ ] EventStore operations (append, load) - target 10k events/sec replay
  - [ ] EventBus operations (publish, subscribe)
  - [ ] End-to-end command flow - target < 10ms
- [ ] Profile with `cargo flamegraph`
- [ ] Optimize hot paths identified
- [ ] Document performance characteristics
- [ ] Add CI benchmark tracking

**Success Criteria**:
- Benchmarks establish baseline performance
- Reducer execution < 1μs for typical state machines
- Effect overhead < 100μs (excluding I/O)
- Event replay >= 10k events/sec
- End-to-end latency < 10ms (excluding external services)
- Identified bottlenecks documented
- Optimizations show measurable improvement

---

## 4. Production Redpanda Features

### 4.1 Consumer Group Management

**Scope**: Advanced consumer group features

**Tasks**:
- [ ] Document consumer group lifecycle
- [ ] Add consumer group rebalancing handling
- [ ] Add partition assignment strategies
- [ ] Test consumer group failover
- [ ] Document scaling patterns
- [ ] Add monitoring for consumer lag

**Success Criteria**:
- Consumer groups rebalance gracefully
- No message loss during rebalancing
- Lag monitoring works

---

### 4.2 Offset Management

**Scope**: Reliable offset tracking

**Tasks**:
- [ ] Document manual commit strategy (already implemented)
- [ ] Add offset commit batching
- [ ] Add offset commit failure handling
- [ ] Test offset recovery scenarios
- [ ] Document offset reset procedures

**Success Criteria**:
- Offset commits batched for efficiency
- Commit failures don't block processing
- Recovery from crash works correctly

---

### 4.3 Event Bus Monitoring

**Scope**: Production monitoring for Redpanda

**Tasks**:
- [ ] Add lag monitoring per consumer group
- [ ] Add throughput metrics (msgs/sec)
- [ ] Add error rate tracking
- [ ] Document alerting thresholds
- [ ] Create monitoring dashboard examples

**Success Criteria**:
- Can monitor consumer lag in real-time
- Alerts fire on high lag or errors

---

## 5. Production Database Features

### 5.1 Connection Pooling

**Scope**: Efficient database connections

**Tasks**:
- [ ] Document sqlx connection pool configuration
- [ ] Add pool size tuning guide
- [ ] Add connection pool metrics
- [ ] Test pool exhaustion scenarios
- [ ] Document pool sizing recommendations

**Success Criteria**:
- Connection pool documented
- Metrics show pool utilization
- No connection leaks

---

### 5.2 Migration Tooling

**Scope**: Database schema migrations

**Tasks**:
- [ ] Create `migrations/` directory
- [ ] Add sqlx migration scripts for:
  - [ ] Event store schema
  - [ ] Indexes for performance
  - [ ] Failed events table (DLQ)
- [ ] Document migration workflow
- [ ] Add migration testing
- [ ] Document rollback procedures

**Success Criteria**:
- Migrations run cleanly
- Schema versioning tracked
- Rollback procedures documented

---

### 5.3 Backup & Restore

**Scope**: Data durability procedures

**Tasks**:
- [ ] Document backup procedures (pg_dump)
- [ ] Document restore procedures
- [ ] Add point-in-time recovery guide
- [ ] Document disaster recovery plan
- [ ] Test backup/restore workflow

**Success Criteria**:
- Backup procedure documented
- Restore tested and works
- RPO/RTO defined

---

## 6. Effect::DispatchCommand (Optional - May Defer to Phase 5)

**Note**: This section adds a new Effect variant for in-process command routing. After Phase 3 review, this is **optional for Phase 4** as:
- EventBus already handles cross-aggregate coordination effectively
- Adding new Effect variants changes core abstractions (better suited for Phase 5: Developer Experience)
- Phase 4 focus is production hardening, not new features

**Decision**: If time permits after all other Phase 4 work, implement this. Otherwise, defer to Phase 5.

### 6.1 In-Process Command Routing

**Scope**: Send commands to other aggregates' stores without EventBus

**Use Case**: Synchronous cross-aggregate commands in same process (e.g., Order → Payment without events)

```rust
pub enum Effect<Action> {
    // ... existing variants
    DispatchCommand {
        target: String,           // Aggregate ID or service name
        command: Box<dyn Any + Send + Sync>,
        on_success: Option<Box<dyn Fn() -> Action + Send + Sync>>,
        on_error: Option<Box<dyn Fn(Error) -> Action + Send + Sync>>,
    },
}
```

**Tasks** (only if time permits):
- [ ] Define DispatchCommand variant
- [ ] Implement command routing in Store
- [ ] Add store registry (aggregate ID → Store reference)
- [ ] Handle command execution and callbacks
- [ ] Add tests for cross-aggregate commands
- [ ] Document command dispatch patterns vs EventBus

**Success Criteria**:
- Can dispatch commands between aggregates in same process
- Type safety preserved
- Callbacks work correctly
- Clear documentation on when to use DispatchCommand vs EventBus

**Alternative**: Continue using EventBus for all cross-aggregate coordination (recommended for Phase 4)

---

## 7. Reference Application: examples/production-ready/

### 7.1 Multi-Aggregate System

**Scope**: Complete production-ready example

**Aggregates**:
- Orders (from Phase 2)
- Payments (from Phase 3)
- Shipping (new)

**Workflow**:
1. Place Order → Order Created
2. Process Payment → Payment Completed
3. Ship Order → Shipment Created
4. Track Shipment → Delivered

**Tasks**:
- [ ] Create `examples/production-ready/` directory
- [ ] Implement Shipping aggregate (state machine)
- [ ] Implement Order-Payment-Shipping saga
- [ ] Wire up PostgreSQL event store
- [ ] Wire up Redpanda event bus
- [ ] Add full observability (tracing, metrics)
- [ ] Add docker-compose.yml (Postgres + Redpanda + app)
- [ ] Add load test scripts (K6 or similar)

**Success Criteria**:
- Complete workflow: Order → Payment → Shipping
- Events flow through real Redpanda
- Full tracing from command to completion
- Metrics collected and exportable

---

### 7.2 Docker Compose Deployment

**Scope**: Local production-like environment

**Services**:
- PostgreSQL (event store)
- Redpanda (event bus)
- Jaeger (tracing)
- Prometheus (metrics)
- Grafana (dashboards)
- Application (Rust service)

**Tasks**:
- [ ] Create docker-compose.yml
- [ ] Configure PostgreSQL with persistence
- [ ] Configure Redpanda with 3 brokers
- [ ] Configure Jaeger for trace collection
- [ ] Configure Prometheus scraping
- [ ] Create Grafana dashboards
- [ ] Document setup and usage
- [ ] Add comprehensive health check endpoints:
  - [ ] `/health/live` - Liveness probe (is process running?)
  - [ ] `/health/ready` - Readiness probe (can serve traffic?)
    - Check PostgreSQL connection
    - Check Redpanda connection
    - Check event bus consumer status
  - [ ] `/health/startup` - Startup probe (initialization complete?)
  - [ ] `/metrics` - Prometheus metrics endpoint

**Success Criteria**:
- `docker-compose up` starts entire stack
- Application connects to all services
- Can view traces in Jaeger
- Can view metrics in Grafana

---

### 7.3 Load Testing

**Scope**: Validate performance under load

**Target**: 1000 commands/sec sustained load

**Tests**:
- Ramp-up test (0 → 1000 cmd/sec over 5 min)
- Sustained load (1000 cmd/sec for 10 min)
- Spike test (burst to 2000 cmd/sec)
- Soak test (500 cmd/sec for 1 hour)

**Tasks**:
- [ ] Install K6 or similar load testing tool
- [ ] Write load test scripts
- [ ] Run baseline performance tests
- [ ] Identify bottlenecks
- [ ] Optimize and re-test
- [ ] Document performance results
- [ ] Create performance regression tests

**Success Criteria**:
- Handles 1000 cmd/sec with p95 latency < 100ms
- No memory leaks during soak test
- Graceful degradation under overload

---

## 8. Alternative Event Bus Documentation

### 8.1 Kafka Migration Guide

**Scope**: Document swapping Redpanda for Kafka

**Tasks**:
- [ ] Document Apache Kafka setup
- [ ] Show configuration changes (none required!)
- [ ] Document performance differences
- [ ] Add Kafka deployment examples

**Success Criteria**:
- Clear migration path from Redpanda to Kafka
- No code changes required (just config)

---

### 8.2 Cloud Provider Guides

**Scope**: Document managed service alternatives

**Tasks**:
- [ ] Document AWS MSK setup (Kafka-compatible)
- [ ] Document Azure Event Hubs setup (Kafka-compatible)
- [ ] Document Confluent Cloud setup
- [ ] Document Redpanda Cloud setup
- [ ] Compare pricing and features

**Success Criteria**:
- Each cloud provider documented
- Connection examples provided
- Trade-offs documented

---

## 9. Documentation

### 9.1 Production Guides

**Tasks**:
- [ ] Create `docs/observability.md`:
  - [ ] Tracing setup and configuration
  - [ ] Metrics collection and export
  - [ ] OpenTelemetry integration
  - [ ] Monitoring dashboards
  - [ ] Alerting best practices
- [ ] Create `docs/error-handling.md`:
  - [ ] Retry policies
  - [ ] Circuit breakers
  - [ ] Dead letter queues
  - [ ] Error correlation
- [ ] Create `docs/performance.md`:
  - [ ] Benchmarking guide
  - [ ] Profiling techniques
  - [ ] Optimization tips
  - [ ] Performance tuning
- [ ] Create `docs/deployment.md`:
  - [ ] Docker deployment
  - [ ] Kubernetes deployment
  - [ ] Cloud deployments
  - [ ] High availability setup

**Success Criteria**:
- Each guide comprehensive and actionable
- Code examples included
- Troubleshooting sections

---

### 9.2 Operations Runbooks

**Tasks**:
- [ ] Create runbooks for:
  - [ ] Service startup/shutdown
  - [ ] Database migrations
  - [ ] Incident response
  - [ ] Disaster recovery
  - [ ] Scaling procedures
  - [ ] Common troubleshooting

**Success Criteria**:
- Runbooks cover common scenarios
- Step-by-step procedures
- Links to relevant documentation

---

## 10. Testing & Validation

### 10.1 Integration Tests

**Tasks**:
- [ ] Full stack integration tests (Postgres + Redpanda + App)
- [ ] Test with testcontainers in CI/CD
- [ ] Network partition tests (chaos engineering)
- [ ] Process crash and recovery tests
- [ ] Rebalancing and failover tests
- [ ] Add missing unit tests from Phase 3:
  - [ ] Reducer composition utilities tests:
    - [ ] `combine_reducers()` with multiple reducers
    - [ ] `scope_reducer()` with action mapping
    - [ ] Combined + scoped reducers together
  - [ ] Saga timeout scenarios:
    - [ ] Payment timeout triggers compensation
    - [ ] Inventory timeout triggers compensation
    - [ ] Multiple timeout handling with `Effect::Delay`
- [ ] Add checkout-saga Redpanda integration tests:
  - [ ] Full saga flow through Redpanda event bus
  - [ ] Compensation flow through Redpanda
  - [ ] Event ordering guarantees in saga

**Success Criteria**:
- Tests run in CI automatically
- Chaos scenarios handled gracefully
- Reducer composition utilities fully tested
- Saga timeout scenarios covered
- Checkout saga validated with real Redpanda

---

### 10.2 Performance Tests

**Tasks**:
- [ ] Benchmark suite for all operations
- [ ] Regression testing in CI
- [ ] Memory profiling
- [ ] CPU profiling
- [ ] I/O profiling

**Success Criteria**:
- Baseline performance documented
- Regressions caught in CI

---

### 10.3 Quality Checks

**Tasks**:
- [ ] All tests pass (target: 100+ tests)
- [ ] Clippy clean (0 warnings)
- [ ] Code formatted (cargo fmt)
- [ ] Documentation complete
- [ ] Examples run successfully
- [ ] Load tests meet targets

---

## 11. Success Criteria

Phase 4 is complete when:

**Core Features**:
- [ ] Full observability (tracing, metrics, OpenTelemetry)
- [ ] Advanced error handling (retries, circuit breakers, DLQ)
- [ ] Performance optimized (1000 cmd/sec target met)
- [ ] `examples/production-ready/` deployed with docker-compose
- [ ] Load tests validate 1000 cmd/sec sustained load
- [ ] All documentation complete (4+ new guides)
- [ ] Can deploy to staging environment
- [ ] Runbooks created for operations
- [ ] All quality checks pass (100+ tests)
- [ ] Ready for production use

**Performance Targets Met** (from architecture.md):
- [ ] Reducer execution < 1μs for typical state machines
- [ ] Effect execution overhead < 100μs (excluding I/O)
- [ ] Event replay >= 10,000 events/second
- [ ] End-to-end command latency < 10ms (excluding external services)
- [ ] Sustained load: 1000 commands/sec for 10+ minutes
- [ ] P95 latency < 100ms under sustained load
- [ ] No memory leaks during 1-hour soak test

**Testing Coverage**:
- [ ] Reducer composition utilities fully tested
- [ ] Saga timeout scenarios covered with Effect::Delay
- [ ] Checkout saga validated with real Redpanda
- [ ] Full stack integration tests (Postgres + Redpanda + App)
- [ ] Chaos engineering tests (network partitions, process crashes)

**Key Quote from Roadmap**: "Can handle 1000 commands/sec sustained load with full observability."

---

## Estimated Time Breakdown

Based on roadmap estimate of 1.5-2 weeks:

1. **Observability (tracing, metrics)**: 2-3 days
2. **Error handling (retries, circuit breakers, DLQ)**: 2-3 days
3. **Performance optimization**: 1-2 days
4. **Redpanda production features**: 1 day
5. **Database features**: 1 day
6. **Effect::DispatchCommand**: 1 day
7. **Production-ready example**: 3-4 days
8. **Load testing**: 1-2 days
9. **Documentation**: 2 days
10. **Validation & polish**: 1 day

**Total**: 15-21 days (2-3 weeks of full-time work)

**Note**: Roadmap estimates 1.5-2 weeks. Budget 2-3 weeks for safety, especially for load testing and optimization.

---

## Notes

### Phase 4 Focus

This phase transforms the framework from "working" to "production-ready." The focus is on:
- **Visibility**: Can we see what's happening?
- **Reliability**: Does it handle failures gracefully?
- **Performance**: Does it meet throughput/latency targets?
- **Operability**: Can we deploy and monitor it easily?

### Production-Ready Checklist

A framework is production-ready when:
- ✅ Observability: Tracing, metrics, logs
- ✅ Error handling: Retries, circuit breakers, DLQ
- ✅ Performance: Meets throughput/latency targets
- ✅ Deployment: Docker, Kubernetes examples
- ✅ Documentation: Operations runbooks
- ✅ Testing: Load tests, chaos tests

### Learning Resources

- OpenTelemetry Rust: https://docs.rs/opentelemetry/
- Tracing: https://docs.rs/tracing/
- Metrics: https://docs.rs/metrics/
- K6 Load Testing: https://k6.io/docs/
- Chaos Engineering: https://principlesofchaos.org/

---

## Conclusion

Phase 4 hardens the framework for production deployment. By adding observability, advanced error handling, and performance optimization, we ensure the framework can handle real-world loads and failures gracefully.

**Philosophy**: Production systems fail. The question is: can we observe, diagnose, and recover quickly?

Let's build production-grade infrastructure! 🚀
