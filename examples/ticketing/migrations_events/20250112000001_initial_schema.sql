-- Initial schema for event store with event versioning support
-- This migration creates the events table with support for schema evolution

-- Events table with event versioning
CREATE TABLE events (
    id BIGSERIAL PRIMARY KEY,
    aggregate_id UUID NOT NULL,
    aggregate_type VARCHAR(255) NOT NULL,
    event_type VARCHAR(255) NOT NULL,
    event_version INT NOT NULL DEFAULT 1,  -- Critical for schema evolution
    event_data BYTEA NOT NULL,             -- bincode serialized event data
    metadata JSONB,                         -- Additional metadata (user_id, correlation_id, etc.)
    sequence_number BIGINT NOT NULL,        -- Per-aggregate sequence for optimistic concurrency
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Ensure no duplicate sequence numbers per aggregate
    UNIQUE(aggregate_id, sequence_number)
);

-- Index for loading aggregate event history (most common query)
CREATE INDEX idx_events_aggregate ON events(aggregate_id, sequence_number);

-- Index for event type + version lookups (for event migration queries)
CREATE INDEX idx_events_type_version ON events(event_type, event_version);

-- Index for time-based queries (for event replay, debugging)
CREATE INDEX idx_events_created_at ON events(created_at);

-- Index for aggregate type queries (for system-wide operations)
CREATE INDEX idx_events_aggregate_type ON events(aggregate_type);

-- Composite index for efficient aggregate type + event type queries
CREATE INDEX idx_events_aggregate_event_type ON events(aggregate_type, event_type);
