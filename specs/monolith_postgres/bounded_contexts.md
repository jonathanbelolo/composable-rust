# Bounded Contexts in PostgreSQL Monolith

> **Extends**: `architecture_fully_typed.md`
>
> This document describes how to implement DDD bounded contexts within a
> PostgreSQL monolith, providing natural isolation while maintaining the
> benefits of a single database.

---

## 1. Two-Layer Event Architecture

### 1.1 The Insight: Global Log + Typed Context Views

We want BOTH:
- **Global event log**: Single source of truth, complete history, JSONB for flexibility
- **Context event tables**: Narrow, typed, efficient for context-specific queries

The solution is a **two-layer architecture**:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         GLOBAL EVENT LOG                                 │
│                    (Single source of truth)                              │
│                                                                          │
│  global.event_log                                                        │
│  ├── id: BIGSERIAL (global ordering)                                     │
│  ├── stream_id: TEXT                                                     │
│  ├── version: INTEGER                                                    │
│  ├── context: TEXT ('sales', 'inventory', 'shipping')                   │
│  ├── event_type: TEXT                                                    │
│  ├── payload: JSONB (opaque, complete event data)                       │
│  ├── metadata: JSONB (correlation, causation, actor)                    │
│  └── created_at: TIMESTAMPTZ                                             │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    │ Triggers project to typed tables
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                      TYPED CONTEXT EVENTS                                │
│               (Narrow, efficient, strongly typed)                        │
│                                                                          │
│  sales.events              inventory.events         shipping.events      │
│  ├── id: FK→global         ├── id: FK→global        ├── id: FK→global   │
│  ├── order_id              ├── product_id           ├── shipment_id     │
│  ├── customer_id           ├── warehouse_id         ├── carrier         │
│  ├── quantity              ├── quantity             ├── tracking_number │
│  └── ... (typed)           └── ... (typed)          └── ... (typed)     │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    │ Triggers update projections
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         PROJECTIONS                                      │
│                    (Read models per context)                             │
│                                                                          │
│  sales.orders_projection   inventory.stock_projection  shipping.shipments│
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### 1.2 Benefits of Two-Layer Architecture

| Benefit | Description |
|---------|-------------|
| **Single source of truth** | Global log is THE event store; everything derives from it |
| **Global ordering** | `global.event_log.id` provides total ordering across all contexts |
| **Complete audit trail** | One table to query for "what happened" |
| **Type safety** | Context tables provide typed columns for efficient queries |
| **Rebuild capability** | Can rebuild any context table from global log |
| **Minimal duplication** | Context tables are narrow; only denormalize needed fields |
| **Cross-cutting queries** | Analytics across all events without joining contexts |

### 1.3 Trade-offs

| Trade-off | Mitigation |
|-----------|------------|
| **Storage overhead** | Context tables are narrow; global log is the "fat" table |
| **Write amplification** | Single INSERT to global triggers typed INSERT; acceptable |
| **Two places to query** | Clear semantics: global for audit, context for business |

---

## 2. Global Event Log Schema

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- GLOBAL SCHEMA
-- The single source of truth for all events across all bounded contexts
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA global;

-- ───────────────────────────────────────────────────────────────────────────
-- Event Log: Append-only, immutable, complete history
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE global.event_log (
    -- Identity (global ordering)
    id              BIGSERIAL PRIMARY KEY,

    -- Stream identity
    stream_id       TEXT NOT NULL,
    version         INTEGER NOT NULL,

    -- Context routing
    context         TEXT NOT NULL,          -- 'sales', 'inventory', 'shipping'
    aggregate_type  TEXT NOT NULL,          -- 'order', 'stock', 'shipment'

    -- Event identity
    event_type      TEXT NOT NULL,

    -- Event data (opaque JSONB - complete event)
    payload         JSONB NOT NULL,

    -- Metadata (also JSONB for flexibility)
    metadata        JSONB NOT NULL DEFAULT '{}',

    -- Timestamps
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Constraints
    UNIQUE (stream_id, version)
);

-- Metadata typically contains:
COMMENT ON COLUMN global.event_log.metadata IS
'Standard metadata fields:
  - correlation_id: TEXT - Request correlation
  - causation_id: TEXT - Causing event ID
  - actor_id: TEXT - User/system that caused the event
  - actor_type: TEXT - "user", "system", "saga"
  - timestamp: TIMESTAMPTZ - When the event occurred (may differ from created_at)
  - schema_version: INTEGER - For event versioning
';

-- ───────────────────────────────────────────────────────────────────────────
-- Indexes for common access patterns
-- ───────────────────────────────────────────────────────────────────────────

-- Primary access: load events for a stream (used by aggregates)
CREATE INDEX idx_global_events_stream
    ON global.event_log (stream_id, version);

-- Context-based queries (used by context projectors)
CREATE INDEX idx_global_events_context
    ON global.event_log (context, id);

-- Event type queries (used by event handlers)
CREATE INDEX idx_global_events_type
    ON global.event_log (event_type, id);

-- Time-based queries (used by audit, analytics)
CREATE INDEX idx_global_events_created
    ON global.event_log (created_at);

-- Correlation tracking (used by debugging, saga tracking)
CREATE INDEX idx_global_events_correlation
    ON global.event_log ((metadata->>'correlation_id'))
    WHERE metadata->>'correlation_id' IS NOT NULL;

-- ───────────────────────────────────────────────────────────────────────────
-- Aggregate type index for cross-context queries
-- ───────────────────────────────────────────────────────────────────────────

CREATE INDEX idx_global_events_aggregate
    ON global.event_log (aggregate_type, id);
```

### 2.1 Global Log Helper Functions

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- Global Event Log Functions
-- ═══════════════════════════════════════════════════════════════════════════

-- Append an event to the global log
CREATE OR REPLACE FUNCTION global.append_event(
    p_stream_id TEXT,
    p_version INTEGER,
    p_context TEXT,
    p_aggregate_type TEXT,
    p_event_type TEXT,
    p_payload JSONB,
    p_metadata JSONB DEFAULT '{}'
)
RETURNS BIGINT
LANGUAGE plpgsql AS $$
DECLARE
    v_event_id BIGINT;
BEGIN
    INSERT INTO global.event_log (
        stream_id, version, context, aggregate_type,
        event_type, payload, metadata
    ) VALUES (
        p_stream_id, p_version, p_context, p_aggregate_type,
        p_event_type, p_payload, p_metadata
    )
    RETURNING id INTO v_event_id;

    RETURN v_event_id;
END;
$$;

-- Load events for a stream (generic, works across contexts)
CREATE OR REPLACE FUNCTION global.load_stream(
    p_stream_id TEXT,
    p_from_version INTEGER DEFAULT 0
)
RETURNS TABLE (
    id          BIGINT,
    version     INTEGER,
    event_type  TEXT,
    payload     JSONB,
    metadata    JSONB,
    created_at  TIMESTAMPTZ
)
LANGUAGE sql STABLE AS $$
    SELECT id, version, event_type, payload, metadata, created_at
    FROM global.event_log
    WHERE stream_id = p_stream_id
      AND version > p_from_version
    ORDER BY version;
$$;

-- Get all events in global order (for replay, analytics)
CREATE OR REPLACE FUNCTION global.get_all_events(
    p_from_id BIGINT DEFAULT 0,
    p_limit INTEGER DEFAULT 1000,
    p_context TEXT DEFAULT NULL,
    p_event_type TEXT DEFAULT NULL
)
RETURNS TABLE (
    id              BIGINT,
    stream_id       TEXT,
    version         INTEGER,
    context         TEXT,
    aggregate_type  TEXT,
    event_type      TEXT,
    payload         JSONB,
    metadata        JSONB,
    created_at      TIMESTAMPTZ
)
LANGUAGE sql STABLE AS $$
    SELECT id, stream_id, version, context, aggregate_type,
           event_type, payload, metadata, created_at
    FROM global.event_log
    WHERE id > p_from_id
      AND (p_context IS NULL OR context = p_context)
      AND (p_event_type IS NULL OR event_type = p_event_type)
    ORDER BY id
    LIMIT p_limit;
$$;

-- Get event count by context (for monitoring)
CREATE OR REPLACE FUNCTION global.event_stats()
RETURNS TABLE (
    context         TEXT,
    aggregate_type  TEXT,
    event_type      TEXT,
    event_count     BIGINT,
    last_event_at   TIMESTAMPTZ
)
LANGUAGE sql STABLE AS $$
    SELECT
        context,
        aggregate_type,
        event_type,
        COUNT(*) as event_count,
        MAX(created_at) as last_event_at
    FROM global.event_log
    GROUP BY context, aggregate_type, event_type
    ORDER BY context, aggregate_type, event_type;
$$;
```

---

## 3. Context Event Tables (Typed Projections)

Each bounded context has a typed events table that references the global log.
These are **projections** of the global log, not independent stores.

