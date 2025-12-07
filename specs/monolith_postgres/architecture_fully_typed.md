# PostgreSQL Monolith Architecture: Fully Typed

> **Companion to**: `architecture.md`
>
> This document extends the base architecture with **full static typing** for all
> data structures: events, commands, state, and projections. No JSONB anywhere.

---

## 1. Philosophy: Types as Specification

When YAML defines your domain model completely:

```yaml
aggregates:
  Order:
    state:
      order_id: { type: string }
      status:
        type: enum
        values: [pending, submitted, cancelled, completed]
        transitions:
          pending: [submitted, cancelled]
          submitted: [completed, cancelled]
      total_amount: { type: decimal, precision: 12, scale: 2, min: 0 }
      items:
        type: list
        of: OrderItem
```

The database schema should **enforce these constraints**, not just store data.

### 1.1 Benefits of Full Typing

| Aspect | JSONB | Full Typing |
|--------|-------|-------------|
| Invalid status | Stored, fails at runtime | Rejected at INSERT |
| Negative quantity | Stored, fails at runtime | CHECK constraint rejects |
| Wrong decimal precision | Silently truncated/rounded | DECIMAL(12,2) enforces |
| Missing required field | NULL in JSON | NOT NULL rejects |
| Invalid state transition | Runtime check | Transition table rejects |
| Schema documentation | Read the code | Read the schema |
| IDE autocomplete | None | Full support |
| Query performance | JSONB operators | Native indexes |

### 1.2 The Trade-off We Accept

- **More verbose SQL**: Each type needs explicit definition
- **Migrations for changes**: But YAML changes trigger regeneration anyway
- **Wider tables**: Events table has columns for all event types

This trade-off is correct when:
- AI generates all code from YAML
- YAML is the single source of truth
- Schema changes = regenerate + migrate

---

## 2. Type Definitions

### 2.1 Enums with State Machine

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER STATUS ENUM
-- Generated from YAML enum definition
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TYPE order_status AS ENUM ('pending', 'submitted', 'cancelled', 'completed');

-- ═══════════════════════════════════════════════════════════════════════════
-- STATE MACHINE TRANSITIONS
-- Generated from YAML transitions definition
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE order_status_transitions (
    from_status order_status NOT NULL,
    to_status order_status NOT NULL,
    PRIMARY KEY (from_status, to_status)
);

INSERT INTO order_status_transitions (from_status, to_status) VALUES
    ('pending', 'submitted'),
    ('pending', 'cancelled'),
    ('submitted', 'completed'),
    ('submitted', 'cancelled');

-- Function to validate transitions (STABLE - reads from transition table)
CREATE OR REPLACE FUNCTION valid_order_transition(
    p_from order_status,
    p_to order_status
) RETURNS BOOLEAN
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM order_status_transitions
        WHERE from_status = p_from AND to_status = p_to
    );
$$;
```

### 2.2 Value Objects as Composite Types

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- VALUE OBJECTS
-- Generated from YAML value_objects section
-- ═══════════════════════════════════════════════════════════════════════════

-- OrderItem value object
CREATE TYPE order_item AS (
    product_id      TEXT,
    quantity        INTEGER,
    unit_price      DECIMAL(12, 2)
);

-- Money value object (if defined in YAML)
CREATE TYPE money AS (
    amount          DECIMAL(12, 2),
    currency        CHAR(3)
);

-- Address value object (if defined in YAML)
CREATE TYPE address AS (
    street          TEXT,
    city            TEXT,
    state           TEXT,
    postal_code     TEXT,
    country         CHAR(2)
);
```

### 2.3 Aggregate State Type

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER STATE
-- Generated from YAML aggregate state definition
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TYPE order_state AS (
    order_id        TEXT,
    customer_id     TEXT,
    status          order_status,
    total_amount    DECIMAL(12, 2),
    item_count      INTEGER,
    items           order_item[]
);

COMMENT ON TYPE order_state IS
'Aggregate state for Order. Generated from YAML.
Fields:
  - order_id: Unique identifier
  - customer_id: Customer who owns this order
  - status: Current state (pending → submitted → completed/cancelled)
  - total_amount: Sum of all item prices
  - item_count: Number of items (denormalized for O(1) access)
  - items: Array of order items';
```

### 2.4 Command Type

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER COMMANDS
-- Union type for all commands. Generated from YAML commands section.
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TYPE order_command AS (
    -- Discriminator
    command_type    TEXT,

    -- CreateOrder fields
    order_id        TEXT,
    customer_id     TEXT,

    -- AddItem fields
    product_id      TEXT,
    quantity        INTEGER,
    unit_price      DECIMAL(12, 2),

    -- CancelOrder fields
    reason          TEXT

    -- SubmitOrder has no additional fields
    -- CompleteOrder has no additional fields
);

COMMENT ON TYPE order_command IS
'Union type for Order commands. Use command_type to discriminate:
  - CreateOrder: order_id, customer_id
  - AddItem: product_id, quantity, unit_price
  - SubmitOrder: (no additional fields)
  - CancelOrder: reason
  - CompleteOrder: (no additional fields)';
```

### 2.5 Event Type

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER EVENTS
-- Union type for all events. Generated from YAML events section.
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TYPE order_event AS (
    -- Discriminator
    event_type      TEXT,

    -- OrderCreated fields
    order_id        TEXT,
    customer_id     TEXT,

    -- ItemAdded fields
    product_id      TEXT,
    quantity        INTEGER,
    unit_price      DECIMAL(12, 2),

    -- OrderSubmitted fields
    submitted_at    TIMESTAMPTZ,

    -- OrderCancelled fields
    reason          TEXT,

    -- OrderCompleted fields
    completed_at    TIMESTAMPTZ
);

COMMENT ON TYPE order_event IS
'Union type for Order events. Use event_type to discriminate:
  - OrderCreated: order_id, customer_id
  - ItemAdded: product_id, quantity, unit_price
  - OrderSubmitted: submitted_at
  - OrderCancelled: reason
  - OrderCompleted: completed_at';
```

### 2.6 Result Type

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- PROCESS RESULT
-- Standard result type for all process functions
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TYPE order_result AS (
    success         BOOLEAN,
    error_code      TEXT,
    error_message   TEXT,
    events          order_event[]
);

COMMENT ON TYPE order_result IS
'Result of order_process(). Check success first:
  - success=true: events contains produced events
  - success=false: error_code and error_message describe the failure';
```

---

## 3. Schema Design

