-- Available seats projection table (queryable read model)
CREATE TABLE IF NOT EXISTS available_seats_projection (
    event_id UUID NOT NULL,
    section TEXT NOT NULL,
    total_capacity INTEGER NOT NULL,
    reserved INTEGER NOT NULL DEFAULT 0,
    sold INTEGER NOT NULL DEFAULT 0,
    available INTEGER NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (event_id, section)
);

-- Index for querying by event
CREATE INDEX IF NOT EXISTS idx_available_seats_event
    ON available_seats_projection(event_id);

-- Index for querying availability
CREATE INDEX IF NOT EXISTS idx_available_seats_availability
    ON available_seats_projection(event_id, section, available DESC);

-- Processed reservations for idempotency tracking
CREATE TABLE IF NOT EXISTS processed_reservations (
    reservation_id UUID PRIMARY KEY,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_processed_reservations_processed_at
    ON processed_reservations(processed_at DESC);