### 3.1 Sales Context Events

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- SALES CONTEXT: Typed Events (projected from global.event_log)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA IF NOT EXISTS sales;

-- Context-specific types
CREATE TYPE sales.order_status AS ENUM (
    'pending', 'submitted', 'confirmed', 'cancelled', 'completed'
);

-- Typed events table (narrow, efficient)
CREATE TABLE sales.events (
    -- Foreign key to global log (this IS the event ID)
    global_event_id     BIGINT PRIMARY KEY REFERENCES global.event_log(id),

    -- Stream identity (denormalized for efficient queries)
    stream_id           TEXT NOT NULL,
    version             INTEGER NOT NULL,
    event_type          TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL,

    -- ───────────────────────────────────────────────────────────────────
    -- Typed columns (extracted from global payload)
    -- ───────────────────────────────────────────────────────────────────

    -- OrderCreated / common fields
    order_id            TEXT,
    customer_id         TEXT,

    -- ItemAdded fields
    product_id          TEXT,
    product_name        TEXT,
    quantity            INTEGER CHECK (quantity IS NULL OR quantity >= 1),
    unit_price          DECIMAL(12, 2) CHECK (unit_price IS NULL OR unit_price >= 0),

    -- OrderSubmitted
    submitted_at        TIMESTAMPTZ,

    -- OrderCancelled
    reason              TEXT,

    -- OrderCompleted
    completed_at        TIMESTAMPTZ,

    -- Constraints
    UNIQUE (stream_id, version)
);

-- Indexes for context-specific queries
CREATE INDEX idx_sales_events_stream ON sales.events (stream_id, version);
CREATE INDEX idx_sales_events_order ON sales.events (order_id) WHERE order_id IS NOT NULL;
CREATE INDEX idx_sales_events_customer ON sales.events (customer_id) WHERE customer_id IS NOT NULL;
CREATE INDEX idx_sales_events_type ON sales.events (event_type, created_at);
```

### 3.2 Projection Trigger (Global → Context)

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- SALES: Project from Global Log to Typed Events
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION sales.project_from_global()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    -- Only process sales context events
    IF NEW.context != 'sales' THEN
        RETURN NEW;
    END IF;

    -- Extract typed fields from JSONB payload
    INSERT INTO sales.events (
        global_event_id,
        stream_id,
        version,
        event_type,
        created_at,
        -- Typed fields extracted from payload
        order_id,
        customer_id,
        product_id,
        product_name,
        quantity,
        unit_price,
        submitted_at,
        reason,
        completed_at
    ) VALUES (
        NEW.id,
        NEW.stream_id,
        NEW.version,
        NEW.event_type,
        NEW.created_at,
        -- Extract from JSONB
        NEW.payload->>'order_id',
        NEW.payload->>'customer_id',
        NEW.payload->>'product_id',
        NEW.payload->>'product_name',
        (NEW.payload->>'quantity')::INTEGER,
        (NEW.payload->>'unit_price')::DECIMAL(12,2),
        (NEW.payload->>'submitted_at')::TIMESTAMPTZ,
        NEW.payload->>'reason',
        (NEW.payload->>'completed_at')::TIMESTAMPTZ
    );

    RETURN NEW;
END;
$$;

CREATE TRIGGER sales_project_global
    AFTER INSERT ON global.event_log
    FOR EACH ROW
    WHEN (NEW.context = 'sales')
    EXECUTE FUNCTION sales.project_from_global();
```

### 3.3 Inventory Context Events

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- INVENTORY CONTEXT: Typed Events
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA IF NOT EXISTS inventory;

CREATE TYPE inventory.reservation_status AS ENUM (
    'pending', 'confirmed', 'released', 'fulfilled'
);

CREATE TABLE inventory.events (
    global_event_id     BIGINT PRIMARY KEY REFERENCES global.event_log(id),

    stream_id           TEXT NOT NULL,
    version             INTEGER NOT NULL,
    event_type          TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL,

    -- Typed columns
    product_id          TEXT,
    warehouse_id        TEXT,
    quantity            INTEGER,
    reservation_id      TEXT,
    order_id            TEXT,           -- Reference to sales context
    reason              TEXT,

    UNIQUE (stream_id, version)
);

CREATE INDEX idx_inventory_events_stream ON inventory.events (stream_id, version);
CREATE INDEX idx_inventory_events_product ON inventory.events (product_id);
CREATE INDEX idx_inventory_events_order ON inventory.events (order_id) WHERE order_id IS NOT NULL;

-- Projection trigger
CREATE OR REPLACE FUNCTION inventory.project_from_global()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.context != 'inventory' THEN
        RETURN NEW;
    END IF;

    INSERT INTO inventory.events (
        global_event_id, stream_id, version, event_type, created_at,
        product_id, warehouse_id, quantity, reservation_id, order_id, reason
    ) VALUES (
        NEW.id, NEW.stream_id, NEW.version, NEW.event_type, NEW.created_at,
        NEW.payload->>'product_id',
        NEW.payload->>'warehouse_id',
        (NEW.payload->>'quantity')::INTEGER,
        NEW.payload->>'reservation_id',
        NEW.payload->>'order_id',
        NEW.payload->>'reason'
    );

    RETURN NEW;
END;
$$;

CREATE TRIGGER inventory_project_global
    AFTER INSERT ON global.event_log
    FOR EACH ROW
    WHEN (NEW.context = 'inventory')
    EXECUTE FUNCTION inventory.project_from_global();
```

### 3.4 Shipping Context Events

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- SHIPPING CONTEXT: Typed Events
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA IF NOT EXISTS shipping;

CREATE TYPE shipping.shipment_status AS ENUM (
    'pending', 'picked', 'packed', 'shipped', 'in_transit', 'delivered', 'failed'
);

CREATE TYPE shipping.carrier AS ENUM ('fedex', 'ups', 'usps', 'dhl');

CREATE TABLE shipping.events (
    global_event_id     BIGINT PRIMARY KEY REFERENCES global.event_log(id),

    stream_id           TEXT NOT NULL,
    version             INTEGER NOT NULL,
    event_type          TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL,

    -- Typed columns
    shipment_id         TEXT,
    order_id            TEXT,
    carrier             shipping.carrier,
    tracking_number     TEXT,
    location            TEXT,
    notes               TEXT,
    event_timestamp     TIMESTAMPTZ,

    UNIQUE (stream_id, version)
);

CREATE INDEX idx_shipping_events_stream ON shipping.events (stream_id, version);
CREATE INDEX idx_shipping_events_order ON shipping.events (order_id);
CREATE INDEX idx_shipping_events_tracking ON shipping.events (tracking_number)
    WHERE tracking_number IS NOT NULL;

-- Projection trigger
CREATE OR REPLACE FUNCTION shipping.project_from_global()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.context != 'shipping' THEN
        RETURN NEW;
    END IF;

    INSERT INTO shipping.events (
        global_event_id, stream_id, version, event_type, created_at,
        shipment_id, order_id, carrier, tracking_number, location, notes, event_timestamp
    ) VALUES (
        NEW.id, NEW.stream_id, NEW.version, NEW.event_type, NEW.created_at,
        NEW.payload->>'shipment_id',
        NEW.payload->>'order_id',
        (NEW.payload->>'carrier')::shipping.carrier,
        NEW.payload->>'tracking_number',
        NEW.payload->>'location',
        NEW.payload->>'notes',
        (NEW.payload->>'event_timestamp')::TIMESTAMPTZ
    );

    RETURN NEW;
END;
$$;

CREATE TRIGGER shipping_project_global
    AFTER INSERT ON global.event_log
    FOR EACH ROW
    WHEN (NEW.context = 'shipping')
    EXECUTE FUNCTION shipping.project_from_global();
```

---

## 4. Writing Events (Single Entry Point)

All event writes go through the global log. The typed context tables are populated automatically.

### 4.1 Context-Aware Append Function

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- Unified Event Append
-- Single entry point for all event writes
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION global.append_events(
    p_stream_id TEXT,
    p_context TEXT,
    p_aggregate_type TEXT,
    p_expected_version INTEGER,  -- For optimistic concurrency
    p_events JSONB[],            -- Array of {event_type, payload} objects
    p_metadata JSONB DEFAULT '{}'
)
RETURNS TABLE (
    global_event_id BIGINT,
    version INTEGER
)
LANGUAGE plpgsql AS $$
DECLARE
    v_current_version INTEGER;
    v_new_version INTEGER;
    v_event JSONB;
    v_event_id BIGINT;