### 3.1 Events Table (Typed Columns)

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- EVENTS TABLE
-- Single table with typed columns for all event types.
-- Each event type uses only its relevant columns; others are NULL.
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE events (
    -- Identity
    id              BIGSERIAL PRIMARY KEY,
    stream_id       TEXT NOT NULL,
    version         INTEGER NOT NULL,
    event_type      TEXT NOT NULL,

    -- Metadata
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    correlation_id  TEXT,
    causation_id    TEXT,
    actor_id        TEXT,

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCreated fields
    -- ───────────────────────────────────────────────────────────────────
    order_id        TEXT,
    customer_id     TEXT,

    -- ───────────────────────────────────────────────────────────────────
    -- ItemAdded fields
    -- ───────────────────────────────────────────────────────────────────
    product_id      TEXT,
    quantity        INTEGER CHECK (quantity IS NULL OR quantity >= 1),
    unit_price      DECIMAL(12, 2) CHECK (unit_price IS NULL OR unit_price >= 0),

    -- ───────────────────────────────────────────────────────────────────
    -- OrderSubmitted fields
    -- ───────────────────────────────────────────────────────────────────
    submitted_at    TIMESTAMPTZ,

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCancelled fields
    -- ───────────────────────────────────────────────────────────────────
    reason          TEXT,

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCompleted fields
    -- ───────────────────────────────────────────────────────────────────
    completed_at    TIMESTAMPTZ,

    -- Constraints
    UNIQUE (stream_id, version)
);

-- ═══════════════════════════════════════════════════════════════════════════
-- EVENT INDEXES
-- ═══════════════════════════════════════════════════════════════════════════

-- Primary access pattern: load events for a stream
CREATE INDEX idx_events_stream ON events (stream_id, version);

-- Partial indexes for event type queries
CREATE INDEX idx_events_order_created ON events (stream_id, created_at)
    WHERE event_type = 'OrderCreated';
CREATE INDEX idx_events_item_added ON events (stream_id, created_at)
    WHERE event_type = 'ItemAdded';

-- Correlation tracking
CREATE INDEX idx_events_correlation ON events (correlation_id)
    WHERE correlation_id IS NOT NULL;
```

### 3.2 Projection Tables (Normalized)

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDERS PROJECTION
-- Strongly typed read model. No JSONB.
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE orders_projection (
    -- Primary key
    order_id        TEXT PRIMARY KEY,

    -- Typed fields
    status          order_status NOT NULL,
    customer_id     TEXT NOT NULL,
    total_amount    DECIMAL(12, 2) NOT NULL DEFAULT 0
                    CHECK (total_amount >= 0),
    item_count      INTEGER NOT NULL DEFAULT 0
                    CHECK (item_count >= 0),

    -- Timestamps
    created_at      TIMESTAMPTZ NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL,
    submitted_at    TIMESTAMPTZ,
    cancelled_at    TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,

    -- Cancellation reason (if cancelled)
    cancel_reason   TEXT,

    -- Version tracking
    last_event_id   BIGINT REFERENCES events(id)
);

-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER ITEMS TABLE
-- Normalized child table instead of JSONB array
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE order_items (
    id              BIGSERIAL PRIMARY KEY,
    order_id        TEXT NOT NULL REFERENCES orders_projection(order_id) ON DELETE CASCADE,

    -- Item details (typed, with constraints from YAML)
    product_id      TEXT NOT NULL,
    quantity        INTEGER NOT NULL CHECK (quantity >= 1),
    unit_price      DECIMAL(12, 2) NOT NULL CHECK (unit_price >= 0),

    -- Computed column
    line_total      DECIMAL(12, 2) GENERATED ALWAYS AS (quantity * unit_price) STORED,

    -- When this item was added
    added_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    added_event_id  BIGINT REFERENCES events(id)
);

-- ═══════════════════════════════════════════════════════════════════════════
-- PROJECTION INDEXES
-- ═══════════════════════════════════════════════════════════════════════════

CREATE INDEX idx_orders_status ON orders_projection (status);
CREATE INDEX idx_orders_customer ON orders_projection (customer_id);
CREATE INDEX idx_orders_created ON orders_projection (created_at DESC);

-- Partial indexes for common queries
CREATE INDEX idx_orders_pending ON orders_projection (created_at DESC)
    WHERE status = 'pending';
CREATE INDEX idx_orders_customer_active ON orders_projection (customer_id, created_at DESC)
    WHERE status IN ('pending', 'submitted');

CREATE INDEX idx_order_items_order ON order_items (order_id);
CREATE INDEX idx_order_items_product ON order_items (product_id);
```

---

## 4. Pure Functions (Functional Core)

### 4.1 Process Function (Command → Events)

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_PROCESS
-- Pure function: (State, Command) → Result
-- IMMUTABLE: No side effects, deterministic output
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_process(
    current_state order_state,
    command order_command,
    p_timestamp TIMESTAMPTZ DEFAULT NULL  -- Caller provides timestamp for determinism
) RETURNS order_result
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    v_result order_result;
    v_event order_event;
