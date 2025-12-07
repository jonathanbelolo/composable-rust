# WASM Browser Simulation Architecture

> **For**: Design Tool Frontend Team
>
> **Purpose**: Enable instant design iteration through browser-based domain logic simulation

---

## 1. Executive Summary

This specification describes a **browser-based simulation environment** that allows designers to test and iterate on domain logic without deploying backend infrastructure. By compiling the pure functional core to WebAssembly (WASM) and using IndexedDB as a browser-side event store, we achieve:

- **Incremental feedback**: Each aggregate compiles in 30-60 seconds
- **Parallel AI generation**: 5 aggregates compile in the same time as 1
- **Zero infrastructure**: No databases, containers, or networking required
- **Full simulation**: Run complete scenarios with event sourcing and projections

The key insight: our **Functional Core / Imperative Shell** architecture means the domain logic (pure `_process` functions) is side-effect-free and compiles identically to both PostgreSQL and WASM targets.

---

## 2. Problem Statement

### Current Pain Point

Traditional development workflow:

```
Design → Generate Code → Deploy Database → Deploy Containers → Test
                              ↓
                         2-5 minutes
                              ↓
                      Change something
                              ↓
                      Redeploy (30-60 sec)
                              ↓
                         Test again
```

This creates a slow feedback loop during the design phase when ideas need rapid iteration.

### Solution

```
Design → AI Generates → Compile WASM → Simulate in Browser
              ↓              ↓               ↓
          30-60 sec      5-10 sec        Instant
              ↓
      (parallel for multiple aggregates)
```

Once compiled, simulation is **instantaneous** - reset state, run scenarios, time-travel through events, all without server round-trips.

---

## 3. Architecture Overview

### 3.1 Dual Compilation Targets

The same domain logic compiles to two targets:

```
                    ┌─────────────────────────────────────────────┐
                    │              YAML Specification              │
                    │                                              │
                    │  contexts:                                   │
                    │    sales:                                    │
                    │      aggregates:                             │
                    │        Order:                                │
                    │          commands: [CreateOrder, AddItem]    │
                    │          events: [OrderCreated, ItemAdded]   │
                    └─────────────────────┬───────────────────────┘
                                          │
                                          ▼
                    ┌─────────────────────────────────────────────┐
                    │           AI Code Generation                 │
                    │                                              │
                    │  Generates pure _process functions           │
                    │  (IMMUTABLE, no side effects)                │
                    └─────────────────────┬───────────────────────┘
                                          │
                         ┌────────────────┴────────────────┐
                         │                                 │
                         ▼                                 ▼
          ┌──────────────────────────┐     ┌──────────────────────────┐
          │   PostgreSQL Target       │     │     WASM Target          │
          │                          │     │                          │
          │  • SQL wrapper functions │     │  • wasm-bindgen exports  │
          │  • Triggers for events   │     │  • IndexedDB storage     │
          │  • Materialized views    │     │  • JS bridge layer       │
          │                          │     │                          │
          │  ┌────────────────────┐  │     │  ┌────────────────────┐  │
          │  │  Functional Core   │  │     │  │  Functional Core   │  │
          │  │  (identical code)  │  │     │  │  (identical code)  │  │
          │  └────────────────────┘  │     │  └────────────────────┘  │
          └──────────────────────────┘     └──────────────────────────┘
                    │                                 │
                    ▼                                 ▼
              Production                      Design Simulation
```

### 3.2 What Compiles to WASM

**Included** (Pure Functional Core):
- `_process` functions (command handlers)
- `_apply` functions (event application)
- `_validate` functions (business rules)
- State reconstruction logic
- Domain types and enums

**Excluded** (Imperative Shell - replaced by browser equivalents):
- Database connections
- Network calls
- File I/O
- System clocks (replaced with controllable simulation clock)

### 3.3 Relationship to Production Architecture

This simulation architecture is designed to mirror the production PostgreSQL architecture described in the companion specifications:

| Aspect | Production (PostgreSQL) | Simulation (WASM/IndexedDB) |
|--------|------------------------|----------------------------|
| **Functional Core** | PL/pgSQL `_process` functions | Rust WASM `_process` functions |
| **Event Store** | `ctx_events.event_log` table (Two-Layer Architecture) | IndexedDB `events` object store |
| **Projections** | Typed context tables + materialized views | IndexedDB `projections` object store |
| **Imperative Shell** | pg-gateway (Rust thin shell) | JavaScript bridge + Dexie.js |
| **Cross-aggregate events** | PostgreSQL LISTEN/NOTIFY or Redpanda | SimulationCoordinator in-browser |

> **See Also**:
> - `specs/monolith_postgres/bounded_contexts.md` - Two-layer event architecture (global JSONB + typed context tables)
> - `specs/monolith_postgres/rust_server_layer.md` - pg-gateway thin shell pattern
> - `specs/monolith_postgres/pg_gateway_integration.md` - Protocol translation and identity propagation

The key insight is that the **Functional Core is identical** - only the Imperative Shell differs. This allows:
1. Verified simulation behavior to translate directly to production
2. Business logic tested in browser works the same in PostgreSQL
3. Event schemas designed in simulation are production-ready

### 3.4 Browser Environment Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Browser Environment                                │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌────────────────────────┐     ┌────────────────────────────────────────┐  │
│  │      Design UI          │     │           WASM Modules                 │  │
│  │      (Svelte)           │     │                                        │  │
│  │                         │     │  ┌──────────┐ ┌──────────┐ ┌────────┐  │  │
│  │  • Aggregate Designer   │◄───►│  │  Order   │ │ Payment  │ │  ...   │  │  │
│  │  • Event Log Viewer     │     │  │  .wasm   │ │  .wasm   │ │        │  │  │
│  │  • Projection Inspector │     │  └──────────┘ └──────────┘ └────────┘  │  │
│  │  • Scenario Runner      │     │                                        │  │
│  │  • Time Travel Debug    │     │  ┌──────────────────────────────────┐  │  │
│  │                         │     │  │     Simulation Coordinator       │  │  │
│  └────────────────────────┘     │  │     (orchestrates aggregates)    │  │  │
│            │                     │  └──────────────────────────────────┘  │  │
│            │                     └────────────────────┬───────────────────┘  │
│            │                                          │                      │
│            │              ┌───────────────────────────┘                      │
│            │              │                                                  │
│            ▼              ▼                                                  │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                        JavaScript Bridge                             │    │
│  │                                                                      │    │
│  │  • WASM ↔ JS type conversion (serde-wasm-bindgen)                   │    │
│  │  • IndexedDB wrapper (Dexie.js)                                     │    │
│  │  • Simulation clock control                                          │    │
│  │  • Event broadcasting between aggregates                            │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│            │                                                                 │
│            ▼                                                                 │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                          IndexedDB                                   │    │
│  │                                                                      │    │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌────────────┐  │    │
│  │  │ event_log   │  │ projections │  │   streams   │  │ sim_runs   │  │    │
│  │  │             │  │             │  │             │  │            │  │    │
│  │  │ stream_id   │  │ key         │  │ stream_id   │  │ id         │  │    │
│  │  │ version     │  │ context     │  │ version     │  │ name       │  │    │
│  │  │ event_type  │  │ data        │  │ aggregate   │  │ created_at │  │    │
│  │  │ payload     │  │ updated_at  │  │             │  │ events_ct  │  │    │
│  │  │ created_at  │  │             │  │             │  │            │  │    │
│  │  └─────────────┘  └─────────────┘  └─────────────┘  └────────────┘  │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. Incremental Compilation Workflow

### 4.1 The Core Insight

The multi-crate architecture means each aggregate is an independent compilation unit. You don't wait for the entire system - you get feedback as each piece completes.

### 4.2 Single Aggregate Flow

```
┌──────────────────────────────────────────────────────────────────────────┐
│                    Single Aggregate Compilation                          │
└──────────────────────────────────────────────────────────────────────────┘

   User Action              Server                         Browser
   ───────────              ──────                         ───────

   Design "Order"     ───►  Receive YAML fragment
   aggregate in UI          │
                            ▼
                      AI generates _process logic
                      (30-60 seconds)
                            │
                            ▼
                      Compile Rust → WASM
                      (5-10 seconds)
                            │
                            ▼
                      Return artifacts:         ───►  Load WASM module
                      • order.wasm                    │
                      • order_bridge.js               ▼
                      • order_schema.json             Initialize simulator
                                                      │
                                                      ▼
                                                Ready to simulate!
                                                      │
                                                      ▼
   Run scenarios      ◄───────────────────────────────┘
   (instant feedback)
```

**Total time**: ~35-70 seconds from design to simulation.

### 4.3 Parallel Multi-Aggregate Flow

