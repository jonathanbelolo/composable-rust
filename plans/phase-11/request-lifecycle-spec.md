# Request Lifecycle Management Specification

**Phase:** 11
**Status:** Planning
**Author:** Claude Code
**Date:** 2025-01-16

---

## Overview

This specification defines a **Request Lifecycle Management** system for HTTP-based event-sourced applications. It introduces a dedicated `RequestLifecycleStore` that tracks the complete lifecycle of HTTP requests from initiation through domain event processing, projection updates, and external operations (emails, notifications, webhooks).

The key insight: **Request lifecycle tracking is NOT business domain logic**. It's infrastructure concern orthogonal to domain aggregates like Event, Reservation, Inventory.

---

## Problem Statement

### Current Architecture Issues

1. **No way to know when a request is "fully processed"**
   - Domain event emitted? ✅
   - Projections updated? ❓
   - Emails sent? ❓
   - External webhooks called? ❓

2. **Tests must guess/sleep to wait for eventual consistency**
   - `tokio::time::sleep(Duration::from_millis(500))` is brittle
   - No reliable signal that projections have caught up

3. **Mixing concerns if we add correlation tracking to domain**
   - `Event` aggregate shouldn't know about HTTP request IDs
   - Business domain polluted with infrastructure concerns

4. **No observability for in-flight requests**
   - Can't answer: "Which requests are stuck?"
   - Can't track: "How long does a request take end-to-end?"

### What We Need

A separate **Request Lifecycle Store** that:
- Tracks HTTP requests from start to completion
- Coordinates multiple async operations (domain events, projections, external calls)
- Provides WebSocket notifications when requests complete
- Enables reliable integration testing
- Keeps business domain clean and reusable

---

## Architecture

### High-Level Flow

```
┌─────────────────┐
│  HTTP Request   │
│  POST /events   │
└────────┬────────┘
         │
         ▼
┌─────────────────────────────────────────────────────────┐
│          RequestLifecycleStore (New!)                   │
│  1. Create RequestLifecycle aggregate                   │
│  2. Assign correlation_id                               │
│  3. Track expected operations (domain event, N          │
│     projections, M external operations)                 │
└────────┬────────────────────────────────────────────────┘
         │
         │ Dispatch business action
         ▼
┌─────────────────────────────────────────────────────────┐
│          Business Domain Store                          │
│          (Event/Reservation/Inventory)                  │
│  1. Process action via reducer                          │
│  2. Emit domain event (e.g., EventCreated)              │
│  3. Publish to EventBus with correlation_id in metadata │
└────────┬────────────────────────────────────────────────┘
         │
         │ Domain event → EventBus (Redpanda)
         │
    ┌────┴─────┐
    │          │
    ▼          ▼
┌─────────┐  ┌──────────────────┐
│Projection│  │RequestLifecycle  │
│Managers │  │Store (subscribes │
│         │  │to EventBus)      │
└────┬────┘  └────────┬─────────┘
     │                │
     │ Update         │ Mark domain_event_emitted = true
     │ projection     │
     │                │
     ▼                ▼
┌─────────────────────────────────────────────────────────┐
│  Projection emits ProjectionCompleted event             │
│  - correlation_id                                       │
│  - projection_name ("events", "available_seats", ...)   │
└────────┬────────────────────────────────────────────────┘
         │
         │ ProjectionCompleted → EventBus
         ▼
┌─────────────────────────────────────────────────────────┐
│          RequestLifecycleStore (consumes event)         │
│  1. Match by correlation_id                             │
│  2. Mark projection as completed                        │
│  3. Check if ALL operations done                        │
│  4. If YES → Emit RequestCompleted event                │
└────────┬────────────────────────────────────────────────┘
         │
         │ RequestCompleted → EventBus → WebSocket
         ▼
┌─────────────────────────────────────────────────────────┐
│          WebSocket broadcasts to clients                │
│  {                                                      │
│    "type": "request_completed",                         │
│    "correlation_id": "uuid",                            │
│    "duration_ms": 47,                                   │
│    "operations_completed": ["domain_event",             │
│      "events_projection", "available_seats_projection"] │
│  }                                                      │
└─────────────────────────────────────────────────────────┘
```

### Key Components

#### 1. RequestLifecycle Aggregate

