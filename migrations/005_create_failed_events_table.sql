-- Create failed_events table for Dead Letter Queue (DLQ)
--
-- This table stores events that failed processing after exhausting retries.
-- Operations teams can query, investigate, and manually reprocess failed events.

CREATE TABLE IF NOT EXISTS failed_events (
    -- Primary identification
    id BIGSERIAL PRIMARY KEY,

    -- Event identification (from SerializedEvent)
    stream_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    event_version INTEGER NOT NULL,
    event_data BYTEA NOT NULL,
    metadata JSONB,

    -- Original event timestamp (when it was first created)
    original_timestamp TIMESTAMPTZ NOT NULL,

    -- Failure information
    error_message TEXT NOT NULL,
    error_details TEXT,  -- Full error debug output for troubleshooting
    retry_count INTEGER NOT NULL DEFAULT 0,

    -- DLQ timestamps
    first_failed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_failed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Processing status
    status TEXT NOT NULL DEFAULT 'pending',
    -- Possible values: 'pending', 'processing', 'resolved', 'discarded'

    resolved_at TIMESTAMPTZ,
    resolved_by TEXT,  -- Username or system identifier
    resolution_notes TEXT,

    -- Ensure status is valid
    CONSTRAINT failed_events_status_check
        CHECK (status IN ('pending', 'processing', 'resolved', 'discarded'))
);

-- Index for querying pending failures (most common operation)
CREATE INDEX IF NOT EXISTS idx_failed_events_status_failed_at
ON failed_events(status, first_failed_at)
WHERE status = 'pending';

-- Index for finding failures by stream
CREATE INDEX IF NOT EXISTS idx_failed_events_stream
ON failed_events(stream_id);

-- Index for finding failures by event type
CREATE INDEX IF NOT EXISTS idx_failed_events_type
ON failed_events(event_type);

-- Index for finding failures by type and version (schema issues)
CREATE INDEX IF NOT EXISTS idx_failed_events_type_version
ON failed_events(event_type, event_version);

-- Composite index for time-based analysis
CREATE INDEX IF NOT EXISTS idx_failed_events_timestamp
ON failed_events(first_failed_at DESC);

-- Add table comment for documentation
COMMENT ON TABLE failed_events IS
'Dead Letter Queue for events that failed processing after exhausting retries. '
'Enables observability, incident response, and manual reprocessing workflows.';

COMMENT ON COLUMN failed_events.status IS
'Processing status: pending (needs attention), processing (being handled), '
'resolved (successfully reprocessed), discarded (permanently failed/ignored)';

COMMENT ON COLUMN failed_events.error_details IS
'Full error debug output including stack trace for troubleshooting';

COMMENT ON COLUMN failed_events.retry_count IS
'Number of times the operation was retried before giving up';
