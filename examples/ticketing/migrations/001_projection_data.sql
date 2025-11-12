-- Generic projection data table (used by PostgresProjectionStore)
CREATE TABLE IF NOT EXISTS projection_data (
    key TEXT PRIMARY KEY,
    data BYTEA NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_projection_data_updated
    ON projection_data(updated_at DESC);