BEGIN
    -- Initialize result
    v_result.success := true;
    v_result.events := ARRAY[]::order_event[];

    CASE command.command_type

    -- ───────────────────────────────────────────────────────────────────
    -- CreateOrder
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'CreateOrder' THEN
        -- Validate: Order must not exist
        IF current_state.order_id IS NOT NULL THEN
            RETURN (
                false,
                'OrderAlreadyExists',
                format('Order %s already exists', current_state.order_id),
                NULL
            )::order_result;
        END IF;

        -- Validate: Required fields
        IF command.order_id IS NULL OR command.order_id = '' THEN
            RETURN (false, 'MissingOrderId', 'order_id is required', NULL)::order_result;
        END IF;

        IF command.customer_id IS NULL OR command.customer_id = '' THEN
            RETURN (false, 'MissingCustomerId', 'customer_id is required', NULL)::order_result;
        END IF;

        -- Produce event
        v_event.event_type := 'OrderCreated';
        v_event.order_id := command.order_id;
        v_event.customer_id := command.customer_id;
        v_result.events := array_append(v_result.events, v_event);

    -- ───────────────────────────────────────────────────────────────────
    -- AddItem
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'AddItem' THEN
        -- Validate: Order must exist
        IF current_state.order_id IS NULL THEN
            RETURN (false, 'OrderNotFound', 'Order does not exist', NULL)::order_result;
        END IF;

        -- Validate: Order must be pending
        IF current_state.status != 'pending' THEN
            RETURN (
                false,
                'InvalidOrderStatus',
                format('Cannot add items to order with status: %s', current_state.status),
                NULL
            )::order_result;
        END IF;

        -- Validate: Quantity must be positive
        IF command.quantity IS NULL OR command.quantity < 1 THEN
            RETURN (false, 'InvalidQuantity', 'Quantity must be at least 1', NULL)::order_result;
        END IF;

        -- Validate: Unit price must be non-negative
        IF command.unit_price IS NULL OR command.unit_price < 0 THEN
            RETURN (false, 'InvalidPrice', 'Unit price must be non-negative', NULL)::order_result;
        END IF;

        -- Validate: Max 100 items
        IF current_state.item_count >= 100 THEN
            RETURN (false, 'MaxItemsExceeded', 'Cannot add more than 100 items', NULL)::order_result;
        END IF;

        -- Produce event
        v_event.event_type := 'ItemAdded';
        v_event.product_id := command.product_id;
        v_event.quantity := command.quantity;
        v_event.unit_price := command.unit_price;
        v_result.events := array_append(v_result.events, v_event);

    -- ───────────────────────────────────────────────────────────────────
    -- SubmitOrder
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'SubmitOrder' THEN
        -- Validate: Order must exist
        IF current_state.order_id IS NULL THEN
            RETURN (false, 'OrderNotFound', 'Order does not exist', NULL)::order_result;
        END IF;

        -- Validate: Must be pending (state machine)
        IF current_state.status != 'pending' THEN
            RETURN (
                false,
                'InvalidOrderStatus',
                format('Cannot submit order with status: %s', current_state.status),
                NULL
            )::order_result;
        END IF;

        -- Validate: Must have items
        IF current_state.item_count = 0 THEN
            RETURN (false, 'EmptyOrder', 'Cannot submit an empty order', NULL)::order_result;
        END IF;

        -- Produce event (timestamp provided by caller for determinism)
        v_event.event_type := 'OrderSubmitted';
        v_event.submitted_at := p_timestamp;
        v_result.events := array_append(v_result.events, v_event);

    -- ───────────────────────────────────────────────────────────────────
    -- CancelOrder
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'CancelOrder' THEN
        -- Validate: Order must exist
        IF current_state.order_id IS NULL THEN
            RETURN (false, 'OrderNotFound', 'Order does not exist', NULL)::order_result;
        END IF;

        -- Validate: State machine allows cancellation
        IF current_state.status NOT IN ('pending', 'submitted') THEN
            RETURN (
                false,
                'InvalidOrderStatus',
                format('Cannot cancel order with status: %s', current_state.status),
                NULL
            )::order_result;
        END IF;

        -- Produce event
        v_event.event_type := 'OrderCancelled';
        v_event.reason := COALESCE(command.reason, 'No reason provided');
        v_result.events := array_append(v_result.events, v_event);

    -- ───────────────────────────────────────────────────────────────────
    -- CompleteOrder
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'CompleteOrder' THEN
        -- Validate: Order must exist
        IF current_state.order_id IS NULL THEN
            RETURN (false, 'OrderNotFound', 'Order does not exist', NULL)::order_result;
        END IF;

        -- Validate: Must be submitted
        IF current_state.status != 'submitted' THEN
            RETURN (
                false,
                'InvalidOrderStatus',
                format('Cannot complete order with status: %s', current_state.status),
                NULL
            )::order_result;
        END IF;

        -- Produce event (timestamp provided by caller for determinism)
        v_event.event_type := 'OrderCompleted';
        v_event.completed_at := p_timestamp;
        v_result.events := array_append(v_result.events, v_event);

    -- ───────────────────────────────────────────────────────────────────
    -- Unknown Command
    -- ───────────────────────────────────────────────────────────────────
    ELSE
        RETURN (
            false,
            'UnknownCommand',
            format('Unknown command type: %s', command.command_type),
            NULL
        )::order_result;

    END CASE;

    RETURN v_result;
END;
$$;

COMMENT ON FUNCTION order_process(order_state, order_command, TIMESTAMPTZ) IS
'Pure command processor for Order aggregate.
Input: Current state + Command + Timestamp (for events that need it)
Output: Result with success flag and produced events (or error)
Properties: IMMUTABLE, no side effects, deterministic
Note: Timestamp is passed in (not computed) to maintain immutability';
```

### 4.2 Apply Function (State + Event → State)

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_APPLY
-- Pure function: (State, Event) → State
-- IMMUTABLE: No side effects, deterministic output
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_apply(
    current_state order_state,
    event order_event
) RETURNS order_state
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    v_state order_state;
    v_item order_item;
BEGIN
    -- Start with current state (or empty if NULL)
    IF current_state IS NULL THEN
        v_state := ROW(NULL, NULL, NULL, 0, 0, ARRAY[]::order_item[])::order_state;
    ELSE
        v_state := current_state;
    END IF;

    CASE event.event_type

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCreated
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderCreated' THEN
        v_state.order_id := event.order_id;
        v_state.customer_id := event.customer_id;
        v_state.status := 'pending';
        v_state.total_amount := 0;
        v_state.item_count := 0;
        v_state.items := ARRAY[]::order_item[];

    -- ───────────────────────────────────────────────────────────────────
    -- ItemAdded
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'ItemAdded' THEN
        -- Create item
        v_item := ROW(
            event.product_id,
            event.quantity,
            event.unit_price
        )::order_item;

        -- Update state
        v_state.items := array_append(v_state.items, v_item);
        v_state.item_count := v_state.item_count + 1;
        v_state.total_amount := v_state.total_amount +
            (event.quantity * event.unit_price);

    -- ───────────────────────────────────────────────────────────────────
    -- OrderSubmitted
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderSubmitted' THEN
        v_state.status := 'submitted';

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCancelled
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderCancelled' THEN
        v_state.status := 'cancelled';

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCompleted
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderCompleted' THEN
        v_state.status := 'completed';

    -- ───────────────────────────────────────────────────────────────────
    -- Unknown Event (ignored for forward compatibility)
    -- ───────────────────────────────────────────────────────────────────
    ELSE
        NULL;

    END CASE;

    RETURN v_state;
END;
$$;

COMMENT ON FUNCTION order_apply(order_state, order_event) IS
'Pure event folder for Order aggregate.
Input: Current state + Event
Output: New state with event applied
Properties: IMMUTABLE, no side effects, deterministic';
```

---

## 5. Testing Pure Functions

### 5.1 Unit Tests (No Database Tables Needed)

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- PURE FUNCTION TESTS
-- These tests run against IMMUTABLE functions only.
-- No tables are queried or modified.
-- ═══════════════════════════════════════════════════════════════════════════

DO $$
DECLARE
    v_state order_state;
    v_command order_command;
    v_result order_result;
    v_event order_event;
    v_test_count INTEGER := 0;
    v_pass_count INTEGER := 0;
