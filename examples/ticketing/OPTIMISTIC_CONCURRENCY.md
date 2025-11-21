# Optimistic Concurrency Control

This document explains the optimistic concurrency control implementation in the Ticketing system, which prevents lost updates when multiple processes attempt to modify the same aggregate concurrently.

## Overview

Optimistic concurrency control (OCC) is a concurrency control method that assumes multiple transactions can complete without affecting each other. The system checks for conflicts only when committing changes, rather than locking resources upfront.

### Why Optimistic Concurrency?

In event-sourced systems, preventing lost updates is critical. Consider this scenario:

1. **Process A** loads event stream at version 5
2. **Process B** loads the same stream at version 5
3. **Process A** appends an event, incrementing to version 6
4. **Process B** appends an event, expecting version 5 but finding version 6

Without OCC, Process B's append would succeed, potentially overwriting Process A's changes. With OCC, Process B's append fails with a `ConcurrencyConflict` error, forcing it to reload the stream and retry.

## Architecture

### 1. Version Type

The framework provides a `Version` type in `composable_rust_core::stream`:

```rust
pub struct Version(u64);

impl Version {
    pub const INITIAL: Self = Self(0);  // Version 0 for new streams
    pub const fn next(self) -> Self;     // Increment by 1
    pub const fn value(self) -> u64;     // Get underlying value
}
```

### 2. Aggregate State

Each aggregate maintains its current version:

```rust
pub struct EventState {
    pub events: HashMap<EventId, Event>,
    pub version: Version,  // Current stream version
}
```

All four aggregates (Event, Inventory, Reservation, Payment) track their version:

- `examples/ticketing/src/types.rs:956-965` (EventState)
- `examples/ticketing/src/types.rs:988-997` (InventoryState)
- `examples/ticketing/src/types.rs:1022-1032` (ReservationState)
- `examples/ticketing/src/types.rs:1050-1063` (PaymentState)

### 3. Version Tracking in Reducers

Each aggregate has a `VersionUpdated` event to track successful appends:

```rust
pub enum EventAction {
    // ... other events

    /// Stream version was updated after successful event append
    #[event]
    VersionUpdated {
        /// New version number
        version: Version,
    },
}
```

The `apply_event` function updates state when this event is received:

```rust
fn apply_event(state: &mut EventState, event: &EventAction) {
    match event {
        // ... other events

        EventAction::VersionUpdated { version } => {
            state.version = *version;
        }
    }
}
```

### 4. Expected Version in Effect Creation

When creating effects to persist events, the reducer captures the current version **before** applying the event:

```rust
// Create and apply event
let event = EventAction::EventCreated { /* ... */ };
let expected_version = state.version;  // Capture BEFORE applying
Self::apply_event(state, &event);

Self::create_effects(event, expected_version, env)
```

The `create_effects` function passes this expected version to the event store:

```rust
fn create_effects(
    event: EventAction,
    expected_version: Version,
    env: &EventEnvironment,
) -> SmallVec<[Effect<EventAction>; 4]> {
    // ... serialization logic

    smallvec![
        append_events! {
            store: env.event_store,
            stream: env.stream_id.as_str(),
            expected_version: Some(expected_version),  // Pass version
            events: vec![serialized.clone()],
            on_success: |version| Some(EventAction::VersionUpdated { version }),
            on_error: |error| Some(EventAction::ValidationFailed {
                error: error.to_string()
            })
        },
        // ... other effects
    ]
}
```

### 5. PostgreSQL Event Store

The PostgreSQL event store (`postgres/src/lib.rs:476-491`) validates the expected version:

```rust
// Check optimistic concurrency
if let Some(expected) = expected_version {
    if current_version != expected {
        tracing::warn!(
            stream_id = %stream_id,
            expected = ?expected,
            actual = ?current_version,
            "Optimistic concurrency conflict detected"
        );
        return Err(EventStoreError::ConcurrencyConflict {
            stream_id,
            expected,
            actual: current_version,
        });
    }
}
```

If the check passes, the event is appended with a `PRIMARY KEY` constraint on `(stream_id, sequence_number)` to prevent race conditions even if two transactions pass the check simultaneously (`postgres/src/lib.rs:516-548`).

## Implementation Pattern

### Aggregate Structure

Each aggregate follows this pattern:

```rust
// 1. State includes version field
pub struct AggregateState {
    // ... domain fields
    pub version: Version,
}

impl AggregateState {
    pub fn new() -> Self {
        Self {
            // ... initialize fields
            version: Version::INITIAL,  // Start at version 0
        }
    }
}

// 2. Action enum includes VersionUpdated
pub enum AggregateAction {
    // ... command and event variants

    #[event]
    VersionUpdated { version: Version },
}

// 3. apply_event updates version
impl AggregateAction {
    fn apply_event(state: &mut AggregateState, event: &AggregateAction) {
        match event {
            // ... other events

            AggregateAction::VersionUpdated { version } => {
                state.version = *version;
            }
        }
    }
}

// 4. create_effects accepts expected_version parameter
impl AggregateReducer {
    fn create_effects(
        event: AggregateAction,
        expected_version: Version,
        env: &AggregateEnvironment,
    ) -> SmallVec<[Effect<AggregateAction>; 4]> {
        // ... serialize event

        smallvec![
            append_events! {
                store: env.event_store,
                stream: env.stream_id.as_str(),
                expected_version: Some(expected_version),
                events: vec![serialized],
                on_success: |version| Some(AggregateAction::VersionUpdated { version }),
                on_error: |error| Some(AggregateAction::ValidationFailed {
                    error: error.to_string()
                })
            },
        ]
    }
}

// 5. reduce captures version before applying event
impl Reducer for AggregateReducer {
    fn reduce(
        &self,
        state: &mut AggregateState,
        action: AggregateAction,
        env: &AggregateEnvironment,
    ) -> SmallVec<[Effect<AggregateAction>; 4]> {
        match action {
            AggregateAction::SomeCommand { /* ... */ } => {
                // Validate command
                // ...

                // Create event
                let event = AggregateAction::SomeEvent { /* ... */ };

                // Capture version BEFORE applying
                let expected_version = state.version;

                // Apply event to state
                Self::apply_event(state, &event);

                // Create effects with expected version
                Self::create_effects(event, expected_version, env)
            }

            // ... other commands
        }
    }
}
```

## Conflict Resolution Strategy

When a `ConcurrencyConflict` error occurs:

1. **In-Memory Store**: The store's retry logic can automatically retry with the updated version
2. **HTTP API**: The client receives a `409 Conflict` response and should retry the request
3. **Event Bus Handlers**: The handler should reload the aggregate state and reprocess the event

### Retry Strategy

The framework provides retry policies in `runtime/src/lib.rs`. For concurrency conflicts, use exponential backoff:

```rust
use composable_rust_runtime::RetryPolicy;

let retry_policy = RetryPolicy::exponential(
    3,                     // max_retries
    Duration::from_millis(100),  // initial delay
    2.0,                   // backoff multiplier
);
```

## Testing

### Unit Tests

Each aggregate has unit tests verifying version tracking:

- Event aggregate: `examples/ticketing/src/aggregates/event.rs:523-753`
- Inventory aggregate: `examples/ticketing/src/aggregates/inventory.rs:723-1120`
- Reservation aggregate: `examples/ticketing/src/aggregates/reservation.rs:526-1013`
- Payment aggregate: `examples/ticketing/src/aggregates/payment.rs:397-590`

### Integration Tests

Concurrency integration tests verify the OCC behavior:

```bash
cargo test -p ticketing --test concurrency_integration_test
```

Test results: **8 passed; 0 failed**

## Performance Considerations

### When to Use Optimistic Concurrency

✅ **Use OCC when:**
- Conflicts are rare (low contention)
- Read-heavy workloads dominate
- You want to avoid locking overhead
- The cost of retry is acceptable

❌ **Consider pessimistic locking when:**
- Conflicts are frequent (high contention)
- Retries would cause cascading failures
- You need guaranteed first-attempt success

### Optimizations

1. **Version Caching**: The version is stored in aggregate state, avoiding repeated database queries
2. **Batch Operations**: Multiple events can share the same version check
3. **Database Constraints**: PostgreSQL `PRIMARY KEY` ensures atomicity even under race conditions

## Observability

The framework emits structured logs and metrics for concurrency conflicts:

### Tracing

```rust
tracing::warn!(
    stream_id = %stream_id,
    expected = ?expected,
    actual = ?current_version,
    "Optimistic concurrency conflict detected"
);
```

### Metrics

```rust
metrics::counter!("event_store.concurrency_conflicts",
    "stream_id" => stream_id.to_string()
).increment(1);
```

Monitor these metrics to detect:
- High conflict rates indicating contention hotspots
- Specific streams experiencing repeated conflicts
- Patterns suggesting architectural issues

## References

- **Core Framework**: `core/src/stream.rs:163-220` (Version type)
- **Event Store**: `postgres/src/lib.rs:476-548` (OCC implementation)
- **Aggregate Examples**:
  - Event: `examples/ticketing/src/aggregates/event.rs:290-310`
  - Inventory: `examples/ticketing/src/aggregates/inventory.rs:387-406`
  - Reservation: `examples/ticketing/src/aggregates/reservation.rs:347-370`
  - Payment: `examples/ticketing/src/aggregates/payment.rs:282-301`
- **Error Handling**: `core/src/event_store.rs:92-106` (ConcurrencyConflict error)

## Summary

Optimistic concurrency control in the Ticketing system:

1. **Tracks versions** at the aggregate level using `Version` type
2. **Validates expected version** before appending events to the event store
3. **Returns conflicts** as errors that trigger retries or user feedback
4. **Uses PostgreSQL constraints** as a safety net against race conditions
5. **Updates state version** via `VersionUpdated` events after successful appends

This approach prevents lost updates while maintaining the benefits of event sourcing and CQRS architecture.
