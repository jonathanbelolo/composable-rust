# Composable Rust Documentation

**Comprehensive guides for building production-ready event-driven systems in Rust**

## Current Status: Phase 5 (~75% Complete) 🚧

Composable Rust is **production-ready** with core functionality complete. Phase 5 focuses on developer experience improvements (templates, CLI tools).

✅ **Phases 0-4 Complete**: Core abstractions, event sourcing, sagas, observability, production hardening
✅ **Web Framework Ready**: HTTP APIs, WebSocket real-time events, authentication
🚧 **Phase 5 In Progress**: Developer experience enhancements (~75% complete)

---

## 📖 Documentation Index

### Getting Started

Start here if you're new to Composable Rust:

1. **[Getting Started Guide](./getting-started.md)** - Tutorial walkthrough with Counter example
   - Five fundamental types (State, Action, Reducer, Effect, Environment)
   - Building your first feature
   - HTTP APIs and WebSocket support
   - Testing patterns

2. **[Core Concepts](./concepts.md)** - Deep dive into architecture
   - State, Action, Reducer, Effect, Environment
   - Effect composition (map, chain, merge)
   - Unidirectional data flow
   - TestStore for deterministic testing

3. **[API Reference](./api-reference.md)** - Complete API documentation
   - `Store::new()`, `Store::send()`, `Store::state()`
   - Effect variants and methods
   - Environment traits (Clock, EventStore, EventBus, EmailProvider)

### Architecture & Patterns

Critical reading for understanding event-driven systems:

- **[Consistency Patterns](./consistency-patterns.md)** ⚠️ **REQUIRED READING**
  - When to use projections vs event store
  - Saga patterns avoiding projection dependencies
  - Event design (fat events vs thin events)
  - WebSocket real-time updates
  - Email notifications with eventual consistency
  - Testing patterns for eventual consistency

- **[Saga Patterns](./saga-patterns.md)** - Multi-aggregate coordination
  - Orchestration vs choreography
  - Compensation and rollback
  - State machines for sagas
  - Checkout saga example (Order + Payment + Inventory)

- **[Event Design Guidelines](./event-design-guidelines.md)** - Event schema best practices
  - Fat events for workflows
  - Versioning strategies
  - Backward/forward compatibility

- **[Implementation Decisions](./implementation-decisions.md)** - Why the architecture is designed this way
  - Why `&mut State`?
  - Why effects as values?
  - Why static dispatch?
  - Alternative approaches considered

### Infrastructure & Integration

Production deployment guides:

- **[Database Setup](./database-setup.md)** - PostgreSQL configuration
  - Local development setup
  - Docker Compose configuration
  - Connection pooling

- **[Production Database](./production-database.md)** - Production operations
  - Migrations with sqlx::migrate!()
  - Backup and restore
  - Performance tuning
  - Monitoring queries

- **[Redpanda Setup](./redpanda-setup.md)** - Kafka-compatible event bus
  - Local development with Docker
  - Kafka protocol configuration
  - Consumer groups
  - Monitoring with Redpanda Console

- **[Event Bus Guide](./event-bus.md)** - Cross-aggregate communication
  - EventBus trait abstraction
  - InMemoryEventBus (testing)
  - RedpandaEventBus (production)
  - At-least-once delivery semantics

### Web & Real-Time Communication

Building HTTP APIs and WebSocket applications:

- **[WebSocket Guide](./websocket.md)** - Real-time bidirectional communication
  - Architecture with Store integration
  - Message protocol (Command/Event/Error/Ping/Pong)
  - JavaScript/TypeScript client examples
  - React hooks
  - Testing, troubleshooting, performance

- **[Email Providers Guide](./email-providers.md)** - Email integration
  - SmtpEmailProvider (production with Lettre)
  - ConsoleEmailProvider (development/testing)
  - Configuration and environment setup
  - HTML email templates
  - Troubleshooting SMTP issues

### Error Handling & Observability

Production-grade reliability:

- **[Error Handling](./error-handling.md)** - Three-tier error model
  - Reducer panics (bugs)
  - Effect failures (runtime errors)
  - Domain errors (business logic)
  - Retry policies, circuit breakers, Dead Letter Queue

- **[Observability](./observability.md)** - Tracing and metrics
  - Tracing integration (OpenTelemetry)
  - Metrics collection
  - Distributed tracing
  - Production monitoring

### Data Management

Event sourcing and projections:

- **[Projections Guide](./projections.md)** - Read model patterns
  - When to use projections
  - PostgreSQL projections
  - Redis projections
  - Elasticsearch projections
  - Projection testing