**State:**
```rust
pub struct RequestLifecycle {
    /// Unique correlation ID for this request
    pub correlation_id: CorrelationId,

    /// HTTP request metadata (method, path, user_id)
    pub request_metadata: RequestMetadata,

    /// When the request was initiated
    pub initiated_at: DateTime<Utc>,

    /// Domain event emitted (e.g., "EventCreated", "ReservationInitiated")
    pub domain_event: Option<String>,

    /// Projections that need to be updated
    pub expected_projections: HashSet<String>,

    /// Projections that have completed
    pub completed_projections: HashSet<String>,

    /// External operations (emails, webhooks, etc.)
    pub expected_external_ops: HashSet<String>,

    /// External operations that have completed
    pub completed_external_ops: HashSet<String>,

    /// Overall request status
    pub status: RequestStatus,

    /// When the request completed (all operations done)
    pub completed_at: Option<DateTime<Utc>>,

    /// Error if request failed
    pub error: Option<String>,
}

pub enum RequestStatus {
    /// Request initiated, waiting for operations
    Pending,

    /// Domain event emitted, waiting for projections
    DomainEventEmitted,

    /// All projections updated, waiting for external ops
    ProjectionsCompleted,

    /// Everything done successfully
    Completed,

    /// Request failed (timeout, error)
    Failed,

    /// Request cancelled by user
    Cancelled,
}

pub struct RequestMetadata {
    pub method: String,      // "POST"
    pub path: String,        // "/api/events"
    pub user_id: Option<Uuid>,
    pub ip_address: Option<String>,
}
```

**Actions:**
```rust
pub enum RequestLifecycleAction {
    /// Initiate a new request lifecycle
    InitiateRequest {
        correlation_id: CorrelationId,
        metadata: RequestMetadata,
        expected_projections: HashSet<String>,
        expected_external_ops: HashSet<String>,
    },

    /// Mark domain event as emitted
    DomainEventEmitted {
        correlation_id: CorrelationId,
        event_type: String,
    },

    /// Mark a projection as completed
    ProjectionCompleted {
        correlation_id: CorrelationId,
        projection_name: String,
    },

    /// Mark an external operation as completed
    ExternalOperationCompleted {
        correlation_id: CorrelationId,
        operation_name: String,
    },

    /// Mark request as failed
    RequestFailed {
        correlation_id: CorrelationId,
        error: String,
    },

    /// Cancel a request
    CancelRequest {
        correlation_id: CorrelationId,
    },

    /// Timeout a request (after X seconds with no completion)
    TimeoutRequest {
        correlation_id: CorrelationId,
    },
}
```

**Reducer Logic:**
```rust
impl Reducer for RequestLifecycleReducer {
    fn reduce(
        &self,
        state: &mut RequestLifecycleState,
        action: RequestLifecycleAction,
        env: &impl RequestLifecycleEnvironment,
    ) -> Vec<Effect<RequestLifecycleAction>> {
        match action {
            RequestLifecycleAction::InitiateRequest { correlation_id, metadata, expected_projections, expected_external_ops } => {
                let lifecycle = RequestLifecycle {
                    correlation_id,
                    request_metadata: metadata,
                    initiated_at: env.clock().now(),
                    domain_event: None,
                    expected_projections,
                    completed_projections: HashSet::new(),
                    expected_external_ops,
                    completed_external_ops: HashSet::new(),
                    status: RequestStatus::Pending,
                    completed_at: None,
                    error: None,
                };

                state.insert(correlation_id, lifecycle);

                // Schedule timeout check (e.g., 30 seconds)
                vec![
                    Effect::Delay(
                        Duration::from_secs(30),
                        Box::new(RequestLifecycleAction::TimeoutRequest { correlation_id })
                    )
                ]
            }

            RequestLifecycleAction::ProjectionCompleted { correlation_id, projection_name } => {
                if let Some(lifecycle) = state.get_mut(&correlation_id) {
                    lifecycle.completed_projections.insert(projection_name);

                    // Check if all projections done
                    if lifecycle.completed_projections == lifecycle.expected_projections
                        && lifecycle.completed_external_ops == lifecycle.expected_external_ops {
                        lifecycle.status = RequestStatus::Completed;
                        lifecycle.completed_at = Some(env.clock().now());

                        // Emit RequestCompleted event
                        vec![
                            Effect::PublishEvent(
                                "request-lifecycle-events",
                                RequestLifecycleEvent::RequestCompleted {
                                    correlation_id,
                                    duration_ms: lifecycle.completed_at.unwrap()
                                        .signed_duration_since(lifecycle.initiated_at)
                                        .num_milliseconds(),
                                }
                            )
                        ]
                    } else if lifecycle.completed_projections == lifecycle.expected_projections {
                        lifecycle.status = RequestStatus::ProjectionsCompleted;
                        vec![Effect::None]
                    } else {
                        vec![Effect::None]
                    }
                } else {
                    vec![Effect::None]
                }
            }

            // ... other actions
        }
    }
}
```

