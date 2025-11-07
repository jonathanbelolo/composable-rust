-- Create projection tables for Phase 5
--
-- This migration creates the foundational tables for the projection system:
-- 1. projection_data: Generic key-value store for projection data
-- 2. projection_checkpoints: Track projection progress through event stream

-- Generic projection data table (key-value store)
--
-- Use this for simple projections that don't need complex queries.
-- For queryable projections, create custom tables with proper indexes.
CREATE TABLE IF NOT EXISTS projection_data (
    key TEXT PRIMARY KEY,
    data BYTEA NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_projection_data_updated
    ON projection_data(updated_at);

-- Projection checkpoint table (tracks progress)
--
-- Each projection maintains a checkpoint showing where it has processed
-- up to in the event stream. This enables resumption after restarts.
CREATE TABLE IF NOT EXISTS projection_checkpoints (
    projection_name TEXT PRIMARY KEY,
    event_offset BIGINT NOT NULL,
    event_timestamp TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_projection_checkpoints_updated
    ON projection_checkpoints(updated_at);

-- Example: Custom projection table for order projections (queryable)
--
-- This shows how to create a custom projection table with JSONB for
-- flexible schema and indexes for fast queries.
CREATE TABLE IF NOT EXISTS order_projections (
    id TEXT PRIMARY KEY,
    customer_id TEXT NOT NULL,
    data JSONB NOT NULL,
    total DECIMAL(10,2),
    status TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Indexes for common query patterns
CREATE INDEX IF NOT EXISTS idx_order_projections_customer
    ON order_projections(customer_id);

CREATE INDEX IF NOT EXISTS idx_order_projections_status
    ON order_projections(status);

CREATE INDEX IF NOT EXISTS idx_order_projections_created
    ON order_projections(created_at DESC);

-- GIN index for JSONB queries (useful for flexible searching)
CREATE INDEX IF NOT EXISTS idx_order_projections_data_gin
    ON order_projections USING gin(data);
