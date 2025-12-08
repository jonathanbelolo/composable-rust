# SpacetimeDB Compilation Target

> **Status**: Research Complete, Specification Draft
>
> **Purpose**: Generate SpacetimeDB modules from Composable Rust definitions

---

## 1. Executive Summary

SpacetimeDB is a compelling compilation target for Composable Rust due to remarkable conceptual alignment:

- **Both use "reducers"** as the core abstraction for state mutations
- **Both compile to WASM** (browser vs server)
- **Both have built-in identity** management
- **Both support time-travel** via event/transaction history

The key insight: **pure business logic functions are fully testable with standard `cargo test`**, while thin SpacetimeDB reducers handle persistence. This mirrors our PostgreSQL monolith architecture.

---

## 2. What is SpacetimeDB?

SpacetimeDB combines a relational database with application server functionality. Clients connect directly to the database and execute application logic within it.

### Key Characteristics

| Feature | Description |
|---------|-------------|
| **Architecture** | Database + Application Server in one |
| **Languages** | Rust, C# (modules compile to WASM) |
| **Performance** | ~100μs transaction latency, ~1M TPS |
| **Real-time** | Native subscriptions via SQL queries |
| **Identity** | Built-in `ctx.sender` for authentication |
| **Time-travel** | Full transaction history, point-in-time rollback |

### Core Concepts

```rust
// Tables - decorated structs define schema
#[table(name = orders, public)]
pub struct Order {
    #[primary_key]
    id: u64,
    customer_id: u64,
    status: String,
}

// Reducers - functions that handle state mutations
#[reducer]
pub fn submit_order(ctx: &ReducerContext, order_id: u64) -> Result<(), String> {
    // Direct database access via ctx.db
    let order = ctx.db.orders().id().find(order_id).ok_or("Not found")?;
    ctx.db.orders().id().update(Order { status: "submitted".into(), ..order });
    Ok(())
}
```

### Resources

