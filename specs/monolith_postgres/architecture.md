# PostgreSQL Monolith Architecture Specification

> **Composable Architecture in Pure PostgreSQL**
>
> This specification describes how to implement the Composable Rust architecture
> entirely within PostgreSQL, preserving the functional core / imperative shell
> separation while leveraging PostgreSQL's transactional guarantees.

---

## Table of Contents

1. [Philosophy](#1-philosophy)
2. [Architecture Overview](#2-architecture-overview)
3. [Schema Design](#3-schema-design)
4. [Function Patterns](#4-function-patterns)
5. [Testing Pure Functions](#5-testing-pure-functions)
6. [Projections](#6-projections)
7. [Real-Time Events](#7-real-time-events)
8. [Transactions Replace Sagas](#8-transactions-replace-sagas)
9. [Rust Integration Layer](#9-rust-integration-layer)
10. [Code Generation Pipeline](#10-code-generation-pipeline)
11. [Migration from Distributed Architecture](#11-migration-from-distributed-architecture)
12. [Performance Considerations](#12-performance-considerations)
13. [Appendix: Complete Example](#13-appendix-complete-example)

---

## 1. Philosophy

### 1.1 Core Principle: Functional Core, Imperative Shell

The fundamental insight that makes Composable Architecture powerful is the separation of:

- **Pure Business Logic**: Deterministic functions that transform state based on commands/events
- **Imperative Shell**: Infrastructure code that handles I/O, persistence, and side effects

This separation is **preserved exactly** in the PostgreSQL implementation:

```
┌─────────────────────────────────────────────────────────────┐
│                    PostgreSQL Database                       │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────────┐    │
│  │              IMPERATIVE SHELL                        │    │
│  │         {aggregate}_handle(command)                  │    │
│  │                                                      │    │
│  │  • Loads events from database                       │    │
│  │  • Calls pure functions                             │    │
│  │  • Persists new events                              │    │
│  │  • Triggers projections                             │    │
│  │  • Sends NOTIFY for real-time                       │    │
│  │                                                      │    │
│  │  [TEMPLATE-GENERATED - Never AI-generated]          │    │
│  └──────────────────────┬───────────────────────────────┘    │
│                         │                                    │
│                         ▼                                    │
│  ┌─────────────────────────────────────────────────────┐    │
│  │               PURE FUNCTIONAL CORE                   │    │
│  │                                                      │    │
│  │  {aggregate}_process(state, command) → result       │    │
│  │  {aggregate}_apply(state, event) → state            │    │
│  │                                                      │    │
│  │  • IMMUTABLE - No side effects                      │    │
│  │  • Deterministic - Same input → Same output         │    │
│  │  • Testable in isolation (no tables needed)         │    │
│  │                                                      │    │
│  │  [AI-GENERATED from YAML business logic]            │    │
│  └─────────────────────────────────────────────────────┘    │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### 1.2 Why PostgreSQL IMMUTABLE?

PostgreSQL's `IMMUTABLE` function attribute provides the same guarantees as Rust's pure functions:

| Rust Pure Function | PostgreSQL IMMUTABLE |
|--------------------|----------------------|
| No mutable state | Cannot access tables |
| No I/O | Cannot use `RAISE NOTICE` side effects |
| Deterministic | Same inputs → same outputs (optimizer can cache) |
| Testable in isolation | Can test with `SELECT` without any schema |

**PostgreSQL enforces these at function creation time.** If you try to access a table from an IMMUTABLE function, PostgreSQL will reject it.

### 1.3 State Loading: Projection, Not Event Folding

A critical performance optimization: **never fold events in the hot path**.

In traditional event sourcing, processing a command requires:
1. Load all events for the stream
2. Fold them to reconstruct current state
3. Process command against state
4. Append new events

This is O(n) where n = number of events. For an aggregate with 1000 events, that's 1000 reads before you can process one command.

**Our approach**: Query the projection directly.

```
Traditional Event Sourcing:
┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│ Load 1000   │───>│ Fold all    │───>│ Process     │
│ events      │    │ events      │    │ command     │
└─────────────┘    └─────────────┘    └─────────────┘
      O(n)              O(n)              O(1)

Our Approach:
┌─────────────┐    ┌─────────────┐
│ Query       │───>│ Process     │
│ projection  │    │ command     │
└─────────────┘    └─────────────┘
      O(1)              O(1)
```

**Why this works**: Projection triggers run synchronously in the same transaction
as event inserts. When `order_handle()` returns, the projection is already updated.
The next command can query the projection directly.

```sql
-- This is O(1):
SELECT * FROM orders_projection WHERE order_id = 'order-123';

-- Instead of O(n):
SELECT * FROM events WHERE stream_id = 'order-123' ORDER BY version;
-- then fold each event...
```

**When we DO fold events**:
- Rebuilding projections after schema changes
- Disaster recovery if projection table is corrupted
- Testing the `apply` function in isolation

### 1.4 Why PL/pgSQL?

For AI code generation, PL/pgSQL is optimal because:

1. **Training Data Abundance**: PL/pgSQL is PostgreSQL's native procedural language. Every tutorial, Stack Overflow answer, and documentation example uses it. This maximizes AI generation accuracy.

2. **Direct Pattern Mapping**: The reducer pattern maps cleanly:
   - Rust: `match (state.status, action) { ... }`
   - PL/pgSQL: `CASE WHEN state->>'status' = ... AND cmd_type = ... THEN`

3. **Type Safety**: PL/pgSQL catches type errors at function creation, not runtime.

4. **Zero Deployment Friction**: No extensions required. Works on all PostgreSQL installations including RDS, Cloud SQL, Supabase, Neon.

5. **JSONB Native**: First-class support for JSONB operations, ideal for event payloads and flexible state.

---

## 2. Architecture Overview

### 2.1 Component Mapping

| Distributed Architecture | PostgreSQL Monolith |
|--------------------------|---------------------|
| PostgresEventStore | `events` table + `{agg}_handle()` |
| InMemoryProjector | Triggers on `events` table |
| RedpandaEventBus | `pg_notify()` + LISTEN |
| Rust Reducer | `{agg}_process()` IMMUTABLE function |
| State reconstruction | `{agg}_apply()` + `{agg}_fold_events()` |
| Saga with compensation | Single transaction with ROLLBACK |
| HTTP Handler (Axum) | Thin Rust layer calling stored procedures |

### 2.2 Data Flow

```
HTTP Request
     │
     ▼
┌─────────────────┐
│  Rust Handler   │  (Thin layer: parse JSON, call procedure, return response)
│  (Axum)         │
└────────┬────────┘
         │
         ▼
┌─────────────────────────────────────────────────────────────────┐
│                        PostgreSQL                                │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  {aggregate}_handle(command JSONB) → JSONB               │   │
│  │                                                           │   │
│  │  1. Load state:  {aggregate}_load_state(id) ← O(1)!      │   │
│  │                  (queries projection, NOT event folding) │   │
│  │  2. Process:     {aggregate}_process(state, command)     │   │
│  │  3. Persist:     INSERT INTO events (...)                │   │
│  │  4. [Trigger]:   {aggregate}_project() runs synchronously│   │
│  │  5. Notify:      pg_notify('events', ...)                │   │
│  │  6. Return:      { success: true, version: N, ... }      │   │
│  └──────────────────────────────────────────────────────────┘   │
│                           │                                      │
│                           ▼                                      │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  TRIGGER: {aggregate}_projection_trigger                  │   │
│  │           (runs SYNCHRONOUSLY in same transaction)        │   │
│  │                                                           │   │
│  │  ON INSERT TO events                                      │   │
│  │  EXECUTE {aggregate}_project()                            │   │
│  │                                                           │   │
│  │  Updates: {aggregate}_projection table ← Always current!  │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
         │
         ▼
┌─────────────────┐
│  LISTEN clients │  (WebSocket connections via Rust)
│  (Real-time)    │
└─────────────────┘
```

### 2.3 Aggregate Structure

Each aggregate consists of these PostgreSQL objects:

```
{aggregate}/
├── Functions (Pure - AI Generated)
│   ├── {aggregate}_process(state, command) → result     IMMUTABLE
│   └── {aggregate}_apply(state, event) → state          IMMUTABLE
│
├── Functions (Imperative Shell - Template Generated)
│   ├── {aggregate}_handle(command) → result             VOLATILE
│   ├── {aggregate}_load_state(id) → state               STABLE  ← O(1) from projection
│   └── {aggregate}_fold_events(stream_id) → state       STABLE  ← For rebuilds only
│
├── Projection (Template Generated)
│   ├── {aggregate}_projection table                     ← Source of truth for reads
│   ├── {aggregate}_project() trigger function
│   └── {aggregate}_projection_trigger
│
└── Types (Optional - for complex domains)
    ├── {aggregate}_command type
    ├── {aggregate}_event type
    └── {aggregate}_state type
```

---

## 3. Schema Design

### 3.1 Core Events Table

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- EVENTS TABLE
-- The append-only event log. Source of truth for all aggregates.
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE events (
    -- Surrogate key for ordering (BIGSERIAL for high-volume systems)
    id              BIGSERIAL PRIMARY KEY,

    -- Aggregate identity
    stream_id       TEXT NOT NULL,

    -- Optimistic concurrency control
    version         INTEGER NOT NULL,

    -- Event type for routing (e.g., "OrderCreated", "ItemAdded")
    event_type      TEXT NOT NULL,

    -- Event data as JSONB for flexibility
    payload         JSONB NOT NULL,

    -- Optional metadata (correlation_id, causation_id, user_id, etc.)
    metadata        JSONB,

    -- Timestamp with timezone (use database time for consistency)
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Ensure version uniqueness per stream (optimistic locking)
    CONSTRAINT events_stream_version_unique UNIQUE (stream_id, version)
);

-- ═══════════════════════════════════════════════════════════════════════════
-- INDEXES
-- ═══════════════════════════════════════════════════════════════════════════

-- Primary access pattern: load all events for a stream
CREATE INDEX idx_events_stream_id ON events (stream_id, version);

-- For projections: find events by type
CREATE INDEX idx_events_event_type ON events (event_type);

-- For temporal queries: find events in time range
CREATE INDEX idx_events_created_at ON events (created_at);

-- For correlation tracking (if using metadata)
CREATE INDEX idx_events_correlation ON events ((metadata->>'correlation_id'))
    WHERE metadata->>'correlation_id' IS NOT NULL;
```

### 3.2 Projection Tables (Per Aggregate)

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDERS PROJECTION
-- Denormalized read model optimized for queries
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE orders_projection (
    -- Primary key matches stream_id pattern
    order_id        TEXT PRIMARY KEY,

    -- Denormalized fields for fast queries
    status          TEXT NOT NULL,
    customer_id     TEXT NOT NULL,
    total_amount    DECIMAL(12, 2) NOT NULL DEFAULT 0,
    item_count      INTEGER NOT NULL DEFAULT 0,

    -- Items array for business logic validation
    -- Stored as JSONB so process() can check item details
    items           JSONB NOT NULL DEFAULT '[]'::jsonb,

    -- Timestamps
    created_at      TIMESTAMPTZ NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL,

    -- Version tracking (for consistency checks)
    last_event_id   BIGINT REFERENCES events(id)
);

-- ═══════════════════════════════════════════════════════════════════════════
-- PROJECTION INDEXES
-- ═══════════════════════════════════════════════════════════════════════════

-- Common query patterns
CREATE INDEX idx_orders_projection_status ON orders_projection (status);
CREATE INDEX idx_orders_projection_customer ON orders_projection (customer_id);
CREATE INDEX idx_orders_projection_created ON orders_projection (created_at DESC);
```

### 3.3 Outbox Pattern (Optional)

For guaranteed delivery to external systems:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- OUTBOX TABLE
-- For reliable delivery to external systems (webhooks, external queues)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE outbox (
    id              BIGSERIAL PRIMARY KEY,
    event_id        BIGINT NOT NULL REFERENCES events(id),
    destination     TEXT NOT NULL,      -- e.g., "webhook:orders", "kafka:inventory"
    payload         JSONB NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',  -- pending, sent, failed
    attempts        INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at    TIMESTAMPTZ,
    error_message   TEXT
);

CREATE INDEX idx_outbox_pending ON outbox (status, created_at)
    WHERE status = 'pending';
```

---

## 4. Function Patterns

### 4.1 Pure Functions (AI-Generated)

These functions contain the business logic. They are:
- **IMMUTABLE**: Cannot access tables or have side effects
- **Deterministic**: Same inputs always produce same outputs
- **Testable**: Can be tested without any database schema

#### 4.1.1 Process Function

The `process` function implements command handling logic. It receives the current state and a command, returning either events to append or an error.

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_PROCESS: Pure Command Handler
--
-- Inputs:
--   current_state: JSONB - The current aggregate state (or empty object {})
--   command: JSONB - The command to process
--
-- Returns: JSONB with one of:
--   { "events": [...] }           - Success: events to append
--   { "error": "...", "message": "..." }  - Business error
--
-- IMMUTABLE: This function has NO side effects. It cannot:
--   - Access any tables
--   - Use RAISE NOTICE/WARNING
--   - Call non-immutable functions
--   - Use NOW() or random()
--
-- AI-GENERATED from business logic YAML
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_process(
    current_state JSONB,
    command JSONB
) RETURNS JSONB
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    cmd_type TEXT;
    status TEXT;
    order_id TEXT;
    customer_id TEXT;
    item JSONB;
    current_items JSONB;
    item_count INTEGER;
BEGIN
    -- Extract command type
    cmd_type := command->>'type';

    -- Extract current status (NULL if new aggregate)
    status := current_state->>'status';

    -- ═══════════════════════════════════════════════════════════════════
    -- Command Routing
    -- ═══════════════════════════════════════════════════════════════════

    CASE cmd_type

    -- ───────────────────────────────────────────────────────────────────
    -- CreateOrder
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'CreateOrder' THEN
        -- Validate: Order must not already exist
        IF status IS NOT NULL THEN
            RETURN jsonb_build_object(
                'error', 'OrderAlreadyExists',
                'message', format('Order already exists with status: %s', status)
            );
        END IF;

        -- Validate: Required fields
        order_id := command->>'order_id';
        customer_id := command->>'customer_id';

        IF order_id IS NULL OR customer_id IS NULL THEN
            RETURN jsonb_build_object(
                'error', 'InvalidCommand',
                'message', 'order_id and customer_id are required'
            );
        END IF;

        -- Success: Return OrderCreated event
        RETURN jsonb_build_object(
            'events', jsonb_build_array(
                jsonb_build_object(
                    'type', 'OrderCreated',
                    'order_id', order_id,
                    'customer_id', customer_id
                )
            )
        );

    -- ───────────────────────────────────────────────────────────────────
    -- AddItem
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'AddItem' THEN
        -- Validate: Order must exist and be in pending status
        IF status IS NULL THEN
            RETURN jsonb_build_object(
                'error', 'OrderNotFound',
                'message', 'Cannot add item to non-existent order'
            );
        END IF;

        IF status <> 'pending' THEN
            RETURN jsonb_build_object(
                'error', 'InvalidOrderStatus',
                'message', format('Cannot add items to order with status: %s', status)
            );
        END IF;

        -- Validate: Item must have required fields
        item := command->'item';
        IF item IS NULL OR item->>'product_id' IS NULL OR item->>'quantity' IS NULL THEN
            RETURN jsonb_build_object(
                'error', 'InvalidItem',
                'message', 'Item must have product_id and quantity'
            );
        END IF;

        -- Validate: Quantity must be positive
        IF (item->>'quantity')::integer <= 0 THEN
            RETURN jsonb_build_object(
                'error', 'InvalidQuantity',
                'message', 'Quantity must be positive'
            );
        END IF;

        -- Business rule: Maximum 100 items per order
        current_items := COALESCE(current_state->'items', '[]'::jsonb);
        item_count := jsonb_array_length(current_items);

        IF item_count >= 100 THEN
            RETURN jsonb_build_object(
                'error', 'MaxItemsExceeded',
                'message', 'Cannot add more than 100 items to an order'
            );
        END IF;

        -- Success: Return ItemAdded event
        RETURN jsonb_build_object(
            'events', jsonb_build_array(
                jsonb_build_object(
                    'type', 'ItemAdded',
                    'product_id', item->>'product_id',
                    'quantity', (item->>'quantity')::integer,
                    'unit_price', COALESCE((item->>'unit_price')::decimal, 0)
                )
            )
        );

    -- ───────────────────────────────────────────────────────────────────
    -- SubmitOrder
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'SubmitOrder' THEN
        -- Validate: Order must exist
        IF status IS NULL THEN
            RETURN jsonb_build_object(
                'error', 'OrderNotFound',
                'message', 'Cannot submit non-existent order'
            );
        END IF;

        -- Validate: Order must be pending
        IF status <> 'pending' THEN
            RETURN jsonb_build_object(
                'error', 'InvalidOrderStatus',
                'message', format('Cannot submit order with status: %s', status)
            );
        END IF;

        -- Validate: Order must have items
        current_items := COALESCE(current_state->'items', '[]'::jsonb);
        IF jsonb_array_length(current_items) = 0 THEN
            RETURN jsonb_build_object(
                'error', 'EmptyOrder',
                'message', 'Cannot submit an empty order'
            );
        END IF;

        -- Success: Return OrderSubmitted event
        RETURN jsonb_build_object(
            'events', jsonb_build_array(
                jsonb_build_object(
                    'type', 'OrderSubmitted',
                    'submitted_at', to_char(
                        (command->>'timestamp')::timestamptz,
                        'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"'
                    )
                )
            )
        );

    -- ───────────────────────────────────────────────────────────────────
    -- CancelOrder
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'CancelOrder' THEN
        -- Validate: Order must exist
        IF status IS NULL THEN
            RETURN jsonb_build_object(
                'error', 'OrderNotFound',
                'message', 'Cannot cancel non-existent order'
            );
        END IF;

        -- Validate: Cannot cancel completed or already cancelled orders
        IF status IN ('completed', 'cancelled') THEN
            RETURN jsonb_build_object(
                'error', 'InvalidOrderStatus',
                'message', format('Cannot cancel order with status: %s', status)
            );
        END IF;

        -- Success: Return OrderCancelled event
        RETURN jsonb_build_object(
            'events', jsonb_build_array(
                jsonb_build_object(
                    'type', 'OrderCancelled',
                    'reason', COALESCE(command->>'reason', 'No reason provided')
                )
            )
        );

    -- ───────────────────────────────────────────────────────────────────
    -- Unknown Command
    -- ───────────────────────────────────────────────────────────────────
    ELSE
        RETURN jsonb_build_object(
            'error', 'UnknownCommand',
            'message', format('Unknown command type: %s', cmd_type)
        );

    END CASE;
END;
$$;

-- Document the function
COMMENT ON FUNCTION order_process(JSONB, JSONB) IS
'Pure command handler for Order aggregate. AI-generated from business logic YAML.
Returns {events: [...]} on success or {error: "...", message: "..."} on failure.';
```

#### 4.1.2 Apply Function

The `apply` function implements event folding. It receives the current state and an event, returning the new state.

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_APPLY: Pure Event Folder
--
-- Inputs:
--   current_state: JSONB - The current aggregate state (or empty object {})
--   event: JSONB - The event to apply
--
-- Returns: JSONB - The new state after applying the event
--
-- IMMUTABLE: This function has NO side effects.
--
-- AI-GENERATED from business logic YAML
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_apply(
    current_state JSONB,
    event JSONB
) RETURNS JSONB
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    event_type TEXT;
    new_state JSONB;
    items JSONB;
    new_item JSONB;
    total DECIMAL;
BEGIN
    event_type := event->>'type';
    new_state := current_state;

    CASE event_type

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCreated
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderCreated' THEN
        new_state := jsonb_build_object(
            'order_id', event->>'order_id',
            'customer_id', event->>'customer_id',
            'status', 'pending',
            'items', '[]'::jsonb,
            'total_amount', 0
        );

    -- ───────────────────────────────────────────────────────────────────
    -- ItemAdded
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'ItemAdded' THEN
        -- Add item to items array
        items := COALESCE(current_state->'items', '[]'::jsonb);
        new_item := jsonb_build_object(
            'product_id', event->>'product_id',
            'quantity', (event->>'quantity')::integer,
            'unit_price', COALESCE((event->>'unit_price')::decimal, 0)
        );
        items := items || jsonb_build_array(new_item);

        -- Calculate new total
        total := COALESCE((current_state->>'total_amount')::decimal, 0) +
                 (COALESCE((event->>'quantity')::integer, 0) *
                  COALESCE((event->>'unit_price')::decimal, 0));

        new_state := current_state || jsonb_build_object(
            'items', items,
            'total_amount', total
        );

    -- ───────────────────────────────────────────────────────────────────
    -- OrderSubmitted
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderSubmitted' THEN
        new_state := current_state || jsonb_build_object(
            'status', 'submitted',
            'submitted_at', event->>'submitted_at'
        );

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCancelled
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderCancelled' THEN
        new_state := current_state || jsonb_build_object(
            'status', 'cancelled',
            'cancelled_reason', event->>'reason'
        );

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCompleted
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderCompleted' THEN
        new_state := current_state || jsonb_build_object(
            'status', 'completed',
            'completed_at', event->>'completed_at'
        );

    -- ───────────────────────────────────────────────────────────────────
    -- Unknown Event (ignored for forward compatibility)
    -- ───────────────────────────────────────────────────────────────────
    ELSE
        -- Unknown events are ignored (allows schema evolution)
        NULL;

    END CASE;

    RETURN new_state;
END;
$$;

COMMENT ON FUNCTION order_apply(JSONB, JSONB) IS
'Pure event folder for Order aggregate. AI-generated from business logic YAML.
Folds a single event into the current state, returning the new state.';
```

### 4.2 Imperative Shell (Template-Generated)

These functions handle I/O and side effects. They are:
- **VOLATILE** or **STABLE**: Can access tables
- **Template-generated**: No business logic, just infrastructure wiring

#### 4.2.1 Load State from Projection (Primary Method)

**Key Optimization**: Since projection triggers run synchronously in the same
transaction as event inserts, the projection table always reflects the current
state. We query it directly instead of folding events.

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_LOAD_STATE: Load Current State from Projection
--
-- Queries the projection table to get current aggregate state.
-- This is O(1) instead of O(n) event folding.
--
-- STABLE: Reads from projection table.
--
-- TEMPLATE-GENERATED
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_load_state(
    p_order_id TEXT
) RETURNS JSONB
LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_state JSONB;
BEGIN
    -- Query projection for current state
    SELECT jsonb_build_object(
        'order_id', order_id,
        'customer_id', customer_id,
        'status', status,
        'total_amount', total_amount,
        'item_count', item_count,
        -- Include items if stored in projection, or load separately
        'items', COALESCE(items, '[]'::jsonb)
    )
    INTO v_state
    FROM orders_projection
    WHERE order_id = p_order_id;

    -- Return empty object if aggregate doesn't exist yet
    RETURN COALESCE(v_state, '{}'::jsonb);
END;
$$;

COMMENT ON FUNCTION order_load_state(TEXT) IS
'Loads Order state from projection. O(1) lookup. TEMPLATE-GENERATED.';
```

#### 4.2.2 Fold Events (For Rebuilds Only)

Keep this function for projection rebuilding and disaster recovery, but
**never use it in the hot path**:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_FOLD_EVENTS: State Reconstruction (Rebuild Only)
--
-- Reconstructs aggregate state by folding all events.
-- ONLY used for:
--   - Rebuilding projections
--   - Disaster recovery
--   - Testing the apply function
--
-- NOT used in normal command processing (use order_load_state instead).
--
-- STABLE: Reads from events table but has no side effects.
--
-- TEMPLATE-GENERATED
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_fold_events(
    p_stream_id TEXT
) RETURNS JSONB
LANGUAGE plpgsql STABLE AS $$
DECLARE
    current_state JSONB := '{}'::jsonb;
    event_record RECORD;
BEGIN
    -- Fold events in version order
    FOR event_record IN
        SELECT payload
        FROM events
        WHERE stream_id = p_stream_id
        ORDER BY version
    LOOP
        current_state := order_apply(current_state, event_record.payload);
    END LOOP;

    RETURN current_state;
END;
$$;

COMMENT ON FUNCTION order_fold_events(TEXT) IS
'Reconstructs Order state by folding events. For rebuilds only. TEMPLATE-GENERATED.';
```

#### 4.2.3 Handle Function (The Imperative Shell)

The main entry point that orchestrates everything:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_HANDLE: Imperative Shell (Command Handler)
--
-- This is the main entry point for the Order aggregate.
-- It orchestrates:
--   1. Loading current state FROM PROJECTION (O(1), not event folding!)
--   2. Calling pure business logic (order_process)
--   3. Persisting new events with optimistic concurrency
--   4. Triggering projections (via trigger - synchronous in same transaction)
--   5. Notifying subscribers (for real-time updates)
--
-- VOLATILE: Has side effects (INSERT, NOTIFY).
--
-- TEMPLATE-GENERATED - All aggregates have identical structure.
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_handle(
    command JSONB
) RETURNS JSONB
LANGUAGE plpgsql AS $$
DECLARE
    -- Identifiers
    v_stream_id TEXT;
    v_correlation_id TEXT;

    -- State management
    v_current_version INTEGER;
    v_current_state JSONB;
    v_new_version INTEGER;

    -- Processing
    v_result JSONB;
    v_events JSONB;
    v_event JSONB;
    v_event_ids BIGINT[] := ARRAY[]::BIGINT[];
    v_event_id BIGINT;

    -- Metadata
    v_metadata JSONB;
    v_timestamp TIMESTAMPTZ := NOW();
BEGIN
    -- ═══════════════════════════════════════════════════════════════════
    -- 1. EXTRACT IDENTIFIERS
    -- ═══════════════════════════════════════════════════════════════════

    -- Stream ID from command (aggregate identity)
    v_stream_id := 'order-' || (command->>'order_id');

    -- Correlation ID for tracing (optional)
    v_correlation_id := COALESCE(
        command->'metadata'->>'correlation_id',
        gen_random_uuid()::text
    );

    -- ═══════════════════════════════════════════════════════════════════
    -- 2. LOAD CURRENT STATE FROM PROJECTION
    -- ═══════════════════════════════════════════════════════════════════

    -- Get current version (for optimistic concurrency)
    SELECT COALESCE(MAX(version), 0)
    INTO v_current_version
    FROM events
    WHERE stream_id = v_stream_id;

    -- Load state from projection (O(1) lookup, NOT event folding!)
    -- The projection is always up-to-date because triggers run synchronously
    v_current_state := order_load_state(command->>'order_id');

    -- ═══════════════════════════════════════════════════════════════════
    -- 3. CALL PURE BUSINESS LOGIC
    -- ═══════════════════════════════════════════════════════════════════

    -- Add timestamp to command for business logic that needs it
    command := command || jsonb_build_object('timestamp', v_timestamp);

    -- Call the pure process function
    v_result := order_process(v_current_state, command);

    -- ═══════════════════════════════════════════════════════════════════
    -- 4. CHECK FOR BUSINESS ERRORS
    -- ═══════════════════════════════════════════════════════════════════

    IF v_result ? 'error' THEN
        -- Return business error (no side effects occurred)
        RETURN jsonb_build_object(
            'success', false,
            'error', v_result->>'error',
            'message', v_result->>'message',
            'correlation_id', v_correlation_id
        );
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- 5. PERSIST EVENTS
    -- ═══════════════════════════════════════════════════════════════════

    v_events := v_result->'events';
    v_new_version := v_current_version;

    -- Build metadata for all events
    v_metadata := jsonb_build_object(
        'correlation_id', v_correlation_id,
        'causation_id', command->>'command_id',
        'user_id', command->'metadata'->>'user_id',
        'timestamp', v_timestamp
    );

    -- Append each event
    FOR v_event IN SELECT * FROM jsonb_array_elements(v_events)
    LOOP
        v_new_version := v_new_version + 1;

        INSERT INTO events (stream_id, version, event_type, payload, metadata, created_at)
        VALUES (
            v_stream_id,
            v_new_version,
            v_event->>'type',
            v_event,
            v_metadata,
            v_timestamp
        )
        RETURNING id INTO v_event_id;

        v_event_ids := array_append(v_event_ids, v_event_id);
    END LOOP;

    -- ═══════════════════════════════════════════════════════════════════
    -- 6. NOTIFY SUBSCRIBERS (Real-time updates)
    -- ═══════════════════════════════════════════════════════════════════

    -- Notify via PostgreSQL LISTEN/NOTIFY
    PERFORM pg_notify('events', jsonb_build_object(
        'stream_id', v_stream_id,
        'version', v_new_version,
        'event_ids', to_jsonb(v_event_ids),
        'events', v_events,
        'correlation_id', v_correlation_id,
        'timestamp', v_timestamp
    )::text);

    -- ═══════════════════════════════════════════════════════════════════
    -- 7. RETURN SUCCESS
    -- ═══════════════════════════════════════════════════════════════════

    RETURN jsonb_build_object(
        'success', true,
        'stream_id', v_stream_id,
        'version', v_new_version,
        'event_ids', to_jsonb(v_event_ids),
        'events', v_events,
        'correlation_id', v_correlation_id
    );

EXCEPTION
    -- Handle optimistic concurrency conflict
    WHEN unique_violation THEN
        RETURN jsonb_build_object(
            'success', false,
            'error', 'VersionConflict',
            'message', 'Concurrent modification detected. Please retry.',
            'correlation_id', v_correlation_id
        );

    -- Handle other database errors
    WHEN OTHERS THEN
        RETURN jsonb_build_object(
            'success', false,
            'error', 'DatabaseError',
            'message', SQLERRM,
            'correlation_id', v_correlation_id
        );
END;
$$;

COMMENT ON FUNCTION order_handle(JSONB) IS
'Imperative shell for Order aggregate. TEMPLATE-GENERATED.
Handles command processing, event persistence, and notifications.';
```

### 4.3 Query Functions

For read operations on projections:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_GET: Query single order from projection
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_get(
    p_order_id TEXT
) RETURNS JSONB
LANGUAGE plpgsql STABLE AS $$
BEGIN
    RETURN (
        SELECT jsonb_build_object(
            'order_id', order_id,
            'status', status,
            'customer_id', customer_id,
            'total_amount', total_amount,
            'item_count', item_count,
            'items', items,
            'created_at', created_at,
            'updated_at', updated_at
        )
        FROM orders_projection
        WHERE order_id = p_order_id
    );
END;
$$;

-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_LIST: Query orders with filtering and pagination
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_list(
    p_customer_id TEXT DEFAULT NULL,
    p_status TEXT DEFAULT NULL,
    p_limit INTEGER DEFAULT 20,
    p_offset INTEGER DEFAULT 0
) RETURNS JSONB
LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_orders JSONB;
    v_total INTEGER;
BEGIN
    -- Get filtered orders
    SELECT jsonb_agg(
        jsonb_build_object(
            'order_id', order_id,
            'status', status,
            'customer_id', customer_id,
            'total_amount', total_amount,
            'item_count', item_count,
            'created_at', created_at
        ) ORDER BY created_at DESC
    )
    INTO v_orders
    FROM (
        SELECT *
        FROM orders_projection
        WHERE (p_customer_id IS NULL OR customer_id = p_customer_id)
          AND (p_status IS NULL OR status = p_status)
        ORDER BY created_at DESC
        LIMIT p_limit
        OFFSET p_offset
    ) sub;

    -- Get total count
    SELECT COUNT(*)
    INTO v_total
    FROM orders_projection
    WHERE (p_customer_id IS NULL OR customer_id = p_customer_id)
      AND (p_status IS NULL OR status = p_status);

    RETURN jsonb_build_object(
        'items', COALESCE(v_orders, '[]'::jsonb),
        'total', v_total,
        'limit', p_limit,
        'offset', p_offset
    );
END;
$$;
```

---

## 5. Testing Pure Functions

### 5.1 Testing Without Tables

Pure functions can be tested with simple `DO` blocks or `SELECT` statements, requiring no database schema:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER AGGREGATE TESTS
--
-- These tests run without any tables. They only test the pure functions.
-- Run these BEFORE deploying to verify business logic is correct.
-- ═══════════════════════════════════════════════════════════════════════════

DO $$
DECLARE
    v_result JSONB;
    v_state JSONB;
    v_test_count INTEGER := 0;
    v_pass_count INTEGER := 0;
BEGIN
    RAISE NOTICE '══════════════════════════════════════════════════════════';
    RAISE NOTICE 'ORDER AGGREGATE TESTS';
    RAISE NOTICE '══════════════════════════════════════════════════════════';

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST: CreateOrder on empty state
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_result := order_process(
        '{}'::jsonb,
        '{"type": "CreateOrder", "order_id": "123", "customer_id": "cust-1"}'::jsonb
    );

    IF v_result ? 'events'
       AND jsonb_array_length(v_result->'events') = 1
       AND v_result->'events'->0->>'type' = 'OrderCreated'
    THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 1: CreateOrder on empty state';
    ELSE
        RAISE NOTICE '✗ TEST 1: CreateOrder on empty state - FAILED';
        RAISE NOTICE '  Expected: events with OrderCreated';
        RAISE NOTICE '  Got: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST: CreateOrder on existing order (should fail)
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_result := order_process(
        '{"status": "pending"}'::jsonb,
        '{"type": "CreateOrder", "order_id": "123", "customer_id": "cust-1"}'::jsonb
    );

    IF v_result->>'error' = 'OrderAlreadyExists' THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 2: CreateOrder on existing order returns error';
    ELSE
        RAISE NOTICE '✗ TEST 2: CreateOrder on existing order - FAILED';
        RAISE NOTICE '  Expected: error = OrderAlreadyExists';
        RAISE NOTICE '  Got: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST: AddItem to pending order
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_state := '{"status": "pending", "items": []}'::jsonb;
    v_result := order_process(
        v_state,
        '{"type": "AddItem", "item": {"product_id": "prod-1", "quantity": 2, "unit_price": 10.00}}'::jsonb
    );

    IF v_result ? 'events'
       AND v_result->'events'->0->>'type' = 'ItemAdded'
       AND (v_result->'events'->0->>'quantity')::integer = 2
    THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 3: AddItem to pending order';
    ELSE
        RAISE NOTICE '✗ TEST 3: AddItem to pending order - FAILED';
        RAISE NOTICE '  Got: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST: AddItem to non-existent order (should fail)
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_result := order_process(
        '{}'::jsonb,
        '{"type": "AddItem", "item": {"product_id": "prod-1", "quantity": 2}}'::jsonb
    );

    IF v_result->>'error' = 'OrderNotFound' THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 4: AddItem to non-existent order returns error';
    ELSE
        RAISE NOTICE '✗ TEST 4: AddItem to non-existent order - FAILED';
        RAISE NOTICE '  Expected: error = OrderNotFound';
        RAISE NOTICE '  Got: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST: AddItem with invalid quantity (should fail)
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_result := order_process(
        '{"status": "pending", "items": []}'::jsonb,
        '{"type": "AddItem", "item": {"product_id": "prod-1", "quantity": 0}}'::jsonb
    );

    IF v_result->>'error' = 'InvalidQuantity' THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 5: AddItem with zero quantity returns error';
    ELSE
        RAISE NOTICE '✗ TEST 5: AddItem with zero quantity - FAILED';
        RAISE NOTICE '  Expected: error = InvalidQuantity';
        RAISE NOTICE '  Got: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST: Apply OrderCreated event
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_state := order_apply(
        '{}'::jsonb,
        '{"type": "OrderCreated", "order_id": "123", "customer_id": "cust-1"}'::jsonb
    );

    IF v_state->>'status' = 'pending'
       AND v_state->>'order_id' = '123'
       AND v_state->>'customer_id' = 'cust-1'
    THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 6: Apply OrderCreated sets correct state';
    ELSE
        RAISE NOTICE '✗ TEST 6: Apply OrderCreated - FAILED';
        RAISE NOTICE '  Got: %', v_state;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST: Apply ItemAdded event
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_state := order_apply(
        '{"status": "pending", "items": [], "total_amount": 0}'::jsonb,
        '{"type": "ItemAdded", "product_id": "prod-1", "quantity": 2, "unit_price": 10.00}'::jsonb
    );

    IF jsonb_array_length(v_state->'items') = 1
       AND (v_state->>'total_amount')::decimal = 20.00
    THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 7: Apply ItemAdded updates items and total';
    ELSE
        RAISE NOTICE '✗ TEST 7: Apply ItemAdded - FAILED';
        RAISE NOTICE '  Got: %', v_state;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST: Full workflow - Create, Add Items, Submit
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;

    -- Start fresh
    v_state := '{}'::jsonb;

    -- Create order
    v_result := order_process(v_state,
        '{"type": "CreateOrder", "order_id": "456", "customer_id": "cust-2"}'::jsonb);
    v_state := order_apply(v_state, v_result->'events'->0);

    -- Add first item
    v_result := order_process(v_state,
        '{"type": "AddItem", "item": {"product_id": "prod-1", "quantity": 1, "unit_price": 25.00}}'::jsonb);
    v_state := order_apply(v_state, v_result->'events'->0);

    -- Add second item
    v_result := order_process(v_state,
        '{"type": "AddItem", "item": {"product_id": "prod-2", "quantity": 3, "unit_price": 10.00}}'::jsonb);
    v_state := order_apply(v_state, v_result->'events'->0);

    -- Submit order
    v_result := order_process(v_state,
        '{"type": "SubmitOrder", "timestamp": "2024-01-15T10:30:00Z"}'::jsonb);
    v_state := order_apply(v_state, v_result->'events'->0);

    IF v_state->>'status' = 'submitted'
       AND jsonb_array_length(v_state->'items') = 2
       AND (v_state->>'total_amount')::decimal = 55.00
    THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 8: Full workflow (Create → Add Items → Submit)';
    ELSE
        RAISE NOTICE '✗ TEST 8: Full workflow - FAILED';
        RAISE NOTICE '  Got: %', v_state;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST: Cannot submit empty order
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_result := order_process(
        '{"status": "pending", "items": []}'::jsonb,
        '{"type": "SubmitOrder"}'::jsonb
    );

    IF v_result->>'error' = 'EmptyOrder' THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 9: Cannot submit empty order';
    ELSE
        RAISE NOTICE '✗ TEST 9: Cannot submit empty order - FAILED';
        RAISE NOTICE '  Got: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST: Unknown command returns error
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_result := order_process(
        '{}'::jsonb,
        '{"type": "UnknownCommand"}'::jsonb
    );

    IF v_result->>'error' = 'UnknownCommand' THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 10: Unknown command returns error';
    ELSE
        RAISE NOTICE '✗ TEST 10: Unknown command - FAILED';
        RAISE NOTICE '  Got: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- SUMMARY
    -- ═══════════════════════════════════════════════════════════════════
    RAISE NOTICE '';
    RAISE NOTICE '══════════════════════════════════════════════════════════';
    RAISE NOTICE 'RESULTS: % / % tests passed', v_pass_count, v_test_count;
    RAISE NOTICE '══════════════════════════════════════════════════════════';

    IF v_pass_count < v_test_count THEN
        RAISE EXCEPTION 'Some tests failed!';
    END IF;
END;
$$;
```

### 5.2 Testing with pgTAP (Optional)

For more structured testing, use pgTAP:

```sql
-- Install pgTAP extension
CREATE EXTENSION IF NOT EXISTS pgtap;

-- Test suite
BEGIN;
SELECT plan(10);

-- Test 1: CreateOrder returns events
SELECT is(
    (order_process('{}'::jsonb,
        '{"type": "CreateOrder", "order_id": "123", "customer_id": "cust-1"}'::jsonb
    ) ? 'events')::boolean,
    true,
    'CreateOrder on empty state returns events'
);

-- Test 2: CreateOrder on existing order fails
SELECT is(
    order_process('{"status": "pending"}'::jsonb,
        '{"type": "CreateOrder", "order_id": "123", "customer_id": "cust-1"}'::jsonb
    )->>'error',
    'OrderAlreadyExists',
    'CreateOrder on existing order returns OrderAlreadyExists error'
);

-- ... more tests ...

SELECT * FROM finish();
ROLLBACK;
```

---

## 6. Projections

### 6.1 Trigger-Based Projections

Projections update automatically when events are inserted:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDERS PROJECTION FUNCTION
--
-- Called by trigger on events table insert.
-- Updates the orders_projection read model.
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION orders_project()
RETURNS TRIGGER AS $$
DECLARE
    v_order_id TEXT;
    v_item_price DECIMAL;
BEGIN
    -- Only process order-related events
    IF NEW.stream_id NOT LIKE 'order-%' THEN
        RETURN NEW;
    END IF;

    -- Extract order_id from stream_id
    v_order_id := SUBSTRING(NEW.stream_id FROM 7);  -- Remove 'order-' prefix

    CASE NEW.event_type

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCreated: Insert new projection row
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderCreated' THEN
        INSERT INTO orders_projection (
            order_id,
            status,
            customer_id,
            total_amount,
            item_count,
            items,
            created_at,
            updated_at,
            last_event_id
        ) VALUES (
            NEW.payload->>'order_id',
            'pending',
            NEW.payload->>'customer_id',
            0,
            0,
            '[]'::jsonb,
            NEW.created_at,
            NEW.created_at,
            NEW.id
        )
        ON CONFLICT (order_id) DO UPDATE SET
            status = 'pending',
            customer_id = EXCLUDED.customer_id,
            items = '[]'::jsonb,
            updated_at = NEW.created_at,
            last_event_id = NEW.id;

    -- ───────────────────────────────────────────────────────────────────
    -- ItemAdded: Increment item count, total, and append to items array
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'ItemAdded' THEN
        v_item_price := COALESCE(
            (NEW.payload->>'quantity')::integer *
            (NEW.payload->>'unit_price')::decimal,
            0
        );

        UPDATE orders_projection
        SET
            item_count = item_count + 1,
            total_amount = total_amount + v_item_price,
            items = items || jsonb_build_array(jsonb_build_object(
                'product_id', NEW.payload->>'product_id',
                'quantity', (NEW.payload->>'quantity')::integer,
                'unit_price', (NEW.payload->>'unit_price')::decimal
            )),
            updated_at = NEW.created_at,
            last_event_id = NEW.id
        WHERE order_id = v_order_id;

    -- ───────────────────────────────────────────────────────────────────
    -- Status changes
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderSubmitted' THEN
        UPDATE orders_projection
        SET
            status = 'submitted',
            updated_at = NEW.created_at,
            last_event_id = NEW.id
        WHERE order_id = v_order_id;

    WHEN 'OrderCancelled' THEN
        UPDATE orders_projection
        SET
            status = 'cancelled',
            updated_at = NEW.created_at,
            last_event_id = NEW.id
        WHERE order_id = v_order_id;

    WHEN 'OrderCompleted' THEN
        UPDATE orders_projection
        SET
            status = 'completed',
            updated_at = NEW.created_at,
            last_event_id = NEW.id
        WHERE order_id = v_order_id;

    ELSE
        -- Unknown event types are ignored
        NULL;

    END CASE;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ═══════════════════════════════════════════════════════════════════════════
-- PROJECTION TRIGGER
-- ═══════════════════════════════════════════════════════════════════════════

DROP TRIGGER IF EXISTS orders_projection_trigger ON events;

CREATE TRIGGER orders_projection_trigger
    AFTER INSERT ON events
    FOR EACH ROW
    WHEN (NEW.stream_id LIKE 'order-%')
    EXECUTE FUNCTION orders_project();

COMMENT ON TRIGGER orders_projection_trigger ON events IS
'Updates orders_projection when order events are inserted.';
```

### 6.2 Rebuilding Projections

If projections get out of sync, rebuild them:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- REBUILD ORDERS PROJECTION
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION orders_rebuild_projection()
RETURNS INTEGER
LANGUAGE plpgsql AS $$
DECLARE
    v_count INTEGER := 0;
    v_stream RECORD;
    v_state JSONB;
    v_last_event RECORD;
BEGIN
    -- Truncate projection
    TRUNCATE orders_projection;

    -- Rebuild from events
    FOR v_stream IN
        SELECT DISTINCT stream_id
        FROM events
        WHERE stream_id LIKE 'order-%'
        ORDER BY stream_id
    LOOP
        -- Fold events to get current state
        v_state := order_fold_events(v_stream.stream_id);

        -- Get last event for this stream
        SELECT id, created_at
        INTO v_last_event
        FROM events
        WHERE stream_id = v_stream.stream_id
        ORDER BY version DESC
        LIMIT 1;

        -- Insert projection row
        INSERT INTO orders_projection (
            order_id,
            status,
            customer_id,
            total_amount,
            item_count,
            items,
            created_at,
            updated_at,
            last_event_id
        ) VALUES (
            v_state->>'order_id',
            v_state->>'status',
            v_state->>'customer_id',
            COALESCE((v_state->>'total_amount')::decimal, 0),
            COALESCE(jsonb_array_length(v_state->'items'), 0),
            COALESCE(v_state->'items', '[]'::jsonb),
            (SELECT MIN(created_at) FROM events WHERE stream_id = v_stream.stream_id),
            v_last_event.created_at,
            v_last_event.id
        );

        v_count := v_count + 1;
    END LOOP;

    RETURN v_count;
END;
$$;

-- Usage:
-- SELECT orders_rebuild_projection();
```

---

## 7. Real-Time Events

### 7.1 PostgreSQL LISTEN/NOTIFY

PostgreSQL's built-in pub/sub replaces external event buses:

```sql
-- Events are notified from order_handle():
PERFORM pg_notify('events', jsonb_build_object(
    'stream_id', v_stream_id,
    'version', v_new_version,
    'event_ids', to_jsonb(v_event_ids),
    'events', v_events,
    'correlation_id', v_correlation_id,
    'timestamp', v_timestamp
)::text);
```

### 7.2 Rust Listener

```rust
use futures_util::StreamExt;
use tokio_postgres::{AsyncMessage, NoTls};

/// Listen for events from PostgreSQL NOTIFY
pub async fn listen_for_events(
    connection_string: &str,
    handler: impl Fn(EventNotification) -> BoxFuture<'static, Result<(), Error>> + Send + Sync,
) -> Result<(), Error> {
    // Connect to PostgreSQL
    let (client, mut connection) = tokio_postgres::connect(connection_string, NoTls).await?;

    // Spawn connection handler
    let connection_handle = tokio::spawn(async move {
        while let Some(msg) = std::future::poll_fn(|cx| connection.poll_message(cx)).await {
            match msg {
                Ok(AsyncMessage::Notification(notification)) => {
                    // Parse notification payload
                    let payload: serde_json::Value =
                        serde_json::from_str(notification.payload())?;

                    let event = EventNotification {
                        stream_id: payload["stream_id"].as_str().unwrap().to_string(),
                        version: payload["version"].as_i64().unwrap() as i32,
                        events: payload["events"].clone(),
                        correlation_id: payload["correlation_id"].as_str().map(String::from),
                    };

                    // Call handler
                    if let Err(e) = handler(event).await {
                        tracing::error!("Event handler error: {}", e);
                    }
                }
                Ok(AsyncMessage::Notice(notice)) => {
                    tracing::debug!("PostgreSQL notice: {}", notice.message());
                }
                Err(e) => {
                    tracing::error!("Connection error: {}", e);
                    break;
                }
            }
        }
        Ok::<_, Error>(())
    });

    // Subscribe to events channel
    client.execute("LISTEN events", &[]).await?;

    // Keep connection alive
    connection_handle.await??;

    Ok(())
}

#[derive(Debug, Clone)]
pub struct EventNotification {
    pub stream_id: String,
    pub version: i32,
    pub events: serde_json::Value,
    pub correlation_id: Option<String>,
}
```

### 7.3 WebSocket Bridge

Forward events to WebSocket clients:

```rust
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use tokio::sync::broadcast;

// Broadcast channel for events
static EVENT_BROADCAST: LazyLock<broadcast::Sender<EventNotification>> =
    LazyLock::new(|| broadcast::channel(1024).0);

// Start PostgreSQL listener that broadcasts to WebSocket clients
pub async fn start_event_broadcaster(connection_string: &str) -> Result<(), Error> {
    let sender = EVENT_BROADCAST.clone();

    listen_for_events(connection_string, move |event| {
        let sender = sender.clone();
        async move {
            // Broadcast to all WebSocket clients
            let _ = sender.send(event);
            Ok(())
        }.boxed()
    }).await
}

// WebSocket handler
pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    let mut rx = EVENT_BROADCAST.subscribe();

    loop {
        tokio::select! {
            // Forward events to WebSocket client
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        let json = serde_json::to_string(&event).unwrap();
                        if socket.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            // Handle client messages (optional)
            Some(msg) = socket.recv() => {
                match msg {
                    Ok(Message::Close(_)) => break,
                    Err(_) => break,
                    _ => {}
                }
            }
        }
    }
}
```

---

## 8. Transactions Replace Sagas

### 8.1 The Insight

In the distributed architecture, sagas coordinate multiple aggregates with compensation:
- Each step may fail
- If step N fails, compensate steps 1..N-1
- Complex error handling, eventual consistency

With PostgreSQL, **everything is in one transaction**:
- Either all steps succeed, or all roll back
- ACID guarantees handle compensation automatically
- Immediate consistency

### 8.2 Multi-Aggregate Operations

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- CHECKOUT: Multi-aggregate operation in single transaction
--
-- This replaces a saga with compensation. The entire operation either:
-- - Succeeds: All aggregates updated atomically
-- - Fails: Entire transaction rolls back (automatic compensation)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION checkout_handle(
    command JSONB
) RETURNS JSONB
LANGUAGE plpgsql AS $$
DECLARE
    v_order_id TEXT;
    v_payment_id TEXT;
    v_order_result JSONB;
    v_payment_result JSONB;
    v_inventory_result JSONB;
    v_correlation_id TEXT;
BEGIN
    v_order_id := command->>'order_id';
    v_payment_id := 'pay-' || gen_random_uuid();
    v_correlation_id := COALESCE(
        command->'metadata'->>'correlation_id',
        gen_random_uuid()::text
    );

    -- ═══════════════════════════════════════════════════════════════════
    -- STEP 1: Validate and lock the order
    -- ═══════════════════════════════════════════════════════════════════

    -- Get current order state (with SELECT FOR UPDATE for pessimistic locking)
    PERFORM 1 FROM events
    WHERE stream_id = 'order-' || v_order_id
    FOR UPDATE;

    -- ═══════════════════════════════════════════════════════════════════
    -- STEP 2: Create Payment
    -- ═══════════════════════════════════════════════════════════════════

    v_payment_result := payment_handle(jsonb_build_object(
        'type', 'CreatePayment',
        'payment_id', v_payment_id,
        'order_id', v_order_id,
        'amount', command->>'amount',
        'payment_method', command->>'payment_method',
        'metadata', jsonb_build_object(
            'correlation_id', v_correlation_id
        )
    ));

    IF NOT (v_payment_result->>'success')::boolean THEN
        -- Payment failed - transaction will rollback
        RETURN jsonb_build_object(
            'success', false,
            'step', 'payment',
            'error', v_payment_result->>'error',
            'message', v_payment_result->>'message',
            'correlation_id', v_correlation_id
        );
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- STEP 3: Reserve Inventory
    -- ═══════════════════════════════════════════════════════════════════

    v_inventory_result := inventory_handle(jsonb_build_object(
        'type', 'ReserveItems',
        'order_id', v_order_id,
        'items', command->'items',
        'metadata', jsonb_build_object(
            'correlation_id', v_correlation_id
        )
    ));

    IF NOT (v_inventory_result->>'success')::boolean THEN
        -- Inventory reservation failed - transaction will rollback
        -- This automatically "compensates" the payment (it never happened)
        RETURN jsonb_build_object(
            'success', false,
            'step', 'inventory',
            'error', v_inventory_result->>'error',
            'message', v_inventory_result->>'message',
            'correlation_id', v_correlation_id
        );
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- STEP 4: Confirm Order
    -- ═══════════════════════════════════════════════════════════════════

    v_order_result := order_handle(jsonb_build_object(
        'type', 'ConfirmOrder',
        'order_id', v_order_id,
        'payment_id', v_payment_id,
        'metadata', jsonb_build_object(
            'correlation_id', v_correlation_id
        )
    ));

    IF NOT (v_order_result->>'success')::boolean THEN
        -- Order confirmation failed - everything rolls back
        RETURN jsonb_build_object(
            'success', false,
            'step', 'order',
            'error', v_order_result->>'error',
            'message', v_order_result->>'message',
            'correlation_id', v_correlation_id
        );
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- SUCCESS: All steps completed
    -- ═══════════════════════════════════════════════════════════════════

    RETURN jsonb_build_object(
        'success', true,
        'order_id', v_order_id,
        'payment_id', v_payment_id,
        'correlation_id', v_correlation_id,
        'order', v_order_result,
        'payment', v_payment_result,
        'inventory', v_inventory_result
    );

EXCEPTION
    WHEN OTHERS THEN
        -- Any unexpected error rolls back everything
        RETURN jsonb_build_object(
            'success', false,
            'error', 'CheckoutFailed',
            'message', SQLERRM,
            'correlation_id', v_correlation_id
        );
END;
$$;
```

### 8.3 When You Still Need Sagas

Some scenarios still require saga-like patterns even with PostgreSQL:

1. **External API calls**: If checkout must call Stripe API
2. **Cross-database operations**: If some data is in another database
3. **Long-running workflows**: If steps happen over hours/days

For these, use the outbox pattern + background worker:

```sql
-- Record intent in outbox (transactional with events)
INSERT INTO outbox (event_id, destination, payload)
VALUES (
    v_event_id,
    'stripe:charge',
    jsonb_build_object('amount', 100.00, 'customer_id', 'cust-123')
);

-- Background worker processes outbox:
-- 1. Lock row
-- 2. Call external API
-- 3. On success: delete from outbox, insert completion event
-- 4. On failure: increment attempts, exponential backoff
```

---

## 9. Rust Integration Layer

### 9.1 Thin HTTP Handler

The Rust layer becomes a thin translation layer:

```rust
use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use sqlx::PgPool;

/// Application state
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
}

/// Create an order
pub async fn create_order(
    State(state): State<AppState>,
    Json(request): Json<CreateOrderRequest>,
) -> impl IntoResponse {
    // Build command
    let command = serde_json::json!({
        "type": "CreateOrder",
        "order_id": request.order_id,
        "customer_id": request.customer_id,
        "metadata": {
            "user_id": request.user_id,
        }
    });

    // Call stored procedure
    let result: serde_json::Value = sqlx::query_scalar(
        "SELECT order_handle($1)"
    )
    .bind(&command)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Translate response
    if result["success"].as_bool() == Some(true) {
        Ok((StatusCode::CREATED, Json(result)))
    } else {
        // Map business errors to HTTP status codes
        let status = match result["error"].as_str() {
            Some("OrderAlreadyExists") => StatusCode::CONFLICT,
            Some("InvalidCommand") => StatusCode::BAD_REQUEST,
            Some("VersionConflict") => StatusCode::CONFLICT,
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        };
        Err((status, Json(result)))
    }
}

/// Add item to order
pub async fn add_item(
    State(state): State<AppState>,
    Path(order_id): Path<String>,
    Json(request): Json<AddItemRequest>,
) -> impl IntoResponse {
    let command = serde_json::json!({
        "type": "AddItem",
        "order_id": order_id,
        "item": {
            "product_id": request.product_id,
            "quantity": request.quantity,
            "unit_price": request.unit_price,
        }
    });

    let result: serde_json::Value = sqlx::query_scalar("SELECT order_handle($1)")
        .bind(&command)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result["success"].as_bool() == Some(true) {
        Ok((StatusCode::OK, Json(result)))
    } else {
        let status = match result["error"].as_str() {
            Some("OrderNotFound") => StatusCode::NOT_FOUND,
            Some("InvalidOrderStatus") => StatusCode::CONFLICT,
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        };
        Err((status, Json(result)))
    }
}

/// Get order from projection
pub async fn get_order(
    State(state): State<AppState>,
    Path(order_id): Path<String>,
) -> impl IntoResponse {
    let result: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT order_get($1)"
    )
    .bind(&order_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match result.flatten() {
        Some(order) => Ok((StatusCode::OK, Json(order))),
        None => Err((StatusCode::NOT_FOUND, "Order not found")),
    }
}

/// Router setup
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/orders", post(create_order))
        .route("/orders/:order_id", get(get_order))
        .route("/orders/:order_id/items", post(add_item))
}
```

### 9.2 Request/Response Types

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CreateOrderRequest {
    pub order_id: String,
    pub customer_id: String,
    pub user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddItemRequest {
    pub product_id: String,
    pub quantity: i32,
    pub unit_price: f64,
}

#[derive(Debug, Serialize)]
pub struct OrderResponse {
    pub order_id: String,
    pub status: String,
    pub customer_id: String,
    pub total_amount: f64,
    pub item_count: i32,
    pub items: Vec<OrderItem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrderItem {
    pub product_id: String,
    pub quantity: i32,
    pub unit_price: f64,
}
```

---

## 10. Code Generation Pipeline

### 10.1 Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    YAML Business Logic                          │
│                                                                  │
│  aggregates:                                                    │
│    order:                                                       │
│      state:                                                     │
│        - status: pending | submitted | cancelled | completed   │
│        - items: Item[]                                          │
│        - total_amount: decimal                                  │
│                                                                  │
│      commands:                                                  │
│        CreateOrder:                                             │
│          requires: []                                           │
│          guards:                                                │
│            - status IS NULL                                     │
│          produces: OrderCreated                                 │
│                                                                  │
│      events:                                                    │
│        OrderCreated:                                            │
│          applies:                                               │
│            status: pending                                      │
│            items: []                                            │
│            total_amount: 0                                      │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    AI Code Generator                            │
│                                                                  │
│  1. Parse YAML                                                  │
│  2. Generate pure functions:                                    │
│     - {aggregate}_process()  ─────► IMMUTABLE PL/pgSQL         │
│     - {aggregate}_apply()    ─────► IMMUTABLE PL/pgSQL         │
│  3. Test pure functions (DO blocks, no tables)                 │
│  4. On test success: generate imperative shell                 │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                   Template Generator                            │
│                                                                  │
│  Templates (never AI-generated):                                │
│  - {aggregate}_handle()     ─────► VOLATILE PL/pgSQL           │
│  - {aggregate}_load_state()  ────► STABLE PL/pgSQL (O(1))      │
│  - {aggregate}_fold_events() ────► STABLE PL/pgSQL (rebuilds)  │
│  - {aggregate}_project()    ─────► Trigger function            │
│  - Projection table DDL                                         │
│  - Rust HTTP handlers                                           │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Deployable Artifacts                         │
│                                                                  │
│  PostgreSQL:                                                    │
│  - migrations/001_events.sql        (Schema)                   │
│  - migrations/002_order.sql         (Pure + Shell + Projection)│
│                                                                  │
│  Rust:                                                          │
│  - src/handlers/order.rs            (HTTP handlers)            │
│  - src/main.rs                      (Router setup)             │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 10.2 YAML Schema

```yaml
# business-domain.yaml
version: "1.0"

aggregates:
  order:
    # State shape
    state:
      order_id: string
      customer_id: string
      status:
        enum: [pending, submitted, cancelled, completed]
      items:
        type: array
        items:
          product_id: string
          quantity: integer
          unit_price: decimal
      total_amount: decimal

    # Commands (write operations)
    commands:
      CreateOrder:
        fields:
          order_id: { type: string, required: true }
          customer_id: { type: string, required: true }
        guards:
          - condition: "status IS NULL"
            error: OrderAlreadyExists
        produces:
          - type: OrderCreated
            payload:
              order_id: "{{command.order_id}}"
              customer_id: "{{command.customer_id}}"

      AddItem:
        fields:
          item:
            product_id: { type: string, required: true }
            quantity: { type: integer, min: 1 }
            unit_price: { type: decimal }
        guards:
          - condition: "status IS NOT NULL"
            error: OrderNotFound
            message: "Cannot add item to non-existent order"
          - condition: "status = 'pending'"
            error: InvalidOrderStatus
            message: "Cannot add items to order with status: {{state.status}}"
          - condition: "jsonb_array_length(items) < 100"
            error: MaxItemsExceeded
        produces:
          - type: ItemAdded
            payload:
              product_id: "{{command.item.product_id}}"
              quantity: "{{command.item.quantity}}"
              unit_price: "{{command.item.unit_price}}"

      SubmitOrder:
        guards:
          - condition: "status = 'pending'"
            error: InvalidOrderStatus
          - condition: "jsonb_array_length(items) > 0"
            error: EmptyOrder
        produces:
          - type: OrderSubmitted
            payload:
              submitted_at: "{{timestamp}}"

      CancelOrder:
        fields:
          reason: { type: string, optional: true }
        guards:
          - condition: "status NOT IN ('completed', 'cancelled')"
            error: InvalidOrderStatus
        produces:
          - type: OrderCancelled
            payload:
              reason: "{{command.reason | default: 'No reason provided'}}"

    # Events (state transitions)
    events:
      OrderCreated:
        applies:
          order_id: "{{event.order_id}}"
          customer_id: "{{event.customer_id}}"
          status: pending
          items: []
          total_amount: 0

      ItemAdded:
        applies:
          items: "{{state.items | append: event_item}}"
          total_amount: "{{state.total_amount + (event.quantity * event.unit_price)}}"

      OrderSubmitted:
        applies:
          status: submitted
          submitted_at: "{{event.submitted_at}}"

      OrderCancelled:
        applies:
          status: cancelled
          cancelled_reason: "{{event.reason}}"

      OrderCompleted:
        applies:
          status: completed
          completed_at: "{{event.completed_at}}"

    # Projection (read model)
    projection:
      table: orders_projection
      primary_key: order_id
      columns:
        order_id: string
        status: string
        customer_id: string
        total_amount: decimal
        item_count: integer
        items: jsonb  # Array of item objects for business logic validation
        created_at: timestamp
        updated_at: timestamp

      handlers:
        OrderCreated:
          insert:
            order_id: "{{event.order_id}}"
            status: pending
            customer_id: "{{event.customer_id}}"
            total_amount: 0
            item_count: 0
            items: "[]"
            created_at: "{{event_timestamp}}"
            updated_at: "{{event_timestamp}}"

        ItemAdded:
          update:
            where: { order_id: "{{stream_id | remove_prefix: 'order-'}}" }
            set:
              item_count: "{{item_count + 1}}"
              total_amount: "{{total_amount + (event.quantity * event.unit_price)}}"
              items: "{{items | append: {product_id: event.product_id, quantity: event.quantity, unit_price: event.unit_price}}}"
              updated_at: "{{event_timestamp}}"

        OrderSubmitted:
          update:
            where: { order_id: "{{stream_id | remove_prefix: 'order-'}}" }
            set:
              status: submitted
              updated_at: "{{event_timestamp}}"

    # Queries (read operations)
    queries:
      get:
        returns: single
        sql: "SELECT * FROM orders_projection WHERE order_id = $1"

      list:
        returns: paginated
        parameters:
          customer_id: { type: string, optional: true }
          status: { type: string, optional: true }
        sql: |
          SELECT * FROM orders_projection
          WHERE ($1 IS NULL OR customer_id = $1)
            AND ($2 IS NULL OR status = $2)
          ORDER BY created_at DESC

# Multi-aggregate workflows
workflows:
  checkout:
    steps:
      - aggregate: payment
        command: CreatePayment
        on_error: abort

      - aggregate: inventory
        command: ReserveItems
        on_error: abort

      - aggregate: order
        command: ConfirmOrder
        on_error: abort
```

### 10.3 Generation Targets

The same YAML compiles to different targets:

```yaml
# generator-config.yaml
targets:
  # PostgreSQL monolith (this spec)
  postgres_monolith:
    output: ./generated/postgres
    features:
      - pure_functions      # {agg}_process, {agg}_apply
      - imperative_shell    # {agg}_handle, {agg}_fold_events
      - projections         # Triggers
      - rust_handlers       # Thin HTTP layer

  # Distributed (Composable Rust)
  distributed:
    output: ./generated/distributed
    features:
      - rust_reducers       # Rust Reducer implementations
      - postgres_events     # PostgresEventStore
      - redpanda_bus        # RedpandaEventBus
      - axum_handlers       # Full Axum handlers
```

---

## 11. Migration from Distributed Architecture

### 11.1 Compatibility Layer

If migrating from the distributed architecture:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- COMPATIBILITY: Match existing events table schema
-- ═══════════════════════════════════════════════════════════════════════════

-- If using bincode in distributed version, add helper to convert:
CREATE OR REPLACE FUNCTION convert_bincode_to_jsonb(data BYTEA)
RETURNS JSONB
LANGUAGE plpgsql AS $$
BEGIN
    -- This would need a Rust extension or external converter
    -- For now, new events are JSONB, old events need migration
    RAISE EXCEPTION 'Bincode conversion not implemented';
END;
$$;

-- Migration: Convert existing events to JSONB format
-- Run this as a one-time migration when switching architectures
CREATE OR REPLACE FUNCTION migrate_events_to_jsonb()
RETURNS INTEGER
LANGUAGE plpgsql AS $$
DECLARE
    v_count INTEGER := 0;
BEGIN
    -- Create new jsonb payload column
    ALTER TABLE events ADD COLUMN IF NOT EXISTS payload_jsonb JSONB;

    -- Convert (implementation depends on your bincode serialization)
    -- This is a placeholder - real implementation needs Rust helper

    -- Swap columns
    ALTER TABLE events RENAME COLUMN payload TO payload_bincode;
    ALTER TABLE events RENAME COLUMN payload_jsonb TO payload;

    RETURN v_count;
END;
$$;
```

### 11.2 Gradual Migration

1. **Phase 1**: Deploy PostgreSQL functions alongside Rust handlers
2. **Phase 2**: Route new aggregates to PostgreSQL, old to Rust
3. **Phase 3**: Migrate existing aggregates one by one
4. **Phase 4**: Remove Rust reducer layer, keep HTTP handlers

---

## 12. Performance Considerations

### 12.1 Projections ARE the Optimization

**Snapshots are unnecessary.** The projection table serves the same purpose:

| Traditional Approach | Our Approach |
|---------------------|--------------|
| Fold 1000 events → O(n) | Query projection → O(1) |
| Need snapshots every N events | Projection always current |
| Snapshot maintenance overhead | Zero maintenance |
| Rebuild snapshots on schema change | Just rebuild projection |

The projection is updated synchronously in the same transaction as event
insertion. When `{aggregate}_handle()` returns, the projection row is already
updated. The next command queries it directly.

```sql
-- This replaces snapshots entirely:
SELECT * FROM orders_projection WHERE order_id = $1;

-- No need for snapshot tables, snapshot triggers, or snapshot management.
-- The projection IS the snapshot, updated in real-time.
```

**For projection rebuilds** (schema changes, disaster recovery), use
`{aggregate}_fold_events()` to reconstruct state from events. But this
is an offline operation, not in the hot path.

### 12.2 Connection Pooling

```rust
use sqlx::postgres::PgPoolOptions;

pub async fn create_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(20)
        .min_connections(5)
        .acquire_timeout(Duration::from_secs(3))
        .idle_timeout(Duration::from_secs(600))
        .max_lifetime(Duration::from_secs(1800))
        .connect(database_url)
        .await
}
```

### 12.3 Query Optimization

```sql
-- Partial indexes for common queries
CREATE INDEX idx_orders_pending ON orders_projection (created_at DESC)
    WHERE status = 'pending';

CREATE INDEX idx_orders_customer_recent ON orders_projection (customer_id, created_at DESC)
    WHERE status IN ('pending', 'submitted');

-- Use covering indexes
CREATE INDEX idx_orders_list ON orders_projection (customer_id, status, created_at DESC)
    INCLUDE (order_id, total_amount, item_count);
```

---

## 13. Appendix: Complete Example

### 13.1 Full Migration Script

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- COMPLETE ORDER AGGREGATE SETUP
-- Run this to set up the entire Order aggregate
-- ═══════════════════════════════════════════════════════════════════════════

-- 1. Core Events Table (if not exists)
CREATE TABLE IF NOT EXISTS events (
    id              BIGSERIAL PRIMARY KEY,
    stream_id       TEXT NOT NULL,
    version         INTEGER NOT NULL,
    event_type      TEXT NOT NULL,
    payload         JSONB NOT NULL,
    metadata        JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT events_stream_version_unique UNIQUE (stream_id, version)
);

CREATE INDEX IF NOT EXISTS idx_events_stream_id ON events (stream_id, version);
CREATE INDEX IF NOT EXISTS idx_events_event_type ON events (event_type);

-- 2. Projection Table
CREATE TABLE IF NOT EXISTS orders_projection (
    order_id        TEXT PRIMARY KEY,
    status          TEXT NOT NULL,
    customer_id     TEXT NOT NULL,
    total_amount    DECIMAL(12, 2) NOT NULL DEFAULT 0,
    item_count      INTEGER NOT NULL DEFAULT 0,
    items           JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL,
    last_event_id   BIGINT REFERENCES events(id)
);

CREATE INDEX IF NOT EXISTS idx_orders_projection_status ON orders_projection (status);
CREATE INDEX IF NOT EXISTS idx_orders_projection_customer ON orders_projection (customer_id);

-- 3. Pure Functions (order_process, order_apply) - see Section 4.1

-- 4. Shell Functions (order_handle, order_fold_events) - see Section 4.2

-- 5. Projection Trigger (orders_project) - see Section 6.1

-- 6. Query Functions (order_get, order_list) - see Section 4.3
```

### 13.2 Complete Test Suite

See Section 5.1 for the full test suite that can be run with:

```sql
-- Run all tests
DO $$ ... $$;

-- Or with pgTAP
SELECT * FROM runtests('order_%');
```

---

## Summary

This architecture provides:

1. **Same guarantees as distributed**: Pure/impure separation, testable business logic
2. **Simpler operations**: Single PostgreSQL database, no Redpanda/Kafka
3. **Immediate consistency**: Transactions replace eventual consistency
4. **Real-time updates**: LISTEN/NOTIFY replaces event bus
5. **AI-friendly code generation**: PL/pgSQL has abundant training data
6. **Incremental adoption**: Can coexist with distributed architecture during migration

The key insight is that PostgreSQL's `IMMUTABLE` functions provide the same purity guarantees as Rust's pure functions, enabling the same testing and verification workflow.