```
┌──────────────────────────────────────────────────────────────────────────┐
│                    Parallel Aggregate Compilation                        │
└──────────────────────────────────────────────────────────────────────────┘

   User designs 5 aggregates, then clicks "Compile All"

   Server receives full YAML:

   ┌─────────────────────────────────────────────────────────────────────┐
   │                      Parallel AI Generation                         │
   │                                                                     │
   │    Order ──────► Claude API ──┐                                     │
   │    Payment ────► Claude API ──┼──► All complete in 30-60 sec       │
   │    Inventory ──► Claude API ──┤    (slowest call determines time)  │
   │    Shipping ───► Claude API ──┤                                     │
   │    Customer ───► Claude API ──┘                                     │
   │                                                                     │
   └─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
   ┌─────────────────────────────────────────────────────────────────────┐
   │                     Parallel Rust Compilation                       │
   │                                                                     │
   │    cargo build -p order --target wasm32    ──┐                      │
   │    cargo build -p payment --target wasm32  ──┼──► 10-15 sec total  │
   │    cargo build -p inventory --target wasm32 ─┤                      │
   │    cargo build -p shipping --target wasm32 ──┤                      │
   │    cargo build -p customer --target wasm32 ──┘                      │
   │                                                                     │
   └─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
   ┌─────────────────────────────────────────────────────────────────────┐
   │                        Artifact Bundle                              │
   │                                                                     │
   │    simulation_bundle.tar.gz containing:                             │
   │    ├── wasm/                                                        │
   │    │   ├── order.wasm                                               │
   │    │   ├── payment.wasm                                             │
   │    │   ├── inventory.wasm                                           │
   │    │   ├── shipping.wasm                                            │
   │    │   └── customer.wasm                                            │
   │    ├── js/                                                          │
   │    │   ├── coordinator.js      (cross-aggregate orchestration)      │
   │    │   └── bridge.js           (IndexedDB wrappers)                 │
   │    └── schema.json             (for UI: types, events, commands)    │
   │                                                                     │
   └─────────────────────────────────────────────────────────────────────┘

   Total time: ~45-75 seconds for complete bounded context
```

### 4.4 Incremental Addition Flow

```
┌──────────────────────────────────────────────────────────────────────────┐
│                    Incremental Design Session                            │
└──────────────────────────────────────────────────────────────────────────┘

   Timeline:

   0:00  ─── Design Order aggregate
   0:05  ─── Click "Compile" ────────────► AI + Compile (35-70 sec)

   0:05  ─── While waiting, design Payment aggregate

   1:00  ─── Order ready! ◄────────────── order.wasm arrives
         ─── Start simulating Order scenarios
         ─── Click "Compile" for Payment ─► AI + Compile (35-70 sec)

   1:00  ─── While waiting, design Inventory aggregate
         ─── Continue testing Order scenarios

   2:00  ─── Payment ready! ◄─────────── payment.wasm arrives
         ─── Now simulate Order + Payment together
         ─── Click "Compile" for Inventory

   ...and so on...

   Result: Continuous productive work with zero idle time
```

---

## 5. AI Code Generation Pipeline

### 5.1 What AI Generates

For each aggregate, the AI (Claude) generates:

```rust
// Generated: sales/order/_process.rs

/// Process a command against current state.
/// IMMUTABLE - no side effects, pure function.
pub fn order_process(
    state: &OrderState,
    command: OrderCommand,
) -> Result<Vec<OrderEvent>, OrderError> {
    match command {
        OrderCommand::Create { customer_id, items } => {
            // Validation
            if items.is_empty() {
                return Err(OrderError::EmptyOrder);
            }

            // Business logic
            let total = items.iter().map(|i| i.price * i.quantity).sum();

            Ok(vec![OrderEvent::Created {
                order_id: state.order_id.clone(),
                customer_id,
                items,
                total,
                created_at: state.current_time,
            }])
        }

        OrderCommand::AddItem { item } => {
            // Can only add items to draft orders
            if state.status != OrderStatus::Draft {
                return Err(OrderError::NotDraft);
            }

            Ok(vec![OrderEvent::ItemAdded {
                order_id: state.order_id.clone(),
                item,
                new_total: state.total + (item.price * item.quantity),
            }])
        }

        // ... more commands
    }
}

/// Apply an event to state.
/// IMMUTABLE - returns new state.
pub fn order_apply(state: &OrderState, event: &OrderEvent) -> OrderState {
    let mut new_state = state.clone();

    match event {
        OrderEvent::Created { items, total, .. } => {
            new_state.items = items.clone();
            new_state.total = *total;
            new_state.status = OrderStatus::Draft;
        }

        OrderEvent::ItemAdded { item, new_total, .. } => {
            new_state.items.push(item.clone());
            new_state.total = *new_total;
        }

        // ... more events
    }

    new_state
}
```

### 5.2 Generation Prompt Structure

```yaml
# Sent to Claude API for each aggregate

system: |
  You are generating Rust code for a domain aggregate following the
  Functional Core / Imperative Shell pattern.

  Requirements:
  - All _process functions are IMMUTABLE and pure
  - No side effects (no I/O, no randomness, no system calls)
  - Return Result<Vec<Event>, Error> for commands
  - Return new State for event application
  - Include comprehensive validation
  - Follow Rust 2024 idioms

user: |
  Generate the domain logic for this aggregate:

  Context: sales
  Aggregate: Order

  Commands:
    - CreateOrder(customer_id, items)
    - AddItem(item)
    - RemoveItem(item_id)
    - Submit()
    - Cancel(reason)

  Events:
    - OrderCreated(customer_id, items, total)
    - ItemAdded(item, new_total)
    - ItemRemoved(item_id, new_total)
    - OrderSubmitted(submitted_at)
    - OrderCancelled(reason, cancelled_at)

  Business Rules:
    - Orders must have at least one item
    - Items can only be added/removed in Draft status
    - Submitted orders cannot be modified
    - Cancelled orders cannot be reactivated
```

### 5.3 Parallel API Calls

```typescript
// Compilation server: parallel AI generation

async function generateAllAggregates(spec: YamlSpec): Promise<GeneratedCode[]> {
  const aggregates = extractAggregates(spec);

  // All AI calls in parallel
  const generationPromises = aggregates.map(aggregate =>
    generateAggregate(aggregate)  // Each calls Claude API
  );

  // Wait for all to complete
  // Total time ≈ slowest single call (30-60 sec)
  const results = await Promise.all(generationPromises);

  return results;
}

async function generateAggregate(aggregate: AggregateSpec): Promise<GeneratedCode> {
  const response = await anthropic.messages.create({
    model: 'claude-sonnet-4-20250514',
    max_tokens: 4096,
    system: GENERATION_SYSTEM_PROMPT,
    messages: [{
      role: 'user',
      content: formatAggregatePrompt(aggregate)
    }]
  });

  return parseGeneratedCode(response);
}
```

---

## 6. IndexedDB Storage Layer

### 6.1 Database Schema

```typescript
// lib/simulation-db.ts

import Dexie, { type Table } from 'dexie';

/**
 * Event stored in the browser-side event log.
 * Mirrors the PostgreSQL event_log structure.
 */
interface StoredEvent {
  id?: number;              // Auto-increment (IndexedDB)
  stream_id: string;        // Aggregate instance ID
  version: number;          // Optimistic concurrency
  event_type: string;       // Fully qualified: "sales.OrderCreated"
  payload: unknown;         // Event data (JSON-serializable)
  metadata: {
    correlation_id?: string;
    causation_id?: string;
    user_id?: string;
    timestamp: string;      // ISO 8601
  };
  created_at: Date;
}

/**
 * Projection state (read models).
 * Key format: "{context}.{projection}:{id}"
 * Example: "sales.order_summary:order-123"
 */
interface Projection {
  key: string;              // Primary key
  context: string;          // Bounded context name
  projection_type: string;  // Projection name
  entity_id: string;        // Entity identifier
  data: unknown;            // Projection state
  version: number;          // For optimistic updates
  updated_at: Date;
}

/**
 * Stream metadata for version tracking.
 */
interface Stream {
  stream_id: string;        // Primary key
  aggregate_type: string;   // "sales.Order"
  current_version: number;  // Latest event version
  created_at: Date;
  updated_at: Date;
}

/**
 * Simulation run for organizing test scenarios.
 */
interface SimulationRun {
  id: string;               // UUID
  name: string;             // User-provided name
  description?: string;
  context: string;          // Primary bounded context
  created_at: Date;
  completed_at?: Date;
  event_count: number;
  status: 'running' | 'completed' | 'failed';
}

/**
 * Simulation database using Dexie.js wrapper.
 */
class SimulationDatabase extends Dexie {
  events!: Table<StoredEvent>;
  projections!: Table<Projection>;
  streams!: Table<Stream>;
  runs!: Table<SimulationRun>;

  constructor() {
    super('ddd-simulation');

    this.version(1).stores({
      // Indexes for efficient queries
      events: '++id, stream_id, [stream_id+version], event_type, created_at, [metadata.correlation_id]',
      projections: 'key, context, projection_type, entity_id, updated_at',
      streams: 'stream_id, aggregate_type, updated_at',
      runs: 'id, context, created_at, status'
    });
  }
}

export const db = new SimulationDatabase();
```

