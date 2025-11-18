# Denormalization Strategy for CQRS Projections

## Overview

In CQRS (Command Query Responsibility Segregation) systems, projections serve as optimized read models that provide fast query performance for common access patterns. The level of denormalization in these projections represents a fundamental architectural trade-off between query performance, storage efficiency, and implementation complexity.

This document explains our approach to denormalization in the ticketing system's inventory projections, the reasoning behind it, and guidance for making similar decisions in other projections.

## The Denormalization Spectrum

### Level 0: Pure Event Sourcing (No Projection)

**Approach**: Query aggregate state by replaying all events from the event store.

**Implementation**:
```rust
async fn get_inventory(event_id: EventId, section: &str) -> Inventory {
    let events = event_store.load_events(&aggregate_id).await?;
    Inventory::from_events(events)
}
```

**Characteristics**:
- **Storage**: Minimal (events only, no derived data)
- **Query Speed**: Slow (O(n) where n = number of events)
- **Consistency**: Perfect (always current)
- **Complexity**: Simple (no synchronization logic)

**When to Use**:
- Low-frequency queries (admin operations, debugging)
- Small event streams (< 100 events per aggregate)
- Audit trails and forensic analysis
- Development/testing environments

**Example**: Loading aggregate state on-demand for command processing.

---

### Level 1: Aggregate Count Projections

**Approach**: Store only aggregate/summary data, reconstruct details from events when needed.

**Implementation**:
```sql
CREATE TABLE available_seats_projection (
    event_id UUID,
    section TEXT,
    total_capacity INT,
    reserved INT,
    sold INT,
    available INT
);
```

**Characteristics**:
- **Storage**: Very small (one row per event/section)
- **Query Speed**: Fast for aggregates (O(1)), slow for details
- **Consistency**: Eventually consistent (projection lag)
- **Complexity**: Moderate (projection event handlers + idempotency)

**When to Use**:
- Dashboard metrics and counts
- Capacity checks and availability queries
- High-level reporting
- Systems where detail queries are rare

**Example**: "How many VIP seats are available for Event X?"

---

### Level 2: Hybrid Denormalization (Our Approach)

**Approach**: Store aggregate counts for fast lookups + individual item details for filtering/pagination.

**Implementation**:
```sql
-- Fast aggregate queries
CREATE TABLE available_seats_projection (
    event_id UUID,
    section TEXT,
    total_capacity INT,
    reserved INT,
    sold INT,
    available INT
);

-- Detailed queries (streamable, filterable)
CREATE TABLE seat_assignments (
    seat_id UUID PRIMARY KEY,
    event_id UUID,
    section TEXT,
    status TEXT,  -- 'available', 'reserved', 'sold'
    seat_number TEXT,
    reserved_by UUID,
    expires_at TIMESTAMPTZ
);
```

**Characteristics**:
- **Storage**: Moderate (counts + detail records)
- **Query Speed**: Fast for both aggregates (O(1)) and details (indexed queries)
- **Consistency**: Eventually consistent
- **Complexity**: Higher (multiple tables, coordinated updates)

**When to Use**:
- Mixed query patterns (counts + detail lookups)
- Large item collections (1,000s+ per aggregate)
- Need for filtering/pagination on detail records
- Real-time dashboards with drill-down capability

**Example Use Cases**:
1. "How many VIP seats are available?" → Query counts table (O(1))
2. "Show me all available seats in Row A" → Query + filter seat_assignments
3. "List first 50 available VIP seats" → Query + paginate seat_assignments
4. "Which seats expire in next 5 minutes?" → Index scan on expires_at

---

### Level 3: Full Snapshot Denormalization

**Approach**: Pre-compute and store complete aggregate snapshots with all relationships materialized.

**Implementation**:
```sql
CREATE TABLE inventory_snapshots (
    event_id UUID,
    section TEXT,
    -- Aggregate data
    total_capacity INT,
    reserved INT,
    sold INT,
    available INT,
    -- Complete snapshot as JSON/JSONB
    full_state JSONB,
    seats JSONB,
    metadata JSONB,
    last_updated TIMESTAMPTZ
);
```

