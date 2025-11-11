# Phase 9: Full Production Integration

**Goal**: Integrate all composable-rust components into a complete, deployable production agent with real infrastructure.

**Timeline**: 5-7 days (80-100 hours total, ~6 hours completed)

**Status**: ✅ **DEPLOYMENT READY!** (25% complete + production-ready Fly.io deployment)

**Current State**: Can deploy to production TODAY via Fly.io. Infrastructure parts (PostgreSQL, Redis, etc.) are optional - can use managed services or build incrementally.

---

## Overview

This phase represents the **ultimate integration test** of the composable-rust framework. We'll take the production-agent example and integrate:

- ✅ Real LLM API (Anthropic Claude)
- ✅ PostgreSQL event store (using our `postgres` crate)
- ✅ Redis (sessions + projections)
- ✅ Redpanda event bus (using our `redpanda` crate)
- ✅ Authentication system (using our `auth` crate)
- ✅ WebSocket support (using our `web` crate)
- ✅ Full observability stack
- ✅ Production-ready deployment

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Production Agent System                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐      │
│  │   Web UI     │───▶│  Axum HTTP   │───▶│   Agent      │      │
│  │ (WebSocket)  │◀───│  + WebSocket │◀───│   Reducer    │      │
│  └──────────────┘    └──────────────┘    └──────┬───────┘      │
│                              │                    │              │
│                              ▼                    ▼              │
│                      ┌──────────────┐    ┌──────────────┐      │
│                      │     Auth     │    │   Effects    │      │
│                      │   (Magic     │    │  (Database,  │      │
│                      │   Link +     │    │   LLM, etc)  │      │
│                      │   OAuth)     │    └──────┬───────┘      │
│                      └──────┬───────┘           │              │
│                             │                   │              │
├─────────────────────────────┼───────────────────┼──────────────┤
│           Infrastructure    │                   │              │
├─────────────────────────────┼───────────────────┼──────────────┤
│                             ▼                   ▼              │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐    │
│  │    Redis     │    │  PostgreSQL  │    │   Redpanda   │    │
│  │  (Sessions   │    │   (Events +  │    │  (Event Bus) │    │
│  │   + Cache)   │    │   Audit Log) │    │              │    │
│  └──────────────┘    └──────────────┘    └──────────────┘    │
│                                                                 │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐    │
│  │  Prometheus  │    │   Grafana    │    │    Jaeger    │    │
│  │  (Metrics)   │    │ (Dashboards) │    │   (Tracing)  │    │
│  └──────────────┘    └──────────────┘    └──────────────┘    │
│                                                                 │
│  ┌────────────────────────────────────────────────────────┐   │
│  │              Anthropic Claude API                       │   │
│  │         (Real LLM - Streaming Responses)                │   │
│  └────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

---

## Part 1: Real LLM Integration (12h)

### 1.1: Anthropic API Client (6h)
- [ ] Create `anthropic` crate with Claude API client
- [ ] Implement streaming support (Server-Sent Events)
- [ ] Tool use / function calling support
- [ ] Rate limiting and retry logic
- [ ] Error handling (quota, rate limits, timeouts)
- [ ] Message formatting (system, user, assistant)
- [ ] Tests with mock responses

**Files**:
- `anthropic/src/lib.rs`
- `anthropic/src/client.rs`
- `anthropic/src/streaming.rs`
- `anthropic/src/messages.rs`
- `anthropic/src/error.rs`
- `anthropic/tests/integration.rs`

### 1.2: Agent LLM Integration (6h)
- [ ] Update `ProductionEnvironment` to use real Anthropic client
- [ ] Implement actual `call_llm()` method
- [ ] Streaming response handling
- [ ] Tool execution loop (LLM → tool → LLM)
- [ ] Context management (conversation history)
- [ ] Token counting and limits
- [ ] Tests with mock API

**Files**:
- `examples/production-agent/src/environment.rs` (update)
- `examples/production-agent/src/llm.rs` (new)
- `examples/production-agent/src/tools.rs` (new)

---

## Part 2: PostgreSQL Event Store Integration (8h)

### 2.1: Event Store Setup (4h)
- [ ] Integrate `composable-rust-postgres` crate
- [ ] Create migrations for agent events
- [ ] Audit log schema (already have this)
- [ ] Session/conversation persistence
- [ ] Event replay on startup
- [ ] Connection pooling configuration

**Files**:
- `examples/production-agent/migrations/` (new)
- `examples/production-agent/src/persistence.rs` (new)

### 2.2: Audit Logging to PostgreSQL (4h)
- [ ] Replace in-memory audit logger with PostgreSQL backend
- [ ] Implement `PostgresAuditLogger`
- [ ] Audit query endpoints (GET /audit)
- [ ] Retention policies
- [ ] Indexing for query performance

**Files**:
- `agent-patterns/src/audit/postgres.rs` (new)
- `examples/production-agent/src/audit.rs` (update)