BEGIN
    -- Lock the stream for optimistic concurrency
    SELECT COALESCE(MAX(version), 0)
    INTO v_current_version
    FROM global.event_log
    WHERE stream_id = p_stream_id
    FOR UPDATE;

    -- Check version
    IF p_expected_version IS NOT NULL AND v_current_version != p_expected_version THEN
        RAISE EXCEPTION 'Version conflict: expected %, actual %',
            p_expected_version, v_current_version
            USING ERRCODE = 'serialization_failure';
    END IF;

    v_new_version := v_current_version;

    -- Append each event
    FOREACH v_event IN ARRAY p_events
    LOOP
        v_new_version := v_new_version + 1;

        INSERT INTO global.event_log (
            stream_id,
            version,
            context,
            aggregate_type,
            event_type,
            payload,
            metadata
        ) VALUES (
            p_stream_id,
            v_new_version,
            p_context,
            p_aggregate_type,
            v_event->>'event_type',
            v_event->'payload',
            p_metadata
        )
        RETURNING id INTO v_event_id;

        -- Return the event info
        global_event_id := v_event_id;
        version := v_new_version;
        RETURN NEXT;
    END LOOP;

    -- Notify for real-time subscriptions
    PERFORM pg_notify(
        p_context || '_events',
        json_build_object(
            'stream_id', p_stream_id,
            'from_version', v_current_version + 1,
            'to_version', v_new_version,
            'context', p_context
        )::text
    );

    -- Also notify global channel
    PERFORM pg_notify(
        'global_events',
        json_build_object(
            'stream_id', p_stream_id,
            'context', p_context,
            'event_count', array_length(p_events, 1)
        )::text
    );
END;
$$;
```

### 4.2 Sales Context Handler (Uses Global Append)

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- SALES: Order Handler (writes to global log)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION sales.order_handle(
    p_order_id TEXT,
    p_command sales.order_command,
    p_correlation_id TEXT DEFAULT NULL,
    p_actor_id TEXT DEFAULT NULL
)
RETURNS sales.order_result
LANGUAGE plpgsql AS $$
DECLARE
    v_stream_id TEXT;
    v_state sales.order_state;
    v_result sales.order_result;
    v_events JSONB[] := ARRAY[]::JSONB[];
    v_event sales.order_event;
    v_metadata JSONB;
    v_current_version INTEGER;
    v_timestamp TIMESTAMPTZ := now();
BEGIN
    v_stream_id := 'order-' || p_order_id;

    -- Build metadata
    v_metadata := jsonb_build_object(
        'correlation_id', p_correlation_id,
        'actor_id', p_actor_id,
        'timestamp', v_timestamp
    );

    -- Load current state from projection
    v_state := sales.order_load_state(p_order_id);

    -- Get current version
    SELECT COALESCE(MAX(version), 0)
    INTO v_current_version
    FROM global.event_log
    WHERE stream_id = v_stream_id;

    -- Process command (pure function)
    v_result := sales.order_process(v_state, p_command, v_timestamp);

    -- If failed, return immediately
    IF NOT v_result.success THEN
        RETURN v_result;
    END IF;

    -- Convert events to JSONB array for global append
    FOREACH v_event IN ARRAY v_result.events
    LOOP
        v_events := array_append(v_events, jsonb_build_object(
            'event_type', v_event.event_type,
            'payload', jsonb_build_object(
                'order_id', v_event.order_id,
                'customer_id', v_event.customer_id,
                'product_id', v_event.product_id,
                'product_name', v_event.product_name,
                'quantity', v_event.quantity,
                'unit_price', v_event.unit_price,
                'submitted_at', v_event.submitted_at,
                'reason', v_event.reason,
                'completed_at', v_event.completed_at
            )
        ));
    END LOOP;

    -- Append to global log (triggers will populate sales.events)
    PERFORM global.append_events(
        v_stream_id,
        'sales',
        'order',
        v_current_version,
        v_events,
        v_metadata
    );

    RETURN v_result;
END;
$$;
```

---

## 5. Rebuilding Context Events from Global Log

One of the key benefits: we can rebuild any context's typed events table from the global log.

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- Rebuild Sales Events from Global Log
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION sales.rebuild_events_from_global()
RETURNS INTEGER
LANGUAGE plpgsql AS $$
DECLARE
    v_count INTEGER := 0;
BEGIN
    -- Clear existing typed events
    TRUNCATE sales.events;

    -- Replay from global log
    INSERT INTO sales.events (
        global_event_id, stream_id, version, event_type, created_at,
        order_id, customer_id, product_id, product_name,
        quantity, unit_price, submitted_at, reason, completed_at
    )
    SELECT
        g.id,
        g.stream_id,
        g.version,
        g.event_type,
        g.created_at,
        g.payload->>'order_id',
        g.payload->>'customer_id',
        g.payload->>'product_id',
        g.payload->>'product_name',
        (g.payload->>'quantity')::INTEGER,
        (g.payload->>'unit_price')::DECIMAL(12,2),
        (g.payload->>'submitted_at')::TIMESTAMPTZ,
        g.payload->>'reason',
        (g.payload->>'completed_at')::TIMESTAMPTZ
    FROM global.event_log g
    WHERE g.context = 'sales'
    ORDER BY g.id;

    GET DIAGNOSTICS v_count = ROW_COUNT;

    RETURN v_count;
END;
$$;

-- Generic rebuild function
CREATE OR REPLACE FUNCTION global.rebuild_context_events(p_context TEXT)
RETURNS INTEGER
LANGUAGE plpgsql AS $$
BEGIN
    CASE p_context
        WHEN 'sales' THEN
            RETURN sales.rebuild_events_from_global();
        WHEN 'inventory' THEN
            RETURN inventory.rebuild_events_from_global();
        WHEN 'shipping' THEN
            RETURN shipping.rebuild_events_from_global();
        ELSE
            RAISE EXCEPTION 'Unknown context: %', p_context;
    END CASE;
END;
$$;
```

---

## 6. Cross-Cutting Queries on Global Log

The global log enables powerful cross-context queries:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- Global Query Functions
-- ═══════════════════════════════════════════════════════════════════════════

-- Get complete timeline for an order (across all contexts)
CREATE OR REPLACE FUNCTION global.order_timeline(p_order_id TEXT)
RETURNS TABLE (
    event_id        BIGINT,
    context         TEXT,
    event_type      TEXT,
    payload         JSONB,
    created_at      TIMESTAMPTZ
)
LANGUAGE sql STABLE AS $$
    SELECT id, context, event_type, payload, created_at
    FROM global.event_log
    WHERE payload->>'order_id' = p_order_id
       OR stream_id = 'order-' || p_order_id
    ORDER BY id;
$$;

-- Get all events for a correlation (saga tracking)
CREATE OR REPLACE FUNCTION global.correlation_timeline(p_correlation_id TEXT)
RETURNS TABLE (
    event_id        BIGINT,
    context         TEXT,
    stream_id       TEXT,
    event_type      TEXT,
    payload         JSONB,
    created_at      TIMESTAMPTZ
)
LANGUAGE sql STABLE AS $$
    SELECT id, context, stream_id, event_type, payload, created_at
    FROM global.event_log
    WHERE metadata->>'correlation_id' = p_correlation_id
    ORDER BY id;
$$;

-- Event rate by context (monitoring)
CREATE OR REPLACE FUNCTION global.event_rate_by_context(
    p_interval INTERVAL DEFAULT '1 hour'
)
RETURNS TABLE (
    context         TEXT,
    event_count     BIGINT,
    events_per_min  NUMERIC
)
LANGUAGE sql STABLE AS $$
    SELECT
        context,
        COUNT(*) as event_count,
        ROUND(COUNT(*)::numeric / (EXTRACT(EPOCH FROM p_interval) / 60), 2)
    FROM global.event_log
    WHERE created_at > now() - p_interval
    GROUP BY context
    ORDER BY event_count DESC;
$$;

-- Find related entities across contexts
CREATE OR REPLACE FUNCTION global.find_related_events(
    p_field TEXT,       -- 'order_id', 'customer_id', etc.
    p_value TEXT
)
RETURNS TABLE (
    event_id        BIGINT,
    context         TEXT,
    stream_id       TEXT,
    event_type      TEXT,
    created_at      TIMESTAMPTZ
)
LANGUAGE sql STABLE AS $$
    SELECT id, context, stream_id, event_type, created_at
    FROM global.event_log
    WHERE payload->>p_field = p_value
    ORDER BY id;
$$;
```

---

## 7. The Problem with One Events Table

The fully-typed architecture creates a single events table with columns for
all event types:

```sql
CREATE TABLE events (
    id BIGSERIAL PRIMARY KEY,
    stream_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    -- Order events
    order_id TEXT,
    customer_id TEXT,
    -- Inventory events
    product_id TEXT,
    warehouse_id TEXT,
    quantity_reserved INTEGER,
    -- Shipping events
    shipment_id TEXT,
    carrier TEXT,
    tracking_number TEXT,
    -- ... columns for every event type in the entire application
);
```

**Problems:**

| Issue | Impact |
|-------|--------|
| Wide table | 100+ columns for large applications |
| Implicit coupling | All aggregates share schema |
| No semantic boundaries | DDD bounded contexts invisible |
| Change coordination | Adding events requires schema migration |
| Performance | Sparse data, wasted storage |

