-- Event-sourcing log (write side). Single source of truth for all aggregates/sagas.
-- Consolidated from the workspace event-store DDL (events table + event_version).
-- This is the table postgres-next's PostgresEventStore reads/appends.

CREATE TABLE IF NOT EXISTS events (
    stream_id     TEXT    NOT NULL,            -- Aggregate/saga stream id (e.g. "saga-reservation-<uuid>")
    version       BIGINT  NOT NULL,            -- Event version within the stream (optimistic concurrency)
    event_type    TEXT    NOT NULL,            -- Event type name for deserialization routing
    event_version INTEGER NOT NULL DEFAULT 1,  -- Schema version of the event type
    event_data    BYTEA   NOT NULL,            -- Bincode-serialized event payload
    metadata      JSONB,                       -- Optional correlation/causation/user metadata
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (stream_id, version)
);

CREATE INDEX IF NOT EXISTS idx_events_created ON events (created_at);
CREATE INDEX IF NOT EXISTS idx_events_type ON events (event_type);
CREATE INDEX IF NOT EXISTS idx_events_type_version ON events (event_type, event_version);

COMMENT ON TABLE events IS 'Immutable append-only event log (write side) for event sourcing';