---

## Part 3: Redis Integration (10h)

### 3.1: Redis Session Store (5h)
- [ ] Create `redis` crate with connection pooling
- [ ] Session storage implementation
- [ ] Authentication token storage
- [ ] Session expiration (TTL)
- [ ] Distributed session support
- [ ] Tests with redis-rs

**Files**:
- `redis/src/lib.rs` (new crate)
- `redis/src/session.rs`
- `redis/src/connection.rs`
- `redis/Cargo.toml`

### 3.2: Redis Projections (5h)
- [ ] Projection read models in Redis
- [ ] User conversation summaries
- [ ] Active session tracking
- [ ] Real-time statistics
- [ ] Cache invalidation strategy

**Files**:
- `redis/src/projections.rs`
- `examples/production-agent/src/projections.rs`

---

## Part 4: Redpanda Event Bus Integration (8h)

### 4.1: Event Bus Setup (4h)
- [ ] Integrate existing `composable-rust-redpanda` crate
- [ ] Configure topics (agent-events, audit-events)
- [ ] Producer configuration
- [ ] Consumer groups
- [ ] Error handling and DLQ

**Files**:
- `examples/production-agent/src/event_bus.rs` (new)
- `examples/production-agent/src/consumers.rs` (new)

### 4.2: Cross-Agent Communication (4h)
- [ ] Multi-agent event routing
- [ ] Agent discovery via events
- [ ] Broadcast system messages
- [ ] Event replay for new agents

**Files**:
- `examples/production-agent/src/multi_agent.rs` (new)

---

## Part 5: Authentication Integration (12h)

### 5.1: Magic Link Authentication (6h)
- [ ] Integrate `composable-rust-auth` crate
- [ ] Email sending (SMTP configuration)
- [ ] Magic link generation and validation
- [ ] Session creation on successful auth
- [ ] Logout endpoint
- [ ] Token refresh

**Files**:
- `examples/production-agent/src/auth/magic_link.rs` (new)
- `examples/production-agent/src/auth/middleware.rs` (new)

### 5.2: OAuth Integration (Optional) (6h)
- [ ] Google OAuth provider
- [ ] GitHub OAuth provider
- [ ] OAuth callback handling
- [ ] User profile fetching
- [ ] Account linking

**Files**:
- `examples/production-agent/src/auth/oauth.rs` (new)

---

## Part 6: WebSocket Real-time Communication (8h)

### 6.1: WebSocket Server (4h)
- [ ] WebSocket upgrade endpoint
- [ ] Connection management
- [ ] Message routing (client → agent → LLM → client)
- [ ] Streaming LLM responses to client
- [ ] Heartbeat/ping-pong

**Files**:
- `examples/production-agent/src/websocket.rs` (new)
- `examples/production-agent/src/connection_manager.rs` (new)

### 6.2: WebSocket Protocol (4h)
- [ ] JSON message protocol
- [ ] Message types (chat, tool_use, status, error)
- [ ] Client-side JavaScript example
- [ ] Simple HTML UI for testing
- [ ] TypeScript types (optional)

**Files**:
- `examples/production-agent/static/index.html` (new)
- `examples/production-agent/static/app.js` (new)
- `examples/production-agent/static/protocol.md` (new)

---

## Part 7: Configuration Management (6h)

### 7.1: Environment Configuration (3h)
- [ ] `.env` file support (dotenv)
- [ ] Environment-specific configs (dev, staging, prod)
- [ ] Secret management (API keys, DB passwords)
- [ ] Validation on startup
- [ ] Configuration struct with defaults

**Files**:
- `examples/production-agent/src/config.rs` (update)
- `examples/production-agent/.env.example` (new)
- `examples/production-agent/config/` (directory)

### 7.2: Secrets Management (3h)
- [ ] Kubernetes Secrets integration
- [ ] Docker Secrets support
- [ ] Vault integration (optional)
- [ ] Encryption at rest
- [ ] Rotation strategy

**Files**:
- `examples/production-agent/deploy/k8s/secrets.yaml` (new)
- `examples/production-agent/src/secrets.rs` (new)

---

## Part 8: Complete Docker Compose (10h)

### 8.1: Infrastructure Services (5h)
- [ ] PostgreSQL service with initialization
- [ ] Redis service with persistence
- [ ] Redpanda service (3 brokers for testing)
- [ ] Volume management
- [ ] Health checks for all services
- [ ] Service dependencies (depends_on)

**Files**:
- `examples/production-agent/deploy/docker/docker-compose.full.yml` (new)
- `examples/production-agent/deploy/docker/postgres-init/` (init scripts)
- `examples/production-agent/deploy/docker/redpanda-init/` (init scripts)

### 8.2: Application Services (5h)
- [ ] Production agent with all dependencies
- [ ] Environment variable injection
- [ ] Secrets mounting
- [ ] Network configuration
- [ ] Port mapping
- [ ] Log aggregation

