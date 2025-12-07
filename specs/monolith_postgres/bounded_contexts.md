# Bounded Contexts in PostgreSQL Monolith

> **Extends**: `architecture_fully_typed.md`
>
> This document describes how to implement DDD bounded contexts within a
> PostgreSQL monolith, providing natural isolation while maintaining the
> benefits of a single database.

---

## 1. The Problem with One Events Table

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

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        PostgreSQL Database                               │
│                                                                          │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │
│  │   sales.*   │  │ inventory.* │  │ shipping.*  │  │ reporting.* │    │
│  │             │  │             │  │             │  │             │    │
│  │ ┌─────────┐ │  │ ┌─────────┐ │  │ ┌─────────┐ │  │ Cross-ctx   │    │
│  │ │ events  │ │  │ │ events  │ │  │ │ events  │ │  │ read models │    │
│  │ │ (typed) │ │  │ │ (typed) │ │  │ │ (typed) │ │  │             │    │
│  │ └────┬────┘ │  │ └────┬────┘ │  │ └────┬────┘ │  └─────────────┘    │
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

### Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Context isolation** | PostgreSQL schemas | Clear boundaries, same database, transactional within context |
| **Context events** | Fully typed per-schema | Narrow tables, context-specific types |
| **Integration events** | JSONB payload | Flexibility for cross-context contracts |
| **Communication** | `pg_notify` + polling | Real-time hints, reliable polling |
| **Cross-context reads** | Dedicated projections | Avoid runtime coupling |

### Benefits Achieved

1. **Type safety within contexts**: Each bounded context has narrow, focused event schemas
2. **Clear boundaries**: Schema = bounded context is visible in database
3. **Independent evolution**: Change internal events without coordination
4. **Published language**: Integration events are explicit contracts
5. **Eventual consistency**: Cross-context is always async (correct semantics)
6. **Single database simplicity**: No distributed transactions within context
7. **Natural event bus**: `integration.domain_events` + `pg_notify`

### Trade-offs Accepted

1. **Some duplication**: Integration events duplicate some data from context events
2. **Eventual consistency**: Cross-context operations are not immediately consistent
3. **More schemas**: Database has more objects to manage
4. **JSONB in integration**: Not fully typed, but provides flexibility for contracts