BEGIN
    RAISE NOTICE '═══════════════════════════════════════════════════════════════';
    RAISE NOTICE 'ORDER AGGREGATE PURE FUNCTION TESTS';
    RAISE NOTICE '═══════════════════════════════════════════════════════════════';

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST 1: CreateOrder on empty state
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_state := NULL;
    v_command := ROW(
        'CreateOrder',      -- command_type
        'order-123',        -- order_id
        'cust-456',         -- customer_id
        NULL, NULL, NULL,   -- AddItem fields
        NULL                -- reason
    )::order_command;

    v_result := order_process(v_state, v_command, NULL);  -- No timestamp needed for CreateOrder

    IF v_result.success
       AND array_length(v_result.events, 1) = 1
       AND v_result.events[1].event_type = 'OrderCreated'
       AND v_result.events[1].order_id = 'order-123'
       AND v_result.events[1].customer_id = 'cust-456'
    THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 1: CreateOrder on empty state';
    ELSE
        RAISE NOTICE '✗ TEST 1: CreateOrder on empty state - FAILED';
        RAISE NOTICE '  Result: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST 2: Apply OrderCreated event
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_state := order_apply(NULL, v_result.events[1]);

    IF v_state.order_id = 'order-123'
       AND v_state.customer_id = 'cust-456'
       AND v_state.status = 'pending'
       AND v_state.total_amount = 0
       AND v_state.item_count = 0
       AND array_length(v_state.items, 1) IS NULL
    THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 2: Apply OrderCreated sets initial state';
    ELSE
        RAISE NOTICE '✗ TEST 2: Apply OrderCreated - FAILED';
        RAISE NOTICE '  State: %', v_state;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST 3: CreateOrder on existing order (should fail)
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_command := ROW('CreateOrder', 'order-999', 'cust-999', NULL, NULL, NULL, NULL)::order_command;
    v_result := order_process(v_state, v_command, NULL);

    IF NOT v_result.success AND v_result.error_code = 'OrderAlreadyExists' THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 3: CreateOrder on existing order returns error';
    ELSE
        RAISE NOTICE '✗ TEST 3: CreateOrder on existing order - FAILED';
        RAISE NOTICE '  Result: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST 4: AddItem to pending order
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_command := ROW(
        'AddItem',
        NULL, NULL,         -- CreateOrder fields
        'prod-1', 2, 10.00, -- product_id, quantity, unit_price
        NULL                -- reason
    )::order_command;

    v_result := order_process(v_state, v_command, NULL);  -- No timestamp needed for AddItem

    IF v_result.success
       AND v_result.events[1].event_type = 'ItemAdded'
       AND v_result.events[1].product_id = 'prod-1'
       AND v_result.events[1].quantity = 2
       AND v_result.events[1].unit_price = 10.00
    THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 4: AddItem to pending order';
    ELSE
        RAISE NOTICE '✗ TEST 4: AddItem to pending order - FAILED';
        RAISE NOTICE '  Result: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST 5: Apply ItemAdded event
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_state := order_apply(v_state, v_result.events[1]);

    IF v_state.item_count = 1
       AND v_state.total_amount = 20.00
       AND array_length(v_state.items, 1) = 1
       AND v_state.items[1].product_id = 'prod-1'
       AND v_state.items[1].quantity = 2
       AND v_state.items[1].unit_price = 10.00
    THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 5: Apply ItemAdded updates state correctly';
    ELSE
        RAISE NOTICE '✗ TEST 5: Apply ItemAdded - FAILED';
        RAISE NOTICE '  State: %', v_state;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST 6: AddItem with invalid quantity (should fail)
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_command := ROW('AddItem', NULL, NULL, 'prod-2', 0, 5.00, NULL)::order_command;
    v_result := order_process(v_state, v_command, NULL);

    IF NOT v_result.success AND v_result.error_code = 'InvalidQuantity' THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 6: AddItem with zero quantity returns error';
    ELSE
        RAISE NOTICE '✗ TEST 6: AddItem with zero quantity - FAILED';
        RAISE NOTICE '  Result: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST 7: SubmitOrder with empty order (should fail)
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    -- Create fresh state with no items
    v_state := ROW('order-empty', 'cust-1', 'pending', 0, 0, ARRAY[]::order_item[])::order_state;
    v_command := ROW('SubmitOrder', NULL, NULL, NULL, NULL, NULL, NULL)::order_command;
    v_result := order_process(v_state, v_command, '2024-01-01 12:00:00+00'::TIMESTAMPTZ);

    IF NOT v_result.success AND v_result.error_code = 'EmptyOrder' THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 7: SubmitOrder on empty order returns error';
    ELSE
        RAISE NOTICE '✗ TEST 7: SubmitOrder on empty order - FAILED';
        RAISE NOTICE '  Result: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST 8: Full workflow: Create → AddItem → Submit
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;

    -- Create order
    v_state := NULL;
    v_command := ROW('CreateOrder', 'order-workflow', 'cust-wf', NULL, NULL, NULL, NULL)::order_command;
    v_result := order_process(v_state, v_command, NULL);
    v_state := order_apply(v_state, v_result.events[1]);

    -- Add item
    v_command := ROW('AddItem', NULL, NULL, 'prod-wf', 3, 15.00, NULL)::order_command;
    v_result := order_process(v_state, v_command, NULL);
    v_state := order_apply(v_state, v_result.events[1]);

    -- Submit (timestamp required)
    v_command := ROW('SubmitOrder', NULL, NULL, NULL, NULL, NULL, NULL)::order_command;
    v_result := order_process(v_state, v_command, '2024-01-01 12:00:00+00'::TIMESTAMPTZ);
    v_state := order_apply(v_state, v_result.events[1]);

    IF v_state.status = 'submitted'
       AND v_state.item_count = 1
       AND v_state.total_amount = 45.00
    THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 8: Full workflow Create → AddItem → Submit';
    ELSE
        RAISE NOTICE '✗ TEST 8: Full workflow - FAILED';
        RAISE NOTICE '  Final state: %', v_state;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST 9: Cannot add items to submitted order
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_command := ROW('AddItem', NULL, NULL, 'prod-x', 1, 10.00, NULL)::order_command;
    v_result := order_process(v_state, v_command, NULL);

    IF NOT v_result.success AND v_result.error_code = 'InvalidOrderStatus' THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 9: Cannot add items to submitted order';
    ELSE
        RAISE NOTICE '✗ TEST 9: AddItem to submitted order - FAILED';
        RAISE NOTICE '  Result: %', v_result;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- TEST 10: Complete submitted order
    -- ═══════════════════════════════════════════════════════════════════
    v_test_count := v_test_count + 1;
    v_command := ROW('CompleteOrder', NULL, NULL, NULL, NULL, NULL, NULL)::order_command;
    v_result := order_process(v_state, v_command, '2024-01-01 14:00:00+00'::TIMESTAMPTZ);
    v_state := order_apply(v_state, v_result.events[1]);

    IF v_state.status = 'completed' THEN
        v_pass_count := v_pass_count + 1;
        RAISE NOTICE '✓ TEST 10: Complete submitted order';
    ELSE
        RAISE NOTICE '✗ TEST 10: Complete submitted order - FAILED';
        RAISE NOTICE '  State: %', v_state;
    END IF;

    -- ═══════════════════════════════════════════════════════════════════
    -- SUMMARY
    -- ═══════════════════════════════════════════════════════════════════
    RAISE NOTICE '═══════════════════════════════════════════════════════════════';
    RAISE NOTICE 'RESULTS: % / % tests passed', v_pass_count, v_test_count;
    RAISE NOTICE '═══════════════════════════════════════════════════════════════';

    IF v_pass_count < v_test_count THEN
        RAISE EXCEPTION 'Some tests failed!';
    END IF;