#### 2. Projection Completion Events

Each projection must emit a completion event after updating:

```rust
// In PostgresEventsProjection::apply_event()
async fn apply_event(&self, event: &TicketingEvent) -> Result<()> {
    // Extract correlation_id from event metadata
    let correlation_id = extract_correlation_id(event);

    // Apply event to projection (insert/update database)
    self.update_projection(event).await?;

    // Emit ProjectionCompleted event
    if let Some(corr_id) = correlation_id {
        self.event_bus.publish(
            "request-lifecycle-events",
            &RequestLifecycleEvent::ProjectionCompleted {
                correlation_id: corr_id,
                projection_name: "events".to_string(),
            }
        ).await?;
    }

    Ok(())
}
```

**All 4 projections must do this:**
- `PostgresEventsProjection` → "events"
- `PostgresAvailableSeatsProjection` → "available_seats"
- `PostgresSalesAnalyticsProjection` → "sales_analytics"
- `PostgresCustomerHistoryProjection` → "customer_history"

#### 3. WebSocket Event Broadcasting

WebSocket must subscribe to ALL EventBus topics and broadcast:

```rust
// src/api/websocket.rs
pub async fn handle_websocket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| async move {
        // Subscribe to ALL topics
        let mut inventory_consumer = state.event_bus.subscribe(&state.config.redpanda.inventory_topic).await.unwrap();
        let mut reservation_consumer = state.event_bus.subscribe(&state.config.redpanda.reservation_topic).await.unwrap();
        let mut payment_consumer = state.event_bus.subscribe(&state.config.redpanda.payment_topic).await.unwrap();
        let mut lifecycle_consumer = state.event_bus.subscribe("request-lifecycle-events").await.unwrap();

        // Broadcast ALL events to client
        loop {
            tokio::select! {
                Some(event) = inventory_consumer.next() => {
                    broadcast_to_client(&socket, event).await;
                }
                Some(event) = reservation_consumer.next() => {
                    broadcast_to_client(&socket, event).await;
                }
                Some(event) = payment_consumer.next() => {
                    broadcast_to_client(&socket, event).await;
                }
                Some(event) = lifecycle_consumer.next() => {
                    broadcast_to_client(&socket, event).await;
                }
            }
        }
    })
}
```

#### 4. HTTP Handler Integration

HTTP handlers wrap business operations:

```rust
// src/api/events.rs
pub async fn create_event(
    session: SessionUser,
    State(state): State<AppState>,
    Json(request): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<CreateEventResponse>), AppError> {
    let correlation_id = CorrelationId::new();
    let event_id = EventId::new();

    // 1. Initiate request lifecycle
    state.request_lifecycle_store.handle(
        RequestLifecycleAction::InitiateRequest {
            correlation_id,
            metadata: RequestMetadata {
                method: "POST".to_string(),
                path: "/api/events".to_string(),
                user_id: Some(session.user_id.0),
                ip_address: None,
            },
            expected_projections: hashset!["events".to_string(), "available_seats".to_string()],
            expected_external_ops: hashset![],
        }
    ).await?;

    // 2. Create domain event with correlation_id in metadata
    let command = EventAction::CreateEvent {
        id: event_id,
        name: request.name,
        venue: request.venue,
        date: request.date,
        pricing_tiers: request.pricing_tiers,
    };

    // Add correlation_id to event metadata before publishing
    let metadata = serde_json::json!({
        "correlation_id": correlation_id.as_uuid(),
        "user_id": session.user_id.0,
    });

    // 3. Handle through business domain store
    state.event_service.handle_with_metadata(
        StreamId::from(format!("event-{}", event_id.as_uuid())),
        command,
        metadata,
    ).await?;

    // 4. Initialize inventory (also with correlation_id)
    for section in &request.venue.sections {
        state.inventory_service.handle_with_metadata(
            StreamId::from(format!("inventory-{}-{}", event_id.as_uuid(), section.name)),
            InventoryAction::InitializeInventory { /* ... */ },
            metadata.clone(),
        ).await?;
    }

    // 5. Return immediately (client waits for RequestCompleted via WebSocket)
    Ok((
        StatusCode::ACCEPTED, // 202 Accepted (not 201 Created - processing async)
        Json(CreateEventResponse {
            event_id: *event_id.as_uuid(),
            correlation_id: *correlation_id.as_uuid(),
            message: "Event creation initiated. Listen for completion on WebSocket.".to_string(),
        }),
    ))
}
```

---

## Integration Testing Pattern

### Before (Broken)