**Characteristics**:
- **Storage**: Large (complete state + redundancy)
- **Query Speed**: Very fast (single query, no joins)
- **Consistency**: Eventually consistent
- **Complexity**: High (schema evolution challenges)

**When to Use**:
- Read-heavy systems (1000:1 read/write ratio)
- Complex nested data structures
- Client-side caching strategies
- Systems where query performance is critical

**Trade-offs**:
- ❌ High storage costs
- ❌ Schema migration complexity (JSONB changes)
- ❌ Potential for data inconsistency
- ✅ Minimal query latency
- ✅ Single-query access patterns

---

## Our Choice: Hybrid Denormalization for Inventory

### Rationale

For the ticketing system's inventory projection, we chose **Level 2: Hybrid Denormalization** because:

1. **Query Pattern Requirements**:
   - Need fast aggregate queries: "Is section sold out?"
   - Need detail queries: "Show available seats in section"
   - Need filtering: "Find seats by status"
   - Need pagination: Handle 50,000+ seat stadiums

2. **Scalability**:
   - Aggregate table stays small (one row per event/section)
   - Detail table can be streamed/paginated
   - Indexes support efficient filtering

3. **Balance**:
   - Not over-engineering with full snapshots
   - Not under-engineering with counts only
   - Right level of complexity for our use case

### Implementation Details

#### Projection Event Handlers

When `InventoryInitialized` event arrives:
```rust
// Insert aggregate counts
INSERT INTO available_seats_projection (event_id, section, total_capacity, ...)
VALUES (?, ?, ?, ...);

// Insert individual seat records
for seat_id in seats {
    INSERT INTO seat_assignments (seat_id, event_id, section, status, ...)
    VALUES (?, ?, ?, 'available', ...);
}
```

When `SeatsReserved` event arrives:
```rust
// Update aggregate counts
UPDATE available_seats_projection
SET reserved = reserved + ?,
    available = available - ?
WHERE event_id = ? AND section = ?;

// Update individual seat statuses
UPDATE seat_assignments
SET status = 'reserved',
    reserved_by = ?,
    expires_at = ?
WHERE seat_id IN (?);
```

#### Aggregate State Loading

When aggregate needs to load state from projection:
```rust
async fn load_inventory(&self, event_id: &EventId, section: &str)
    -> Option<((u32, u32, u32, u32), Vec<SeatAssignment>)>
{
    // Load aggregate counts (fast, small)
    let counts = query_as(
        "SELECT total_capacity, reserved, sold, available
         FROM available_seats_projection
         WHERE event_id = ? AND section = ?"
    ).fetch_one().await?;

    // Load individual seat assignments (streamable)
    let seats = query_as(
        "SELECT * FROM seat_assignments
         WHERE event_id = ? AND section = ?
         ORDER BY seat_number"
    ).fetch_all().await?;

    Some((counts, seats))
}
```

### Database Schema Design

#### Indexes for Performance

```sql
-- Fast aggregate lookups
CREATE INDEX idx_availability_lookup
    ON available_seats_projection(event_id, section);

-- Seat assignment queries
CREATE INDEX idx_seat_assignments_event_section
    ON seat_assignments(event_id, section);

-- Status filtering
CREATE INDEX idx_seat_assignments_status
    ON seat_assignments(event_id, section, status);

-- Expiration queries (for cleanup jobs)
CREATE INDEX idx_seat_assignments_expires
    ON seat_assignments(expires_at)
    WHERE expires_at IS NOT NULL;
```

#### Idempotency

Track processed reservations to handle duplicate events:
```sql
CREATE TABLE processed_reservations (
    reservation_id UUID PRIMARY KEY,
    processed_at TIMESTAMPTZ DEFAULT NOW()
);
```

## Decision Framework

### Questions to Ask

When designing a new projection, consider:

1. **Query Patterns**:
   - What queries will users run most frequently?
   - Are they aggregate queries, detail queries, or both?
   - Do we need filtering? Pagination? Sorting?

2. **Data Volume**:
   - How many items per aggregate?
   - What's the growth rate?
   - Can we fit all data in memory? In a single query?

3. **Performance Requirements**:
   - What's the acceptable query latency?
   - What's the read/write ratio?
   - Is this a hot path or admin operation?

4. **Consistency Tolerance**:
   - Can we tolerate projection lag?
   - How critical is real-time accuracy?
   - Are there compensating controls?

### Decision Matrix

| Scenario | Recommended Level | Reason |
|----------|-------------------|---------|
| Admin dashboard with <100 aggregates | Level 1 (Counts) | Small scale, simple queries |
| Product catalog with 10k items, mostly browsing | Level 2 (Hybrid) | Need counts + filtering |
| User profiles (1:1 user:aggregate) | Level 0 (Event Sourcing) | Low frequency, perfect consistency |
| Analytics dashboard (append-only metrics) | Level 3 (Full Snapshot) | Read-heavy, complex aggregations |
| Inventory with seat selection UI | Level 2 (Hybrid) | Our case: counts + details |

## Performance Characteristics

### Benchmarks (Hypothetical)

**Query**: Get availability for event X, section Y

| Level | Storage | Query Time | Notes |
|-------|---------|------------|-------|
| 0 (Event Sourcing) | 10 KB/aggregate | ~50ms | Replay 200 events |
| 1 (Counts Only) | 100 bytes/section | ~2ms | Single row lookup |
| 2 (Hybrid) | 10 KB + 100 bytes/seat | ~5ms | Counts + 100 seats |
| 3 (Full Snapshot) | 50 KB/aggregate | ~3ms | JSONB deserialize |

**Query**: Find next 50 available seats in section Y

| Level | Query Time | Notes |
|-------|------------|-------|
| 0 (Event Sourcing) | ~200ms | Reconstruct + filter all seats |
| 1 (Counts Only) | N/A | Can't query details |
| 2 (Hybrid) | ~8ms | Index scan + LIMIT 50 |
| 3 (Full Snapshot) | ~15ms | JSONB array filter in app |

## Migration Path

If requirements change over time, you can migrate between levels:

### From Level 1 → Level 2 (Add Detail Table)
1. Add seat_assignments table
2. Update projection handlers to populate both tables
3. Rebuild projection from events
4. Update query code to use detail table

### From Level 2 → Level 3 (Add Full Snapshot)
1. Add snapshot JSONB column
2. Update handlers to serialize complete state
3. Gradual rollout (keep existing tables during transition)
4. Verify snapshot queries, then remove old tables

### From Level 3 → Level 2 (Extract Details)
1. Create normalized detail table from JSONB
2. Update handlers to populate both formats
3. Migrate queries to use normalized table
4. Remove JSONB column

## Best Practices

1. **Start Simple**: Begin with Level 0 or 1, measure, then optimize
2. **Measure First**: Profile actual query patterns before denormalizing
3. **Idempotency**: Always track processed events for exactly-once semantics
4. **Indexes**: Add indexes for your query patterns, not all columns
5. **Rebuild Support**: Projection rebuild should be a one-command operation
6. **Monitoring**: Track projection lag and query performance
7. **Schema Versioning**: Plan for projection schema evolution

## Conclusion

Our hybrid denormalization strategy (Level 2) strikes the right balance for inventory management:
- **Fast aggregates** for availability checks
- **Queryable details** for seat selection UI
- **Scalable** to large venues (50k+ seats)
- **Maintainable** with clear separation of concerns

The key insight: **Projections are snapshots, not just indexes**. They should package together all the data needed by query patterns, denormalized and pre-computed during event processing, not on-demand during queries.

When in doubt, measure your actual query patterns and choose the simplest level that meets your performance requirements. You can always evolve to higher levels of denormalization as your system scales.