END;
$$;
```

---

## 6. Shell Functions (Imperative Shell)

### 6.1 Load State from Projection

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_LOAD_STATE
-- Load current state from projection + items table
-- O(1) for order + O(n) for items where n = item count
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_load_state(p_order_id TEXT)
RETURNS order_state
LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_state order_state;
    v_items order_item[];
BEGIN
    -- Load main projection
    SELECT
        o.order_id,
        o.customer_id,
        o.status,
        o.total_amount,
        o.item_count,
        NULL::order_item[]  -- Will be populated below
    INTO v_state
    FROM orders_projection o
    WHERE o.order_id = p_order_id;

    -- If not found, return NULL state
    IF v_state.order_id IS NULL THEN
        RETURN NULL;
    END IF;

    -- Load items from normalized table
    SELECT array_agg(ROW(i.product_id, i.quantity, i.unit_price)::order_item)
    INTO v_items
    FROM order_items i
    WHERE i.order_id = p_order_id
    ORDER BY i.id;

    v_state.items := COALESCE(v_items, ARRAY[]::order_item[]);

    RETURN v_state;
END;
$$;
```

### 6.2 Handle Function (Full Command Processing)

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_HANDLE
-- Complete command handler: Load → Process → Persist → Project → Notify
-- This is the ONLY function that performs I/O
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_handle(
    p_stream_id TEXT,
    p_command order_command,
    p_correlation_id TEXT DEFAULT NULL,
    p_actor_id TEXT DEFAULT NULL
) RETURNS order_result
LANGUAGE plpgsql AS $$
DECLARE
    v_state order_state;
    v_result order_result;
    v_event order_event;
    v_current_version INTEGER;
    v_new_version INTEGER;
    v_event_id BIGINT;
    v_timestamp TIMESTAMPTZ := now();
BEGIN
    -- ───────────────────────────────────────────────────────────────────
    -- 1. LOAD: Get current state from projection
    -- ───────────────────────────────────────────────────────────────────
    v_state := order_load_state(SUBSTRING(p_stream_id FROM 7));  -- Remove 'order-' prefix

    -- Get current version
    SELECT COALESCE(MAX(version), 0)
    INTO v_current_version
    FROM events
    WHERE stream_id = p_stream_id;

    -- ───────────────────────────────────────────────────────────────────
    -- 2. PROCESS: Call pure function (pass timestamp for determinism)
    -- ───────────────────────────────────────────────────────────────────
    v_result := order_process(v_state, p_command, v_timestamp);

    -- If processing failed, return immediately (no persistence)
    IF NOT v_result.success THEN
        RETURN v_result;
    END IF;

    -- ───────────────────────────────────────────────────────────────────
    -- 3. PERSIST: Store events with typed columns
    -- ───────────────────────────────────────────────────────────────────
    v_new_version := v_current_version;

    FOREACH v_event IN ARRAY v_result.events
    LOOP
        v_new_version := v_new_version + 1;

        INSERT INTO events (
            stream_id,
            version,
            event_type,
            created_at,
            correlation_id,
            causation_id,
            actor_id,
            -- Event-specific columns
            order_id,
            customer_id,
            product_id,
            quantity,
            unit_price,
            submitted_at,
            reason,
            completed_at
        ) VALUES (
            p_stream_id,
            v_new_version,
            v_event.event_type,
            v_timestamp,
            p_correlation_id,
            NULL,  -- causation_id would be previous event
            p_actor_id,
            -- Event-specific values
            v_event.order_id,
            v_event.customer_id,
            v_event.product_id,
            v_event.quantity,
            v_event.unit_price,
            v_event.submitted_at,
            v_event.reason,
            v_event.completed_at
        )
        RETURNING id INTO v_event_id;
    END LOOP;

    -- ───────────────────────────────────────────────────────────────────
    -- 4. NOTIFY: Broadcast for real-time subscribers
    -- ───────────────────────────────────────────────────────────────────
    PERFORM pg_notify('order_events', json_build_object(
        'stream_id', p_stream_id,
        'version', v_new_version,
        'event_count', array_length(v_result.events, 1),
        'correlation_id', p_correlation_id
    )::text);

    RETURN v_result;
END;
$$;
```

### 6.3 Projection Trigger

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDERS_PROJECT
-- Trigger function to update projection on event insert
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION orders_project()
RETURNS TRIGGER AS $$
DECLARE
    v_order_id TEXT;
BEGIN
    -- Only process order events
    IF NEW.stream_id NOT LIKE 'order-%' THEN
        RETURN NEW;
    END IF;

    v_order_id := SUBSTRING(NEW.stream_id FROM 7);

    CASE NEW.event_type

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCreated
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderCreated' THEN
        INSERT INTO orders_projection (
            order_id,
            status,
            customer_id,
            total_amount,
            item_count,
            created_at,
            updated_at,
            last_event_id
        ) VALUES (
            NEW.order_id,
            'pending',
            NEW.customer_id,
            0,
            0,
            NEW.created_at,
            NEW.created_at,
            NEW.id
        )
        ON CONFLICT (order_id) DO UPDATE SET
            status = 'pending',
            customer_id = EXCLUDED.customer_id,
            updated_at = NEW.created_at,
            last_event_id = NEW.id;

    -- ───────────────────────────────────────────────────────────────────
    -- ItemAdded
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'ItemAdded' THEN
        -- Insert into normalized items table
        INSERT INTO order_items (
            order_id,
            product_id,
            quantity,
            unit_price,
            added_at,
            added_event_id
        ) VALUES (
            v_order_id,
            NEW.product_id,
            NEW.quantity,
            NEW.unit_price,
            NEW.created_at,
            NEW.id
        );

        -- Update projection summary
        UPDATE orders_projection
        SET
            item_count = item_count + 1,
            total_amount = total_amount + (NEW.quantity * NEW.unit_price),
            updated_at = NEW.created_at,
            last_event_id = NEW.id
        WHERE order_id = v_order_id;

    -- ───────────────────────────────────────────────────────────────────
    -- OrderSubmitted
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderSubmitted' THEN
        UPDATE orders_projection
        SET
            status = 'submitted',
            submitted_at = NEW.submitted_at,
            updated_at = NEW.created_at,
            last_event_id = NEW.id
        WHERE order_id = v_order_id;

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCancelled
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderCancelled' THEN
        UPDATE orders_projection
        SET
            status = 'cancelled',
            cancelled_at = NEW.created_at,
            cancel_reason = NEW.reason,
            updated_at = NEW.created_at,
            last_event_id = NEW.id
        WHERE order_id = v_order_id;

    -- ───────────────────────────────────────────────────────────────────
    -- OrderCompleted
    -- ───────────────────────────────────────────────────────────────────
    WHEN 'OrderCompleted' THEN
        UPDATE orders_projection
        SET
            status = 'completed',
            completed_at = NEW.completed_at,
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
    EXECUTE FUNCTION orders_project();
```