```rust
#[tokio::test]
async fn test_event_crud_operations() {
    // Create event
    let response = client.post("/api/events").json(&payload).send().await?;
    assert_eq!(response.status(), 201);

    // Immediately query (WRONG - projection not updated yet!)
    let list = client.get("/api/events").send().await?;
    assert_eq!(list.status(), 500); // ❌ FAILS - projection empty
}
```

### After (Correct)

```rust
#[tokio::test]
async fn test_event_crud_operations() {
    // 1. Open WebSocket FIRST
    let (mut ws_stream, _) = tokio_tungstenite::connect_async(
        "ws://localhost:8080/ws"
    ).await?;

    // 2. Create event
    let response = client.post("/api/events").json(&payload).send().await?;
    assert_eq!(response.status(), 202); // 202 Accepted

    let body: CreateEventResponse = response.json().await?;
    let correlation_id = body.correlation_id;

    // 3. Wait for events via WebSocket
    let mut domain_event_received = false;
    let mut events_projection_completed = false;
    let mut available_seats_projection_completed = false;
    let mut request_completed = false;

    while let Some(Ok(msg)) = ws_stream.next().await {
        if let Message::Text(text) = msg {
            let event: serde_json::Value = serde_json::from_str(&text)?;

            // Check correlation_id matches
            if event["correlation_id"] != correlation_id {
                continue;
            }

            match event["type"].as_str() {
                Some("event_created") => {
                    println!("  ✅ Domain event received: EventCreated");
                    domain_event_received = true;
                }
                Some("projection_completed") => {
                    match event["projection_name"].as_str() {
                        Some("events") => {
                            println!("  ✅ Events projection updated");
                            events_projection_completed = true;
                        }
                        Some("available_seats") => {
                            println!("  ✅ Available seats projection updated");
                            available_seats_projection_completed = true;
                        }
                        _ => {}
                    }
                }
                Some("request_completed") => {
                    println!("  ✅ Request fully processed in {}ms", event["duration_ms"]);
                    request_completed = true;
                    break;
                }
                _ => {}
            }
        }
    }

    // 4. Assert all events received
    assert!(domain_event_received, "Should receive domain event");
    assert!(events_projection_completed, "Events projection should complete");
    assert!(available_seats_projection_completed, "Available seats projection should complete");
    assert!(request_completed, "Request should complete");

    // 5. NOW query read side (guaranteed consistent)
    let list = client.get("/api/events").send().await?;
    assert_eq!(list.status(), 200); // ✅ PASSES - projection updated!

    let events: ListEventsResponse = list.json().await?;
    assert!(events.events.iter().any(|e| e.id == body.event_id));
}
```

---

## Implementation Plan

### Phase 11.1: RequestLifecycle Core

**Files to create:**
- `src/request_lifecycle/mod.rs` - Module export
- `src/request_lifecycle/types.rs` - CorrelationId, RequestLifecycle, RequestMetadata, RequestStatus
- `src/request_lifecycle/actions.rs` - RequestLifecycleAction enum
- `src/request_lifecycle/reducer.rs` - RequestLifecycleReducer implementation
- `src/request_lifecycle/environment.rs` - RequestLifecycleEnvironment trait
- `src/request_lifecycle/store.rs` - RequestLifecycleStore (wraps Store<RequestLifecycleReducer>)

**Tests:**
- Unit tests for reducer logic
- Test correlation_id tracking
- Test completion detection (all projections done)
- Test timeout handling

### Phase 11.2: Projection Completion Events

**Files to modify:**
- `src/projections/available_seats_postgres.rs` - Emit ProjectionCompleted
- `src/projections/events_postgres.rs` - Emit ProjectionCompleted
- `src/projections/sales_analytics_postgres.rs` - Emit ProjectionCompleted
- `src/projections/customer_history_postgres.rs` - Emit ProjectionCompleted

**Changes needed:**
- Add `event_bus: Arc<dyn EventBus>` to each projection struct
- Extract correlation_id from event metadata
- After updating projection, publish ProjectionCompleted event

**Tests:**
- Verify each projection emits completion event
- Verify correlation_id is preserved

### Phase 11.3: WebSocket Event Broadcasting

**Files to modify:**
- `src/api/websocket.rs` - Subscribe to all topics, broadcast all events

**Changes needed:**
- Subscribe to: inventory_topic, reservation_topic, payment_topic, request-lifecycle-events
- Use `tokio::select!` to multiplex all consumers
- Broadcast every event to connected clients (with filtering by correlation_id on client side)

**Tests:**
- Integration test: Connect WebSocket, create event, verify all events received

### Phase 11.4: HTTP Handler Integration

