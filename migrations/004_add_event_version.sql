-- Add event_version column for schema evolution support
-- This enables multiple versions of the same event type to coexist

-- Add event_version column with default value of 1 for backward compatibility
ALTER TABLE events
ADD COLUMN IF NOT EXISTS event_version INTEGER NOT NULL DEFAULT 1;

-- Backfill existing events with version 1 (extracted from event_type suffix if present)
-- Events with ".v2", ".v3", etc. will get their version extracted
-- Events without version suffix default to version 1
UPDATE events
SET event_version = CASE
    -- Extract version from ".vN" suffix (e.g., "OrderPlaced.v2" -> 2)
    WHEN event_type ~ '\.v[0-9]+$' THEN
        CAST(REGEXP_REPLACE(event_type, '.*\.v([0-9]+)$', '\1') AS INTEGER)
    ELSE 1
END
WHERE event_version = 1;

-- Create index for querying events by type and version
CREATE INDEX IF NOT EXISTS idx_events_type_version ON events(event_type, event_version);

-- Add comment for documentation
COMMENT ON COLUMN events.event_version IS 'Schema version extracted from event_type suffix (e.g., ".v2" -> 2), defaults to 1 for backward compatibility';