### 6.2 Event Store Operations

```typescript
// lib/simulation-event-store.ts

import { db, type StoredEvent, type Stream } from './simulation-db';

export class SimulationEventStore {
  /**
   * Append events to a stream with optimistic concurrency.
   * Mirrors PostgreSQL append_events() behavior.
   */
  async appendEvents(
    streamId: string,
    expectedVersion: number,
    events: Array<{ type: string; payload: unknown }>
  ): Promise<number> {
    return await db.transaction('rw', [db.events, db.streams], async () => {
      // Check current version
      const stream = await db.streams.get(streamId);
      const currentVersion = stream?.current_version ?? 0;

      if (currentVersion !== expectedVersion) {
        throw new ConcurrencyError(
          `Expected version ${expectedVersion}, found ${currentVersion}`
        );
      }

      // Append events
      const now = new Date();
      let version = currentVersion;

      for (const event of events) {
        version++;
        await db.events.add({
          stream_id: streamId,
          version,
          event_type: event.type,
          payload: event.payload,
          metadata: {
            timestamp: now.toISOString()
          },
          created_at: now
        });
      }

      // Update stream version
      await db.streams.put({
        stream_id: streamId,
        aggregate_type: extractAggregateType(streamId),
        current_version: version,
        created_at: stream?.created_at ?? now,
        updated_at: now
      });

      return version;
    });
  }

  /**
   * Load all events for a stream.
   * Used for state reconstruction.
   */
  async loadEvents(streamId: string): Promise<StoredEvent[]> {
    return await db.events
      .where('stream_id')
      .equals(streamId)
      .sortBy('version');
  }

  /**
   * Load events by type across all streams.
   * Used for projections and debugging.
   */
  async loadEventsByType(eventType: string): Promise<StoredEvent[]> {
    return await db.events
      .where('event_type')
      .equals(eventType)
      .toArray();
  }

  /**
   * Get current version for optimistic concurrency.
   */
  async getVersion(streamId: string): Promise<number> {
    const stream = await db.streams.get(streamId);
    return stream?.current_version ?? 0;
  }

  /**
   * Clear all events (for simulation reset).
   */
  async clear(): Promise<void> {
    await db.transaction('rw', [db.events, db.streams], async () => {
      await db.events.clear();
      await db.streams.clear();
    });
  }
}

export class ConcurrencyError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'ConcurrencyError';
  }
}
```

### 6.3 Projection Store Operations

```typescript
// lib/simulation-projection-store.ts

import { db, type Projection } from './simulation-db';

export class SimulationProjectionStore {
  /**
   * Get a projection by key.
   */
  async get<T>(key: string): Promise<T | undefined> {
    const projection = await db.projections.get(key);
    return projection?.data as T | undefined;
  }

  /**
   * Set a projection (upsert).
   */
  async set<T>(
    context: string,
    projectionType: string,
    entityId: string,
    data: T
  ): Promise<void> {
    const key = `${context}.${projectionType}:${entityId}`;
    const existing = await db.projections.get(key);

    await db.projections.put({
      key,
      context,
      projection_type: projectionType,
      entity_id: entityId,
      data,
      version: (existing?.version ?? 0) + 1,
      updated_at: new Date()
    });
  }

  /**
   * Query projections by type.
   */
  async queryByType<T>(
    context: string,
    projectionType: string
  ): Promise<Array<{ id: string; data: T }>> {
    const projections = await db.projections
      .where('projection_type')
      .equals(projectionType)
      .and(p => p.context === context)
      .toArray();

    return projections.map(p => ({
      id: p.entity_id,
      data: p.data as T
    }));
  }

  /**
   * Clear all projections (for simulation reset).
   */
  async clear(): Promise<void> {
    await db.projections.clear();
  }
}
```

---

## 7. WASM Module Structure

### 7.1 Rust WASM Crate

```rust
// wasm-simulator/src/lib.rs

use wasm_bindgen::prelude::*;
use serde::{Deserialize, Serialize};

mod storage;
mod aggregates;

use storage::{WasmEventStore, WasmProjectionStore};

/// Main simulator entry point exposed to JavaScript.
#[wasm_bindgen]
pub struct Simulator {
    event_store: WasmEventStore,
    projection_store: WasmProjectionStore,
    clock: SimulationClock,
}

#[wasm_bindgen]
impl Simulator {
    /// Create a new simulator instance.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        // Set panic hook for better error messages
        console_error_panic_hook::set_once();

        Self {
            event_store: WasmEventStore::new(),
            projection_store: WasmProjectionStore::new(),
            clock: SimulationClock::new(),
        }
    }

    /// Execute a command against an aggregate.
    ///
    /// Returns the resulting events (or error).
    #[wasm_bindgen]
    pub async fn execute_command(
        &self,
        context: &str,
        aggregate_type: &str,
        aggregate_id: &str,
        command: JsValue,
    ) -> Result<JsValue, JsValue> {
        // 1. Load current events for this aggregate
        let stream_id = format!("{}.{}:{}", context, aggregate_type, aggregate_id);
        let events = self.event_store.load_events(&stream_id).await?;
        let current_version = events.len() as i64;

        // 2. Reconstruct state from events (pure function)
        let state = self.reconstruct_state(context, aggregate_type, &events)?;

        // 3. Process command (pure function - the functional core)
        let command_data: serde_json::Value = serde_wasm_bindgen::from_value(command)?;
        let result = self.process_command(
            context,
            aggregate_type,
            &state,
            &command_data,
            self.clock.now(),
        )?;

        // 4. Append new events (imperative shell - IndexedDB)
        if !result.events.is_empty() {
            self.event_store
                .append_events(&stream_id, current_version, &result.events)
                .await?;
        }

        // 5. Update projections (imperative shell - IndexedDB)
        for update in &result.projection_updates {
            self.projection_store
                .set(&update.key, &update.data)
                .await?;
        }

        // 6. Return result to JavaScript
        Ok(serde_wasm_bindgen::to_value(&result)?)
    }

    /// Get current state of an aggregate (for UI display).
    #[wasm_bindgen]
    pub async fn get_state(
        &self,
        context: &str,
        aggregate_type: &str,
        aggregate_id: &str,
    ) -> Result<JsValue, JsValue> {
        let stream_id = format!("{}.{}:{}", context, aggregate_type, aggregate_id);
        let events = self.event_store.load_events(&stream_id).await?;
        let state = self.reconstruct_state(context, aggregate_type, &events)?;
        Ok(serde_wasm_bindgen::to_value(&state)?)
    }

    /// Get a projection (for UI display).
    #[wasm_bindgen]
    pub async fn get_projection(&self, key: &str) -> Result<JsValue, JsValue> {
        let data = self.projection_store.get(key).await?;
        Ok(serde_wasm_bindgen::to_value(&data)?)
    }

    /// Get all events for a stream (for event log viewer).
    #[wasm_bindgen]
    pub async fn get_events(&self, stream_id: &str) -> Result<JsValue, JsValue> {
        let events = self.event_store.load_events(stream_id).await?;
        Ok(serde_wasm_bindgen::to_value(&events)?)
    }

    /// Advance simulation clock.
    #[wasm_bindgen]
    pub fn advance_time(&mut self, seconds: i64) {
        self.clock.advance(seconds);
    }

    /// Set simulation clock to specific time.
    #[wasm_bindgen]
    pub fn set_time(&mut self, iso_timestamp: &str) {
        self.clock.set(iso_timestamp);
    }

    /// Reset all state (clear events and projections).
    #[wasm_bindgen]
    pub async fn reset(&self) -> Result<(), JsValue> {
        self.event_store.clear().await?;
        self.projection_store.clear().await?;
        Ok(())
    }

    // Private methods that dispatch to generated code

    fn reconstruct_state(
        &self,
        context: &str,
        aggregate_type: &str,
        events: &[serde_json::Value],
    ) -> Result<serde_json::Value, JsValue> {
        // Dispatch to generated aggregate module
        aggregates::reconstruct(context, aggregate_type, events)
    }

    fn process_command(
        &self,
        context: &str,
        aggregate_type: &str,
        state: &serde_json::Value,
        command: &serde_json::Value,
        current_time: &str,
    ) -> Result<CommandResult, JsValue> {
        // Dispatch to generated aggregate module
        aggregates::process(context, aggregate_type, state, command, current_time)
    }
}

/// Result of processing a command.
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandResult {
    pub events: Vec<serde_json::Value>,
    pub projection_updates: Vec<ProjectionUpdate>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectionUpdate {
    pub key: String,
    pub data: serde_json::Value,
}

/// Controllable clock for simulation.
struct SimulationClock {
    current: chrono::DateTime<chrono::Utc>,
}

impl SimulationClock {
    fn new() -> Self {
        Self {
            current: chrono::Utc::now(),
        }
    }

    fn now(&self) -> String {
        self.current.to_rfc3339()
    }

    fn advance(&mut self, seconds: i64) {
        self.current = self.current + chrono::Duration::seconds(seconds);
    }

    fn set(&mut self, iso_timestamp: &str) {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso_timestamp) {
            self.current = dt.with_timezone(&chrono::Utc);
        }
    }
}
```