### 6.4 Rebuild Projection

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDERS_REBUILD_PROJECTION
-- Rebuilds the entire projection from events (for recovery/repair)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION orders_rebuild_projection()
RETURNS INTEGER
LANGUAGE plpgsql AS $$
DECLARE
    v_count INTEGER := 0;
    v_event RECORD;
BEGIN
    -- Clear existing projection data
    TRUNCATE order_items CASCADE;
    TRUNCATE orders_projection CASCADE;

    -- Disable trigger during rebuild
    ALTER TABLE events DISABLE TRIGGER orders_projection_trigger;

    -- Replay all order events in order
    FOR v_event IN
        SELECT *
        FROM events
        WHERE stream_id LIKE 'order-%'
        ORDER BY stream_id, version
    LOOP
        -- Simulate the trigger by manually calling project logic
        PERFORM orders_project_event(v_event);
        v_count := v_count + 1;
    END LOOP;

    -- Re-enable trigger
    ALTER TABLE events ENABLE TRIGGER orders_projection_trigger;

    RETURN v_count;
END;
$$;

-- Helper function for rebuild (same logic as trigger but takes record)
CREATE OR REPLACE FUNCTION orders_project_event(p_event RECORD)
RETURNS VOID
LANGUAGE plpgsql AS $$
DECLARE
    v_order_id TEXT;
BEGIN
    IF p_event.stream_id NOT LIKE 'order-%' THEN
        RETURN;
    END IF;

    v_order_id := SUBSTRING(p_event.stream_id FROM 7);

    CASE p_event.event_type

    WHEN 'OrderCreated' THEN
        INSERT INTO orders_projection (
            order_id, status, customer_id, total_amount, item_count,
            created_at, updated_at, last_event_id
        ) VALUES (
            p_event.order_id, 'pending', p_event.customer_id, 0, 0,
            p_event.created_at, p_event.created_at, p_event.id
        )
        ON CONFLICT (order_id) DO UPDATE SET
            status = 'pending',
            customer_id = EXCLUDED.customer_id,
            updated_at = p_event.created_at,
            last_event_id = p_event.id;

    WHEN 'ItemAdded' THEN
        INSERT INTO order_items (
            order_id, product_id, quantity, unit_price, added_at, added_event_id
        ) VALUES (
            v_order_id, p_event.product_id, p_event.quantity,
            p_event.unit_price, p_event.created_at, p_event.id
        );

        UPDATE orders_projection SET
            item_count = item_count + 1,
            total_amount = total_amount + (p_event.quantity * p_event.unit_price),
            updated_at = p_event.created_at,
            last_event_id = p_event.id
        WHERE order_id = v_order_id;

    WHEN 'OrderSubmitted' THEN
        UPDATE orders_projection SET
            status = 'submitted',
            submitted_at = p_event.submitted_at,
            updated_at = p_event.created_at,
            last_event_id = p_event.id
        WHERE order_id = v_order_id;

    WHEN 'OrderCancelled' THEN
        UPDATE orders_projection SET
            status = 'cancelled',
            cancelled_at = p_event.created_at,
            cancel_reason = p_event.reason,
            updated_at = p_event.created_at,
            last_event_id = p_event.id
        WHERE order_id = v_order_id;

    WHEN 'OrderCompleted' THEN
        UPDATE orders_projection SET
            status = 'completed',
            completed_at = p_event.completed_at,
            updated_at = p_event.created_at,
            last_event_id = p_event.id
        WHERE order_id = v_order_id;

    ELSE
        NULL;

    END CASE;
END;
$$;

COMMENT ON FUNCTION orders_rebuild_projection() IS
'Rebuilds the orders projection from scratch by replaying all events.
Use for:
  - Recovery after data corruption
  - Schema changes that affect projection structure
  - Adding new derived fields
Returns: Number of events processed';
```

---

## 7. Query Functions

```sql
-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_GET
-- Get single order with items
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_get(p_order_id TEXT)
RETURNS TABLE (
    order_id        TEXT,
    status          order_status,
    customer_id     TEXT,
    total_amount    DECIMAL(12, 2),
    item_count      INTEGER,
    created_at      TIMESTAMPTZ,
    updated_at      TIMESTAMPTZ,
    submitted_at    TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    cancelled_at    TIMESTAMPTZ,
    cancel_reason   TEXT,
    items           order_item[]
)
LANGUAGE plpgsql STABLE AS $$
BEGIN
    RETURN QUERY
    SELECT
        o.order_id,
        o.status,
        o.customer_id,
        o.total_amount,
        o.item_count,
        o.created_at,
        o.updated_at,
        o.submitted_at,
        o.completed_at,
        o.cancelled_at,
        o.cancel_reason,
        COALESCE(
            (SELECT array_agg(ROW(i.product_id, i.quantity, i.unit_price)::order_item)
             FROM order_items i WHERE i.order_id = o.order_id),
            ARRAY[]::order_item[]
        ) AS items
    FROM orders_projection o
    WHERE o.order_id = p_order_id;
END;
$$;

-- ═══════════════════════════════════════════════════════════════════════════
-- ORDER_LIST
-- List orders with filtering and pagination
-- ═══════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION order_list(
    p_customer_id TEXT DEFAULT NULL,
    p_status order_status DEFAULT NULL,
    p_limit INTEGER DEFAULT 20,
    p_offset INTEGER DEFAULT 0
)
RETURNS TABLE (
    order_id        TEXT,
    status          order_status,
    customer_id     TEXT,
    total_amount    DECIMAL(12, 2),
    item_count      INTEGER,
    created_at      TIMESTAMPTZ
)
LANGUAGE plpgsql STABLE AS $$
BEGIN
    RETURN QUERY
    SELECT
        o.order_id,
        o.status,
        o.customer_id,
        o.total_amount,
        o.item_count,
        o.created_at
    FROM orders_projection o
    WHERE (p_customer_id IS NULL OR o.customer_id = p_customer_id)
      AND (p_status IS NULL OR o.status = p_status)
    ORDER BY o.created_at DESC
    LIMIT p_limit
    OFFSET p_offset;