- [SpacetimeDB Documentation](https://spacetimedb.com/docs/)
- [Rust Module Quickstart](https://spacetimedb.com/docs/modules/rust/quickstart/)
- [Rust SDK Reference](https://spacetimedb.com/docs/sdks/rust)
- [docs.rs API Reference](https://docs.rs/spacetimedb/latest/spacetimedb/)
- [GitHub Repository](https://github.com/clockworklabs/SpacetimeDB)

---

## 3. Architectural Alignment

### Concept Mapping

| Composable Rust | SpacetimeDB | Notes |
|-----------------|-------------|-------|
| `State` struct | `#[table]` struct | Aggregate state as table rows |
| `Action` enum | Reducer parameters | Each command variant → reducer |
| `Reducer::reduce()` | `#[reducer]` fn body | State transition logic |
| `Effect::Database` | `ctx.db.*` operations | Direct CRUD |
| `Effect::PublishEvent` | Insert to events table | Event sourcing layer |
| `Environment::clock()` | `ctx.timestamp` | Deterministic time |
| `Identity` (user/tenant) | `ctx.sender` | Built-in identity |
| Event replay | Transaction history | Native time-travel |

### Key Difference: Imperative vs Functional

**Composable Rust** (functional):
```rust
fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
    state.status = OrderStatus::Submitted;
    vec![Effect::Database(SaveState), Effect::PublishEvent(event)]
}
```

**SpacetimeDB** (imperative):
```rust
#[reducer]
pub fn submit_order(ctx: &ReducerContext, order_id: u64) -> Result<(), String> {
    ctx.db.orders().id().update(Order { status: "submitted", .. });
    ctx.db.domain_events().insert(event);
    Ok(())
}
```

---

## 4. The Pure Function Architecture

### Why Pure Functions Are Essential

SpacetimeDB has a **testing limitation**: `cargo test` fails on modules because `ReducerContext` relies on external symbols provided by the SpacetimeDB runtime.

```
fatal error LNK1120: 14 unresolved externals
- datastore_insert_bsatn
- console_log
- table_id_from_name
...
```

See: [GitHub Issue #2788](https://github.com/clockworklabs/SpacetimeDB/issues/2788)

**Solution**: Separate pure business logic from the imperative shell.

### The Pattern

```rust
// ═══════════════════════════════════════════════════════════════════════════
// PURE BUSINESS LOGIC - No ctx, no side effects, fully testable with cargo test
// ═══════════════════════════════════════════════════════════════════════════

/// Domain error types
#[derive(Debug, Clone, PartialEq)]
pub enum OrderError {
    NotFound,
    InvalidStateTransition { from: OrderStatus, to: OrderStatus },
    InsufficientInventory,
}

/// Pure reducer - takes state + action, returns new state + events
pub fn order_reduce(
    state: OrderState,
    action: OrderAction,
) -> Result<(OrderState, Vec<OrderEvent>), OrderError> {
    match action {
        OrderAction::Submit { customer_id } => {
            // Validation
            if state.status != OrderStatus::Draft {
                return Err(OrderError::InvalidStateTransition {
                    from: state.status,
                    to: OrderStatus::Submitted,
                });
            }

            // State transition
            let new_state = OrderState {
                status: OrderStatus::Submitted,
                customer_id: Some(customer_id),
                submitted_at: Some(state.current_time),
                ..state
            };

            // Event generation
            let events = vec![OrderEvent::Submitted {
                order_id: state.id,
                customer_id,
                timestamp: state.current_time,
            }];

            Ok((new_state, events))
        }

        OrderAction::Cancel { reason } => {
            if !state.status.is_cancellable() {
                return Err(OrderError::InvalidStateTransition {
                    from: state.status,
                    to: OrderStatus::Cancelled,
                });
            }

            let new_state = OrderState {
                status: OrderStatus::Cancelled,
                cancelled_at: Some(state.current_time),
                cancellation_reason: Some(reason.clone()),
                ..state
            };

            let events = vec![OrderEvent::Cancelled {
                order_id: state.id,
                reason,
                timestamp: state.current_time,
            }];

            Ok((new_state, events))
        }

        // ... other actions
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// THIN IMPERATIVE SHELL - SpacetimeDB reducer (~5 lines of boilerplate)
// ═══════════════════════════════════════════════════════════════════════════

#[reducer]
pub fn submit_order(
    ctx: &ReducerContext,
    order_id: u64,
    customer_id: u64,
) -> Result<(), String> {
    // 1. Load current state from table
    let row = ctx.db.orders().id().find(order_id)
        .ok_or("Order not found")?;

    // 2. Inject current time and convert to domain state
    let state = OrderState::from_row(row, ctx.timestamp);

    // 3. Call pure function (ALL business logic lives here)
    let (new_state, events) = order_reduce(state, OrderAction::Submit { customer_id })
        .map_err(|e| format!("{:?}", e))?;

    // 4. Persist new state
    ctx.db.orders().id().update(new_state.into_row());

    // 5. Append domain events
    for event in events {
        ctx.db.domain_events().insert(event.into_row(order_id));
    }

    Ok(())
}
```

### Benefits

1. **Testability**: Pure functions work with standard `cargo test`
2. **Portability**: Same logic can target PostgreSQL, WASM browser, SpacetimeDB
3. **Debuggability**: No runtime dependencies to mock
4. **Trust**: Thin shell is ~5 lines, trivially correct if pure fn is correct

---

## 5. Testing Strategy

### Testing Matrix

| Layer | `cargo test`? | Strategy |
|-------|---------------|----------|
| Pure reducers | ✅ Yes | Unit tests, property-based tests |
| State transitions | ✅ Yes | Exhaustive state machine tests |
| Event generation | ✅ Yes | Snapshot tests |
| Validation rules | ✅ Yes | Parameterized tests |
| Thin shell | ❌ No | Trust (trivial boilerplate) |
| End-to-end | ❌ No | Integration tests via client SDK |

### Unit Test Example

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submit_draft_order_succeeds() {
        let state = OrderState {
            id: 1,
            status: OrderStatus::Draft,
            current_time: 1000,
            ..Default::default()
        };

        let result = order_reduce(state, OrderAction::Submit { customer_id: 42 });

        let (new_state, events) = result.expect("should succeed");
        assert_eq!(new_state.status, OrderStatus::Submitted);
        assert_eq!(new_state.customer_id, Some(42));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], OrderEvent::Submitted { customer_id: 42, .. }));
    }

    #[test]
    fn test_submit_already_submitted_order_fails() {
        let state = OrderState {
            id: 1,
            status: OrderStatus::Submitted,
            ..Default::default()
        };

        let result = order_reduce(state, OrderAction::Submit { customer_id: 42 });

        assert!(matches!(
            result,
            Err(OrderError::InvalidStateTransition {
                from: OrderStatus::Submitted,
                to: OrderStatus::Submitted
            })
        ));
    }

    #[test]
    fn test_cancel_submitted_order_succeeds() {
        let state = OrderState {
            id: 1,
            status: OrderStatus::Submitted,
            current_time: 2000,
            ..Default::default()
        };

        let result = order_reduce(state, OrderAction::Cancel {
            reason: "Customer request".into()
        });

        let (new_state, events) = result.expect("should succeed");
        assert_eq!(new_state.status, OrderStatus::Cancelled);
        assert_eq!(new_state.cancellation_reason, Some("Customer request".into()));
    }
}
```

### Integration Test Example (Client SDK)

```rust
// tests/integration/order_flow.rs
// Requires: spacetime start (local instance running)

use spacetimedb_sdk::{DbConnection, credentials};

#[tokio::test]
async fn test_full_order_flow() {
    let conn = DbConnection::builder()
        .with_module_name("my-orders-db")
        .with_uri("http://localhost:3000")
        .build()
        .await
        .expect("connection failed");

    // Create order
    conn.reducers.create_order(1, "draft").await.unwrap();

    // Submit order
    conn.reducers.submit_order(1, 42).await.unwrap();

    // Verify via SQL
    let orders: Vec<Order> = conn.sql("SELECT * FROM orders WHERE id = 1").await.unwrap();
    assert_eq!(orders[0].status, "submitted");

    // Verify events
    let events: Vec<DomainEvent> = conn
        .sql("SELECT * FROM domain_events WHERE stream_id = 'order:1' ORDER BY sequence")
        .await
        .unwrap();
    assert_eq!(events.len(), 2); // Created + Submitted
}
```

---

## 6. Code Generation Strategy

### Input: Composable Rust Definition

```rust
// Domain definition (source of truth)
#[derive(State)]
pub struct Order {
    pub id: u64,
    pub customer_id: Option<u64>,
    pub status: OrderStatus,
    pub items: Vec<LineItem>,
    pub total: Decimal,
}

#[derive(Action)]
pub enum OrderAction {
    Create { id: u64 },
    AddItem { sku: String, quantity: u32 },
    Submit { customer_id: u64 },
    Cancel { reason: String },
}

#[derive(Event)]
pub enum OrderEvent {
    Created { id: u64, timestamp: u64 },
    ItemAdded { sku: String, quantity: u32 },
    Submitted { customer_id: u64, timestamp: u64 },
    Cancelled { reason: String, timestamp: u64 },
}

impl Reducer for OrderReducer {
    type State = Order;
    type Action = OrderAction;
    // ... reduce implementation
}
```

### Output: SpacetimeDB Module

```rust
// ═══════════════════════════════════════════════════════════════════════════
// GENERATED: SpacetimeDB Tables
// ═══════════════════════════════════════════════════════════════════════════

use spacetimedb::{table, reducer, ReducerContext, Timestamp, SpacetimeType};

#[table(name = orders, public)]
pub struct OrderRow {
    #[primary_key]
    pub id: u64,
    pub customer_id: Option<u64>,
    pub status: String,
    pub items_json: String,  // JSON-serialized Vec<LineItem>
    pub total: String,       // Decimal as string
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[table(name = domain_events, public)]
pub struct DomainEventRow {
    #[auto_inc]
    #[primary_key]
    pub sequence: u64,
    pub stream_id: String,
    pub event_type: String,
    pub payload: String,  // JSON
    pub timestamp: Timestamp,
}

// ═══════════════════════════════════════════════════════════════════════════
// GENERATED: Pure Business Logic (from Composable Rust Reducer)
// ═══════════════════════════════════════════════════════════════════════════

// ... (copied/generated from source reducer)
pub fn order_reduce(state: OrderState, action: OrderAction) -> Result<...> { ... }

// ═══════════════════════════════════════════════════════════════════════════
// GENERATED: SpacetimeDB Reducers (thin shells)
// ═══════════════════════════════════════════════════════════════════════════

#[reducer]
pub fn create_order(ctx: &ReducerContext, id: u64) -> Result<(), String> {
    let state = OrderState::default_with_time(ctx.timestamp.to_micros_since_epoch());
    let (new_state, events) = order_reduce(state, OrderAction::Create { id })
        .map_err(|e| format!("{:?}", e))?;

    ctx.db.orders().insert(new_state.into_row(ctx.timestamp));
    for event in events {
        ctx.db.domain_events().insert(event.into_row(format!("order:{}", id)));
    }
    Ok(())
}

#[reducer]
pub fn add_item(ctx: &ReducerContext, order_id: u64, sku: String, quantity: u32) -> Result<(), String> {
    let row = ctx.db.orders().id().find(order_id).ok_or("Order not found")?;
    let state = OrderState::from_row(row, ctx.timestamp.to_micros_since_epoch());

    let (new_state, events) = order_reduce(state, OrderAction::AddItem { sku, quantity })
        .map_err(|e| format!("{:?}", e))?;

    ctx.db.orders().id().update(new_state.into_row(ctx.timestamp));
    for event in events {
        ctx.db.domain_events().insert(event.into_row(format!("order:{}", order_id)));
    }
    Ok(())
}

#[reducer]
pub fn submit_order(ctx: &ReducerContext, order_id: u64, customer_id: u64) -> Result<(), String> {
    let row = ctx.db.orders().id().find(order_id).ok_or("Order not found")?;
    let state = OrderState::from_row(row, ctx.timestamp.to_micros_since_epoch());

    let (new_state, events) = order_reduce(state, OrderAction::Submit { customer_id })
        .map_err(|e| format!("{:?}", e))?;

    ctx.db.orders().id().update(new_state.into_row(ctx.timestamp));
    for event in events {
        ctx.db.domain_events().insert(event.into_row(format!("order:{}", order_id)));
    }
    Ok(())
}

#[reducer]
pub fn cancel_order(ctx: &ReducerContext, order_id: u64, reason: String) -> Result<(), String> {
    let row = ctx.db.orders().id().find(order_id).ok_or("Order not found")?;
    let state = OrderState::from_row(row, ctx.timestamp.to_micros_since_epoch());

    let (new_state, events) = order_reduce(state, OrderAction::Cancel { reason })
        .map_err(|e| format!("{:?}", e))?;

    ctx.db.orders().id().update(new_state.into_row(ctx.timestamp));
    for event in events {
        ctx.db.domain_events().insert(event.into_row(format!("order:{}", order_id)));
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// GENERATED: Lifecycle Reducers
// ═══════════════════════════════════════════════════════════════════════════

#[reducer(init)]
pub fn init(_ctx: &ReducerContext) {
    // Module initialization
}

#[reducer(client_connected)]
pub fn client_connected(ctx: &ReducerContext) {
    // Track connected clients if needed
    log::info!("Client connected: {:?}", ctx.sender);
}

#[reducer(client_disconnected)]
pub fn client_disconnected(ctx: &ReducerContext) {
    log::info!("Client disconnected: {:?}", ctx.sender);
}
```

---

## 7. Event Sourcing Layer

SpacetimeDB is state-based by default. We implement event sourcing on top:

### Events Table

```rust
#[table(name = domain_events, public)]
pub struct DomainEventRow {
    #[auto_inc]
    #[primary_key]
    pub sequence: u64,           // Global sequence number

    pub stream_id: String,       // e.g., "order:123", "payment:456"
    pub stream_version: u64,     // Version within stream (for optimistic locking)
    pub event_type: String,      // e.g., "OrderSubmitted"
    pub payload: String,         // JSON-serialized event data
    pub timestamp: Timestamp,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[index(btree)]
pub stream_id: String;  // For loading aggregate events
```

### State Reconstruction (Optional)

```rust
/// Reconstruct aggregate state from events (for debugging/auditing)
pub fn reconstruct_order(events: Vec<DomainEventRow>) -> OrderState {
    events.iter().fold(OrderState::default(), |state, event| {
        let domain_event: OrderEvent = serde_json::from_str(&event.payload).unwrap();
        apply_event(state, domain_event)
    })
}
```

### Real-Time Subscriptions

Clients subscribe to events via SQL:

```typescript
// TypeScript client
connection.subscribe([
    "SELECT * FROM domain_events WHERE stream_id LIKE 'order:%' ORDER BY sequence DESC LIMIT 100"
]);

connection.on('domain_events', (event) => {
    console.log('New event:', event.event_type, event.payload);
});
```

---

## 8. Multi-Aggregate Coordination (Sagas)

### Option 1: Scheduled Reducers

```rust
#[reducer]
pub fn order_submitted_handler(ctx: &ReducerContext, order_id: u64) -> Result<(), String> {
    // React to OrderSubmitted event
    let order = ctx.db.orders().id().find(order_id).ok_or("Order not found")?;

    // Trigger inventory reservation
    reserve_inventory(ctx, order_id, order.items_json)?;

    Ok(())
}

// Schedule via client or from another reducer
ctx.db.scheduled_reducers().insert(ScheduledCall {
    reducer: "order_submitted_handler",
    args: order_id.to_string(),
    run_at: ctx.timestamp,
});
```

### Option 2: Event-Driven via Subscriptions

Client-side saga coordinator subscribes to events and dispatches commands:

```typescript
// Saga coordinator (runs as client)
connection.on('domain_events', async (event) => {
    if (event.event_type === 'OrderSubmitted') {
        const payload = JSON.parse(event.payload);
        await connection.reducers.reserve_inventory(payload.order_id, payload.items);
    }

    if (event.event_type === 'InventoryReserved') {
        const payload = JSON.parse(event.payload);
        await connection.reducers.process_payment(payload.order_id, payload.total);
    }

    // Compensation
    if (event.event_type === 'PaymentFailed') {
        const payload = JSON.parse(event.payload);
        await connection.reducers.release_inventory(payload.order_id);
    }
});
```

---

## 9. Comparison: Compilation Targets

| Feature | WASM Browser | SpacetimeDB | PostgreSQL Monolith |
|---------|--------------|-------------|---------------------|
| **Deployment** | Client-side | Distributed server | Self-hosted DB |
| **Persistence** | IndexedDB | Built-in relational | PostgreSQL |
| **Real-time** | Manual WebSocket | Native subscriptions | pg_notify + WebSocket |
| **Multi-tenant** | Manual | Built-in Identity | RLS policies |
| **Scaling** | N/A (local) | Automatic | Manual (replicas) |
| **Latency** | Instant | ~100μs | ~1-10ms |
| **Testing** | Pure fns + browser | Pure fns + integration | Pure fns + pgTAP |
| **Use case** | Prototyping, offline | Multiplayer, real-time | Enterprise, existing infra |

---

## 10. Limitations and Considerations

### Current Limitations

1. **No `cargo test` for modules**: Must use pure function separation
2. **No `spacetime test` command**: Integration tests require running instance
3. **Limited schema migrations**: Some changes require `--delete-data`
4. **Young ecosystem**: Less mature than PostgreSQL

### When to Use SpacetimeDB Target

**Good fit:**
- Real-time multiplayer applications
- Collaborative tools
- Games with shared state
- Rapid prototyping with built-in scaling

**Not ideal for:**
- Complex reporting/analytics (use PostgreSQL)
- Existing PostgreSQL infrastructure
- Regulatory requirements (data residency)
- Complex multi-table transactions

### Migration Path

```
Local Development     →    SpacetimeDB Cloud    →    Self-hosted
(spacetime start)          (Managed)                 (Your infra)
```

---

## 11. Next Steps

1. **Create proof-of-concept**: Generate SpacetimeDB module from simple Composable Rust aggregate
2. **Validate testing strategy**: Confirm pure functions work with cargo test
3. **Build code generator**: Automate State → Table, Action → Reducer generation
4. **Integration test harness**: Client SDK-based test runner
5. **Documentation**: Developer guide for SpacetimeDB target

---

## 12. References

- [SpacetimeDB Documentation](https://spacetimedb.com/docs/)
- [Rust Module Quickstart](https://spacetimedb.com/docs/modules/rust/quickstart/)
- [Rust Client SDK](https://spacetimedb.com/docs/sdks/rust)
- [CLI Reference](https://spacetimedb.com/docs/cli-reference/)
- [docs.rs API](https://docs.rs/spacetimedb/latest/spacetimedb/)
- [GitHub - SpacetimeDB](https://github.com/clockworklabs/SpacetimeDB)
- [Testing Issue #2788](https://github.com/clockworklabs/SpacetimeDB/issues/2788)
