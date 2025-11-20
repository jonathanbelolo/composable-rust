-- Add owner_id column to events_projection for efficient ownership queries
-- This extracts owner_id from the JSONB data for indexing and fast lookups

-- Add owner_id column (nullable initially to handle existing rows)
ALTER TABLE events_projection
ADD COLUMN IF NOT EXISTS owner_id UUID;

-- Backfill owner_id from existing JSONB data (if any rows exist)
UPDATE events_projection
SET owner_id = (data->>'owner_id')::UUID
WHERE owner_id IS NULL;

-- Make owner_id NOT NULL for new rows
ALTER TABLE events_projection
ALTER COLUMN owner_id SET NOT NULL;

-- Create index for efficient "find all events by owner" queries
CREATE INDEX IF NOT EXISTS idx_events_owner_id
    ON events_projection(owner_id);

-- Create composite index for common query pattern: "find my events with specific status"
CREATE INDEX IF NOT EXISTS idx_events_owner_status
    ON events_projection(owner_id, (data->>'status'));
