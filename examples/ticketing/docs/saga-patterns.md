# Saga Patterns Guide

This document covers patterns specific to implementing sagas in the Composable Rust framework. For general aggregate patterns (clock injection, version capture, event-first persistence), see [aggregate-patterns.md](./aggregate-patterns.md).

## Table of Contents

1. [What is a Saga?](#1-what-is-a-saga)
2. [The Feedback Loop Pattern](#2-the-feedback-loop-pattern)
3. [Saga State Machine](#3-saga-state-machine)
4. [Child Aggregate Orchestration](#4-child-aggregate-orchestration)
5. [Parallel Effect Coordination](#5-parallel-effect-coordination)
6. [Recovery and Replay](#6-recovery-and-replay)
7. [Error Handling](#7-error-handling)
8. [Code Quality Patterns](#8-code-quality-patterns)
9. [Testing Sagas](#9-testing-sagas)
10. [Checklist](#10-checklist)

---

## 1. What is a Saga?

A saga coordinates a multi-step workflow across multiple aggregates. In Composable Rust, **sagas are just reducers** - they use the same `Reducer` trait, `Effect` system, and event sourcing patterns as regular aggregates.

### When to Use a Saga

Use a saga when:
- Multiple aggregates must be updated in a coordinated way
- The workflow has multiple steps that depend on each other
- You need to track progress across asynchronous operations
- Failure in one step may require compensation in others

### Example: Event-Inventory Saga

The `event_inventory_saga.rs` coordinates:
1. Creating an Event (via Event aggregate)
2. Initializing Inventory for each section (via Inventory aggregate)

Without the saga, these would be separate, uncoordinated operations.

### Note on Compensation

This guide uses `event_inventory_saga.rs` as its primary example. This saga doesn't require compensation because inventory initialization is idempotent and failure simply marks the saga as failed.

For sagas that need compensation (e.g., reversing a payment after shipping fails), see `.claude/skills/composable-rust-sagas/SKILL.md` which covers compensation patterns in detail.

---

## 2. The Feedback Loop Pattern

The key pattern that makes sagas work in Composable Rust is the **feedback loop**:

```text
1. Command arrives
   → Reducer processes, returns Effects
   → Effects execute asynchronously
   → Effects return NEW Actions
   → Actions feed back into the reducer
   → Cycle continues until saga completes
```

### Visual Flow

```text
┌─────────────────────────────────────────────────────────────────────┐
│                         SAGA REDUCER                                │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  CreateEventWithInventory ──┬──► EventCreationInitiated (persist)  │
│         (command)           │                                       │
│                             └──► Effect: call Event aggregate       │
│                                        │                            │
│                                        ▼                            │
│  EventCreated ◄────────────────── returns EventCreated action       │
│    (feedback)                                                       │
│         │                                                           │
│         ├──► EventCreated (persist)                                 │
│         │                                                           │
│         └──► Effects: call Inventory aggregate (N times, parallel)  │
│                        │                                            │
│                        ▼                                            │
│  SectionInventoryInitialized ◄─── returns per section               │
│         (feedback)                                                  │
│         │                                                           │
│         ├──► SectionInventoryInitialized (persist)                  │
│         │                                                           │
│         └──► When all done: return EventCreationCompleted           │
│                        │                                            │
│                        ▼                                            │
│  EventCreationCompleted ◄──────── final feedback                    │
│         │                                                           │
│         └──► EventCreationCompleted (persist) ──► SAGA COMPLETE     │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### Code Example

```rust
// Step 1: Command handler returns effect that will feed back
EventInventorySagaAction::CreateEventWithInventory { event_id, name, ... } => {
    // ... validation and state updates ...

    effects.push(Effect::Future(Box::pin(async move {
        let event_store = create_event_store(event_id);

        match event_store.send(create_action).await {
            Ok(_) => {
                // This action feeds BACK into the reducer!
                Some(EventInventorySagaAction::EventCreated {
                    event_id,
                    sections,
                    created_at: now,
                })
            }
            Err(e) => {
                Some(EventInventorySagaAction::EventCreationFailed {
                    event_id,
                    error: e.to_string(),
                    failed_at: now,
                })
            }
        }
    })));

    effects
}

// Step 2: Feedback action handler - continues the workflow
EventInventorySagaAction::EventCreated { event_id, sections, ... } => {
    // Process the feedback, start next step...
}
```

### Benefits

1. **Explicit state machine** - Every transition is visible in the reducer
2. **Testable** - Each step can be unit tested independently
3. **Traceable** - Event history shows exactly what happened
4. **Recoverable** - Replay events to restore saga state after crash

---

## 3. Saga State Machine

Saga state tracks workflow progress with clear status flags:

```rust
pub struct EventInventorySagaState {
    /// The event being created (None if not yet initiated)
    pub event_id: Option<EventId>,

    /// Sections that still need inventory initialized
    pub pending_sections: HashSet<String>,

    /// Section capacities from the venue
    pub section_capacities: HashMap<String, Capacity>,

    /// Progress flags
    pub event_created: bool,
    pub inventory_complete: bool,
    pub completed: bool,
    pub failed: bool,

    /// Error tracking
    pub last_error: Option<String>,

    /// Event sourcing version
    pub version: Version,
}
```

### State Transitions

```text
Initial State
    │
    ▼ CreateEventWithInventory
┌───────────────────┐
│ event_id: Some    │
│ event_created: ✗  │
│ completed: ✗      │
└───────────────────┘
    │
    ▼ EventCreated
┌───────────────────┐
│ event_created: ✓  │
│ pending_sections: │
│   [VIP, General]  │
└───────────────────┘
    │
    ▼ SectionInventoryInitialized (VIP)
┌───────────────────┐
│ pending_sections: │
│   [General]       │
└───────────────────┘
    │
    ▼ SectionInventoryInitialized (General)
┌───────────────────┐
│ pending_sections: │
│   []              │
│ inventory_complete│
│   : ✓             │
└───────────────────┘
    │
    ▼ EventCreationCompleted
┌───────────────────┐
│ completed: ✓      │
└───────────────────┘
```

### Status Helper Methods

```rust
impl EventInventorySagaState {
    /// Check if saga is complete
    pub const fn is_complete(&self) -> bool {
        self.completed
    }

    /// Check if event creation is in progress
    pub const fn is_in_progress(&self) -> bool {
        self.event_id.is_some() && !self.completed && !self.failed
    }
}
```

---

## 4. Child Aggregate Orchestration

Sagas coordinate child aggregates using **factory functions** passed via the environment:

### Environment Setup

```rust
pub struct EventInventorySagaEnvironment {
    pub clock: Arc<dyn Clock>,
    pub event_store: Arc<dyn EventStore>,
    pub stream_id: StreamId,

    /// Factory function to create Event aggregate stores
    pub create_event_store: Arc<
        dyn Fn(EventId) -> Store<EventState, EventAction, EventEnvironment, EventReducer>
            + Send + Sync,
    >,

    /// Factory function to create Inventory aggregate stores
    pub create_inventory_store: Arc<
        dyn Fn(EventId) -> Store<InventoryState, InventoryAction, InventoryEnvironment, InventoryReducer>
            + Send + Sync,
    >,
}
```

### Why Factory Functions?

1. **Dependency injection** - Different stores for production vs test
2. **Per-aggregate streams** - Each aggregate has its own event stream
3. **Isolation** - Saga doesn't know implementation details of children

### Usage in Effects

```rust
let create_event_store = env.create_event_store.clone();

effects.push(Effect::Future(Box::pin(async move {
    // Create a fresh store for this aggregate
    let event_store = create_event_store(event_id);

    // Send command to child aggregate
    match event_store.send(create_action).await {
        Ok(_) => Some(EventInventorySagaAction::EventCreated { ... }),
        Err(e) => Some(EventInventorySagaAction::EventCreationFailed { ... }),
    }
})));
```

---

## 5. Parallel Effect Coordination

Sagas can coordinate multiple parallel operations and track their completion:

### Launching Parallel Effects

```rust
EventInventorySagaAction::EventCreated { event_id, sections, ... } => {
    let mut effects = smallvec![Self::create_persist_effect(event, expected_version, env)];

    // Create one effect per section (all run in parallel)
    for section_name in &sections {
        let section = section_name.clone();
        let capacity = state.section_capacities.get(&section).copied()
            .unwrap_or(Capacity::new(0));
        let store_factory = create_inventory_store.clone();

        effects.push(Effect::Future(Box::pin(async move {
            let inventory_store = store_factory(event_id);

            match inventory_store.send(init_action).await {
                Ok(_) => Some(EventInventorySagaAction::SectionInventoryInitialized {
                    event_id,
                    section,
                    initialized_at: ts,
                }),
                Err(e) => Some(EventInventorySagaAction::InventoryInitializationFailed {
                    event_id,
                    section,
                    error: e.to_string(),
                    failed_at: ts,
                }),
            }
        })));
    }

    effects
}
```

### Tracking Completion

Use a `HashSet` to track pending work:

```rust
EventInventorySagaAction::SectionInventoryInitialized { section, ... } => {
    // Remove from pending
    state.pending_sections.remove(&section);

    // Check if ALL sections are done
    if state.pending_sections.is_empty() && state.event_created && !state.failed {
        // Trigger completion
        effects.push(Effect::Future(Box::pin(async move {
            Some(EventInventorySagaAction::EventCreationCompleted { ... })
        })));
    }

    effects
}
```

---

## 6. Recovery and Replay

Sagas must handle event replay for crash recovery. Replay occurs when:
- The application restarts and reconstructs state from persisted events
- A saga resumes after a crash mid-workflow
- State is loaded from the event store for any reason

When replaying, events should update state but NOT create new effects (effects were already executed when the event was originally processed).

### Pattern: Separate Replay Handler

```rust
// Replay handler - events that were already persisted
ref action @ EventInventorySagaAction::EventCreationInitiated { ref event_id, .. } => {
    tracing::debug!(
        event_id = %event_id.as_uuid(),
        "Saga: EventCreationInitiated replayed (recovery)"
    );

    // Apply to state (idempotent)
    Self::apply_event(state, action);

    // NO EFFECTS - event was already persisted
    SmallVec::new()
}
```

### Use `ref @` Pattern

For replay handlers that only apply state, use `ref @` to avoid reconstructing the action:

```rust
// Good: borrows the action, passes reference to apply_event
ref action @ EventInventorySagaAction::EventCreationInitiated { ref event_id, .. } => {
    Self::apply_event(state, action);
    SmallVec::new()
}

// Avoid: reconstructs the action unnecessarily
EventInventorySagaAction::EventCreationInitiated { event_id, name, ... } => {
    Self::apply_event(state, &EventInventorySagaAction::EventCreationInitiated {
        event_id, name, ...  // Reconstructing what we just destructured
    });
    SmallVec::new()
}
```

---

## 7. Error Handling

### Serialization Errors

Always handle serialization failures explicitly:

```rust
fn create_persist_effect(
    event: EventInventorySagaAction,
    expected_version: Version,
    env: &EventInventorySagaEnvironment,
) -> Effect<EventInventorySagaAction> {
    let ticketing_event = TicketingEvent::EventInventorySaga(event);
    let serialized = match ticketing_event.serialize() {
        Ok(s) => s,
        Err(e) => {
            // Return error action instead of silent failure
            return Effect::Future(Box::pin(async move {
                Some(EventInventorySagaAction::SerializationFailed {
                    error: format!("Failed to serialize saga event: {e}"),
                })
            }));
        }
    };

    // ... continue with append_events!
}
```

### Child Aggregate Failures

Map child failures to saga-specific error actions:

```rust
match event_store.send(create_action).await {
    Ok(_) => Some(EventInventorySagaAction::EventCreated { ... }),
    Err(e) => {
        tracing::error!(
            event_id = %event_id.as_uuid(),
            error = %e,
            "Saga: Event aggregate failed"
        );
        Some(EventInventorySagaAction::EventCreationFailed {
            event_id,
            error: e.to_string(),
            failed_at: now,
        })
    }
}
```

### Idempotency Guards

Prevent duplicate processing with idempotency checks:

```rust
EventInventorySagaAction::SectionInventoryInitialized { section, ... } => {
    // Idempotency: skip if section not in pending
    if !state.pending_sections.contains(&section) {
        tracing::debug!(
            section = %section,
            "Saga: Section already processed, skipping"
        );
        return SmallVec::new();
    }

    // ... process normally
}
```

---

## 8. Code Quality Patterns

### Combine Identical Match Arms

When multiple actions have identical handling, combine them:

```rust
// Good: combined with or-pattern and ref @
ref action @ (EventInventorySagaAction::VersionUpdated { .. }
| EventInventorySagaAction::ValidationFailed { .. }
| EventInventorySagaAction::SerializationFailed { .. }) => {
    Self::apply_event(state, action);
    SmallVec::new()
}

// Avoid: separate identical handlers
EventInventorySagaAction::VersionUpdated { version } => {
    Self::apply_event(state, &EventInventorySagaAction::VersionUpdated { version });
    SmallVec::new()
}
EventInventorySagaAction::ValidationFailed { error } => {
    Self::apply_event(state, &EventInventorySagaAction::ValidationFailed { error });
    SmallVec::new()
}
```

### Allow Attributes for Saga Reducers

Saga reducers are inherently complex. Use appropriate allows:

```rust
impl Reducer for EventInventorySaga {
    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
    fn reduce(&self, state: &mut Self::State, action: Self::Action, env: &Self::Environment)
        -> SmallVec<[Effect<Self::Action>; 4]>
    {
        // Saga state machines have inherently complex branching
        match action { ... }
    }
}
```

### Avoid Unnecessary Clones

When creating error actions from owned values, don't clone:

```rust
// Good: moves owned error into action
if let Err(error) = Self::validate(state, &name) {
    Self::apply_event(state, &EventInventorySagaAction::ValidationFailed { error });
    return SmallVec::new();
}

// Avoid: unnecessary clone
if let Err(error) = Self::validate(state, &name) {
    Self::apply_event(state, &EventInventorySagaAction::ValidationFailed {
        error: error.clone()  // Wasteful - we own error and don't use it after
    });
    return SmallVec::new();
}
```

---

## 9. Testing Sagas

### Test Each Step Independently

```rust
#[test]
fn test_step2_event_created_starts_inventory_initialization() {
    let event_id = EventId::new();

    // Start with state after Step 1
    let mut initial_state = EventInventorySagaState::new();
    initial_state.event_id = Some(event_id);
    initial_state.section_capacities.insert("VIP".to_string(), Capacity::new(50));
    initial_state.section_capacities.insert("General".to_string(), Capacity::new(150));

    ReducerTest::new(EventInventorySaga::new())
        .with_env(create_test_env())
        .given_state(initial_state)
        .when_action(EventInventorySagaAction::EventCreated {
            event_id,
            sections: vec!["VIP".to_string(), "General".to_string()],
            created_at: test_time(),
        })
        .then_state(|state| {
            assert!(state.event_created);
            assert_eq!(state.pending_sections.len(), 2);
        })
        .then_effects(|effects| {
            // 1 persist + 2 inventory effects
            assert_eq!(effects.len(), 3);
        })
        .run();
}
```

### Use Deterministic Time

```rust
/// Returns a fixed test time for deterministic tests
fn test_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp")
}
```

### Test Idempotency

```rust
#[test]
fn test_duplicate_section_initialized_is_idempotent() {
    let event_id = EventId::new();

    // State where VIP is already processed
    let mut initial_state = EventInventorySagaState::new();
    initial_state.event_id = Some(event_id);
    initial_state.event_created = true;
    initial_state.pending_sections.insert("General".to_string()); // Only General pending

    ReducerTest::new(EventInventorySaga::new())
        .with_env(create_test_env())
        .given_state(initial_state)
        .when_action(EventInventorySagaAction::SectionInventoryInitialized {
            event_id,
            section: "VIP".to_string(), // Already processed!
            initialized_at: test_time(),
        })
        .then_state(|state| {
            // State unchanged
            assert!(!state.pending_sections.contains("VIP"));
            assert!(state.pending_sections.contains("General"));
        })
        .then_effects(assertions::assert_no_effects)
        .run();
}
```

### Test Error Paths

```rust
#[test]
fn test_inventory_failed_marks_saga_failed() {
    let event_id = EventId::new();

    let mut initial_state = EventInventorySagaState::new();
    initial_state.event_id = Some(event_id);
    initial_state.event_created = true;
    initial_state.pending_sections.insert("VIP".to_string());

    ReducerTest::new(EventInventorySaga::new())
        .with_env(create_test_env())
        .given_state(initial_state)
        .when_action(EventInventorySagaAction::InventoryInitializationFailed {
            event_id,
            section: "VIP".to_string(),
            error: "Capacity exceeded".to_string(),
            failed_at: test_time(),
        })
        .then_state(|state| {
            assert!(state.failed);
            assert!(!state.completed);
            assert!(!state.pending_sections.contains("VIP"));
            assert!(state.last_error.is_some());
        })
        .run();
}
```

---

## 10. Checklist

Use this checklist when implementing or reviewing sagas:

### Architecture
- [ ] Saga uses feedback loop pattern (effects return actions)
- [ ] Factory functions for child aggregate stores in environment
- [ ] Clear state machine with progress tracking flags
- [ ] `pending_*` collection for tracking parallel work

### State Management
- [ ] Capture `expected_version` BEFORE calling `apply_event`
- [ ] Use `clone_from()` for HashMap/Vec assignments
- [ ] Clear `last_error` on successful state transitions
- [ ] Use `env.clock.now()` for timestamps (never `Utc::now()`)

### Replay/Recovery
- [ ] Replay handlers use `ref @` pattern
- [ ] Replay handlers return no effects (just apply state)
- [ ] Idempotency checks for duplicate event handling

### Error Handling
- [ ] `SerializationFailed` action variant exists
- [ ] `create_persist_effect` returns error action on serialization failure
- [ ] Child aggregate failures mapped to saga error actions
- [ ] Validation errors don't persist events

### Code Quality
- [ ] `#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]` on reduce
- [ ] `#[allow(clippy::struct_excessive_bools)]` on state if needed
- [ ] Identical match arms combined with or-pattern
- [ ] No unnecessary `.clone()` on owned values

### Testing
- [ ] Deterministic `test_time()` helper
- [ ] Each saga step tested independently
- [ ] Idempotency tested
- [ ] Error paths tested
- [ ] Uses shared test utilities from `crate::test_utils`

---

## Reference

- **Example Implementation**: [`../src/aggregates/event_inventory_saga.rs`](../src/aggregates/event_inventory_saga.rs)
- **Shared Patterns**: [aggregate-patterns.md](./aggregate-patterns.md)
- **Framework Skill**: `.claude/skills/composable-rust-sagas/SKILL.md` (from repo root)
