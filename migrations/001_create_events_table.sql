-- Create events table for event sourcing
-- This is an immutable append-only log with optimistic concurrency control

CREATE TABLE IF NOT EXISTS events (
    stream_id TEXT NOT NULL,           -- Aggregate ID (e.g., "order-123")
    version BIGINT NOT NULL,            -- Event version within the stream (for optimistic concurrency)
    event_type TEXT NOT NULL,           -- Event type name for deserialization routing
    event_data BYTEA NOT NULL,          -- Bincode-serialized event payload
    metadata JSONB,                     -- Optional metadata (correlation IDs, causation IDs, etc.)
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Composite primary key ensures unique version per stream
    PRIMARY KEY (stream_id, version)
);

-- Index for querying events by creation time (useful for projections)
CREATE INDEX IF NOT EXISTS idx_events_created ON events(created_at);

-- Index for querying events by type (useful for cross-aggregate queries)
CREATE INDEX IF NOT EXISTS idx_events_type ON events(event_type);

-- Comments for documentation
COMMENT ON TABLE events IS 'Immutable append-only event log for event sourcing';
COMMENT ON COLUMN events.stream_id IS 'Unique identifier for the event stream (aggregate instance)';
COMMENT ON COLUMN events.version IS 'Sequential version number for optimistic concurrency control';
COMMENT ON COLUMN events.event_type IS 'Event type name used for deserialization routing';
COMMENT ON COLUMN events.event_data IS 'Bincode-serialized event payload (binary data)';
COMMENT ON COLUMN events.metadata IS 'Optional metadata in JSONB format for correlation, causation, user context, etc.';
