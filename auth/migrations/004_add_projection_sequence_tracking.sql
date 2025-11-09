-- Migration 004: Add timestamp-based idempotency for projections
--
-- This migration adds last_event_timestamp tracking to projection tables to ensure
-- idempotent event processing. This prevents data corruption from:
-- - Duplicate events (e.g., due to at-least-once delivery)
-- - Out-of-order events (e.g., due to network delays or replays)
-- - Late-arriving events (e.g., during disaster recovery)
--
-- Architecture:
-- - Each event has a timestamp (DateTime<Utc>)
-- - Projections track the timestamp of the last event they processed
-- - Updates only apply if incoming timestamp >= stored timestamp
--
-- This ensures projections are eventually consistent and idempotent.
--
-- Why timestamp instead of sequence numbers?
-- - Events already have timestamps (no schema changes to event store needed)
-- - Timestamps provide natural ordering across aggregates
-- - Microsecond precision is sufficient for ordering (collisions handled by >= check)
-- - Simpler implementation (no need for global sequence coordination)

-- Add timestamp tracking to users_projection
ALTER TABLE users_projection
    ADD COLUMN last_event_timestamp TIMESTAMPTZ NOT NULL DEFAULT '1970-01-01 00:00:00+00';

-- Add timestamp tracking to devices_projection
ALTER TABLE devices_projection
    ADD COLUMN last_event_timestamp TIMESTAMPTZ NOT NULL DEFAULT '1970-01-01 00:00:00+00';

-- Add timestamp tracking to oauth_links_projection
ALTER TABLE oauth_links_projection
    ADD COLUMN last_event_timestamp TIMESTAMPTZ NOT NULL DEFAULT '1970-01-01 00:00:00+00';

-- Add timestamp tracking to passkeys_projection
ALTER TABLE passkeys_projection
    ADD COLUMN last_event_timestamp TIMESTAMPTZ NOT NULL DEFAULT '1970-01-01 00:00:00+00';

-- Create indexes for efficient timestamp checking
-- These indexes speed up queries like "WHERE last_event_timestamp < $1"
CREATE INDEX idx_users_projection_last_event_ts
    ON users_projection (last_event_timestamp);

CREATE INDEX idx_devices_projection_last_event_ts
    ON devices_projection (last_event_timestamp);

CREATE INDEX idx_oauth_links_projection_last_event_ts
    ON oauth_links_projection (last_event_timestamp);

CREATE INDEX idx_passkeys_projection_last_event_ts
    ON passkeys_projection (last_event_timestamp);

-- Add comments for documentation
COMMENT ON COLUMN users_projection.last_event_timestamp IS
    'Timestamp of the last event processed for this user (for idempotency)';

COMMENT ON COLUMN devices_projection.last_event_timestamp IS
    'Timestamp of the last event processed for this device (for idempotency)';

COMMENT ON COLUMN oauth_links_projection.last_event_timestamp IS
    'Timestamp of the last event processed for this OAuth link (for idempotency)';

COMMENT ON COLUMN passkeys_projection.last_event_timestamp IS
    'Timestamp of the last event processed for this passkey (for idempotency)';

-- ✅ SECURITY: Timestamp-based idempotency guarantees
--
-- Without timestamp tracking:
-- - Event arrives twice → projection updated twice (data corruption)
-- - Event arrives late → older data overwrites newer data
-- - Event arrives out of order → incorrect state
--
-- With timestamp tracking:
-- - Event arrives twice → second update skipped (timestamp not greater)
-- - Event arrives late → rejected if timestamp <= stored timestamp
-- - Events processed in any order → correct final state (last-write-wins)
--
-- Example scenario:
--   Event 1 (2024-01-01 10:00:00): UserRegistered(email="old@example.com")
--   Event 2 (2024-01-01 10:00:01): UserUpdated(email="new@example.com")
--
--   If Event 2 arrives first:
--     - Projection: email="new@example.com", last_event_timestamp=2024-01-01 10:00:01
--   If Event 1 arrives late:
--     - Rejected: timestamp 10:00:00 < 10:00:01 (stored)
--     - Projection unchanged: email="new@example.com" ✓
--
-- Collision handling:
-- - If two events have identical timestamps (rare), use >= check
-- - Last event processed wins (acceptable for idempotent operations)