### Examples & References

Working code examples:

- **[Examples Overview](./examples.md)** - All example applications
  - Counter (Phase 1): Core abstractions
  - Order Processing (Phase 2): Event sourcing + HTTP + WebSocket
  - Order Projection (Phase 2): Projection patterns
  - Checkout Saga (Phase 3): Multi-aggregate coordination
  - Ticketing (Phase 5): Complete production application

---

## 🎯 Learning Paths

### Path 1: First-Time User

**Goal**: Build your first Composable Rust application

1. [Getting Started Guide](./getting-started.md) - Understand the five types
2. [Counter Example](../examples/counter/README.md) - See it in action
3. [Core Concepts](./concepts.md) - Deep dive
4. [API Reference](./api-reference.md) - Complete reference

**Next**: Build a simple TODO list following the Counter pattern

### Path 2: Event Sourcing

**Goal**: Build an event-sourced aggregate

1. [Getting Started Guide](./getting-started.md) - Foundation
2. [Database Setup](./database-setup.md) - PostgreSQL configuration
3. [Order Processing Example](../examples/order-processing/) - Complete implementation
4. [Production Database](./production-database.md) - Migrations and operations

**Next**: Build your own event-sourced aggregate

### Path 3: Sagas & Coordination

**Goal**: Coordinate multiple aggregates in a workflow

1. [Consistency Patterns](./consistency-patterns.md) - ⚠️ **Required**
2. [Saga Patterns](./saga-patterns.md) - Orchestration vs choreography
3. [Event Design Guidelines](./event-design-guidelines.md) - Fat events
4. [Redpanda Setup](./redpanda-setup.md) - Event bus configuration
5. [Checkout Saga Example](../examples/checkout-saga/) - Working implementation

**Next**: Build a multi-aggregate saga

### Path 4: Web & Real-Time

**Goal**: Build HTTP APIs with WebSocket real-time updates

1. [Getting Started Guide](./getting-started.md) - See "Building HTTP APIs" section
2. [WebSocket Guide](./websocket.md) - Real-time communication
3. [Order Processing Example](../examples/order-processing/) - HTTP + WebSocket
4. [Email Providers Guide](./email-providers.md) - Notifications

**Next**: Add HTTP API and WebSocket to your application

### Path 5: Production Deployment

**Goal**: Deploy to production

1. [Production Database](./production-database.md) - Migrations, backups
2. [Redpanda Setup](./redpanda-setup.md) - Event bus deployment
3. [Observability](./observability.md) - Tracing and metrics
4. [Error Handling](./error-handling.md) - Retries, circuit breakers, DLQ
5. [Ticketing Example](../examples/ticketing/) - Complete production setup

**Next**: Deploy your application

---

## 🔧 Crate Documentation

### Core Crates

- **[`composable-rust-core`](../core/)** - Core traits and types
  - Reducer, Effect, Environment traits
  - EventStore, EventBus abstractions
  - No I/O dependencies (pure abstractions)

- **[`composable-rust-runtime`](../runtime/)** - Store and effect execution
  - Store implementation
  - Effect executor
  - Tracing, metrics, retry policies, circuit breakers

- **[`composable-rust-testing`](../testing/)** - Test utilities and mocks
  - TestStore for deterministic testing
  - FixedClock for time control
  - InMemoryEventStore, InMemoryEventBus
  - MockDatabase, MockHttpClient

### Infrastructure Crates

- **[`composable-rust-postgres`](../postgres/)** - PostgreSQL event store
  - Event persistence (append/load)
  - Migrations with sqlx::migrate!()
  - Batch operations
  - Connection pooling

- **[`composable-rust-redpanda`](../redpanda/)** - Kafka-compatible event bus
  - RedpandaEventBus implementation
  - At-least-once delivery
  - Consumer groups
  - Testcontainers integration

### Web & Auth Crates

- **[`composable-rust-web`](../web/)** - HTTP & WebSocket framework
  - Generic WebSocket handler
  - Health check handlers
  - Axum integration

- **[`composable-rust-auth`](../auth/)** - Authentication framework
  - Magic link authentication
  - OAuth providers (Google, GitHub)
  - Passkey/WebAuthn support
  - Email providers (SMTP, Console)
  - Rate limiting, risk scoring

---

## 🛠 Development Resources

### Architecture & Planning

- **[Architecture Specification](../specs/architecture.md)** - Comprehensive design (2,800+ lines)
  - Complete architectural vision
  - CQRS and Event Sourcing patterns
  - Phase-by-phase implementation plan