---

## 2. Solution: Schema-per-Bounded-Context

PostgreSQL schemas provide the perfect abstraction for bounded contexts:

```
database: ecommerce
│
├── schema: sales              -- Sales Bounded Context
│   ├── order_status           -- Types local to Sales
│   ├── order_event
│   ├── order_state
│   ├── events                 -- Only Order events
│   ├── orders_projection
│   ├── order_items
│   └── order_* functions
│
├── schema: inventory          -- Inventory Bounded Context
│   ├── stock_event
│   ├── stock_state
│   ├── events                 -- Only Stock events
│   ├── stock_projection
│   └── inventory_* functions
│
├── schema: shipping           -- Shipping Bounded Context
│   ├── shipment_status
│   ├── shipment_event
│   ├── events                 -- Only Shipment events
│   ├── shipments_projection
│   └── shipping_* functions
│
└── schema: integration        -- Cross-Context Communication
    ├── domain_events          -- Published integration events
    ├── subscriptions          -- Who subscribes to what
    └── outbox                 -- Transactional outbox pattern
```

### 2.1 Benefits

| Benefit | Description |
|---------|-------------|
| **Narrow tables** | Each events table has only relevant columns |
| **Type safety** | Each context defines its own types |
| **Clear boundaries** | Schema = bounded context, visible in DB |
| **Independent evolution** | Change internal events without coordination |
| **Natural ownership** | Each context owns its data completely |
| **Single database** | Transactions within context, simple operations |

---

## 3. Bounded Context Schema Structure

### 3.1 Sales Bounded Context

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- SALES BOUNDED CONTEXT
-- Owns: Orders, Order Items, Order Lifecycle
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA sales;

-- ───────────────────────────────────────────────────────────────────────────
-- Types (local to this context)
-- ───────────────────────────────────────────────────────────────────────────

CREATE TYPE sales.order_status AS ENUM (
    'pending', 'submitted', 'confirmed', 'cancelled', 'completed'
);

CREATE TYPE sales.order_item AS (
    product_id      TEXT,
    product_name    TEXT,       -- Denormalized from catalog (at order time)
    quantity        INTEGER,
    unit_price      DECIMAL(12, 2)
);

CREATE TYPE sales.order_state AS (
    order_id        TEXT,
    customer_id     TEXT,
    status          sales.order_status,
    total_amount    DECIMAL(12, 2),
    item_count      INTEGER,
    items           sales.order_item[]
);

CREATE TYPE sales.order_command AS (
    command_type    TEXT,
    order_id        TEXT,
    customer_id     TEXT,
    product_id      TEXT,
    product_name    TEXT,
    quantity        INTEGER,
    unit_price      DECIMAL(12, 2),
    reason          TEXT
);

CREATE TYPE sales.order_event AS (
    event_type      TEXT,
    order_id        TEXT,
    customer_id     TEXT,
    product_id      TEXT,
    product_name    TEXT,
    quantity        INTEGER,
    unit_price      DECIMAL(12, 2),
    submitted_at    TIMESTAMPTZ,
    reason          TEXT,
    completed_at    TIMESTAMPTZ
);

CREATE TYPE sales.order_result AS (
    success         BOOLEAN,
    error_code      TEXT,
    error_message   TEXT,
    events          sales.order_event[]
);

-- ───────────────────────────────────────────────────────────────────────────
-- Events Table (narrow - only Order columns)
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE sales.events (
    id              BIGSERIAL PRIMARY KEY,
    stream_id       TEXT NOT NULL,
    version         INTEGER NOT NULL,
    event_type      TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    correlation_id  TEXT,
    causation_id    TEXT,
    actor_id        TEXT,

    -- OrderCreated / ItemAdded fields
    order_id        TEXT,
    customer_id     TEXT,
    product_id      TEXT,
    product_name    TEXT,
    quantity        INTEGER CHECK (quantity IS NULL OR quantity >= 1),
    unit_price      DECIMAL(12, 2) CHECK (unit_price IS NULL OR unit_price >= 0),

    -- OrderSubmitted
    submitted_at    TIMESTAMPTZ,

    -- OrderCancelled
    reason          TEXT,

    -- OrderCompleted
    completed_at    TIMESTAMPTZ,

    UNIQUE (stream_id, version)
);

CREATE INDEX idx_sales_events_stream ON sales.events (stream_id, version);
CREATE INDEX idx_sales_events_correlation ON sales.events (correlation_id)
    WHERE correlation_id IS NOT NULL;

-- ───────────────────────────────────────────────────────────────────────────
-- Projections
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE sales.orders_projection (
    order_id        TEXT PRIMARY KEY,
    status          sales.order_status NOT NULL,
    customer_id     TEXT NOT NULL,
    total_amount    DECIMAL(12, 2) NOT NULL DEFAULT 0,
    item_count      INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL,
    submitted_at    TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    cancelled_at    TIMESTAMPTZ,
    cancel_reason   TEXT,
    last_event_id   BIGINT REFERENCES sales.events(id)
);