### 7.2 Storage Bridge (WASM ↔ IndexedDB)

```rust
// wasm-simulator/src/storage.rs

use wasm_bindgen::prelude::*;
use serde_json::Value;

#[wasm_bindgen]
extern "C" {
    // Event store operations (implemented in JavaScript)
    #[wasm_bindgen(js_namespace = ["window", "simulationBridge"], catch)]
    async fn append_events(
        stream_id: &str,
        expected_version: i64,
        events: JsValue,
    ) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "simulationBridge"], catch)]
    async fn load_events(stream_id: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "simulationBridge"], catch)]
    async fn clear_events() -> Result<(), JsValue>;

    // Projection store operations
    #[wasm_bindgen(js_namespace = ["window", "simulationBridge"], catch)]
    async fn get_projection(key: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "simulationBridge"], catch)]
    async fn set_projection(key: &str, data: JsValue) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "simulationBridge"], catch)]
    async fn clear_projections() -> Result<(), JsValue>;
}

pub struct WasmEventStore;

impl WasmEventStore {
    pub fn new() -> Self {
        Self
    }

    pub async fn append_events(
        &self,
        stream_id: &str,
        expected_version: i64,
        events: &[Value],
    ) -> Result<(), JsValue> {
        let events_js = serde_wasm_bindgen::to_value(events)?;
        append_events(stream_id, expected_version, events_js).await?;
        Ok(())
    }

    pub async fn load_events(&self, stream_id: &str) -> Result<Vec<Value>, JsValue> {
        let events_js = load_events(stream_id).await?;
        let events: Vec<Value> = serde_wasm_bindgen::from_value(events_js)?;
        Ok(events)
    }

    pub async fn clear(&self) -> Result<(), JsValue> {
        clear_events().await
    }
}

pub struct WasmProjectionStore;

impl WasmProjectionStore {
    pub fn new() -> Self {
        Self
    }

    pub async fn get(&self, key: &str) -> Result<Option<Value>, JsValue> {
        let data_js = get_projection(key).await?;
        if data_js.is_undefined() || data_js.is_null() {
            return Ok(None);
        }
        let data: Value = serde_wasm_bindgen::from_value(data_js)?;
        Ok(Some(data))
    }

    pub async fn set(&self, key: &str, data: &Value) -> Result<(), JsValue> {
        let data_js = serde_wasm_bindgen::to_value(data)?;
        set_projection(key, data_js).await
    }

    pub async fn clear(&self) -> Result<(), JsValue> {
        clear_projections().await
    }
}
```

### 7.3 JavaScript Bridge Implementation

```typescript
// lib/simulation-bridge.ts

import { SimulationEventStore, ConcurrencyError } from './simulation-event-store';
import { SimulationProjectionStore } from './simulation-projection-store';

const eventStore = new SimulationEventStore();
const projectionStore = new SimulationProjectionStore();

/**
 * Bridge exposed to WASM module.
 * This is the imperative shell running in the browser.
 */
(window as any).simulationBridge = {
  /**
   * Append events to a stream with optimistic concurrency.
   */
  async append_events(
    streamId: string,
    expectedVersion: number,
    events: Array<{ type: string; payload: unknown }>
  ): Promise<number> {
    try {
      return await eventStore.appendEvents(streamId, expectedVersion, events);
    } catch (error) {
      if (error instanceof ConcurrencyError) {
        throw new Error(`CONCURRENCY_ERROR: ${error.message}`);
      }
      throw error;
    }
  },

  /**
   * Load all events for a stream.
   */
  async load_events(streamId: string): Promise<unknown[]> {
    const events = await eventStore.loadEvents(streamId);
    return events.map(e => ({
      version: e.version,
      event_type: e.event_type,
      payload: e.payload,
      metadata: e.metadata,
      created_at: e.created_at.toISOString()
    }));
  },

  /**
   * Clear all events.
   */
  async clear_events(): Promise<void> {
    await eventStore.clear();
  },

  /**
   * Get a projection.
   */
  async get_projection(key: string): Promise<unknown> {
    return await projectionStore.get(key);
  },

  /**
   * Set a projection.
   */
  async set_projection(key: string, data: unknown): Promise<void> {
    const [contextProjection, entityId] = key.split(':');
    const [context, projectionType] = contextProjection.split('.');
    await projectionStore.set(context, projectionType, entityId, data);
  },

  /**
   * Clear all projections.
   */
  async clear_projections(): Promise<void> {
    await projectionStore.clear();
  }
};
```

---

## 8. Compilation Server

### 8.1 API Endpoints

```
POST /api/compile
  Request: YAML specification
  Response: Compiled artifact bundle (tar.gz)

POST /api/compile/aggregate
  Request: Single aggregate YAML + context info
  Response: Single WASM module + bridge code

GET /api/compile/status/{job_id}
  Response: Compilation status and progress

GET /api/artifacts/{hash}
  Response: Cached artifact bundle (if exists)
```

### 8.2 Compilation Flow

```typescript
// server/compilation-service.ts

interface CompilationJob {
  id: string;
  yamlHash: string;
  status: 'pending' | 'generating' | 'compiling' | 'complete' | 'failed';
  progress: number;
  artifacts?: ArtifactBundle;
  error?: string;
}

interface ArtifactBundle {
  hash: string;
  wasmModules: Array<{
    name: string;
    bytes: Uint8Array;
    size: number;
  }>;
  bridgeCode: string;
  schema: SchemaInfo;
  createdAt: Date;
}

class CompilationService {
  private jobs = new Map<string, CompilationJob>();
  private cache = new Map<string, ArtifactBundle>();

  async compile(yaml: string): Promise<string> {
    const yamlHash = await sha256(yaml);

    // Check cache first
    if (this.cache.has(yamlHash)) {
      return yamlHash; // Return immediately
    }

    // Create job
    const jobId = crypto.randomUUID();
    const job: CompilationJob = {
      id: jobId,
      yamlHash,
      status: 'pending',
      progress: 0
    };
    this.jobs.set(jobId, job);

    // Start async compilation
    this.runCompilation(job, yaml);

    return jobId;
  }

  private async runCompilation(job: CompilationJob, yaml: string): Promise<void> {
    try {
      const spec = parseYaml(yaml);
      const aggregates = extractAggregates(spec);

      // Phase 1: AI Generation (parallel)
      job.status = 'generating';
      const generatedCode = await this.generateAllAggregates(aggregates, (progress) => {
        job.progress = progress * 0.6; // 0-60%
      });

      // Phase 2: Rust Compilation (parallel)
      job.status = 'compiling';
      const wasmModules = await this.compileAllModules(generatedCode, (progress) => {
        job.progress = 60 + progress * 0.35; // 60-95%
      });

      // Phase 3: Bundle creation
      const bundle = await this.createBundle(wasmModules, spec);
      job.progress = 100;
      job.status = 'complete';
      job.artifacts = bundle;

      // Cache by hash
      this.cache.set(job.yamlHash, bundle);

    } catch (error) {
      job.status = 'failed';
      job.error = error.message;
    }
  }

  private async generateAllAggregates(
    aggregates: AggregateSpec[],
    onProgress: (progress: number) => void
  ): Promise<GeneratedCode[]> {
    let completed = 0;

    const results = await Promise.all(
      aggregates.map(async (aggregate) => {
        const code = await this.generateAggregate(aggregate);
        completed++;
        onProgress(completed / aggregates.length);
        return code;
      })
    );

    return results;
  }

  private async compileAllModules(
    generatedCode: GeneratedCode[],
    onProgress: (progress: number) => void
  ): Promise<WasmModule[]> {
    // Write generated code to temp workspace
    const workspace = await this.createTempWorkspace(generatedCode);

    // Compile all crates in parallel
    const results = await Promise.all(
      generatedCode.map(async (code, index) => {
        const module = await this.compileModule(workspace, code.crateName);
        onProgress((index + 1) / generatedCode.length);
        return module;
      })
    );

    // Cleanup
    await this.cleanupWorkspace(workspace);

    return results;
  }

  private async compileModule(workspace: string, crateName: string): Promise<WasmModule> {
    // Run cargo build for WASM target
    await exec(`cargo build -p ${crateName} --target wasm32-unknown-unknown --release`, {
      cwd: workspace
    });

    // Run wasm-bindgen
    await exec(`wasm-bindgen target/wasm32-unknown-unknown/release/${crateName}.wasm --out-dir dist --target web`, {
      cwd: workspace
    });

    // Optimize with wasm-opt (optional)
    await exec(`wasm-opt -O3 dist/${crateName}_bg.wasm -o dist/${crateName}_bg.wasm`, {
      cwd: workspace
    });

    // Read compiled WASM
    const wasmBytes = await fs.readFile(`${workspace}/dist/${crateName}_bg.wasm`);

    return {
      name: crateName,
      bytes: new Uint8Array(wasmBytes),
      size: wasmBytes.length
    };
  }
}
```