- **[Implementation Roadmap](../plans/implementation-roadmap.md)** - Development timeline
  - Phase 0-7 breakdown
  - Completion criteria for each phase
  - Strategic decisions (serialization, event store, event bus)

### Phase Reviews

- [Phase 1 Review](../plans/phase-1/PHASE1_REVIEW.md) - Core abstractions completion
- [Phase 2 Review](../plans/phase-2/PHASE2B_COMPLETE.md) - Event sourcing completion
- [Phase 3 Review](../plans/phase-3/TODO.md) - Sagas & coordination completion
- [Phase 4 Review](../plans/phase-4/TODO.md) - Production hardening completion
- [Phase 5 Status](../plans/phase-5/TODO.md) - Developer experience (current)

### Code Quality

- **[CLAUDE.md](../CLAUDE.md)** - AI agent instructions
  - Modern Rust patterns (Edition 2024)
  - Coding standards (clippy pedantic)
  - Project structure
  - Development workflows

- **[CONTRIBUTING.md](../CONTRIBUTING.md)** - Contribution guidelines
  - Code standards
  - Testing requirements
  - Documentation requirements

---

## 🆘 Troubleshooting & Support

### Common Issues

1. **Projection lag / race conditions** → [Consistency Patterns](./consistency-patterns.md)
2. **SMTP email errors** → [Email Providers Guide](./email-providers.md) - Troubleshooting section
3. **WebSocket connection issues** → [WebSocket Guide](./websocket.md) - Troubleshooting section
4. **Database migrations** → [Production Database](./production-database.md)
5. **Redpanda/Kafka setup** → [Redpanda Setup](./redpanda-setup.md)
6. **Error handling strategies** → [Error Handling](./error-handling.md)

### Getting Help

- **Architecture questions**: [Architecture Specification](../specs/architecture.md)
- **API questions**: [API Reference](./api-reference.md)
- **Implementation patterns**: [Examples](./examples.md)
- **Production issues**: [Observability](./observability.md), [Error Handling](./error-handling.md)

---

## 📊 Framework Capabilities

### ✅ Production-Ready Features

| Capability | Status | Documentation |
|------------|--------|---------------|
| Core Abstractions | ✅ Complete | [Getting Started](./getting-started.md), [Concepts](./concepts.md) |
| Event Sourcing | ✅ Complete | [Database Setup](./database-setup.md), [Production Database](./production-database.md) |
| Sagas & Coordination | ✅ Complete | [Saga Patterns](./saga-patterns.md), [Consistency Patterns](./consistency-patterns.md) |
| HTTP APIs | ✅ Complete | [Getting Started](./getting-started.md), [Order Processing Example](../examples/order-processing/) |
| WebSocket Real-Time | ✅ Complete | [WebSocket Guide](./websocket.md) |
| Authentication | ✅ Complete | Auth crate (magic links, OAuth, passkeys) |
| Email Integration | ✅ Complete | [Email Providers Guide](./email-providers.md) |
| Observability | ✅ Complete | [Observability](./observability.md) |
| Error Handling | ✅ Complete | [Error Handling](./error-handling.md) |
| Production Database | ✅ Complete | [Production Database](./production-database.md) |

### 🚧 In Development

| Capability | Status | ETA |
|------------|--------|-----|
| Project Templates | Planning | Phase 5 |
| CLI Scaffolding | Planning | Phase 5 |
| Additional Examples | Planning | Phase 5 |

---

## 📚 External Resources

### Rust Edition 2024

- [Rust 1.85.0 Release Notes](https://blog.rust-lang.org/2025/01/09/Rust-1.85.0.html)
- [Edition 2024 Features](https://doc.rust-lang.org/edition-guide/rust-2024/index.html)

### CQRS & Event Sourcing

- [CQRS Pattern](https://martinfowler.com/bliki/CQRS.html) - Martin Fowler
- [Event Sourcing](https://martinfowler.com/eaaDev/EventSourcing.html) - Martin Fowler
- [Eventual Consistency](https://www.allthingsdistributed.com/2008/12/eventually_consistent.html) - Werner Vogels

### Functional Architecture

- [Swift Composable Architecture (TCA)](https://github.com/pointfreeco/swift-composable-architecture) - Inspiration
- [Redux Pattern](https://redux.js.org/understanding/thinking-in-redux/three-principles) - Unidirectional data flow

---

**Last Updated**: 2025-01-09
**Framework Version**: Phase 5 (~75% Complete)
**Status**: ✅ Production-Ready (Core functionality complete)