CREATE TABLE sales.order_items (
    id              BIGSERIAL PRIMARY KEY,
    order_id        TEXT NOT NULL REFERENCES sales.orders_projection(order_id) ON DELETE CASCADE,
    product_id      TEXT NOT NULL,
    product_name    TEXT NOT NULL,
    quantity        INTEGER NOT NULL CHECK (quantity >= 1),
    unit_price      DECIMAL(12, 2) NOT NULL CHECK (unit_price >= 0),
    line_total      DECIMAL(12, 2) GENERATED ALWAYS AS (quantity * unit_price) STORED,
    added_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Indexes for common queries
CREATE INDEX idx_sales_orders_status ON sales.orders_projection (status);
CREATE INDEX idx_sales_orders_customer ON sales.orders_projection (customer_id);
CREATE INDEX idx_sales_order_items_order ON sales.order_items (order_id);
```

### 3.2 Inventory Bounded Context

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- INVENTORY BOUNDED CONTEXT
-- Owns: Stock levels, Reservations, Warehouse locations
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA inventory;

-- ───────────────────────────────────────────────────────────────────────────
-- Types (local to this context)
-- ───────────────────────────────────────────────────────────────────────────

CREATE TYPE inventory.reservation_status AS ENUM (
    'pending', 'confirmed', 'released', 'fulfilled'
);

CREATE TYPE inventory.stock_event AS (
    event_type          TEXT,
    product_id          TEXT,
    warehouse_id        TEXT,
    quantity            INTEGER,
    reservation_id      TEXT,
    order_id            TEXT,          -- Reference to sales context (opaque ID)
    reason              TEXT
);

CREATE TYPE inventory.stock_state AS (
    product_id          TEXT,
    warehouse_id        TEXT,
    available_quantity  INTEGER,
    reserved_quantity   INTEGER,
    reorder_point       INTEGER
);

CREATE TYPE inventory.stock_result AS (
    success             BOOLEAN,
    error_code          TEXT,
    error_message       TEXT,
    events              inventory.stock_event[]
);

-- ───────────────────────────────────────────────────────────────────────────
-- Events Table (narrow - only Inventory columns)
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE inventory.events (
    id              BIGSERIAL PRIMARY KEY,
    stream_id       TEXT NOT NULL,          -- 'stock-{product_id}-{warehouse_id}'
    version         INTEGER NOT NULL,
    event_type      TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    correlation_id  TEXT,
    causation_id    TEXT,
    actor_id        TEXT,

    -- Stock events
    product_id      TEXT,
    warehouse_id    TEXT,
    quantity        INTEGER,
    reservation_id  TEXT,
    order_id        TEXT,                   -- External reference (from Sales)
    reason          TEXT,

    UNIQUE (stream_id, version)
);

CREATE INDEX idx_inventory_events_stream ON inventory.events (stream_id, version);
CREATE INDEX idx_inventory_events_order ON inventory.events (order_id)
    WHERE order_id IS NOT NULL;

-- ───────────────────────────────────────────────────────────────────────────
-- Projections
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE inventory.stock_projection (
    product_id          TEXT NOT NULL,
    warehouse_id        TEXT NOT NULL,
    available_quantity  INTEGER NOT NULL DEFAULT 0,
    reserved_quantity   INTEGER NOT NULL DEFAULT 0,
    reorder_point       INTEGER NOT NULL DEFAULT 10,
    last_updated        TIMESTAMPTZ NOT NULL,
    last_event_id       BIGINT REFERENCES inventory.events(id),
    PRIMARY KEY (product_id, warehouse_id)
);

CREATE TABLE inventory.reservations (
    reservation_id      TEXT PRIMARY KEY,
    order_id            TEXT NOT NULL,
    product_id          TEXT NOT NULL,
    warehouse_id        TEXT NOT NULL,
    quantity            INTEGER NOT NULL,
    status              inventory.reservation_status NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL,
    expires_at          TIMESTAMPTZ,
    fulfilled_at        TIMESTAMPTZ
);

CREATE INDEX idx_inventory_reservations_order ON inventory.reservations (order_id);
CREATE INDEX idx_inventory_reservations_status ON inventory.reservations (status)
    WHERE status = 'pending';
```

### 3.3 Shipping Bounded Context

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- SHIPPING BOUNDED CONTEXT
-- Owns: Shipments, Carriers, Tracking, Delivery
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA shipping;

-- ───────────────────────────────────────────────────────────────────────────
-- Types
-- ───────────────────────────────────────────────────────────────────────────

CREATE TYPE shipping.shipment_status AS ENUM (
    'pending', 'picked', 'packed', 'shipped', 'in_transit', 'delivered', 'failed'
);

CREATE TYPE shipping.carrier AS ENUM (
    'fedex', 'ups', 'usps', 'dhl'
);

CREATE TYPE shipping.address AS (
    street          TEXT,
    city            TEXT,
    state           TEXT,
    postal_code     TEXT,
    country         CHAR(2)
);

CREATE TYPE shipping.shipment_event AS (
    event_type      TEXT,
    shipment_id     TEXT,
    order_id        TEXT,
    carrier         shipping.carrier,
    tracking_number TEXT,
    location        TEXT,
    notes           TEXT,
    timestamp       TIMESTAMPTZ
);

-- ───────────────────────────────────────────────────────────────────────────
-- Events Table (narrow - only Shipping columns)
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE shipping.events (
    id              BIGSERIAL PRIMARY KEY,
    stream_id       TEXT NOT NULL,          -- 'shipment-{shipment_id}'
    version         INTEGER NOT NULL,
    event_type      TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    correlation_id  TEXT,
    causation_id    TEXT,
    actor_id        TEXT,

    -- Shipment events
    shipment_id     TEXT,
    order_id        TEXT,
    carrier         shipping.carrier,
    tracking_number TEXT,
    destination     shipping.address,
    location        TEXT,
    notes           TEXT,
    event_timestamp TIMESTAMPTZ,

    UNIQUE (stream_id, version)
);

-- ───────────────────────────────────────────────────────────────────────────
-- Projections
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE shipping.shipments_projection (
    shipment_id     TEXT PRIMARY KEY,
    order_id        TEXT NOT NULL,
    status          shipping.shipment_status NOT NULL,
    carrier         shipping.carrier,
    tracking_number TEXT,
    destination     shipping.address,
    created_at      TIMESTAMPTZ NOT NULL,
    shipped_at      TIMESTAMPTZ,
    delivered_at    TIMESTAMPTZ,
    last_location   TEXT,
    last_event_id   BIGINT REFERENCES shipping.events(id)
);

CREATE INDEX idx_shipping_shipments_order ON shipping.shipments_projection (order_id);
CREATE INDEX idx_shipping_shipments_status ON shipping.shipments_projection (status);
CREATE INDEX idx_shipping_shipments_tracking ON shipping.shipments_projection (tracking_number)
    WHERE tracking_number IS NOT NULL;
```

---

## 4. Integration Layer

The integration schema handles cross-context communication. This is where
bounded contexts publish events that other contexts care about.

### 4.1 Integration Events Table

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- INTEGRATION SCHEMA
-- Cross-context communication layer
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA integration;

-- ───────────────────────────────────────────────────────────────────────────
-- Domain Events (Published Language)
-- These are the events that cross context boundaries.
-- Uses JSONB because these are contracts between contexts.
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE integration.domain_events (
    id              BIGSERIAL PRIMARY KEY,

    -- Event identity
    event_id        UUID NOT NULL DEFAULT gen_random_uuid(),
    event_type      TEXT NOT NULL,

    -- Source context
    source_context  TEXT NOT NULL,          -- 'sales', 'inventory', 'shipping'
    source_stream   TEXT NOT NULL,          -- Original stream_id
    source_version  INTEGER NOT NULL,       -- Original version

    -- Event data (JSONB for cross-context flexibility)
    payload         JSONB NOT NULL,

    -- Metadata
    correlation_id  TEXT,
    causation_id    TEXT,
    published_at    TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Uniqueness: prevent duplicate publishing
    UNIQUE (source_context, source_stream, source_version)
);

-- Index for subscription queries
CREATE INDEX idx_integration_events_type ON integration.domain_events (event_type);
CREATE INDEX idx_integration_events_published ON integration.domain_events (published_at);
CREATE INDEX idx_integration_events_correlation ON integration.domain_events (correlation_id)
    WHERE correlation_id IS NOT NULL;

-- ───────────────────────────────────────────────────────────────────────────
-- Subscriptions
-- Track which contexts subscribe to which event types
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE integration.subscriptions (
    id              BIGSERIAL PRIMARY KEY,
    subscriber      TEXT NOT NULL,          -- 'inventory', 'shipping', etc.
    event_type      TEXT NOT NULL,          -- 'OrderSubmitted', 'InventoryReserved', etc.
    handler         TEXT NOT NULL,          -- Function to call or channel to notify
    enabled         BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (subscriber, event_type)
);

-- ───────────────────────────────────────────────────────────────────────────
-- Consumer Positions
-- Track where each consumer is in the event stream
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE integration.consumer_positions (
    consumer_id     TEXT PRIMARY KEY,       -- 'inventory:order_handler'
    last_event_id   BIGINT NOT NULL DEFAULT 0,
    last_processed  TIMESTAMPTZ NOT NULL DEFAULT now(),
    error_count     INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT
);

-- ───────────────────────────────────────────────────────────────────────────
-- Dead Letter Queue
-- Failed integration events for manual intervention
-- ───────────────────────────────────────────────────────────────────────────

CREATE TABLE integration.dead_letter_queue (
    id              BIGSERIAL PRIMARY KEY,
    event_id        BIGINT NOT NULL REFERENCES integration.domain_events(id),
    consumer_id     TEXT NOT NULL,
    error_message   TEXT NOT NULL,
    error_details   JSONB,
    retry_count     INTEGER NOT NULL DEFAULT 0,
    first_failed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_failed_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### 4.2 Integration Event Types

Define the "published language" - events that cross context boundaries:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- INTEGRATION EVENT CATALOG
-- These are the official contracts between bounded contexts.
-- Document the expected payload structure for each event type.
-- ═══════════════════════════════════════════════════════════════════════════

COMMENT ON TABLE integration.domain_events IS
'Cross-context domain events. Payload schemas by event_type:

FROM SALES CONTEXT:
  OrderSubmitted:
    order_id: TEXT
    customer_id: TEXT
    items: [{product_id, quantity, unit_price}]
    total_amount: DECIMAL
    submitted_at: TIMESTAMPTZ

  OrderCancelled:
    order_id: TEXT
    reason: TEXT
    cancelled_at: TIMESTAMPTZ

  OrderCompleted:
    order_id: TEXT
    completed_at: TIMESTAMPTZ

FROM INVENTORY CONTEXT:
  InventoryReserved:
    reservation_id: TEXT
    order_id: TEXT
    product_id: TEXT
    warehouse_id: TEXT
    quantity: INTEGER

  InventoryReservationFailed:
    order_id: TEXT
    product_id: TEXT
    requested_quantity: INTEGER
    available_quantity: INTEGER
    reason: TEXT

  InventoryReleased:
    reservation_id: TEXT
    order_id: TEXT
    reason: TEXT

FROM SHIPPING CONTEXT:
  ShipmentCreated:
    shipment_id: TEXT
    order_id: TEXT
    carrier: TEXT
    tracking_number: TEXT

  ShipmentDelivered:
    shipment_id: TEXT
    order_id: TEXT
    delivered_at: TIMESTAMPTZ
';
```

---

## 5. Publishing Integration Events

### 5.1 From Bounded Context to Integration Layer

Each bounded context publishes to integration when it produces events
that other contexts care about:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- SALES CONTEXT: Publish Integration Events
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION sales.publish_integration_event()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
DECLARE
    v_payload JSONB;
    v_event_type TEXT;
BEGIN
    -- Only publish certain events to integration layer
    CASE NEW.event_type

    WHEN 'OrderSubmitted' THEN
        v_event_type := 'OrderSubmitted';
        v_payload := jsonb_build_object(
            'order_id', NEW.order_id,
            'customer_id', NEW.customer_id,
            'submitted_at', NEW.submitted_at,
            'total_amount', (
                SELECT total_amount FROM sales.orders_projection
                WHERE order_id = NEW.order_id
            ),
            'items', (
                SELECT jsonb_agg(jsonb_build_object(
                    'product_id', product_id,
                    'product_name', product_name,
                    'quantity', quantity,
                    'unit_price', unit_price
                ))
                FROM sales.order_items
                WHERE order_id = NEW.order_id
            )
        );

    WHEN 'OrderCancelled' THEN
        v_event_type := 'OrderCancelled';
        v_payload := jsonb_build_object(
            'order_id', NEW.order_id,
            'reason', NEW.reason,
            'cancelled_at', NEW.created_at
        );

    WHEN 'OrderCompleted' THEN
        v_event_type := 'OrderCompleted';
        v_payload := jsonb_build_object(
            'order_id', NEW.order_id,
            'completed_at', NEW.completed_at
        );

    ELSE
        -- Internal event, don't publish
        RETURN NEW;

    END CASE;

    -- Insert into integration layer
    INSERT INTO integration.domain_events (
        event_type,
        source_context,
        source_stream,
        source_version,
        payload,
        correlation_id,
        causation_id
    ) VALUES (
        v_event_type,
        'sales',
        NEW.stream_id,
        NEW.version,
        v_payload,
        NEW.correlation_id,
        NEW.causation_id
    );

    -- Notify subscribers
    PERFORM pg_notify('integration_events', json_build_object(
        'event_type', v_event_type,
        'source_context', 'sales',
        'event_id', currval('integration.domain_events_id_seq')
    )::text);

    RETURN NEW;
END;
$$;

CREATE TRIGGER sales_publish_integration
    AFTER INSERT ON sales.events
    FOR EACH ROW
    EXECUTE FUNCTION sales.publish_integration_event();
```

### 5.2 Inventory Context Publishing

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- INVENTORY CONTEXT: Publish Integration Events
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION inventory.publish_integration_event()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
DECLARE
    v_payload JSONB;
    v_event_type TEXT;
BEGIN
    CASE NEW.event_type

    WHEN 'InventoryReserved' THEN
        v_event_type := 'InventoryReserved';
        v_payload := jsonb_build_object(
            'reservation_id', NEW.reservation_id,
            'order_id', NEW.order_id,
            'product_id', NEW.product_id,
            'warehouse_id', NEW.warehouse_id,
            'quantity', NEW.quantity
        );

    WHEN 'InventoryReservationFailed' THEN
        v_event_type := 'InventoryReservationFailed';
        v_payload := jsonb_build_object(
            'order_id', NEW.order_id,
            'product_id', NEW.product_id,
            'requested_quantity', NEW.quantity,
            'reason', NEW.reason
        );

    WHEN 'InventoryReleased' THEN
        v_event_type := 'InventoryReleased';
        v_payload := jsonb_build_object(
            'reservation_id', NEW.reservation_id,
            'order_id', NEW.order_id,
            'reason', NEW.reason
        );

    ELSE
        RETURN NEW;

    END CASE;

    INSERT INTO integration.domain_events (
        event_type, source_context, source_stream, source_version,
        payload, correlation_id, causation_id
    ) VALUES (
        v_event_type, 'inventory', NEW.stream_id, NEW.version,
        v_payload, NEW.correlation_id, NEW.causation_id
    );

    PERFORM pg_notify('integration_events', json_build_object(
        'event_type', v_event_type,
        'source_context', 'inventory',
        'event_id', currval('integration.domain_events_id_seq')
    )::text);

    RETURN NEW;
END;
$$;

CREATE TRIGGER inventory_publish_integration
    AFTER INSERT ON inventory.events
    FOR EACH ROW
    EXECUTE FUNCTION inventory.publish_integration_event();
```

---

## 6. Consuming Integration Events

### 6.1 Poll-based Consumer

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- INTEGRATION: Event Consumer Functions
-- ═══════════════════════════════════════════════════════════════════════════

-- Get next batch of events for a consumer
CREATE OR REPLACE FUNCTION integration.get_pending_events(
    p_consumer_id TEXT,
    p_event_types TEXT[],
    p_batch_size INTEGER DEFAULT 100
)
RETURNS TABLE (
    event_id        BIGINT,
    event_type      TEXT,
    source_context  TEXT,
    payload         JSONB,
    correlation_id  TEXT,
    causation_id    TEXT,
    published_at    TIMESTAMPTZ
)
LANGUAGE plpgsql AS $$
DECLARE
    v_last_id BIGINT;
BEGIN
    -- Get consumer's last position
    SELECT COALESCE(last_event_id, 0)
    INTO v_last_id
    FROM integration.consumer_positions
    WHERE consumer_id = p_consumer_id;

    -- If consumer doesn't exist, create it
    IF v_last_id IS NULL THEN
        INSERT INTO integration.consumer_positions (consumer_id, last_event_id)
        VALUES (p_consumer_id, 0)
        ON CONFLICT (consumer_id) DO NOTHING;
        v_last_id := 0;
    END IF;

    -- Return next batch
    RETURN QUERY
    SELECT
        de.id,
        de.event_type,
        de.source_context,
        de.payload,
        de.correlation_id,
        de.causation_id,
        de.published_at
    FROM integration.domain_events de
    WHERE de.id > v_last_id
      AND de.event_type = ANY(p_event_types)
    ORDER BY de.id
    LIMIT p_batch_size;
END;
$$;

-- Acknowledge processed events
CREATE OR REPLACE FUNCTION integration.ack_events(
    p_consumer_id TEXT,
    p_last_event_id BIGINT
)
RETURNS VOID
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO integration.consumer_positions (consumer_id, last_event_id, last_processed)
    VALUES (p_consumer_id, p_last_event_id, now())
    ON CONFLICT (consumer_id) DO UPDATE SET
        last_event_id = EXCLUDED.last_event_id,
        last_processed = EXCLUDED.last_processed,
        error_count = 0;
END;
$$;

-- Record processing failure
CREATE OR REPLACE FUNCTION integration.nack_event(
    p_consumer_id TEXT,
    p_event_id BIGINT,
    p_error_message TEXT,
    p_error_details JSONB DEFAULT NULL
)
RETURNS VOID
LANGUAGE plpgsql AS $$
BEGIN
    -- Update consumer error count
    UPDATE integration.consumer_positions
    SET error_count = error_count + 1,
        last_error = p_error_message
    WHERE consumer_id = p_consumer_id;

    -- Add to DLQ if exists, update if already there
    INSERT INTO integration.dead_letter_queue (
        event_id, consumer_id, error_message, error_details
    ) VALUES (
        p_event_id, p_consumer_id, p_error_message, p_error_details
    )
    ON CONFLICT (event_id, consumer_id) DO UPDATE SET
        retry_count = integration.dead_letter_queue.retry_count + 1,
        last_failed_at = now(),
        error_message = EXCLUDED.error_message,
        error_details = EXCLUDED.error_details;
END;
$$;
```

### 6.2 Inventory Consuming Sales Events

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- INVENTORY: Handle Order Events from Sales
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION inventory.handle_order_submitted(
    p_event JSONB,
    p_correlation_id TEXT
)
RETURNS VOID
LANGUAGE plpgsql AS $$
DECLARE
    v_order_id TEXT;
    v_item JSONB;
    v_reservation_id TEXT;
    v_available INTEGER;
    v_command inventory.stock_command;
    v_result inventory.stock_result;
BEGIN
    v_order_id := p_event->>'order_id';

    -- Process each item in the order
    FOR v_item IN SELECT * FROM jsonb_array_elements(p_event->'items')
    LOOP
        v_reservation_id := gen_random_uuid()::text;

        -- Check availability and reserve
        -- (This would call the inventory aggregate's command handler)
        -- For now, simplified inline logic:

        SELECT available_quantity
        INTO v_available
        FROM inventory.stock_projection
        WHERE product_id = v_item->>'product_id'
        ORDER BY available_quantity DESC
        LIMIT 1;

        IF COALESCE(v_available, 0) >= (v_item->>'quantity')::integer THEN
            -- Reserve inventory
            INSERT INTO inventory.events (
                stream_id, version, event_type,
                product_id, warehouse_id, quantity,
                reservation_id, order_id, correlation_id
            ) VALUES (
                'stock-' || (v_item->>'product_id'),
                COALESCE((
                    SELECT MAX(version) + 1
                    FROM inventory.events
                    WHERE stream_id = 'stock-' || (v_item->>'product_id')
                ), 1),
                'InventoryReserved',
                v_item->>'product_id',
                'default-warehouse',
                (v_item->>'quantity')::integer,
                v_reservation_id,
                v_order_id,
                p_correlation_id
            );
        ELSE
            -- Reservation failed
            INSERT INTO inventory.events (
                stream_id, version, event_type,
                product_id, quantity, order_id, reason, correlation_id
            ) VALUES (
                'stock-' || (v_item->>'product_id'),
                COALESCE((
                    SELECT MAX(version) + 1
                    FROM inventory.events
                    WHERE stream_id = 'stock-' || (v_item->>'product_id')
                ), 1),
                'InventoryReservationFailed',
                v_item->>'product_id',
                (v_item->>'quantity')::integer,
                v_order_id,
                format('Requested %s but only %s available',
                    v_item->>'quantity', COALESCE(v_available, 0)),
                p_correlation_id
            );
        END IF;
    END LOOP;
END;
$$;

-- Handler for OrderCancelled - release reservations
CREATE OR REPLACE FUNCTION inventory.handle_order_cancelled(
    p_event JSONB,
    p_correlation_id TEXT
)
RETURNS VOID
LANGUAGE plpgsql AS $$
DECLARE
    v_order_id TEXT;
    v_reservation RECORD;
BEGIN
    v_order_id := p_event->>'order_id';

    -- Find and release all reservations for this order
    FOR v_reservation IN
        SELECT reservation_id, product_id, warehouse_id, quantity
        FROM inventory.reservations
        WHERE order_id = v_order_id
          AND status = 'pending'
    LOOP
        INSERT INTO inventory.events (
            stream_id, version, event_type,
            product_id, warehouse_id, quantity,
            reservation_id, order_id, reason, correlation_id
        ) VALUES (
            'stock-' || v_reservation.product_id,
            COALESCE((
                SELECT MAX(version) + 1
                FROM inventory.events
                WHERE stream_id = 'stock-' || v_reservation.product_id
            ), 1),
            'InventoryReleased',
            v_reservation.product_id,
            v_reservation.warehouse_id,
            v_reservation.quantity,
            v_reservation.reservation_id,
            v_order_id,
            'Order cancelled: ' || COALESCE(p_event->>'reason', 'No reason provided'),
            p_correlation_id
        );
    END LOOP;
END;
$$;
```

### 6.3 Processing Loop (Application Layer)

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- Integration Event Processing Loop
-- Called periodically by the application or a background worker
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION inventory.process_integration_events(
    p_batch_size INTEGER DEFAULT 100
)
RETURNS INTEGER
LANGUAGE plpgsql AS $$
DECLARE
    v_consumer_id TEXT := 'inventory:order_handler';
    v_event RECORD;
    v_count INTEGER := 0;
    v_last_id BIGINT := 0;
BEGIN
    -- Get pending events this consumer cares about
    FOR v_event IN
        SELECT *
        FROM integration.get_pending_events(
            v_consumer_id,
            ARRAY['OrderSubmitted', 'OrderCancelled', 'OrderCompleted'],
            p_batch_size
        )
    LOOP
        BEGIN
            -- Route to appropriate handler
            CASE v_event.event_type
                WHEN 'OrderSubmitted' THEN
                    PERFORM inventory.handle_order_submitted(
                        v_event.payload,
                        v_event.correlation_id
                    );
                WHEN 'OrderCancelled' THEN
                    PERFORM inventory.handle_order_cancelled(
                        v_event.payload,
                        v_event.correlation_id
                    );
                WHEN 'OrderCompleted' THEN
                    -- Mark reservations as fulfilled
                    UPDATE inventory.reservations
                    SET status = 'fulfilled', fulfilled_at = now()
                    WHERE order_id = v_event.payload->>'order_id'
                      AND status = 'confirmed';
            END CASE;

            v_last_id := v_event.event_id;
            v_count := v_count + 1;

        EXCEPTION WHEN OTHERS THEN
            -- Record failure but continue processing
            PERFORM integration.nack_event(
                v_consumer_id,
                v_event.event_id,
                SQLERRM,
                jsonb_build_object('sqlstate', SQLSTATE)
            );
        END;
    END LOOP;

    -- Acknowledge all successfully processed events
    IF v_last_id > 0 THEN
        PERFORM integration.ack_events(v_consumer_id, v_last_id);
    END IF;

    RETURN v_count;
END;
$$;
```

---

## 7. Real-Time Notifications

### 7.1 Using LISTEN/NOTIFY

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- Real-time notification setup
-- Contexts can LISTEN for integration events
-- ═══════════════════════════════════════════════════════════════════════════

-- Application connects and runs:
-- LISTEN integration_events;

-- When an event is published, they receive:
-- {"event_type": "OrderSubmitted", "source_context": "sales", "event_id": 123}

-- The application then fetches the full event:
-- SELECT * FROM integration.domain_events WHERE id = 123;
```

### 7.2 Notification Router

```sql
-- Route notifications to context-specific channels
CREATE OR REPLACE FUNCTION integration.route_notification()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
DECLARE
    v_subscriber RECORD;
BEGIN
    -- Notify global channel
    PERFORM pg_notify('integration_events', json_build_object(
        'event_id', NEW.id,
        'event_type', NEW.event_type,
        'source_context', NEW.source_context
    )::text);

    -- Notify context-specific channels based on subscriptions
    FOR v_subscriber IN
        SELECT DISTINCT subscriber
        FROM integration.subscriptions
        WHERE event_type = NEW.event_type
          AND enabled = true
    LOOP
        PERFORM pg_notify(
            v_subscriber.subscriber || '_events',
            json_build_object(
                'event_id', NEW.id,
                'event_type', NEW.event_type
            )::text
        );
    END LOOP;

    RETURN NEW;
END;
$$;

CREATE TRIGGER integration_notify
    AFTER INSERT ON integration.domain_events
    FOR EACH ROW
    EXECUTE FUNCTION integration.route_notification();
```

---

## 8. Anti-Corruption Layer

Each bounded context should translate external concepts into its own language:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- INVENTORY: Anti-Corruption Layer
-- Translates Sales concepts into Inventory concepts
-- ═══════════════════════════════════════════════════════════════════════════

-- Sales calls it an "order", Inventory calls it a "reservation request"
CREATE OR REPLACE FUNCTION inventory.translate_order_to_reservation_request(
    p_order_event JSONB
)
RETURNS TABLE (
    product_id      TEXT,
    quantity        INTEGER,
    priority        INTEGER,
    source_order_id TEXT
)
LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
    RETURN QUERY
    SELECT
        item->>'product_id',
        (item->>'quantity')::integer,
        CASE
            WHEN (p_order_event->>'total_amount')::numeric > 1000 THEN 1  -- High value = priority
            ELSE 2
        END,
        p_order_event->>'order_id'
    FROM jsonb_array_elements(p_order_event->'items') AS item;
END;
$$;

COMMENT ON FUNCTION inventory.translate_order_to_reservation_request(JSONB) IS
'Anti-corruption layer: Translates Sales.OrderSubmitted into Inventory''s
internal representation. Sales speaks of "orders", Inventory speaks of
"reservation requests". This function bridges the semantic gap.';
```

---

## 9. Cross-Context Transactions

### 9.1 The Challenge

Cross-context operations are eventually consistent. If you need atomicity:

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- SAGA PATTERN: Coordinating Across Contexts
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA saga;

CREATE TYPE saga.saga_status AS ENUM (
    'started', 'pending', 'completed', 'compensating', 'failed'
);

CREATE TABLE saga.order_fulfillment (
    saga_id             TEXT PRIMARY KEY,
    order_id            TEXT NOT NULL,
    status              saga.saga_status NOT NULL,

    -- Step tracking
    order_submitted     BOOLEAN NOT NULL DEFAULT false,
    inventory_reserved  BOOLEAN NOT NULL DEFAULT false,
    payment_captured    BOOLEAN NOT NULL DEFAULT false,
    shipment_created    BOOLEAN NOT NULL DEFAULT false,

    -- Compensation tracking
    inventory_released  BOOLEAN NOT NULL DEFAULT false,
    payment_refunded    BOOLEAN NOT NULL DEFAULT false,

    -- Metadata
    started_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at        TIMESTAMPTZ,
    failed_at           TIMESTAMPTZ,
    failure_reason      TEXT,

    -- Correlation
    correlation_id      TEXT NOT NULL
);

-- Saga state machine function
CREATE OR REPLACE FUNCTION saga.process_order_fulfillment(
    p_saga_id TEXT,
    p_event_type TEXT,
    p_success BOOLEAN,
    p_details JSONB DEFAULT NULL
)
RETURNS saga.saga_status
LANGUAGE plpgsql AS $$
DECLARE
    v_saga RECORD;
    v_new_status saga.saga_status;
BEGIN
    SELECT * INTO v_saga FROM saga.order_fulfillment WHERE saga_id = p_saga_id;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'Saga not found: %', p_saga_id;
    END IF;

    -- State machine transitions
    CASE p_event_type
        WHEN 'OrderSubmitted' THEN
            UPDATE saga.order_fulfillment
            SET order_submitted = p_success
            WHERE saga_id = p_saga_id;

        WHEN 'InventoryReserved' THEN
            IF p_success THEN
                UPDATE saga.order_fulfillment
                SET inventory_reserved = true
                WHERE saga_id = p_saga_id;
            ELSE
                -- Start compensation
                UPDATE saga.order_fulfillment
                SET status = 'compensating',
                    failure_reason = p_details->>'reason'
                WHERE saga_id = p_saga_id;

                -- Trigger compensation events
                -- (Application layer handles this)
            END IF;

        WHEN 'ShipmentCreated' THEN
            IF p_success THEN
                UPDATE saga.order_fulfillment
                SET shipment_created = true,
                    status = 'completed',
                    completed_at = now()
                WHERE saga_id = p_saga_id;
            END IF;

        -- ... more transitions
    END CASE;

    SELECT status INTO v_new_status
    FROM saga.order_fulfillment WHERE saga_id = p_saga_id;

    RETURN v_new_status;
END;
$$;
```

---

## 10. Querying Across Contexts

### 10.1 Cross-Context Read Models

Sometimes you need data from multiple contexts. Options:

**Option A: Build a dedicated cross-context projection**

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- REPORTING SCHEMA: Cross-Context Read Models
-- ═══════════════════════════════════════════════════════════════════════════

CREATE SCHEMA reporting;

-- Order fulfillment status (combines Sales + Inventory + Shipping)
CREATE TABLE reporting.order_fulfillment_status (
    order_id            TEXT PRIMARY KEY,

    -- From Sales
    customer_id         TEXT,
    order_status        sales.order_status,
    total_amount        DECIMAL(12, 2),
    submitted_at        TIMESTAMPTZ,

    -- From Inventory
    all_items_reserved  BOOLEAN,
    reservation_status  TEXT,

    -- From Shipping
    shipment_id         TEXT,
    shipment_status     shipping.shipment_status,
    tracking_number     TEXT,
    carrier             shipping.carrier,

    -- Meta
    last_updated        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Update from integration events
CREATE OR REPLACE FUNCTION reporting.update_fulfillment_status()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    CASE NEW.event_type
    WHEN 'OrderSubmitted' THEN
        INSERT INTO reporting.order_fulfillment_status (
            order_id, customer_id, order_status, total_amount, submitted_at
        ) VALUES (
            NEW.payload->>'order_id',
            NEW.payload->>'customer_id',
            'submitted',
            (NEW.payload->>'total_amount')::decimal,
            (NEW.payload->>'submitted_at')::timestamptz
        )
        ON CONFLICT (order_id) DO UPDATE SET
            order_status = 'submitted',
            last_updated = now();

    WHEN 'InventoryReserved' THEN
        UPDATE reporting.order_fulfillment_status
        SET reservation_status = 'reserved',
            last_updated = now()
        WHERE order_id = NEW.payload->>'order_id';

    WHEN 'ShipmentCreated' THEN
        UPDATE reporting.order_fulfillment_status
        SET shipment_id = NEW.payload->>'shipment_id',
            shipment_status = 'pending',
            tracking_number = NEW.payload->>'tracking_number',
            carrier = (NEW.payload->>'carrier')::shipping.carrier,
            last_updated = now()
        WHERE order_id = NEW.payload->>'order_id';

    -- ... more cases
    END CASE;

    RETURN NEW;
END;
$$;

CREATE TRIGGER update_reporting
    AFTER INSERT ON integration.domain_events
    FOR EACH ROW
    EXECUTE FUNCTION reporting.update_fulfillment_status();
```

**Option B: Query-time joins (for ad-hoc queries)**

```sql
-- Ad-hoc cross-context query (use sparingly - couples contexts at query time)
CREATE OR REPLACE FUNCTION reporting.get_order_details(p_order_id TEXT)
RETURNS TABLE (
    order_id        TEXT,
    customer_id     TEXT,
    order_status    sales.order_status,
    total_amount    DECIMAL(12, 2),
    reservations    JSONB,
    shipment        JSONB
)
LANGUAGE plpgsql STABLE AS $$
BEGIN
    RETURN QUERY
    SELECT
        o.order_id,
        o.customer_id,
        o.status,
        o.total_amount,
        (
            SELECT jsonb_agg(jsonb_build_object(
                'reservation_id', r.reservation_id,
                'product_id', r.product_id,
                'quantity', r.quantity,
                'status', r.status
            ))
            FROM inventory.reservations r
            WHERE r.order_id = o.order_id
        ),
        (
            SELECT jsonb_build_object(
                'shipment_id', s.shipment_id,
                'status', s.status,
                'carrier', s.carrier,
                'tracking', s.tracking_number
            )
            FROM shipping.shipments_projection s
            WHERE s.order_id = o.order_id
        )
    FROM sales.orders_projection o
    WHERE o.order_id = p_order_id;
END;
$$;
```

---

## 11. Summary

### Architecture Overview: Two-Layer Event System

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        PostgreSQL Database                               │
│                                                                          │
│  ┌───────────────────────────────────────────────────────────────────┐  │
│  │                      global.event_log                              │  │
│  │               (Single source of truth, JSONB)                      │  │
│  │                                                                    │  │
│  │  id │ stream_id │ version │ context │ event_type │ payload(JSONB) │  │
│  │ ────┼───────────┼─────────┼─────────┼────────────┼──────────────── │  │
│  │  1  │ order-123 │    1    │ sales   │ Submitted  │ {...}          │  │
│  │  2  │ stock-456 │    1    │ invntry │ Reserved   │ {...}          │  │
│  │  3  │ order-123 │    2    │ sales   │ Confirmed  │ {...}          │  │
│  └──────────────────────────┬────────────────────────────────────────┘  │
│                             │                                            │
│            Triggers project │ to typed context tables                    │
│                             ▼                                            │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │
│  │   sales.*   │  │ inventory.* │  │ shipping.*  │  │ reporting.* │    │
│  │             │  │             │  │             │  │             │    │
│  │ ┌─────────┐ │  │ ┌─────────┐ │  │ ┌─────────┐ │  │ Cross-ctx   │    │
│  │ │ events  │ │  │ │ events  │ │  │ │ events  │ │  │ read models │    │
│  │ │ (typed) │ │  │ │ (typed) │ │  │ │ (typed) │ │  │             │    │
│  │ │ FK→glbl │ │  │ │ FK→glbl │ │  │ │ FK→glbl │ │  └─────────────┘    │
│  │ └────┬────┘ │  │ └────┬────┘ │  │ └────┬────┘ │                      │
│  │      │      │  │      │      │  │      │      │                      │
│  │ projections │  │ projections │  │ projections │                      │
│  └──────┼──────┘  └──────┼──────┘  └──────┼──────┘                      │
│         │                │                │                              │
│         └────────────────┼────────────────┘                              │
│                          │                                               │
│                          ▼                                               │
│              ┌─────────────────────┐                                     │
│              │   integration.*     │                                     │
│              │                     │                                     │
│              │  domain_events      │◄──── LISTEN/NOTIFY                  │
│              │  (JSONB payload)    │                                     │
│              │                     │                                     │
│              │  consumer_positions │                                     │
│              │  subscriptions      │                                     │
│              │  dead_letter_queue  │                                     │
│              └─────────────────────┘                                     │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### Data Flow

```
                    Write Path                          Query Path
                    ──────────                          ──────────

    Command ──► global.append_event()                 sales.orders_projection
                        │                                     ▲
                        ▼                                     │
                global.event_log ─────► sales.events ─────────┤
                (JSONB, id=N)          (typed, FK=N)          │
                        │                                     │
                        └──────────────────────────────────────┘
                         Cross-cutting: SELECT * FROM global.event_log
```

### Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Global event log** | JSONB in `global.event_log` | Single source of truth, complete history, flexible schema |
| **Context events** | Typed tables with FK to global | Narrow tables, efficient queries, rebuild from global |
| **Context isolation** | PostgreSQL schemas | Clear boundaries, same database, transactional within context |
| **Event ordering** | Global `BIGSERIAL id` | Total ordering across all contexts |
| **Integration events** | JSONB payload | Flexibility for cross-context contracts |
| **Communication** | `pg_notify` + polling | Real-time hints, reliable polling |
| **Cross-context reads** | Dedicated projections | Avoid runtime coupling |

### Benefits Achieved

1. **Single source of truth**: Global event log is THE event store; everything derives from it
2. **Global ordering**: `global.event_log.id` provides total ordering across all contexts
3. **Complete audit trail**: One table to query for "what happened across the system"
4. **Type safety within contexts**: Each bounded context has narrow, focused event schemas
5. **Rebuild capability**: Can rebuild any context table from global log
6. **Clear boundaries**: Schema = bounded context is visible in database
7. **Independent evolution**: Change internal events without coordination
8. **Published language**: Integration events are explicit contracts
9. **Eventual consistency**: Cross-context is always async (correct semantics)
10. **Single database simplicity**: No distributed transactions within context
11. **Natural event bus**: `integration.domain_events` + `pg_notify`

### Trade-offs Accepted

1. **Storage overhead**: Context tables duplicate some data from global log (mitigated: narrow tables)
2. **Write amplification**: INSERT to global triggers INSERT to context (acceptable overhead)
3. **Eventual consistency**: Cross-context operations are not immediately consistent
4. **More schemas**: Database has more objects to manage
5. **JSONB in global/integration**: Not fully typed (but provides flexibility and rebuild capability)