**Files to modify:**
- `src/api/events.rs` - Wrap create_event, update_event, delete_event
- `src/api/reservations.rs` - Wrap create_reservation, cancel_reservation
- `src/api/payments.rs` - Wrap process_payment

**Changes needed:**
- Generate correlation_id for each request
- Initiate RequestLifecycle before business operation
- Add correlation_id to event metadata
- Return 202 Accepted (not 201 Created)
- Return correlation_id in response

**Tests:**
- Verify correlation_id returned in response
- Verify 202 status code

### Phase 11.5: Integration Tests Rewrite

**Files to modify:**
- `tests/full_deployment_test.rs` - Rewrite all 7 tests

**Pattern for each test:**
1. Open WebSocket
2. Make HTTP request
3. Wait for RequestCompleted event (with correlation_id)
4. Assert all intermediate events received
5. Query read side
6. Assert results

**Tests to rewrite:**
- test_event_crud_operations
- test_availability_queries
- test_reservation_flow
- test_payment_processing
- test_analytics_queries
- test_magic_link_authentication (may not need lifecycle tracking)
- test_health_check (no lifecycle tracking)

---

## Future Extensions

### 1. External Operations Tracking

Track emails, SMS, webhooks:

```rust
// After sending email
state.request_lifecycle_store.handle(
    RequestLifecycleAction::ExternalOperationCompleted {
        correlation_id,
        operation_name: "confirmation_email_sent".to_string(),
    }
).await?;
```

### 2. Request Timeline Projection

Create a projection that stores full request timelines:

```sql
CREATE TABLE request_timelines (
    correlation_id UUID PRIMARY KEY,
    request_metadata JSONB,
    initiated_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    duration_ms INT,
    events JSONB[] -- Array of all events in order
);
```

Useful for:
- Debugging slow requests
- User-facing "order status" pages
- Compliance/audit logs

### 3. Request Metrics

Emit metrics for observability:
- `request_duration_ms` histogram
- `request_status` counter (completed, failed, timeout)
- `projection_lag_ms` histogram (time from domain event to projection completion)

### 4. Idempotency Keys

Tie correlation_id to idempotency keys for safe retries:

```rust
pub struct RequestLifecycle {
    pub correlation_id: CorrelationId,
    pub idempotency_key: Option<String>, // From Idempotency-Key header
    // ...
}
```

If client retries with same idempotency key → return existing correlation_id.

---

## Benefits Summary

### For Testing
✅ **Deterministic integration tests** - No more `tokio::time::sleep()`
✅ **Observable async operations** - See exactly what happened and when
✅ **Reliable assertions** - Query read side only after full consistency

### For Production
✅ **Request observability** - Track in-flight requests, detect stuck operations
✅ **Performance monitoring** - Measure end-to-end latency by component
✅ **User experience** - Real-time progress updates via WebSocket
✅ **Debugging** - Full audit trail of every request

### For Architecture
✅ **Clean separation** - Business domain knows nothing about HTTP/correlation
✅ **Reusable domain logic** - Same reducers work for HTTP, CLI, gRPC
✅ **Event-driven coordination** - Everything flows through EventBus
✅ **Composable** - Easy to add new projections, external operations

---

## Open Questions

1. **Correlation ID generation**: Should we use UUIDv7 for time-ordering?
2. **Request lifecycle persistence**: Should we persist to event store or keep in-memory?
3. **Cleanup strategy**: How long to keep completed RequestLifecycle records?
4. **Timeout values**: What's reasonable timeout for request completion? (30s? 60s?)
5. **Error handling**: If one projection fails, should we mark entire request as failed or partial success?
6. **WebSocket filtering**: Should server filter events by correlation_id or let client filter?

---

## Success Criteria

This feature is complete when:

1. ✅ All 4 projections emit ProjectionCompleted events
2. ✅ RequestLifecycleStore tracks requests from start to finish
3. ✅ WebSocket broadcasts all events (domain + projection + lifecycle)
4. ✅ HTTP handlers return correlation_id in 202 Accepted responses
5. ✅ All 7 integration tests pass using WebSocket-based waiting
6. ✅ No more `tokio::time::sleep()` in integration tests
7. ✅ Business domain aggregates have no knowledge of correlation IDs or HTTP concerns

---

## References

- **Saga Pattern**: [docs/saga-patterns.md](../../docs/saga-patterns.md)
- **Event Sourcing**: [docs/event-design-guidelines.md](../../docs/event-design-guidelines.md)
- **CQRS**: [docs/concepts.md](../../docs/concepts.md)
- **WebSocket Protocol**: [docs/websocket.md](../../docs/websocket.md)
- **Composable Architecture**: [specs/architecture.md](../../specs/architecture.md)