### 8.3 Caching Strategy

```typescript
// server/artifact-cache.ts

interface CacheEntry {
  hash: string;
  bundle: ArtifactBundle;
  accessCount: number;
  lastAccessed: Date;
  createdAt: Date;
}

class ArtifactCache {
  private memoryCache = new Map<string, CacheEntry>();
  private diskCachePath: string;
  private maxMemoryEntries = 100;
  private maxDiskSize = 10 * 1024 * 1024 * 1024; // 10 GB

  async get(hash: string): Promise<ArtifactBundle | null> {
    // Check memory first
    const memEntry = this.memoryCache.get(hash);
    if (memEntry) {
      memEntry.accessCount++;
      memEntry.lastAccessed = new Date();
      return memEntry.bundle;
    }

    // Check disk
    const diskPath = `${this.diskCachePath}/${hash}.tar.gz`;
    if (await fs.exists(diskPath)) {
      const bundle = await this.loadFromDisk(diskPath);

      // Promote to memory cache
      this.addToMemory(hash, bundle);

      return bundle;
    }

    return null;
  }

  async set(hash: string, bundle: ArtifactBundle): Promise<void> {
    // Always save to disk
    await this.saveToDisk(hash, bundle);

    // Add to memory cache
    this.addToMemory(hash, bundle);
  }

  private addToMemory(hash: string, bundle: ArtifactBundle): void {
    // Evict if necessary (LRU)
    if (this.memoryCache.size >= this.maxMemoryEntries) {
      const lruKey = this.findLRU();
      this.memoryCache.delete(lruKey);
    }

    this.memoryCache.set(hash, {
      hash,
      bundle,
      accessCount: 1,
      lastAccessed: new Date(),
      createdAt: new Date()
    });
  }
}
```

---

## 9. Frontend Integration

