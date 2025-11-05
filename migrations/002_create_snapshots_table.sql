-- Create snapshots table for performance optimization
-- Snapshots allow rebuilding aggregate state without replaying all events

CREATE TABLE IF NOT EXISTS snapshots (
    stream_id TEXT PRIMARY KEY,         -- Aggregate ID (same as events.stream_id)
    version BIGINT NOT NULL,            -- Event version at the time of snapshot
    state_data BYTEA NOT NULL,          -- Bincode-serialized aggregate state
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Comments for documentation
COMMENT ON TABLE snapshots IS 'State snapshots for performance optimization in event sourcing';
COMMENT ON COLUMN snapshots.stream_id IS 'Unique identifier for the event stream (aggregate instance)';
COMMENT ON COLUMN snapshots.version IS 'Event version at the time this snapshot was created';
COMMENT ON COLUMN snapshots.state_data IS 'Bincode-serialized aggregate state (binary data)';
COMMENT ON COLUMN snapshots.created_at IS 'Timestamp when this snapshot was created';
