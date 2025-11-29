# Aggregate Patterns Guide

This document defines the standard patterns for implementing aggregates in the ticketing system. Use it as a reference when reviewing existing aggregates or writing new ones.

**Reference Implementation**: `src/aggregates/event.rs`

---

## Table of Contents

1. [File Structure](#1-file-structure)
2. [Action Enum Design](#2-action-enum-design)
3. [Environment Pattern](#3-environment-pattern)
4. [Reducer Structure](#4-reducer-structure)
5. [Validation Patterns](#5-validation-patterns)
6. [Two-Phase Async Pattern](#6-two-phase-async-pattern)
7. [Effect Patterns](#7-effect-patterns)
8. [State Management](#8-state-management)
9. [Testing Strategy and Discipline](#9-testing-strategy-and-discipline)
10. [Checklist](#10-checklist)
11. [Code Quality Patterns](#11-code-quality-patterns)

---

## 1. File Structure

Every aggregate file follows this organization:

```rust
//! Module-level documentation explaining the aggregate's purpose

use crate::projections::{...};  // Projection queries
use crate::types::{...};        // Domain types
use chrono::{DateTime, Utc};
use composable_rust_core::{...};
use std::collections::HashSet;  // If needed for validation
use std::sync::Arc;

// ============================================================================
// Actions (Commands + Events)
// ============================================================================

#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum {Aggregate}Action { ... }

// ============================================================================
// Environment
// ============================================================================

#[derive(Clone)]
pub struct {Aggregate}Environment { ... }

impl {Aggregate}Environment {
    pub fn new(...) -> Self { ... }
}

// ============================================================================
// Reducer
// ============================================================================

#[derive(Clone, Debug)]
pub struct {Aggregate}Reducer;

impl {Aggregate}Reducer {
    pub const fn new() -> Self { Self }

    // Private helper methods:
    fn create_effects(...) -> SmallVec<[Effect<{Aggregate}Action>; 4]> { ... }
    fn validate_*(...) -> Result<(), String> { ... }
    fn apply_event(state: &mut {Aggregate}State, action: &{Aggregate}Action) { ... }
}

impl Default for {Aggregate}Reducer { ... }
impl Reducer for {Aggregate}Reducer { ... }

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests { ... }
```

---

## 2. Action Enum Design

Actions are organized into distinct categories with derive macro attributes:

### 2.1 Action Categories

```rust
#[derive(Action, Clone, Debug, Serialize, Deserialize)]
pub enum {Aggregate}Action {
    // ─────────────────────────────────────────────────────────────────────
    // COMMANDS: External API requests that initiate state changes
    // ─────────────────────────────────────────────────────────────────────

    #[command]
    DoSomething {
        id: EntityId,
        // ... command parameters
        #[serde(skip)]
        respond_to: ResponseChannel,  // For projection confirmation
    },

    // ─────────────────────────────────────────────────────────────────────
    // EVENTS: Record what happened (persisted to event store)
    // ─────────────────────────────────────────────────────────────────────

    #[event]
    SomethingDone {
        id: EntityId,
        // ... event data (fat events include all relevant data)
        done_at: DateTime<Utc>,
        #[serde(skip)]
        respond_to: ResponseChannel,
    },

    // ─────────────────────────────────────────────────────────────────────
    // INTERNAL ACTIONS: Two-phase pattern (async load → sync execute)
    // ─────────────────────────────────────────────────────────────────────

    #[doc(hidden)]
    ExecuteDoSomething {
        id: EntityId,
        loaded_entity: Entity,           // Data loaded from projection
        current_version: Version,        // For optimistic concurrency
        // Optional: updated_at: DateTime<Utc>,  // If timestamp captured in Phase 1
    },

    // ─────────────────────────────────────────────────────────────────────
    // QUERIES: Read-only operations
    // ─────────────────────────────────────────────────────────────────────

    #[command]
    GetEntity { id: EntityId },

    #[event]
    EntityQueried { id: EntityId, entity: Option<Entity> },

    // ─────────────────────────────────────────────────────────────────────
    // ERROR & INFRASTRUCTURE ACTIONS
    // ─────────────────────────────────────────────────────────────────────

    #[event]
    ValidationFailed { error: String },

    #[event]
    SerializationFailed { error: String },

    #[event]
    VersionUpdated { version: Version },

    #[event]
    {Aggregate}ProjectionConfirmed { id: EntityId },

    #[event]
    {Aggregate}ProjectionFailed { id: EntityId, reason: String },
}
```

### 2.2 Key Principles

| Category | Purpose | Persistence | Side Effects |
|----------|---------|-------------|--------------|
| Commands | Express intent | No | Trigger validation + events |
| Events | Record state changes | Yes (event store) | Update local state |
| Execute* | Two-phase pattern | No | Trigger effects after async load |
| Queries | Read data | No | Load from projection |
| Errors | Track failures | No | Update `last_error` |
| Infrastructure | System coordination | Depends | Version tracking, projection sync |

### 2.3 ResponseChannel Pattern

**Not all commands need `ResponseChannel`**. Only include it when the API handler needs to wait for projection completion before responding to the client.

**Commands WITH ResponseChannel** (need projection confirmation):
- Create operations (e.g., `CreateEvent`) - client needs the created entity
- Update operations that affect queries (e.g., `UpdatePricingTiers`, `AddVenueSections`)

**Commands WITHOUT ResponseChannel** (fire-and-forget):
- Simple state transitions (e.g., `PublishEvent`, `OpenSales`, `CloseSales`, `CancelEvent`)
- Queries (e.g., `GetEvent`, `ListEvents`) - they return data directly

```rust
// WITH ResponseChannel - needs projection confirmation
#[command]
CreateEntity {
    id: EntityId,
    // ... other fields
    #[serde(skip)]           // Don't serialize channels
    respond_to: ResponseChannel,
}

// WITHOUT ResponseChannel - simple state transition
#[command]
PublishEntity {
    entity_id: EntityId,
}
```

When a command has `ResponseChannel`, the corresponding event also carries it:

```rust
#[event]
EntityCreated {
    id: EntityId,
    // ... other fields
    #[serde(skip)]
    respond_to: ResponseChannel,
}
```

---

## 3. Environment Pattern

The environment provides all external dependencies via dependency injection.

### 3.1 Standard Environment Structure

```rust
#[derive(Clone)]
pub struct {Aggregate}Environment {
    /// Clock for timestamps
    pub clock: Arc<dyn Clock>,

    /// Event store for persistence
    pub event_store: Arc<dyn EventStore>,

    /// Stream ID for this aggregate instance
    pub stream_id: StreamId,

    /// Projection query for loading this aggregate's state
    pub projection: Arc<dyn {Aggregate}ProjectionQuery>,

    /// Global action channels for cross-aggregate coordination
    pub global_actions: GlobalActionChannels,

    // OPTIONAL: Query for related aggregate data (only if needed)
    // pub event_query: Arc<dyn EventProjectionQuery>,
}
```

**Required fields**: `clock`, `event_store`, `stream_id`, `projection`, `global_actions`

**Optional fields**: Additional projection queries for related aggregates (e.g., Inventory needs `EventProjectionQuery` to load pricing data)

### 3.2 Constructor Pattern

```rust
impl {Aggregate}Environment {
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        event_store: Arc<dyn EventStore>,
        stream_id: StreamId,
        projection: Arc<dyn {Aggregate}ProjectionQuery>,
        global_actions: GlobalActionChannels,
    ) -> Self {
        Self {
            clock,
            event_store,
            stream_id,
            projection,
            global_actions,
        }
    }
}
```

### 3.3 Projection Query Traits

Define traits for the data this aggregate needs to load:

```rust
// In src/projections/mod.rs or src/aggregates/{aggregate}.rs
#[async_trait]
pub trait {Aggregate}ProjectionQuery: Send + Sync {
    async fn load_entity(&self, id: &EntityId) -> Result<Option<Entity>, String>;
    async fn load_entities(&self, filter: Option<Status>) -> Result<Vec<Entity>, String>;
}
```

---

## 4. Reducer Structure

### 4.1 Reducer Implementation

```rust
#[derive(Clone, Debug)]
pub struct {Aggregate}Reducer;

impl {Aggregate}Reducer {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for {Aggregate}Reducer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reducer for {Aggregate}Reducer {
    type State = {Aggregate}State;
    type Action = {Aggregate}Action;
    type Environment = {Aggregate}Environment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> SmallVec<[Effect<Self::Action>; 4]> {
        match action {
            // Commands, Events, Queries...
        }
    }
}
```

### 4.2 reduce() Method Organization

Organize match arms by category with clear section comments:

```rust
fn reduce(&self, state: &mut Self::State, action: Self::Action, env: &Self::Environment)
    -> SmallVec<[Effect<Self::Action>; 4]>
{
    match action {
        // ═══════════════════════════════════════════════════════════════════
        // COMMANDS: External API requests
        // ═══════════════════════════════════════════════════════════════════

        {Aggregate}Action::CreateEntity { ... } => { ... }
        {Aggregate}Action::UpdateEntity { ... } => { ... }

        // ═══════════════════════════════════════════════════════════════════
        // COMMANDS WITH ASYNC VALIDATION (Two-phase pattern)
        // ═══════════════════════════════════════════════════════════════════

        {Aggregate}Action::UpdateWithLoad { ... } => { ... }
        {Aggregate}Action::ExecuteUpdateWithLoad { ... } => { ... }

        // ═══════════════════════════════════════════════════════════════════
        // QUERIES: Read-only operations
        // ═══════════════════════════════════════════════════════════════════

        {Aggregate}Action::GetEntity { ... } => { ... }
        {Aggregate}Action::ListEntities { ... } => { ... }

        // ═══════════════════════════════════════════════════════════════════
        // EVENTS: State updates (from event store replay only)
        // ═══════════════════════════════════════════════════════════════════
        //
        // This catch-all handles EVENT REPLAY during state reconstruction.
        // It does NOT handle live events during normal operation.
        //
        // When processing live commands:
        //   1. Command handler calls apply_event() directly
        //   2. Command handler calls create_effects() with broadcast_on_success
        //   3. broadcast_on_success broadcasts WITHOUT re-entering reducer
        //   4. NO catch-all needed for live events
        //
        // When replaying from event store:
        //   1. Persisted events arrive as actions
        //   2. This catch-all applies them to state
        //   3. No effects returned (they were already executed)

        event => {
            Self::apply_event(state, &event);
            SmallVec::new()
        }
    }
}
```

---

## 5. Validation Patterns

### 5.1 Synchronous Validation (Local State)

For commands that only need local state:

```rust
fn validate_create_entity(
    state: &{Aggregate}State,
    id: &EntityId,
    name: &str,
) -> Result<(), String> {
    // Entity must not already exist
    if state.exists(id) {
        return Err(format!("Entity with ID {id} already exists"));
    }

    // Name validation
    if name.is_empty() {
        return Err("Entity name cannot be empty".to_string());
    }

    if name.len() > 200 {
        return Err(format!("Name too long: {} characters (max 200)", name.len()));
    }

    Ok(())
}
```

### 5.2 Async Validation (With Loaded Data)

For commands that need projection data:

```rust
fn validate_update_with_loaded_entity(
    entity: &Entity,
    new_data: &Data,
) -> Result<(), String> {
    // Cannot update cancelled entities
    if entity.status == Status::Cancelled {
        return Err("Cannot update cancelled entity".to_string());
    }

    // Validate new_data against loaded entity
    if !entity.sections.contains(&new_data.section) {
        return Err(format!("Section '{}' does not exist", new_data.section));
    }

    Ok(())
}
```

### 5.3 HashSet for Efficient Lookups

Use `HashSet` when validating against collections:

```rust
fn validate_sections(entity: &Entity, sections: &[Section]) -> Result<(), String> {
    let existing: HashSet<&str> = entity.sections.iter().map(|s| s.name.as_str()).collect();

    for section in sections {
        if existing.contains(section.name.as_str()) {
            return Err(format!("Section '{}' already exists", section.name));
        }
    }

    Ok(())
}
```

---

## 6. Two-Phase Async Pattern

Use when commands need to load data from projections before validation.

### 6.1 Phase 1: Command → Effect::Future → Execute Action

```rust
{Aggregate}Action::UpdateEntity { entity_id, new_data, respond_to: _ } => {
    // ┌─────────────────────────────────────────────────────────────────┐
    // │ STEP 1: Early sync validation (before async load)              │
    // │ Fail fast for obviously invalid requests                       │
    // └─────────────────────────────────────────────────────────────────┘
    if new_data.is_empty() {
        Self::apply_event(state, &{Aggregate}Action::ValidationFailed {
            error: "No fields to update".to_string(),
        });
        return SmallVec::new();
    }

    // ┌─────────────────────────────────────────────────────────────────┐
    // │ STEP 2: Clone dependencies for async block                     │
    // │ Include clock if you need timestamps inside async              │
    // └─────────────────────────────────────────────────────────────────┘
    let projection = env.projection.clone();
    let event_store = env.event_store.clone();
    let stream_id = env.stream_id.clone();
    let clock = env.clock.clone();  // Capture clock for async use

    smallvec![Effect::Future(Box::pin(async move {
        // ┌─────────────────────────────────────────────────────────────┐
        // │ STEP 3: Load data in parallel using tokio::join!           │
        // └─────────────────────────────────────────────────────────────┘
        let (projection_result, version_result) = tokio::join!(
            projection.load_entity(&entity_id),
            event_store.get_stream_version(stream_id)
        );

        // Handle projection result
        let loaded_entity = match projection_result {
            Ok(Some(entity)) => entity,
            Ok(None) => {
                return Some({Aggregate}Action::ValidationFailed {
                    error: format!("Entity {entity_id} not found"),
                });
            }
            Err(e) => {
                return Some({Aggregate}Action::ValidationFailed {
                    error: format!("Failed to load entity: {e}"),
                });
            }
        };

        // Handle version result
        let current_version = match version_result {
            Ok(version) => version,
            Err(e) => {
                return Some({Aggregate}Action::ValidationFailed {
                    error: format!("Failed to load version: {e}"),
                });
            }
        };

        // ┌─────────────────────────────────────────────────────────────┐
        // │ STEP 4: Validate with loaded data                          │
        // │ Can use validation function OR inline check                │
        // └─────────────────────────────────────────────────────────────┘
        // Option A: Call validation function
        if let Err(error) = Self::validate_update_with_loaded_entity(&loaded_entity, &new_data) {
            return Some({Aggregate}Action::ValidationFailed { error });
        }

        // Option B: Inline validation (for simple checks)
        // if loaded_entity.status == Status::Cancelled {
        //     return Some({Aggregate}Action::ValidationFailed {
        //         error: "Cannot update cancelled entity".to_string(),
        //     });
        // }

        // ┌─────────────────────────────────────────────────────────────┐
        // │ STEP 5: Return Execute action with loaded data             │
        // │ Use clock.now() here if timestamp needed                   │
        // └─────────────────────────────────────────────────────────────┘
        Some({Aggregate}Action::ExecuteUpdateEntity {
            entity_id,
            new_data,
            loaded_entity,
            current_version,
            updated_at: clock.now(),  // Timestamp captured in async context
        })
    }))]
}
```

### 6.2 Phase 2: Execute Action → Apply + Effects

```rust
{Aggregate}Action::ExecuteUpdateEntity {
    entity_id,
    new_data,
    loaded_entity,
    current_version,
    updated_at,  // Timestamp from Phase 1 (or capture here with env.clock.now())
} => {
    // Insert loaded entity into local state for tracking
    state.entities.insert(entity_id, loaded_entity);

    // Create and apply event (with placeholder respond_to for local state)
    let event_for_state = {Aggregate}Action::EntityUpdated {
        entity_id,
        new_data: new_data.clone(),
        updated_at,
        respond_to: ResponseChannel::none(),
    };
    Self::apply_event(state, &event_for_state);

    // Create effects (use current_version directly for optimistic concurrency)
    let mut effects = Self::create_effects(event_for_state, current_version, env);

    // Add projection confirmation effect (if this aggregate uses ResponseChannel)
    let id_success = entity_id;
    let id_error = entity_id;
    effects.push(Effect::PublishWithResponse {
        // Use the appropriate channel: event_actions, inventory_actions, etc.
        channel: env.global_actions.event_actions.clone(),
        create_action: Box::new(move |respond_to| {Aggregate}Action::EntityUpdated {
            entity_id,
            new_data,
            updated_at,
            respond_to,
        }),
        on_success: Box::new(move || Some({Aggregate}Action::{Aggregate}ProjectionConfirmed {
            id: id_success,
        })),
        on_error: Box::new(move |reason| Some({Aggregate}Action::{Aggregate}ProjectionFailed {
            id: id_error,
            reason,
        })),
    });

    effects
}
```

**Note on timestamps**: Always use the **injected clock from the environment** (`env.clock`) - never `Utc::now()` directly. This keeps reducers pure and testable with `FixedClock`. Two patterns:

1. **Capture in Phase 1**: Clone `env.clock` before async block, call `clock.now()` inside (shown above)
2. **Capture in Phase 2**: Call `env.clock.now()` directly in the Execute handler

Option 1 is preferred when the timestamp should reflect when async validation completed.

---

## 7. Effect Patterns

### 7.1 The Echo Problem and Its Solution

When using `send_and_wait_for`, observers listen on the broadcast channel to detect when specific actions occur. Previously, this required "echoing" events back via `Effect::Future`:

```rust
// ❌ OLD PATTERN (problematic - causes reducer re-entry)
smallvec![
    append_events! { ... },
    Effect::Future(Box::pin(async move { Some(event) }))  // Echoes event back
]
```

**Problems with the old pattern:**

1. **Re-entry**: The echoed event re-enters the reducer, requiring handlers
2. **Infinite loops**: If the handler calls `create_effects` again → infinite loop
3. **Catch-all needed**: Either explicit handlers or a catch-all to swallow re-entry
4. **Race condition**: The echo could broadcast BEFORE persistence completes

**The clean solution** uses `broadcast_on_success` in the `append_events!` macro:

```rust
// ✅ NEW PATTERN (clean - no reducer re-entry)
append_events! {
    store: env.event_store,
    stream: env.stream_id.as_str(),
    expected_version: Some(expected_version),
    events: vec![serialized],
    broadcast_on_success: event,  // Broadcast AFTER persist, NO re-entry
    on_success: |version| Some({Aggregate}Action::VersionUpdated { version }),
    on_error: |error| Some({Aggregate}Action::ValidationFailed { error: error.to_string() })
}
```

**Benefits:**

1. **No re-entry**: The event is broadcast but doesn't feed back into reducer
2. **Guaranteed ordering**: Broadcast happens AFTER successful persistence
3. **No catch-all needed**: Reducers stay lean with only meaningful handlers
4. **No infinite loops**: Architecturally impossible

### 7.2 create_effects Helper

Standard helper for persistence effects with broadcast:

```rust
fn create_effects(
    event: {Aggregate}Action,
    expected_version: Version,
    env: &{Aggregate}Environment,
) -> SmallVec<[Effect<{Aggregate}Action>; 4]> {
    // Wrap in TicketingEvent for serialization
    // TicketingEvent variants match aggregate names: Event, Inventory, Reservation, Payment
    let ticketing_event = TicketingEvent::Event(event.clone());  // For Event aggregate
    // let ticketing_event = TicketingEvent::Inventory(event.clone());  // For Inventory aggregate

    let serialized = match ticketing_event.serialize() {
        Ok(s) => s,
        Err(e) => {
            return smallvec![Effect::Future(Box::pin(async move {
                Some({Aggregate}Action::SerializationFailed {
                    error: format!("Failed to serialize event: {e}"),
                })
            }))];
        }
    };

    smallvec![
        append_events! {
            store: env.event_store,
            stream: env.stream_id.as_str(),
            expected_version: Some(expected_version),
            events: vec![serialized],
            broadcast_on_success: event,  // Broadcast for send_and_wait_for detection
            on_success: |version| Some({Aggregate}Action::VersionUpdated { version }),
            on_error: |error| Some({Aggregate}Action::ValidationFailed {
                error: error.to_string()
            })
        }
    ]
}
```

### 7.3 Effect::BroadcastOnly for Non-Persistence Cases

For cases where you need to broadcast an action without event store operations:

```rust
// Broadcast an action for observers without re-entering reducer
Effect::BroadcastOnly(Box::new({Aggregate}Action::SomeNotification { ... }))
```

**Use `broadcast_on_success`** when:
- You're persisting events AND need to signal completion to `send_and_wait_for`
- Guaranteed ordering (broadcast after persist) is required

**Use `Effect::BroadcastOnly`** when:
- You're NOT persisting to event store but need to notify observers
- The action should NOT re-enter the reducer

**TicketingEvent variants** (defined in `src/projections/mod.rs`):
- `TicketingEvent::Event(EventAction)` - Event aggregate
- `TicketingEvent::Inventory(InventoryAction)` - Inventory aggregate
- `TicketingEvent::Reservation(ReservationAction)` - Reservation aggregate
- `TicketingEvent::Payment(PaymentAction)` - Payment aggregate

### 7.4 Query Effects

For read-only queries:

```rust
{Aggregate}Action::GetEntity { entity_id } => {
    let projection = env.projection.clone();
    smallvec![Effect::Future(Box::pin(async move {
        match projection.load_entity(&entity_id).await {
            Ok(entity) => Some({Aggregate}Action::EntityQueried { entity_id, entity }),
            Err(e) => Some({Aggregate}Action::ValidationFailed {
                error: format!("Failed to load entity: {e}"),
            }),
        }
    }))]
}
```

---

## 8. State Management

### 8.1 apply_event Method

Centralized state update logic:

```rust
#[allow(clippy::too_many_lines)]  // If needed - add justification comment
fn apply_event(state: &mut {Aggregate}State, action: &{Aggregate}Action) {
    match action {
        {Aggregate}Action::EntityCreated { id, name, created_at, .. } => {
            let entity = Entity::new(*id, name.clone(), *created_at);
            state.entities.insert(*id, entity);
            state.last_error = None;
        }

        {Aggregate}Action::EntityUpdated { entity_id, name, .. } => {
            if let Some(entity) = state.entities.get_mut(entity_id) {
                if let Some(new_name) = name {
                    entity.name.clone_from(new_name);
                }
            }
            state.last_error = None;
        }

        {Aggregate}Action::VersionUpdated { version } => {
            state.version = *version;
        }

        {Aggregate}Action::ValidationFailed { error }
        | {Aggregate}Action::SerializationFailed { error } => {
            state.last_error = Some(error.clone());
        }

        // Commands and queries don't modify state
        {Aggregate}Action::CreateEntity { .. }
        | {Aggregate}Action::UpdateEntity { .. }
        | {Aggregate}Action::GetEntity { .. }
        | {Aggregate}Action::EntityQueried { .. }
        | {Aggregate}Action::{Aggregate}ProjectionConfirmed { .. }
        | {Aggregate}Action::{Aggregate}ProjectionFailed { .. }
        | {Aggregate}Action::ExecuteUpdateEntity { .. } => {}
    }
}
```

### 8.2 State Struct Requirements

```rust
// In src/types.rs
pub struct {Aggregate}State {
    pub entities: HashMap<EntityId, Entity>,
    pub version: Version,
    pub last_error: Option<String>,
}

impl {Aggregate}State {
    pub fn new() -> Self {
        Self {
            entities: HashMap::new(),
            version: Version::new(0),
            last_error: None,
        }
    }

    pub fn exists(&self, id: &EntityId) -> bool {
        self.entities.contains_key(id)
    }

    pub fn get(&self, id: &EntityId) -> Option<&Entity> {
        self.entities.get(id)
    }

    pub fn count(&self) -> usize {
        self.entities.len()
    }
}
```

---

## 9. Testing Strategy and Discipline

This section defines the testing philosophy for aggregates using the two-phase async pattern.

### 9.1 Testing Philosophy

For the two-phase async pattern, we have two distinct layers of behavior:

| Layer | What Happens | Testing Tool |
|-------|--------------|--------------|
| **Sync Validation** | Command received → immediate validation → reject or proceed to async | `ReducerTest` |
| **Full Async Flow** | Command → Effect::Future → Execute action → Terminal event → State updated | `TestStore` |

**Key Principle**: Never test internal `Execute*` actions directly—they are implementation details of the two-phase pattern.

### 9.2 Why This Matters

**❌ Testing Execute actions is problematic:**
```rust
// This tests implementation details!
.when_action({Aggregate}Action::ExecuteUpdateEntity {
    entity_id: id,
    current_version: Version::new(1),  // ← Internal detail
    loaded_entity,                      // ← Internal detail
    updated_at,                         // ← Internal detail
})
```

Problems:
- Exposes internal implementation details
- Tests break if we refactor the two-phase pattern
- Requires manufacturing internal state (`current_version`, `loaded_entity`)
- Duplicates what async TestStore tests already cover

**✅ Test at the right abstraction level:**
- **ReducerTest**: Test COMMAND actions for sync validation
- **TestStore**: Test full flows from command to terminal event

### 9.3 Test Structure

Organize tests into clear sections:

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use composable_rust_core::environment::SystemClock;
    use composable_rust_testing::{assertions, mocks::InMemoryEventStore, ReducerTest, TestStore};
    use std::time::Duration;

    // =========================================================================
    // Test Infrastructure
    // =========================================================================

    fn create_test_env() -> {Aggregate}Environment { ... }
    fn create_test_env_with_projection(projection: Arc<dyn Query>) -> {Aggregate}Environment { ... }

    // Configurable mock for different test scenarios
    #[derive(Clone, Default)]
    struct ConfigurableMockQuery {
        entities: Arc<RwLock<HashMap<EntityId, Entity>>>,
        load_error: Arc<RwLock<Option<String>>>,
    }

    impl ConfigurableMockQuery {
        fn new() -> Self { Self::default() }
        fn with_entity(self, entity: Entity) -> Self { ... }
        fn with_error(self, error: &str) -> Self { ... }
    }

    // =========================================================================
    // Sync Validation Tests (ReducerTest)
    // =========================================================================
    //
    // Fast, pure tests that verify commands with invalid inputs are rejected
    // immediately without triggering async effects. These test the COMMAND
    // actions, NOT the internal Execute actions.

    #[test]
    fn test_create_entity_empty_name_rejected() { ... }

    #[test]
    fn test_create_entity_duplicate_rejected() { ... }

    // =========================================================================
    // Full Flow Tests (TestStore)
    // =========================================================================
    //
    // Test complete async behavior from command to terminal event.
    // These test the REAL behavior without knowing about internal Execute actions.

    // -------------------------------------------------------------------------
    // Happy Paths
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_create_entity_success() { ... }

    #[tokio::test]
    async fn test_update_entity_success() { ... }

    // -------------------------------------------------------------------------
    // Async Validation Failures
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_update_entity_not_found() { ... }

    #[tokio::test]
    async fn test_update_cancelled_entity_rejected() { ... }

    // -------------------------------------------------------------------------
    // Query Operations
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_get_entity() { ... }

    #[tokio::test]
    async fn test_list_entities() { ... }
}
```

### 9.4 ReducerTest for Sync Validation

Use `ReducerTest` to verify that commands with invalid inputs fail immediately:

```rust
#[test]
fn test_create_entity_empty_name_rejected() {
    ReducerTest::new({Aggregate}Reducer::new())
        .with_env(create_test_env())
        .given_state({Aggregate}State::new())
        .when_action({Aggregate}Action::CreateEntity {
            id: EntityId::new(),
            name: String::new(),  // Invalid!
            respond_to: ResponseChannel::none(),
        })
        .then_state(|state| {
            assert_eq!(state.count(), 0);
            assert!(state.last_error.is_some());
            assert!(state.last_error.as_ref().unwrap().contains("cannot be empty"));
        })
        .then_effects(assertions::assert_no_effects)
        .run();
}

#[test]
fn test_create_entity_duplicate_rejected() {
    let id = EntityId::new();

    ReducerTest::new({Aggregate}Reducer::new())
        .with_env(create_test_env())
        .given_state({
            let mut state = {Aggregate}State::new();
            state.entities.insert(id, Entity::new(id, "Existing".to_string()));
            state
        })
        .when_action({Aggregate}Action::CreateEntity {
            id,  // Duplicate!
            name: "New Entity".to_string(),
            respond_to: ResponseChannel::none(),
        })
        .then_state(move |state| {
            // Original entity unchanged
            let entity = state.get(&id).unwrap();
            assert_eq!(entity.name, "Existing");
            // Error recorded
            assert!(state.last_error.as_ref().unwrap().contains("already exists"));
        })
        .then_effects(assertions::assert_no_effects)
        .run();
}
```

### 9.5 TestStore for Full Async Flows

Use `TestStore` to test complete behavior from command to terminal event:

```rust
#[tokio::test]
async fn test_update_entity_success() {
    let entity_id = EntityId::new();

    // Configure mock projection with existing entity
    let existing = Entity::new(entity_id, "Original Name".to_string());
    let mock = ConfigurableMockQuery::new().with_entity(existing);
    let env = create_test_env_with_projection(Arc::new(mock));
    let store = TestStore::new({Aggregate}Reducer::new(), env, {Aggregate}State::new());

    // Send command and wait for TERMINAL action (EntityUpdated)
    // NOT the intermediate Execute action!
    let result = store
        .send_and_wait_for(
            {Aggregate}Action::UpdateEntity {
                entity_id,
                new_name: "Updated Name".to_string(),
            },
            |action| {
                matches!(
                    action,
                    {Aggregate}Action::EntityUpdated { .. }
                        | {Aggregate}Action::ValidationFailed { .. }
                )
            },
            Duration::from_secs(5),
        )
        .await;

    assert!(result.is_ok(), "Should receive EntityUpdated");
    let action = result.unwrap();
    assert!(
        matches!(action, {Aggregate}Action::EntityUpdated { .. }),
        "Expected EntityUpdated, got {action:?}"
    );

    // No sleep needed - when terminal action is broadcast, state is already updated
    let state = store.state(|s| s.clone()).await;
    let entity = state.get(&entity_id).unwrap();
    assert_eq!(entity.name, "Updated Name");

    store.clear_queue();
}
```

### 9.6 Testing Async Validation Failures

Test that async validation (in Phase 1's Effect::Future) correctly returns `ValidationFailed`:

```rust
#[tokio::test]
async fn test_update_entity_not_found() {
    // Empty projection - entity doesn't exist
    let env = create_test_env_with_projection(Arc::new(ConfigurableMockQuery::new()));
    let store = TestStore::new({Aggregate}Reducer::new(), env, {Aggregate}State::new());

    let result = store
        .send_and_wait_for(
            {Aggregate}Action::UpdateEntity {
                entity_id: EntityId::new(),
                new_name: "Whatever".to_string(),
            },
            |action| {
                matches!(
                    action,
                    {Aggregate}Action::EntityUpdated { .. }
                        | {Aggregate}Action::ValidationFailed { .. }
                )
            },
            Duration::from_secs(5),
        )
        .await;

    assert!(result.is_ok(), "Should receive ValidationFailed");
    match result.unwrap() {
        {Aggregate}Action::ValidationFailed { error } => {
            assert!(error.contains("not found"), "Error should mention 'not found': {error}");
        }
        other => panic!("Expected ValidationFailed, got {other:?}"),
    }

    store.clear_queue();
}

#[tokio::test]
async fn test_update_cancelled_entity_rejected() {
    let entity_id = EntityId::new();

    // Entity exists but is cancelled
    let mut cancelled = Entity::new(entity_id, "Cancelled Entity".to_string());
    cancelled.status = Status::Cancelled;
    let mock = ConfigurableMockQuery::new().with_entity(cancelled);
    let env = create_test_env_with_projection(Arc::new(mock));
    let store = TestStore::new({Aggregate}Reducer::new(), env, {Aggregate}State::new());

    let result = store
        .send_and_wait_for(
            {Aggregate}Action::UpdateEntity {
                entity_id,
                new_name: "New Name".to_string(),
            },
            |action| {
                matches!(
                    action,
                    {Aggregate}Action::EntityUpdated { .. }
                        | {Aggregate}Action::ValidationFailed { .. }
                )
            },
            Duration::from_secs(5),
        )
        .await;

    match result.unwrap() {
        {Aggregate}Action::ValidationFailed { error } => {
            assert!(error.contains("cancelled"), "Error should mention 'cancelled': {error}");
        }
        other => panic!("Expected ValidationFailed, got {other:?}"),
    }

    store.clear_queue();
}
```

### 9.7 Testing Query Operations

Don't forget to test query actions:

```rust
#[tokio::test]
async fn test_get_entity() {
    let entity_id = EntityId::new();
    let entity = Entity::new(entity_id, "Test Entity".to_string());
    let mock = ConfigurableMockQuery::new().with_entity(entity.clone());
    let env = create_test_env_with_projection(Arc::new(mock));
    let store = TestStore::new({Aggregate}Reducer::new(), env, {Aggregate}State::new());

    let result = store
        .send_and_wait_for(
            {Aggregate}Action::GetEntity { entity_id },
            |action| matches!(action, {Aggregate}Action::EntityQueried { .. } | {Aggregate}Action::ValidationFailed { .. }),
            Duration::from_secs(5),
        )
        .await;

    match result.unwrap() {
        {Aggregate}Action::EntityQueried { entity_id: id, entity: Some(e) } => {
            assert_eq!(id, entity_id);
            assert_eq!(e.name, "Test Entity");
        }
        other => panic!("Expected EntityQueried with entity, got {other:?}"),
    }

    store.clear_queue();
}

#[tokio::test]
async fn test_get_entity_not_found() {
    let env = create_test_env_with_projection(Arc::new(ConfigurableMockQuery::new()));
    let store = TestStore::new({Aggregate}Reducer::new(), env, {Aggregate}State::new());

    let result = store
        .send_and_wait_for(
            {Aggregate}Action::GetEntity { entity_id: EntityId::new() },
            |action| matches!(action, {Aggregate}Action::EntityQueried { .. } | {Aggregate}Action::ValidationFailed { .. }),
            Duration::from_secs(5),
        )
        .await;

    match result.unwrap() {
        {Aggregate}Action::EntityQueried { entity: None, .. } => {
            // Expected - entity not found returns None
        }
        other => panic!("Expected EntityQueried with None, got {other:?}"),
    }

    store.clear_queue();
}
```

### 9.8 Configurable Mock Pattern

Create a configurable mock that supports different test scenarios:

```rust
#[derive(Clone, Default)]
struct ConfigurableMockQuery {
    entities: Arc<RwLock<HashMap<EntityId, Entity>>>,
    load_error: Arc<RwLock<Option<String>>>,
}

impl ConfigurableMockQuery {
    fn new() -> Self {
        Self::default()
    }

    /// Add an entity to be returned by load queries
    fn with_entity(self, entity: Entity) -> Self {
        self.entities.write().unwrap().insert(entity.id, entity);
        self
    }

    /// Configure load queries to return an error
    #[allow(dead_code)]
    fn with_error(self, error: &str) -> Self {
        *self.load_error.write().unwrap() = Some(error.to_string());
        self
    }
}

#[async_trait::async_trait]
impl {Aggregate}ProjectionQuery for ConfigurableMockQuery {
    async fn load_entity(&self, id: &EntityId) -> Result<Option<Entity>, String> {
        if let Some(ref error) = *self.load_error.read().unwrap() {
            return Err(error.clone());
        }
        Ok(self.entities.read().unwrap().get(id).cloned())
    }

    async fn load_entities(&self, filter: Option<Status>) -> Result<Vec<Entity>, String> {
        if let Some(ref error) = *self.load_error.read().unwrap() {
            return Err(error.clone());
        }
        let entities: Vec<Entity> = self.entities.read().unwrap()
            .values()
            .filter(|e| filter.map_or(true, |f| e.status == f))
            .cloned()
            .collect();
        Ok(entities)
    }
}
```

### 9.9 Terminal vs Intermediate Actions

**Critical**: When using `send_and_wait_for`, wait for **terminal** actions, not **intermediate** actions:

| Action Type | Examples | Wait For? |
|-------------|----------|-----------|
| **Command** | `CreateEntity`, `UpdateEntity` | Send this |
| **Intermediate** | `ExecuteCreateEntity`, `ExecuteUpdateEntity` | ❌ No |
| **Terminal** | `EntityCreated`, `EntityUpdated`, `ValidationFailed` | ✅ Yes |

**Why this matters:**

The broadcast happens after the action is processed. When you wait for the terminal action (`EntityUpdated`), the state is already updated. If you wait for the intermediate action (`ExecuteUpdateEntity`), the state is still being processed.

```rust
// ❌ WRONG: Waiting for intermediate action requires sleep
let result = store.send_and_wait_for(
    command,
    |a| matches!(a, ExecuteUpdateEntity { .. }),  // Intermediate!
    timeout,
).await;
tokio::time::sleep(Duration::from_millis(100)).await;  // Hack needed!

// ✅ CORRECT: Waiting for terminal action - no sleep needed
let result = store.send_and_wait_for(
    command,
    |a| matches!(a, EntityUpdated { .. } | ValidationFailed { .. }),  // Terminal!
    timeout,
).await;
// State is already updated when terminal action is received
```

### 9.10 Test Coverage Summary

For each aggregate, ensure coverage of:

| Category | Testing Tool | What to Test |
|----------|--------------|--------------|
| **Sync Validation** | `ReducerTest` | Zero values, empty strings, duplicates, invalid state |
| **Happy Paths** | `TestStore` | Each command's success flow → terminal event |
| **Async Validation** | `TestStore` | Entity not found, wrong status, business rule violations |
| **Query Operations** | `TestStore` | Get single, list with filters, not found cases |

**Example test count for a typical aggregate:**

```
Sync Validation (ReducerTest):     4 tests
Happy Paths (TestStore):           3 tests
Async Validation (TestStore):      3 tests
Query Operations (TestStore):      3 tests
─────────────────────────────────────────
Total:                            13 tests
```

---

## 10. Checklist

Use this checklist when reviewing or creating aggregates:

### Action Enum
- [ ] Commands marked with `#[command]`
- [ ] Events marked with `#[event]`
- [ ] Only commands needing projection confirmation have `respond_to: ResponseChannel`
- [ ] `ResponseChannel` fields have `#[serde(skip)]`
- [ ] Corresponding events also have `respond_to: ResponseChannel` with `#[serde(skip)]`
- [ ] Execute* actions for two-phase patterns marked with `#[doc(hidden)]`
- [ ] Execute* actions include `current_version: Version` for optimistic concurrency
- [ ] `ValidationFailed` and `SerializationFailed` error actions present
- [ ] `VersionUpdated` infrastructure action present
- [ ] Projection confirmation/failure actions present (if using ResponseChannel)

### Environment
- [ ] All dependencies are `Arc<dyn Trait>` for trait objects
- [ ] `clock`, `event_store`, `stream_id` present
- [ ] Projection query trait(s) appropriate for this aggregate
- [ ] `global_actions` for cross-aggregate coordination
- [ ] Constructor with `#[must_use]`

### Reducer
- [ ] `new()` is `const fn` with `#[must_use]`
- [ ] `Default` implementation calls `new()`
- [ ] `create_effects()` helper for standard persistence
- [ ] `apply_event()` handles all events (clear `last_error` on success)
- [ ] Validation methods return `Result<(), String>`
- [ ] Match arms organized by category with section comments
- [ ] Two-phase pattern for commands needing async data
- [ ] Early sync validation before async load (fail fast)
- [ ] Clock captured for async blocks when timestamp needed inside

### Validation
- [ ] All commands validated before state changes
- [ ] Use `HashSet` for efficient collection lookups
- [ ] Error messages are descriptive and actionable
- [ ] Bounds checking (length limits, capacity > 0, etc.)

### Effects
- [ ] `append_events!` macro used for persistence
- [ ] `Effect::PublishWithResponse` for projection confirmation
- [ ] No unnecessary `.clone()` on consumed values
- [ ] Use `current_version` directly (no redundant assignments)

### State
- [ ] `last_error` cleared on successful events
- [ ] `version` updated via `VersionUpdated` action
- [ ] Commands and queries don't modify state in `apply_event`

### Tests (See Section 9 for detailed guidance)
- [ ] Test module has `#[allow(clippy::unwrap_used)]`
- [ ] `create_test_env()` and `create_test_env_with_projection()` helpers
- [ ] `ConfigurableMockQuery` for flexible test scenarios
- [ ] **Never test Execute* actions directly** (implementation details)
- [ ] **Sync validation** tested with `ReducerTest` (COMMAND actions)
- [ ] **Full async flows** tested with `TestStore` (command → terminal event)
- [ ] Wait for **terminal** actions (`EntityUpdated`), not intermediate (`ExecuteUpdate`)
- [ ] Query operations tested (get, list, not found cases)
- [ ] No `tokio::time::sleep` hacks in tests

### Code Quality
- [ ] No `std::collections::HashSet` (import and use `HashSet`)
- [ ] No `crate::types::Type::Variant` in tests (use imported types)
- [ ] No unused imports
- [ ] Comments explain "why", not "what"
- [ ] `#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]` with justification if needed
- [ ] `#[allow(clippy::type_complexity)]` on traits with complex future return types
- [ ] Doc comments use backticks for identifiers (clippy `doc_markdown`)
- [ ] Extract helper functions for duplicate logic across handlers
- [ ] Capture values BEFORE creating events (avoid redundant match extraction)
- [ ] Use `is_none_or` instead of `map_or(true, ...)`
- [ ] No redundant variable rebinding (`let x = ...; let mut x = x;`)
- [ ] No needless borrows (`&format!(...)` when owned String accepted)

---

## 11. Code Quality Patterns

### 11.1 Extract Helper Functions for Duplicate Logic

When multiple action handlers share similar logic, extract a helper function:

```rust
// ❌ BAD: Duplicate logic in ReleaseReservation and ExpireReservation
InventoryAction::ReleaseReservation { reservation_id } => {
    let seats = Self::find_seats_by_reservation(state, &reservation_id);
    if seats.is_empty() { return SmallVec::new(); }
    let Some((event_id, section)) = Self::find_reservation_location(state, &reservation_id) else {
        return SmallVec::new();
    };
    let event = InventoryAction::SeatsReleased { ... };
    Self::apply_event(state, &event);
    Self::create_effects(event, state.version, env)
}

InventoryAction::ExpireReservation { reservation_id } => {
    // Same logic repeated...
}

// ✅ GOOD: Extract helper function
fn handle_release_seats(
    state: &mut InventoryState,
    reservation_id: ReservationId,
    env: &InventoryEnvironment,
) -> SmallVec<[Effect<InventoryAction>; 4]> {
    let seats = Self::find_seats_by_reservation(state, &reservation_id);
    if seats.is_empty() { return SmallVec::new(); }
    // ... common logic
}

InventoryAction::ReleaseReservation { reservation_id } => {
    Self::handle_release_seats(state, reservation_id, env)
}

InventoryAction::ExpireReservation { reservation_id } => {
    // In production, might add different analytics/metrics here
    Self::handle_release_seats(state, reservation_id, env)
}
```

### 11.2 Capture Values Before Creating Events

When you need values from an event for closures, capture them BEFORE creating the event:

```rust
// ❌ BAD: Creating event then extracting values with match
let event = InventoryAction::InventoryInitialized {
    seats: seats.clone(),
    initialized_at: env.clock.now(),
    ...
};

// Redundant - we just created this event!
let seats_for_channel = match &event {
    InventoryAction::InventoryInitialized { seats, .. } => seats.clone(),
    _ => vec![],
};
let initialized_at_for_channel = match &event {
    InventoryAction::InventoryInitialized { initialized_at, .. } => *initialized_at,
    _ => env.clock.now(),
};

// ✅ GOOD: Capture values before creating the event
let initialized_at = env.clock.now();
let seats_for_channel = seats.clone();
let initialized_at_for_channel = initialized_at;

let event = InventoryAction::InventoryInitialized {
    seats,
    initialized_at,
    ...
};
```

### 11.3 Idiomatic Rust Patterns

**Use `is_none_or` instead of `map_or(true, ...)`**:

```rust
// ❌ Less idiomatic
tier.available_until.map_or(true, |until| now <= until)

// ✅ More idiomatic (Rust 1.82+)
tier.available_until.is_none_or(|until| now <= until)
```

**Avoid redundant variable rebinding**:

```rust
// ❌ BAD: Redundant rebinding
let inventory = Inventory::new(event_id, section, capacity);
let mut inventory = inventory;

// ✅ GOOD: Direct mutable binding
let mut inventory = Inventory::new(event_id, section, capacity);
```

**Avoid needless borrows**:

```rust
// ❌ BAD: Unnecessary borrow for functions taking impl AsRef<str>
let stream_id = StreamId::new(&format!("inventory-{}", event_id));

// ✅ GOOD: Pass owned String directly
let stream_id = StreamId::new(format!("inventory-{}", event_id));
```

### 11.4 Clippy Allow Attributes

For complex aggregate code, use appropriate allow attributes with justification comments:

```rust
// Trait with complex future return types (dyn-compatibility requirement)
#[allow(clippy::type_complexity)] // Complex future types required for dyn-compatibility
pub trait InventoryProjectionQuery: Send + Sync {
    fn load_inventory(...) -> Pin<Box<dyn Future<Output = Result<...>> + Send + '_>>;
}

// Complex state management functions
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // Complex state management required
fn apply_event(state: &mut InventoryState, action: &InventoryAction) {
    match action {
        // Many variants...
    }
}

// Complex business logic in reducer
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)] // Complex business logic required
fn reduce(&self, state: &mut Self::State, action: Self::Action, env: &Self::Environment)
    -> SmallVec<[Effect<Self::Action>; 4]>
{
    // Many action handlers...
}
```

### 11.5 Documentation Standards

**Use backticks for identifiers in doc comments** (clippy `doc_markdown` lint):

```rust
// ❌ Triggers clippy warning
/// Returns (counts, seat_assignments) where counts is (total_capacity, reserved, sold, available).
/// Pricing tiers have time-based availability (EarlyBird, Regular, LastMinute).

// ✅ Correct - identifiers in backticks
/// Returns (counts, `seat_assignments`) where counts is (`total_capacity`, reserved, sold, available).
/// Pricing tiers have time-based availability (`EarlyBird`, `Regular`, `LastMinute`).
```

**Product names may also need backticks** if they look like CamelCase:

```rust
// May trigger warning
/// Creates effects for persisting events (PostgreSQL only, no Redpanda)

// Safe from warning
/// Creates effects for persisting events (`PostgreSQL` only, no Redpanda)
```

---

## Quick Reference: Command Processing Flow

```
┌──────────────────────────────────────────────────────────────────────────┐
│                           SYNC COMMAND FLOW                              │
├──────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  Command ──► validate_*() ──► Create event with timestamp                │
│     │              │                │                                    │
│     │         Err? ├──► ValidationFailed (no effects)                    │
│     │              │                │                                    │
│     │         Ok   ├──► apply_event() ──► create_effects()               │
│     │                                           │                        │
│     │                                           ├──► (+ PublishWithResponse
│     │                                           │     if ResponseChannel)│
│     │                                           │                        │
│     │                                           ├──► Return effects      │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────────┐
│                          ASYNC COMMAND FLOW                              │
├──────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  Command ──► Early sync validation (fail fast)                           │
│     │              │                                                     │
│     │         Err? ├──► ValidationFailed (no effects)                    │
│     │              │                                                     │
│     │         Ok   ├──► Effect::Future                                   │
│     │                        │                                           │
│     │                        ├──► Load from projection + event store     │
│     │                        │                                           │
│     │                   Err? ├──► ValidationFailed                       │
│     │                        │                                           │
│     │                   Ok   ├──► validate_*_with_entity()               │
│     │                                    │                               │
│     │                               Err? ├──► ValidationFailed           │
│     │                                    │                               │
│     │                               Ok   ├──► ExecuteAction              │
│     │                                                                    │
│     ▼                                                                    │
│  ExecuteAction ──► apply_event() ──► create_effects() + PublishWithResponse
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
```