END;
$$;
```

---

## 8. YAML → PostgreSQL Code Generation

### 8.1 Type Mappings

```yaml
# YAML to PostgreSQL type mappings
type_mappings:
  primitives:
    string: TEXT
    uuid: UUID
    integer: INTEGER
    bigint: BIGINT
    decimal: "DECIMAL({{precision}}, {{scale}})"
    boolean: BOOLEAN
    timestamp: TIMESTAMPTZ
    date: DATE
    time: TIME
    json: JSONB  # Only for truly schemaless data

  complex:
    enum: |
      CREATE TYPE {{name}} AS ENUM ({{values | map: quote | join: ', '}});

    list: |
      -- Child table for {{parent}}.{{field}}
      CREATE TABLE {{parent}}_{{field}} (
          id BIGSERIAL PRIMARY KEY,
          {{parent}}_id {{parent_key_type}} NOT NULL
              REFERENCES {{parent}}_projection({{parent_key}}) ON DELETE CASCADE,
          {{#each item_fields}}
          {{name}} {{type}}{{#if constraints}} {{constraints}}{{/if}},
          {{/each}}
          created_at TIMESTAMPTZ NOT NULL DEFAULT now()
      );

    value_object: |
      CREATE TYPE {{name}} AS (
          {{#each fields}}
          {{name}} {{type}}{{#unless @last}},{{/unless}}
          {{/each}}
      );

# Constraint mappings
constraint_mappings:
  required: "NOT NULL"
  min: "CHECK ({{field}} >= {{value}})"
  max: "CHECK ({{field}} <= {{value}})"
  min_length: "CHECK (length({{field}}) >= {{value}})"
  max_length: "CHECK (length({{field}}) <= {{value}})"
  pattern: "CHECK ({{field}} ~ '{{regex}}')"
  unique: "UNIQUE"
```

### 8.2 Generated Artifacts per Aggregate

For each aggregate defined in YAML, generate:

```
1. Types
   ├── CREATE TYPE {aggregate}_status AS ENUM (...)     -- If has status enum
   ├── CREATE TYPE {aggregate}_state AS (...)           -- State composite type
   ├── CREATE TYPE {aggregate}_command AS (...)         -- Command union type
   ├── CREATE TYPE {aggregate}_event AS (...)           -- Event union type
   └── CREATE TYPE {aggregate}_result AS (...)          -- Result type

2. Tables
   ├── events (add columns for this aggregate's events)
   ├── {aggregate}_projection (main projection)
   ├── {aggregate}_{list_field} (for each list field)
   └── {aggregate}_status_transitions (if has state machine)

3. Functions
   ├── {aggregate}_process(state, command, timestamp) → result  -- Pure, IMMUTABLE
   ├── {aggregate}_apply(state, event) → state                  -- Pure, IMMUTABLE
   ├── {aggregate}_load_state(id) → state                       -- STABLE
   ├── {aggregate}_handle(stream_id, command) → result          -- Full I/O
   ├── {aggregate}_project() → trigger function
   ├── {aggregate}_project_event(event) → void                  -- For rebuild
   ├── {aggregate}_rebuild_projection() → count                 -- Disaster recovery
   ├── {aggregate}_get(id) → row                                -- Query
   └── {aggregate}_list(...) → rows                             -- Query

4. Triggers
   └── {aggregate}_projection_trigger ON events
```

### 8.3 Example YAML

```yaml
domain: ecommerce
version: "1.0"

value_objects:
  Money:
    amount: { type: decimal, precision: 12, scale: 2, min: 0 }
    currency: { type: string, length: 3 }

  Address:
    street: { type: string, max_length: 200 }
    city: { type: string, max_length: 100 }
    state: { type: string, max_length: 50 }
    postal_code: { type: string, pattern: "^[0-9]{5}(-[0-9]{4})?$" }
    country: { type: string, length: 2 }

aggregates:
  Order:
    state:
      order_id: { type: string, required: true }
      customer_id: { type: string, required: true }
      status:
        type: enum
        values: [pending, submitted, cancelled, completed]
        transitions:
          pending: [submitted, cancelled]
          submitted: [completed, cancelled]
      total_amount: { type: decimal, precision: 12, scale: 2, min: 0 }
      items:
        type: list
        of: OrderItem

    value_objects:
      OrderItem:
        product_id: { type: string, required: true }
        quantity: { type: integer, min: 1, max: 1000 }
        unit_price: { type: decimal, precision: 12, scale: 2, min: 0 }

    commands:
      CreateOrder:
        fields:
          order_id: { type: string, required: true }
          customer_id: { type: string, required: true }
        guards:
          - condition: "state.order_id IS NULL"
            error: OrderAlreadyExists
        produces:
          - event: OrderCreated

      AddItem:
        fields:
          product_id: { type: string, required: true }
          quantity: { type: integer, min: 1 }
          unit_price: { type: decimal, min: 0 }
        guards:
          - condition: "state.status = 'pending'"
            error: InvalidOrderStatus
          - condition: "state.item_count < 100"
            error: MaxItemsExceeded
        produces:
          - event: ItemAdded

      SubmitOrder:
        guards:
          - condition: "state.status = 'pending'"
            error: InvalidOrderStatus
          - condition: "state.item_count > 0"
            error: EmptyOrder
        produces:
          - event: OrderSubmitted

      CancelOrder:
        fields:
          reason: { type: string, optional: true }
        guards:
          - condition: "state.status IN ('pending', 'submitted')"
            error: InvalidOrderStatus
        produces:
          - event: OrderCancelled

      CompleteOrder:
        guards:
          - condition: "state.status = 'submitted'"
            error: InvalidOrderStatus
        produces:
          - event: OrderCompleted

    events:
      OrderCreated:
        fields:
          order_id: { from: command }
          customer_id: { from: command }
        applies:
          order_id: "{{event.order_id}}"
          customer_id: "{{event.customer_id}}"
          status: pending
          total_amount: 0
          item_count: 0
          items: []

      ItemAdded:
        fields:
          product_id: { from: command }
          quantity: { from: command }
          unit_price: { from: command }
        applies:
          items: "{{state.items | append: new_item}}"
          item_count: "{{state.item_count + 1}}"
          total_amount: "{{state.total_amount + (event.quantity * event.unit_price)}}"

      OrderSubmitted:
        fields:
          submitted_at: { type: timestamp, default: now }
        applies:
          status: submitted

      OrderCancelled:
        fields:
          reason: { from: command, default: "No reason provided" }
        applies:
          status: cancelled

      OrderCompleted:
        fields:
          completed_at: { type: timestamp, default: now }
        applies:
          status: completed

    projection:
      table: orders_projection
      primary_key: order_id
      indexes:
        - columns: [status]
        - columns: [customer_id]
        - columns: [created_at]
          order: DESC
        - columns: [customer_id, status, created_at]
          where: "status IN ('pending', 'submitted')"

    queries:
      get:
        returns: single_with_items

      list:
        parameters:
          customer_id: { type: string, optional: true }
          status: { type: order_status, optional: true }
        returns: paginated
```

---

## 9. Rust Integration

### 9.1 Type Definitions

```rust
use rust_decimal::Decimal;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, Type};

/// Order status enum - matches PostgreSQL ENUM
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type)]
#[sqlx(type_name = "order_status", rename_all = "lowercase")]
pub enum OrderStatus {
    Pending,
    Submitted,
    Cancelled,
    Completed,
}

/// Order item - matches PostgreSQL composite type
#[derive(Debug, Clone, FromRow)]
pub struct OrderItem {
    pub product_id: String,
    pub quantity: i32,
    pub unit_price: Decimal,
}

/// Order state - matches PostgreSQL composite type
#[derive(Debug, Clone)]
pub struct OrderState {
    pub order_id: Option<String>,
    pub customer_id: Option<String>,
    pub status: Option<OrderStatus>,
    pub total_amount: Decimal,
    pub item_count: i32,
    pub items: Vec<OrderItem>,
}

/// Order command - matches PostgreSQL composite type
#[derive(Debug, Clone)]
pub struct OrderCommand {
    pub command_type: String,
    pub order_id: Option<String>,
    pub customer_id: Option<String>,
    pub product_id: Option<String>,
    pub quantity: Option<i32>,
    pub unit_price: Option<Decimal>,
    pub reason: Option<String>,
}

impl OrderCommand {
    pub fn create_order(order_id: impl Into<String>, customer_id: impl Into<String>) -> Self {
        Self {
            command_type: "CreateOrder".into(),
            order_id: Some(order_id.into()),
            customer_id: Some(customer_id.into()),
            product_id: None,
            quantity: None,
            unit_price: None,
            reason: None,
        }
    }

    pub fn add_item(product_id: impl Into<String>, quantity: i32, unit_price: Decimal) -> Self {
        Self {
            command_type: "AddItem".into(),
            order_id: None,
            customer_id: None,
            product_id: Some(product_id.into()),
            quantity: Some(quantity),
            unit_price: Some(unit_price),
            reason: None,
        }
    }

    pub fn submit_order() -> Self {
        Self {
            command_type: "SubmitOrder".into(),
            order_id: None,
            customer_id: None,
            product_id: None,
            quantity: None,
            unit_price: None,
            reason: None,
        }
    }

    pub fn cancel_order(reason: Option<impl Into<String>>) -> Self {
        Self {
            command_type: "CancelOrder".into(),
            order_id: None,
            customer_id: None,
            product_id: None,
            quantity: None,
            unit_price: None,
            reason: reason.map(Into::into),
        }
    }

    pub fn complete_order() -> Self {
        Self {
            command_type: "CompleteOrder".into(),
            order_id: None,
            customer_id: None,
            product_id: None,
            quantity: None,
            unit_price: None,
            reason: None,
        }
    }
}

/// Order event - matches PostgreSQL composite type
#[derive(Debug, Clone)]
pub struct OrderEvent {
    pub event_type: String,
    pub order_id: Option<String>,
    pub customer_id: Option<String>,
    pub product_id: Option<String>,
    pub quantity: Option<i32>,
    pub unit_price: Option<Decimal>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Order result - matches PostgreSQL composite type
#[derive(Debug, Clone)]
pub struct OrderResult {
    pub success: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub events: Vec<OrderEvent>,
}

/// Order projection row (from order_get function)
#[derive(Debug, Clone, FromRow)]
pub struct OrderRow {
    pub order_id: String,
    pub status: OrderStatus,
    pub customer_id: String,
    pub total_amount: Decimal,
    pub item_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub cancel_reason: Option<String>,
    pub items: Vec<OrderItem>,  // From normalized order_items table
}
```

### 9.2 Repository

```rust
use sqlx::{PgPool, postgres::PgRow, Row};

pub struct OrderRepository {
    pool: PgPool,
}

impl OrderRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Execute a command against an order aggregate
    pub async fn handle(
        &self,
        order_id: &str,
        command: OrderCommand,
        correlation_id: Option<&str>,
        actor_id: Option<&str>,
    ) -> Result<OrderResult, sqlx::Error> {
        let stream_id = format!("order-{}", order_id);

        let row = sqlx::query(
            r#"
            SELECT * FROM order_handle(
                $1::TEXT,
                ROW($2, $3, $4, $5, $6, $7, $8)::order_command,
                $9::TEXT,
                $10::TEXT
            )
            "#
        )
        .bind(&stream_id)
        .bind(&command.command_type)
        .bind(&command.order_id)
        .bind(&command.customer_id)
        .bind(&command.product_id)
        .bind(command.quantity)
        .bind(command.unit_price)
        .bind(&command.reason)
        .bind(correlation_id)
        .bind(actor_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(OrderResult {
            success: row.get("success"),
            error_code: row.get("error_code"),
            error_message: row.get("error_message"),
            events: vec![], // Parse from row if needed
        })
    }

    /// Get a single order by ID
    pub async fn get(&self, order_id: &str) -> Result<Option<OrderRow>, sqlx::Error> {
        sqlx::query_as::<_, OrderRow>(
            "SELECT * FROM order_get($1)"
        )
        .bind(order_id)
        .fetch_optional(&self.pool)
        .await
    }

    /// List orders with optional filters
    pub async fn list(
        &self,
        customer_id: Option<&str>,
        status: Option<OrderStatus>,
        limit: i32,
        offset: i32,
    ) -> Result<Vec<OrderRow>, sqlx::Error> {
        sqlx::query_as::<_, OrderRow>(
            "SELECT * FROM order_list($1, $2, $3, $4)"
        )
        .bind(customer_id)
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
    }
}
```

---

## 10. Summary

### What's Different from `architecture.md`

| Aspect | Base Architecture | Fully Typed |
|--------|-------------------|-------------|
| Event payload | `JSONB` | Typed columns |
| State in functions | `JSONB` | Composite types |
| Commands | `JSONB` | Composite types |
| Events (output) | `JSONB` | Composite types |
| Items storage | `JSONB` array | Normalized table |
| Status field | `TEXT` | `ENUM` |
| Validation | Runtime in PL/pgSQL | Database constraints |
| State machine | Code in process() | Transition table |

### Benefits

1. **Database-enforced correctness**: Invalid data can't be inserted
2. **Self-documenting schema**: Types ARE the specification
3. **Better performance**: Native types beat JSONB operations
4. **IDE support**: Autocomplete works with typed functions
5. **Cleaner migrations**: `ALTER TYPE`, `ALTER TABLE` are explicit
6. **State machine clarity**: Transition table is queryable

### Trade-offs Accepted

1. **More verbose SQL**: Each type needs explicit definition
2. **Wider events table**: Columns for all event types (sparse)
3. **Schema changes require migration**: But YAML changes do too

### When to Use This

Use **Fully Typed** when:
- YAML is the single source of truth
- AI generates all code
- Schema changes = regenerate + migrate
- You want maximum database-level validation

Use **Base Architecture** (JSONB) when:
- Rapid prototyping with frequent schema changes
- Forward compatibility is critical
- Schema is truly dynamic/user-defined