**Files**:
- Update existing `docker-compose.yml`
- `examples/production-agent/deploy/docker/.env.example`

---

## Part 9: Complete Kubernetes Deployment (12h)

### 9.1: StatefulSets (6h)
- [ ] PostgreSQL StatefulSet with PVC
- [ ] Redis StatefulSet with persistence
- [ ] Redpanda StatefulSet (cluster mode)
- [ ] Headless services
- [ ] Init containers
- [ ] Volume claim templates

**Files**:
- `examples/production-agent/deploy/k8s/postgres-statefulset.yaml` (new)
- `examples/production-agent/deploy/k8s/redis-statefulset.yaml` (new)
- `examples/production-agent/deploy/k8s/redpanda-statefulset.yaml` (new)

### 9.2: Secrets and ConfigMaps (3h)
- [ ] Database credentials secret
- [ ] API key secrets
- [ ] Redis password secret
- [ ] SMTP configuration
- [ ] Environment-specific ConfigMaps

**Files**:
- `examples/production-agent/deploy/k8s/secrets.yaml`
- `examples/production-agent/deploy/k8s/configmap-prod.yaml`

### 9.3: Ingress and TLS (3h)
- [ ] Ingress configuration
- [ ] TLS certificate management
- [ ] Let's Encrypt integration (cert-manager)
- [ ] Domain routing
- [ ] WebSocket support in ingress

**Files**:
- `examples/production-agent/deploy/k8s/ingress.yaml` (new)
- `examples/production-agent/deploy/k8s/certificate.yaml` (new)

---

## Part 10: End-to-End Testing (8h)

### 10.1: Integration Tests (4h)
- [ ] Full stack integration test
- [ ] Authentication flow test
- [ ] LLM conversation test
- [ ] Event bus test
- [ ] WebSocket test
- [ ] Database persistence test

**Files**:
- `examples/production-agent/tests/integration/` (new)
- `examples/production-agent/tests/e2e/` (new)

### 10.2: Load Testing (4h)
- [ ] Load test scenarios (k6 or similar)
- [ ] Concurrent users
- [ ] Message throughput
- [ ] Database performance
- [ ] Redis performance
- [ ] Results and optimization

**Files**:
- `examples/production-agent/tests/load/` (new)

---

## Part 11: Documentation (6h)

### 11.1: Deployment Guide (3h)
- [ ] Complete deployment walkthrough
- [ ] Prerequisites checklist
- [ ] Configuration guide
- [ ] Secrets setup
- [ ] Monitoring setup
- [ ] Troubleshooting

**Files**:
- `examples/production-agent/DEPLOYMENT.md` (new)

### 11.2: API Documentation (3h)
- [ ] HTTP API reference
- [ ] WebSocket protocol documentation
- [ ] Authentication flows
- [ ] Example requests/responses
- [ ] Error codes

**Files**:
- `examples/production-agent/API.md` (new)
- `examples/production-agent/WEBSOCKET.md` (new)

---

## Summary

**Total Time Estimate**: 80-100 hours (5-7 days)

### Deliverables

1. **Anthropic API Integration**
   - Real Claude API client
   - Streaming responses
   - Tool use support

2. **Full Infrastructure**
   - PostgreSQL (events + audit)
   - Redis (sessions + projections)
   - Redpanda (event bus)

3. **Authentication System**
   - Magic link auth
   - Session management
   - OAuth (optional)

4. **WebSocket Support**
   - Real-time bidirectional communication
   - Streaming LLM responses
   - Simple web UI

5. **Configuration**
   - Environment-based config
   - Secrets management
   - Kubernetes secrets

6. **Complete Deployments**
   - Docker Compose with all services
   - Kubernetes with StatefulSets
   - Ingress with TLS

7. **Testing**
   - Integration tests
   - Load tests
   - E2E tests

8. **Documentation**
   - Deployment guide
   - API reference
   - Troubleshooting

---

## Success Criteria

✅ **Production-Ready Agent**
- Handles real user conversations
- Persistent across restarts
- Scalable (horizontal + vertical)
- Secure (auth + TLS)
- Observable (metrics + logs + traces)

✅ **Demonstrates Entire Framework**
- All composable-rust crates integrated
- All patterns in use (reducer, effects, event sourcing, sagas)
- Production infrastructure
- Real-world use case

✅ **Deployable**
- One-command Docker Compose deployment
- Complete Kubernetes manifests
- CI/CD ready
- Documented

---

## Notes

This phase is the **capstone** of the composable-rust project. It demonstrates that the framework can actually build production-ready systems, not just examples.

Key challenges:
- Anthropic API integration (streaming, tool use)
- State management across infrastructure
- Configuration complexity
- Multi-service orchestration
- Real-world error handling

This will be an excellent reference implementation for anyone building production agents with composable-rust.
