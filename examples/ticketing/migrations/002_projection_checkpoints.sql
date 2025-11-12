-- Projection checkpoint tracking (used by PostgresProjectionCheckpoint)
CREATE TABLE IF NOT EXISTS projection_checkpoints (
    projection_name TEXT PRIMARY KEY,
    event_offset BIGINT NOT NULL,
    event_timestamp TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_projection_checkpoints_updated
    ON projection_checkpoints(updated_at DESC);
