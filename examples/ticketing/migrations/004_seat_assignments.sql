-- Create seat_assignments table for inventory snapshot projection
--
-- This table stores individual seat assignments as part of the denormalized
-- inventory snapshot. Combined with available_seats_projection (aggregate counts),
-- this provides a complete, queryable snapshot of inventory state.
--
-- Design rationale:
-- - Aggregate counts (available_seats_projection): Fast, small, frequently queried
-- - Individual seats (this table): Streamable, filterable, supports large inventories
--
-- This hybrid approach balances:
-- - Performance: Counts are instant, seats can be streamed/paginated
-- - Scalability: Handles 50,000+ seats per section
-- - Queryability: Can filter by status, lookup specific seats

CREATE TABLE IF NOT EXISTS seat_assignments (
    seat_id UUID PRIMARY KEY,
    event_id UUID NOT NULL,
    section TEXT NOT NULL,
    status TEXT NOT NULL,  -- 'available', 'reserved', 'sold'
    seat_number TEXT,  -- NULL for general admission
    reserved_by UUID,  -- Reservation ID (NULL if not reserved)
    expires_at TIMESTAMPTZ,  -- Reservation expiration (NULL if not reserved)
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Index for loading all seats for an event/section
CREATE INDEX idx_seat_assignments_event_section
    ON seat_assignments(event_id, section);

-- Index for filtering by status (e.g., "find available seats")
CREATE INDEX idx_seat_assignments_status
    ON seat_assignments(event_id, section, status);

-- Index for reservation expiration cleanup queries
CREATE INDEX idx_seat_assignments_expires
    ON seat_assignments(expires_at)
    WHERE expires_at IS NOT NULL;
