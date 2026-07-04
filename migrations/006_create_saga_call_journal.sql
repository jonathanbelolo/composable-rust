-- Create saga_call_journal table: the registry-side projection of the
-- durable-saga call journal ($saga.call_dispatched / $saga.call_completed
-- marker events persisted in each saga's own stream).
--
-- One row per dispatched call. completed_at IS NULL means the call is
-- outstanding: the saga instance needs recovery (Handler::resume).
--
-- Deployment note: this migrations directory is embedded by BOTH the legacy
-- postgres crate and postgres-next via sqlx::migrate!. Once this migration
-- is applied to a database, binaries compiled before it will fail
-- run_migrations() (sqlx version-missing check) — rebuild binaries before
-- deploying against a migrated database.

CREATE TABLE IF NOT EXISTS saga_call_journal (
    stream_id     TEXT NOT NULL,                        -- Saga instance stream (e.g. "spec-saga-123")
    call_id       BIGINT NOT NULL,                      -- Stream version of the $saga.call_dispatched marker
    logic_tag     TEXT NOT NULL,                        -- DurableBusinessLogic::LOGIC_TAG of the saga type
    dispatched_at TIMESTAMPTZ NOT NULL DEFAULT now(),   -- From the marker's metadata timestamp when available
    completed_at  TIMESTAMPTZ,                          -- NULL = outstanding (needs recovery)

    -- One row per dispatched call; idempotent projection upserts key on this
    PRIMARY KEY (stream_id, call_id)
);

-- Recovery sweeps are O(outstanding): partial index over incomplete calls only
CREATE INDEX IF NOT EXISTS idx_saga_call_journal_outstanding
    ON saga_call_journal(logic_tag) WHERE completed_at IS NULL;

-- Comments for documentation
COMMENT ON TABLE saga_call_journal IS 'Registry projection of the durable-saga call journal (outstanding = dispatched minus completed)';
COMMENT ON COLUMN saga_call_journal.stream_id IS 'Event stream of the saga instance the call belongs to';
COMMENT ON COLUMN saga_call_journal.call_id IS 'CallId = stream version of the $saga.call_dispatched marker event';
COMMENT ON COLUMN saga_call_journal.logic_tag IS 'Stable saga type tag; recovery sweeps filter by it so streams are only resumed by the logic that can decode them';
COMMENT ON COLUMN saga_call_journal.dispatched_at IS 'When the call was dispatched (marker metadata timestamp, or projection time as fallback)';
COMMENT ON COLUMN saga_call_journal.completed_at IS 'When the completion marker was projected; NULL while the call is outstanding';