> **Note**: This section uses [Composable Svelte](https://github.com/composable-svelte/composable-svelte), a TCA-inspired architecture library for Svelte 5 with pure reducers, declarative effects, and predictable state management.

### 9.1 Simulation Feature: State & Actions

```typescript
// lib/features/simulation/types.ts

import type { Simulator } from '$lib/wasm/simulation';

/**
 * Simulation panel state for a single aggregate.
 */
export interface SimulationState {
  // Identity
  context: string;
  aggregateType: string;
  aggregateId: string;

  // WASM simulator reference
  simulator: Simulator | null;

  // Data
  events: StoredEvent[];
  aggregateState: unknown | null;

  // UI state
  status: 'idle' | 'loading' | 'executing' | 'error';
  error: string | null;
  selectedEventIndex: number | null;

  // Time control
  simulationTime: string; // ISO 8601
}

/**
 * All actions for the simulation panel.
 */
export type SimulationAction =
  // Lifecycle
  | { type: 'mounted' }
  | { type: 'unmounted' }

  // Data loading
  | { type: 'loadStateRequested' }
  | { type: 'stateLoaded'; aggregateState: unknown; events: StoredEvent[] }
  | { type: 'loadFailed'; error: string }

  // Command execution
  | { type: 'commandSubmitted'; command: unknown }
  | { type: 'commandSucceeded'; result: CommandResult }
  | { type: 'commandFailed'; error: string }

  // Reset
  | { type: 'resetTapped' }
  | { type: 'resetCompleted' }

  // Event selection (for detail view)
  | { type: 'eventSelected'; index: number }
  | { type: 'eventDeselected' }

  // Time control
  | { type: 'timeAdvanced'; seconds: number }
  | { type: 'timeSet'; isoTimestamp: string };

export interface StoredEvent {
  version: number;
  event_type: string;
  payload: unknown;
  metadata: {
    timestamp: string;
    correlation_id?: string;
  };
}

export interface CommandResult {
  events: unknown[];
  projection_updates: unknown[];
}
```

### 9.2 Simulation Feature: Reducer

```typescript
// lib/features/simulation/reducer.ts

import { Effect, type Reducer } from '@composable-svelte/core';
import type { SimulationState, SimulationAction, StoredEvent } from './types';

export const initialSimulationState = (
  context: string,
  aggregateType: string,
  aggregateId: string
): SimulationState => ({
  context,
  aggregateType,
  aggregateId,
  simulator: null,
  events: [],
  aggregateState: null,
  status: 'idle',
  error: null,
  selectedEventIndex: null,
  simulationTime: new Date().toISOString(),
});

export const simulationReducer: Reducer<SimulationState, SimulationAction> = (
  state,
  action
) => {
  switch (action.type) {
    // ─────────────────────────────────────────────────────────────────
    // Lifecycle
    // ─────────────────────────────────────────────────────────────────
    case 'mounted':
      return [
        { ...state, status: 'loading' },
        Effect.run(async (dispatch) => {
          dispatch({ type: 'loadStateRequested' });
        }),
      ];

    case 'unmounted':
      return [state, Effect.none()];

    // ─────────────────────────────────────────────────────────────────
    // Data Loading
    // ─────────────────────────────────────────────────────────────────
    case 'loadStateRequested': {
      if (!state.simulator) {
        return [state, Effect.none()];
      }

      const { simulator, context, aggregateType, aggregateId } = state;
      const streamId = `${context}.${aggregateType}:${aggregateId}`;

      return [
        { ...state, status: 'loading', error: null },
        Effect.run(async (dispatch) => {
          try {
            const [aggregateState, events] = await Promise.all([
              simulator.get_state(context, aggregateType, aggregateId),
              simulator.get_events(streamId),
            ]);

            dispatch({
              type: 'stateLoaded',
              aggregateState,
              events: events as StoredEvent[],
            });
          } catch (e) {
            dispatch({ type: 'loadFailed', error: (e as Error).message });
          }
        }),
      ];
    }

    case 'stateLoaded':
      return [
        {
          ...state,
          status: 'idle',
          aggregateState: action.aggregateState,
          events: action.events,
          error: null,
        },
        Effect.none(),
      ];

    case 'loadFailed':
      return [
        { ...state, status: 'error', error: action.error },
        Effect.none(),
      ];

    // ─────────────────────────────────────────────────────────────────
    // Command Execution
    // ─────────────────────────────────────────────────────────────────
    case 'commandSubmitted': {
      if (!state.simulator) {
        return [state, Effect.none()];
      }

      const { simulator, context, aggregateType, aggregateId } = state;

      return [
        { ...state, status: 'executing', error: null },
        Effect.run(async (dispatch) => {
          try {
            const result = await simulator.execute_command(
              context,
              aggregateType,
              aggregateId,
              action.command
            );

            dispatch({ type: 'commandSucceeded', result });
          } catch (e) {
            dispatch({ type: 'commandFailed', error: (e as Error).message });
          }
        }),
      ];
    }

    case 'commandSucceeded':
      // Reload state to get updated events and aggregate state
      return [
        { ...state, status: 'loading' },
        Effect.run(async (dispatch) => {
          dispatch({ type: 'loadStateRequested' });
        }),
      ];

    case 'commandFailed':
      return [
        { ...state, status: 'error', error: action.error },
        Effect.none(),
      ];

    // ─────────────────────────────────────────────────────────────────
    // Reset
    // ─────────────────────────────────────────────────────────────────
    case 'resetTapped': {
      if (!state.simulator) {
        return [state, Effect.none()];
      }

      const { simulator } = state;

      return [
        { ...state, status: 'loading' },
        Effect.run(async (dispatch) => {
          await simulator.reset();
          dispatch({ type: 'resetCompleted' });
        }),
      ];
    }

    case 'resetCompleted':
      return [
        {
          ...state,
          events: [],
          aggregateState: null,
          selectedEventIndex: null,
          status: 'idle',
        },
        Effect.none(),
      ];

    // ─────────────────────────────────────────────────────────────────
    // Event Selection
    // ─────────────────────────────────────────────────────────────────
    case 'eventSelected':
      return [{ ...state, selectedEventIndex: action.index }, Effect.none()];

    case 'eventDeselected':
      return [{ ...state, selectedEventIndex: null }, Effect.none()];

    // ─────────────────────────────────────────────────────────────────
    // Time Control
    // ─────────────────────────────────────────────────────────────────
    case 'timeAdvanced': {
      if (!state.simulator) {
        return [state, Effect.none()];
      }

      // Pure calculation of new time
      const newTime = new Date(
        new Date(state.simulationTime).getTime() + action.seconds * 1000
      ).toISOString();

      // Capture simulator reference for effect
      const { simulator } = state;
      const { seconds } = action;

      return [
        { ...state, simulationTime: newTime },
        // Side effect: update WASM simulator's internal clock
        Effect.fireAndForget(() => {
          simulator.advance_time(seconds);
        }),
      ];
    }

    case 'timeSet': {
      if (!state.simulator) {
        return [state, Effect.none()];
      }

      // Capture simulator reference for effect
      const { simulator } = state;
      const { isoTimestamp } = action;

      return [
        { ...state, simulationTime: isoTimestamp },
        // Side effect: update WASM simulator's internal clock
        Effect.fireAndForget(() => {
          simulator.set_time(isoTimestamp);
        }),
      ];
    }

    default: {
      const _exhaustive: never = action;
      return [state, Effect.none()];
    }
  }
};
```

### 9.3 Simulation Panel Component

```svelte
<!-- components/SimulationPanel.svelte -->

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { createStore } from '@composable-svelte/core';
  import {
    simulationReducer,
    initialSimulationState,
  } from '$lib/features/simulation/reducer';
  import type { Simulator } from '$lib/wasm/simulation';

  import EventLogViewer from './EventLogViewer.svelte';
  import StateInspector from './StateInspector.svelte';
  import CommandInput from './CommandInput.svelte';
  import ProjectionViewer from './ProjectionViewer.svelte';
  import TimeController from './TimeController.svelte';

  interface Props {
    simulator: Simulator;
    context: string;
    aggregateType: string;
    aggregateId: string;
  }

  let { simulator, context, aggregateType, aggregateId }: Props = $props();

  // Create store with initial state including simulator reference
  const store = createStore({
    initialState: {
      ...initialSimulationState(context, aggregateType, aggregateId),
      simulator,
    },
    reducer: simulationReducer,
  });

  // Derived state for UI
  const isLoading = $derived(
    store.state.status === 'loading' || store.state.status === 'executing'
  );
  const selectedEvent = $derived(
    store.state.selectedEventIndex !== null
      ? store.state.events[store.state.selectedEventIndex]
      : null
  );

  onMount(() => {
    store.dispatch({ type: 'mounted' });
  });

  onDestroy(() => {
    store.dispatch({ type: 'unmounted' });
    store.destroy();
  });
</script>

<div class="simulation-panel">
  <header>
    <h2>{aggregateType}: {aggregateId}</h2>
    <TimeController
      time={store.state.simulationTime}
      onAdvance={(seconds) => store.dispatch({ type: 'timeAdvanced', seconds })}
      onSet={(isoTimestamp) => store.dispatch({ type: 'timeSet', isoTimestamp })}
    />
    <button
      onclick={() => store.dispatch({ type: 'resetTapped' })}
      disabled={isLoading}
    >
      Reset Simulation
    </button>
  </header>

  {#if store.state.error}
    <div class="error">{store.state.error}</div>
  {/if}

  <div class="grid">
    <section class="command-section">
      <h3>Execute Command</h3>
      <CommandInput
        {aggregateType}
        {context}
        onSubmit={(command) => store.dispatch({ type: 'commandSubmitted', command })}
        disabled={isLoading}
      />
    </section>

    <section class="state-section">
      <h3>Current State</h3>
      <StateInspector state={store.state.aggregateState} loading={isLoading} />
    </section>

    <section class="events-section">
      <h3>Event Log ({store.state.events.length} events)</h3>
      <EventLogViewer
        events={store.state.events}
        loading={isLoading}
        selectedIndex={store.state.selectedEventIndex}
        onSelect={(index) => store.dispatch({ type: 'eventSelected', index })}
        onDeselect={() => store.dispatch({ type: 'eventDeselected' })}
      />
    </section>

    <section class="projections-section">
      <h3>Projections</h3>
      <ProjectionViewer {context} {aggregateType} {aggregateId} {simulator} />
    </section>
  </div>
</div>
```

### 9.4 Simulator Store (Root Level)

```typescript
// lib/features/simulator/types.ts

import type { Simulator } from '$lib/wasm/simulation';

/**
 * Root simulator state - manages WASM module loading.
 */
export interface SimulatorState {
  status: 'unloaded' | 'loading' | 'ready' | 'error';
  simulator: Simulator | null;
  loadedModules: string[];
  error: string | null;
}

export type SimulatorAction =
  | { type: 'initializeRequested' }
  | { type: 'initializeSucceeded'; simulator: Simulator }
  | { type: 'initializeFailed'; error: string }
  | { type: 'moduleLoaded'; moduleName: string }
  | { type: 'resetRequested' };
```

```typescript
// lib/features/simulator/reducer.ts

import { Effect, type Reducer } from '@composable-svelte/core';
import init, { Simulator } from '$lib/wasm/simulation';
import type { SimulatorState, SimulatorAction } from './types';

export const initialSimulatorState: SimulatorState = {
  status: 'unloaded',
  simulator: null,
  loadedModules: [],
  error: null,
};

export const simulatorReducer: Reducer<SimulatorState, SimulatorAction> = (
  state,
  action
) => {
  switch (action.type) {
    case 'initializeRequested':
      return [
        { ...state, status: 'loading', error: null },
        Effect.run(async (dispatch) => {
          try {
            // Initialize WASM module
            await init();

            // Create simulator instance
            const simulator = new Simulator();

            dispatch({ type: 'initializeSucceeded', simulator });
          } catch (e) {
            dispatch({ type: 'initializeFailed', error: (e as Error).message });
          }
        }),
      ];

    case 'initializeSucceeded':
      return [
        {
          ...state,
          status: 'ready',
          simulator: action.simulator,
          error: null,
        },
        Effect.none(),
      ];

    case 'initializeFailed':
      return [
        { ...state, status: 'error', error: action.error },
        Effect.none(),
      ];

    case 'moduleLoaded':
      return [
        {
          ...state,
          loadedModules: [...state.loadedModules, action.moduleName],
        },
        Effect.none(),
      ];

    case 'resetRequested':
      return [initialSimulatorState, Effect.none()];

    default: {
      const _exhaustive: never = action;
      return [state, Effect.none()];
    }
  }
};
```

### 9.5 Event Log Viewer

```svelte
<!-- components/EventLogViewer.svelte -->

<script lang="ts">
  import type { StoredEvent } from '$lib/features/simulation/types';

  interface Props {
    events: StoredEvent[];
    loading?: boolean;
    selectedIndex: number | null;
    onSelect: (index: number) => void;
    onDeselect: () => void;
  }

  let {
    events,
    loading = false,
    selectedIndex,
    onSelect,
    onDeselect,
  }: Props = $props();

  const selectedEvent = $derived(
    selectedIndex !== null ? events[selectedIndex] : null
  );

  function formatTimestamp(iso: string): string {
    return new Date(iso).toLocaleTimeString();
  }

  function getEventColor(eventType: string): string {
    if (eventType.includes('Created')) return 'bg-green-100';
    if (eventType.includes('Failed') || eventType.includes('Cancelled'))
      return 'bg-red-100';
    if (eventType.includes('Updated') || eventType.includes('Changed'))
      return 'bg-blue-100';
    return 'bg-gray-100';
  }
</script>

<div class="event-log">
  {#if loading}
    <div class="loading">Loading events...</div>
  {:else if events.length === 0}
    <div class="empty">No events yet. Execute a command to begin.</div>
  {:else}
    <div class="timeline">
      {#each events as event, index}
        <button
          class="event-item {getEventColor(event.event_type)}"
          class:selected={selectedIndex === index}
          onclick={() =>
            selectedIndex === index ? onDeselect() : onSelect(index)}
        >
          <span class="version">v{event.version}</span>
          <span class="type">{event.event_type}</span>
          <span class="time">{formatTimestamp(event.metadata.timestamp)}</span>
        </button>
      {/each}
    </div>

    {#if selectedEvent}
      <div class="event-detail">
        <h4>{selectedEvent.event_type}</h4>
        <pre>{JSON.stringify(selectedEvent.payload, null, 2)}</pre>
        <div class="metadata">
          <span>Version: {selectedEvent.version}</span>
          <span>Time: {selectedEvent.metadata.timestamp}</span>
          {#if selectedEvent.metadata.correlation_id}
            <span>Correlation: {selectedEvent.metadata.correlation_id}</span>
          {/if}
        </div>
      </div>
    {/if}
  {/if}
</div>
```

---

## 10. Time Analysis

### 10.1 Workflow Comparison

| Scenario | Full Backend Deploy | WASM Simulation |
|----------|--------------------|--------------------|
| **First aggregate** | 2-5 min (DB + containers) | 35-70 sec |
| **Add 2nd aggregate** | 30-60 sec redeploy | 35-70 sec (parallel work) |
| **Add 5 aggregates at once** | 2-3 min redeploy | 45-75 sec (parallel AI) |
| **Run test scenario** | Network round-trip | Instant (local) |
| **Reset and retry** | Truncate + restart | Instant (clear IndexedDB) |
| **Time-travel debug** | Not available | Instant (replay events) |

### 10.2 Breakdown by Phase

```
Single Aggregate Compilation:

  AI Generation:     30-60 seconds
  ├── API call latency: 1-2 sec
  ├── Token generation: 25-50 sec
  └── Response parsing: 1-2 sec

  Rust Compilation:  5-10 seconds
  ├── cargo build:   3-7 sec
  ├── wasm-bindgen:  1-2 sec
  └── wasm-opt:      1 sec

  Transfer:          1-3 seconds
  ├── Compress:      <1 sec
  ├── Network:       1-2 sec
  └── Decompress:    <1 sec

  WASM Init:         <1 second
  ├── Compile:       <0.5 sec
  └── Instantiate:   <0.5 sec

  Total:             ~40-75 seconds
```

### 10.3 Parallel Scaling

```
Aggregates    Sequential AI    Parallel AI    Savings
─────────────────────────────────────────────────────
    1            45 sec          45 sec         0%
    2            90 sec          50 sec        44%
    3           135 sec          55 sec        59%
    5           225 sec          60 sec        73%
   10           450 sec          70 sec        84%
```

The parallel approach has diminishing returns after ~5-7 aggregates due to:
- API rate limits
- Server resource contention
- Network bandwidth

But for typical design sessions (1-5 aggregates at a time), parallel AI generation provides massive speedup.

---

## 11. Multi-Aggregate Simulation

### 11.1 Coordinator Feature: State & Actions

When multiple aggregates need to interact (e.g., Order → Payment → Inventory), we use a Composable Svelte reducer to coordinate cross-aggregate events:

```typescript
// lib/features/coordinator/types.ts

import type { Simulator } from '$lib/wasm/simulation';
import type { CommandResult } from '$lib/features/simulation/types';

/**
 * Saga subscription - maps event types to commands.
 */
export interface SagaSubscription {
  eventType: string; // e.g., "sales.OrderSubmitted"
  targetContext: string;
  targetAggregate: string;
  getAggregateId: (event: DomainEvent) => string;
  getCommand: (event: DomainEvent) => unknown;
}

export interface DomainEvent {
  event_type: string;
  payload: Record<string, unknown>;
}

/**
 * Coordinator state - manages multi-aggregate simulation.
 */
export interface CoordinatorState {
  simulator: Simulator | null;
  registeredAggregates: Map<string, boolean>; // key -> registered
  sagaSubscriptions: SagaSubscription[];
  pendingEvents: DomainEvent[];
  executionLog: ExecutionLogEntry[];
  status: 'idle' | 'executing' | 'error';
  error: string | null;
}

export interface ExecutionLogEntry {
  timestamp: string;
  type: 'command' | 'event' | 'saga_triggered';
  context: string;
  aggregate: string;
  aggregateId: string;
  data: unknown;
}

export type CoordinatorAction =
  // Setup
  | { type: 'simulatorSet'; simulator: Simulator }
  | { type: 'aggregateRegistered'; context: string; aggregateType: string }
  | { type: 'sagaSubscribed'; subscription: SagaSubscription }

  // Command execution
  | {
      type: 'commandExecuted';
      context: string;
      aggregateType: string;
      aggregateId: string;
      command: unknown;
    }
  | { type: 'commandSucceeded'; result: CommandResult }
  | { type: 'commandFailed'; error: string }

  // Event routing
  | { type: 'eventEmitted'; event: DomainEvent }
  | { type: 'sagaTriggered'; subscription: SagaSubscription; event: DomainEvent }

  // Reset
  | { type: 'resetRequested' }
  | { type: 'resetCompleted' };
```

### 11.2 Coordinator Reducer

```typescript
// lib/features/coordinator/reducer.ts

import { Effect, type Reducer } from '@composable-svelte/core';
import type { CoordinatorState, CoordinatorAction, DomainEvent } from './types';

export const initialCoordinatorState: CoordinatorState = {
  simulator: null,
  registeredAggregates: new Map(),
  sagaSubscriptions: [],
  pendingEvents: [],
  executionLog: [],
  status: 'idle',
  error: null,
};

export const coordinatorReducer: Reducer<CoordinatorState, CoordinatorAction> = (
  state,
  action
) => {
  switch (action.type) {
    // ─────────────────────────────────────────────────────────────────
    // Setup
    // ─────────────────────────────────────────────────────────────────
    case 'simulatorSet':
      return [{ ...state, simulator: action.simulator }, Effect.none()];

    case 'aggregateRegistered': {
      const key = `${action.context}.${action.aggregateType}`;
      const newMap = new Map(state.registeredAggregates);
      newMap.set(key, true);
      return [{ ...state, registeredAggregates: newMap }, Effect.none()];
    }

    case 'sagaSubscribed':
      return [
        {
          ...state,
          sagaSubscriptions: [...state.sagaSubscriptions, action.subscription],
        },
        Effect.none(),
      ];

    // ─────────────────────────────────────────────────────────────────
    // Command Execution
    // ─────────────────────────────────────────────────────────────────
    case 'commandExecuted': {
      if (!state.simulator) {
        return [
          { ...state, status: 'error', error: 'Simulator not initialized' },
          Effect.none(),
        ];
      }

      const { simulator } = state;
      const { context, aggregateType, aggregateId, command } = action;

      // Log the command
      const logEntry = {
        timestamp: new Date().toISOString(),
        type: 'command' as const,
        context,
        aggregate: aggregateType,
        aggregateId,
        data: command,
      };

      return [
        {
          ...state,
          status: 'executing',
          executionLog: [...state.executionLog, logEntry],
        },
        Effect.run(async (dispatch) => {
          try {
            const result = await simulator.execute_command(
              context,
              aggregateType,
              aggregateId,
              command
            );
            dispatch({ type: 'commandSucceeded', result });
          } catch (e) {
            dispatch({ type: 'commandFailed', error: (e as Error).message });
          }
        }),
      ];
    }

    case 'commandSucceeded': {
      // Emit events for saga processing
      const effects = action.result.events.map((event) =>
        Effect.run(async (dispatch) => {
          dispatch({ type: 'eventEmitted', event: event as DomainEvent });
        })
      );

      return [
        { ...state, status: 'idle' },
        effects.length > 0 ? Effect.batch(...effects) : Effect.none(),
      ];
    }

    case 'commandFailed':
      return [
        { ...state, status: 'error', error: action.error },
        Effect.none(),
      ];

    // ─────────────────────────────────────────────────────────────────
    // Event Routing (Saga Pattern)
    // ─────────────────────────────────────────────────────────────────
    case 'eventEmitted': {
      const { event } = action;

      // Log the event
      const logEntry = {
        timestamp: new Date().toISOString(),
        type: 'event' as const,
        context: event.event_type.split('.')[0],
        aggregate: event.event_type.split('.')[1]?.split(/(?=[A-Z])/)[0] ?? '',
        aggregateId: (event.payload.order_id as string) ?? 'unknown',
        data: event,
      };

      // Find matching saga subscriptions
      const matchingSubs = state.sagaSubscriptions.filter(
        (sub) => sub.eventType === event.event_type || sub.eventType === '*'
      );

      // Trigger saga effects
      const sagaEffects = matchingSubs.map((sub) =>
        Effect.run(async (dispatch) => {
          dispatch({ type: 'sagaTriggered', subscription: sub, event });
        })
      );

      return [
        { ...state, executionLog: [...state.executionLog, logEntry] },
        sagaEffects.length > 0 ? Effect.batch(...sagaEffects) : Effect.none(),
      ];
    }

    case 'sagaTriggered': {
      const { subscription, event } = action;
      const aggregateId = subscription.getAggregateId(event);
      const command = subscription.getCommand(event);

      // Log saga trigger
      const logEntry = {
        timestamp: new Date().toISOString(),
        type: 'saga_triggered' as const,
        context: subscription.targetContext,
        aggregate: subscription.targetAggregate,
        aggregateId,
        data: { triggeredBy: event.event_type, command },
      };

      return [
        { ...state, executionLog: [...state.executionLog, logEntry] },
        Effect.run(async (dispatch) => {
          dispatch({
            type: 'commandExecuted',
            context: subscription.targetContext,
            aggregateType: subscription.targetAggregate,
            aggregateId,
            command,
          });
        }),
      ];
    }

    // ─────────────────────────────────────────────────────────────────
    // Reset
    // ─────────────────────────────────────────────────────────────────
    case 'resetRequested': {
      if (!state.simulator) {
        return [state, Effect.none()];
      }

      const { simulator } = state;
      return [
        { ...state, status: 'executing' },
        Effect.run(async (dispatch) => {
          await simulator.reset();
          dispatch({ type: 'resetCompleted' });
        }),
      ];
    }

    case 'resetCompleted':
      return [
        { ...state, executionLog: [], pendingEvents: [], status: 'idle' },
        Effect.none(),
      ];

    default: {
      const _exhaustive: never = action;
      return [state, Effect.none()];
    }
  }
};
```

### 11.3 Saga Configuration Example

```typescript
// lib/features/coordinator/checkout-saga.ts

import type { SagaSubscription, DomainEvent } from './types';

/**
 * Checkout saga subscriptions.
 * Coordinates Order → Inventory → Payment → Order confirmation.
 */
export const checkoutSagaSubscriptions: SagaSubscription[] = [
  // When order submitted → reserve inventory
  {
    eventType: 'sales.OrderSubmitted',
    targetContext: 'inventory',
    targetAggregate: 'Reservation',
    getAggregateId: (event) => event.payload.order_id as string,
    getCommand: (event) => ({
      type: 'Reserve',
      items: event.payload.items,
    }),
  },

  // When inventory reserved → process payment
  {
    eventType: 'inventory.ReservationConfirmed',
    targetContext: 'payments',
    targetAggregate: 'Payment',
    getAggregateId: (event) => event.payload.order_id as string,
    getCommand: (event) => ({
      type: 'Process',
      amount: event.payload.total,
    }),
  },

  // When payment completed → confirm order
  {
    eventType: 'payments.PaymentCompleted',
    targetContext: 'sales',
    targetAggregate: 'Order',
    getAggregateId: (event) => event.payload.order_id as string,
    getCommand: () => ({ type: 'Confirm' }),
  },

  // COMPENSATION: When payment failed → release inventory
  {
    eventType: 'payments.PaymentFailed',
    targetContext: 'inventory',
    targetAggregate: 'Reservation',
    getAggregateId: (event) => event.payload.order_id as string,
    getCommand: (event) => ({
      type: 'Release',
      reason: `Payment failed: ${event.payload.reason}`,
    }),
  },
];

/**
 * Initialize coordinator with checkout saga.
 */
export function setupCheckoutSaga(
  dispatch: (action: CoordinatorAction) => void
): void {
  for (const subscription of checkoutSagaSubscriptions) {
    dispatch({ type: 'sagaSubscribed', subscription });
  }
}
```

---

## 12. Error Handling and Validation

### 12.1 Validation Feedback

The simulation provides immediate validation feedback:

```typescript
interface ValidationResult {
  valid: boolean;
  errors: ValidationError[];
  warnings: ValidationWarning[];
}

interface ValidationError {
  field: string;
  message: string;
  code: string;
}

// Example: Validation in command execution
async function executeWithValidation(
  simulator: Simulator,
  command: any
): Promise<{ result?: CommandResult; validation: ValidationResult }> {
  try {
    const result = await simulator.execute_command(...);
    return {
      result,
      validation: { valid: true, errors: [], warnings: [] }
    };
  } catch (error) {
    // Parse domain validation errors
    if (error.message.startsWith('VALIDATION_ERROR:')) {
      const errors = parseValidationErrors(error.message);
      return {
        validation: { valid: false, errors, warnings: [] }
      };
    }

    // Re-throw unexpected errors
    throw error;
  }
}
```

### 12.2 Concurrency Error Handling

```typescript
async function executeWithRetry(
  simulator: Simulator,
  command: any,
  maxRetries = 3
): Promise<CommandResult> {
  for (let attempt = 1; attempt <= maxRetries; attempt++) {
    try {
      return await simulator.execute_command(...);
    } catch (error) {
      if (error.message.startsWith('CONCURRENCY_ERROR:') && attempt < maxRetries) {
        // Retry with fresh state
        console.log(`Concurrency conflict, retrying (${attempt}/${maxRetries})`);
        await new Promise(r => setTimeout(r, 100 * attempt));
        continue;
      }
      throw error;
    }
  }
  throw new Error('Max retries exceeded');
}
```

---

## 13. Security Considerations

### 13.1 WASM Sandbox

WASM runs in a sandboxed environment:
- No filesystem access
- No network access (from WASM itself)
- Memory isolated from JavaScript
- CPU time bounded by browser

The only I/O happens through the JavaScript bridge, which is controlled.

### 13.2 Compilation Server Security

```typescript
// Server-side validation

function validateYamlSpec(yaml: string): ValidationResult {
  // 1. Size limits
  if (yaml.length > 1_000_000) {
    return { valid: false, error: 'Spec too large (max 1MB)' };
  }

  // 2. Parse and validate structure
  const spec = parseYaml(yaml);

  // 3. Limit complexity
  const aggregateCount = countAggregates(spec);
  if (aggregateCount > 50) {
    return { valid: false, error: 'Too many aggregates (max 50)' };
  }

  // 4. Validate naming (prevent injection)
  for (const name of extractNames(spec)) {
    if (!/^[a-zA-Z][a-zA-Z0-9_]*$/.test(name)) {
      return { valid: false, error: `Invalid name: ${name}` };
    }
  }

  return { valid: true };
}
```

### 13.3 Rate Limiting

```typescript
// Compilation rate limits

const compilationLimiter = rateLimit({
  windowMs: 60 * 1000,  // 1 minute
  max: 10,              // 10 compilations per minute per user
  keyGenerator: (req) => req.user.id
});

const aiGenerationLimiter = rateLimit({
  windowMs: 60 * 1000,
  max: 50,              // 50 AI calls per minute per user
  keyGenerator: (req) => req.user.id
});
```

---

## 14. Future Extensions

### 14.1 Collaborative Simulation

Multiple users simulating the same system:

```typescript
// WebSocket-based sync for collaborative sessions
interface SimulationSync {
  sessionId: string;
  participants: string[];
  eventLog: SharedEventLog;
}

// Events broadcast to all participants
ws.on('command_executed', (event) => {
  broadcast(sessionId, {
    type: 'event_appended',
    event
  });
});
```

### 14.2 Scenario Recording and Playback

```typescript
interface Scenario {
  id: string;
  name: string;
  description: string;
  steps: ScenarioStep[];
}

interface ScenarioStep {
  type: 'command' | 'assertion' | 'time_advance';
  data: any;
}

// Record a scenario
function recordScenario(events: any[]): Scenario {
  return {
    id: generateId(),
    name: 'Recorded scenario',
    steps: events.map(e => ({
      type: 'command',
      data: e.command
    }))
  };
}

// Playback a scenario
async function playScenario(simulator: Simulator, scenario: Scenario) {
  await simulator.reset();

  for (const step of scenario.steps) {
    if (step.type === 'command') {
      await simulator.execute_command(...step.data);
    } else if (step.type === 'time_advance') {
      simulator.advance_time(step.data.seconds);
    }
  }
}
```

### 14.3 Property-Based Testing in Browser

```typescript
// Generate random commands to find edge cases
import fc from 'fast-check';

async function fuzzAggregate(simulator: Simulator, aggregateType: string) {
  const commandArbitrary = fc.oneof(
    fc.record({ type: fc.constant('Create'), ...createArbitrary }),
    fc.record({ type: fc.constant('Update'), ...updateArbitrary }),
    // ... more commands
  );

  await fc.assert(
    fc.asyncProperty(fc.array(commandArbitrary), async (commands) => {
      await simulator.reset();

      for (const command of commands) {
        try {
          await simulator.execute_command(...);
        } catch (e) {
          // Validation errors are OK
          if (!e.message.startsWith('VALIDATION_ERROR:')) {
            throw e;
          }
        }
      }

      // Invariant: state should be consistent
      const state = await simulator.get_state(...);
      return validateStateInvariants(state);
    })
  );
}
```

---

## 15. Summary

The WASM browser simulation architecture provides:

1. **Instant Iteration**: 35-70 seconds from design to simulation (vs 2-5 minutes for full deploy)

2. **Parallel Compilation**: 5 aggregates compile in the same time as 1 thanks to parallel AI generation

3. **Zero Infrastructure**: Everything runs in the browser with IndexedDB storage

4. **Full Fidelity**: Same domain logic runs in both PostgreSQL (production) and WASM (simulation)

5. **Rich Debugging**: Time-travel, event inspection, state visualization

6. **Incremental Workflow**: Design and test one aggregate at a time, add more incrementally

The key enabler is the **Functional Core / Imperative Shell** architecture: pure domain logic compiles identically to both targets, while only the storage layer differs.