-- Create saga_correlation_index table: the query-side projection of the
-- saga await/wake correlation index ($saga.awaiting / $saga.awaiting_cleared
-- marker events persisted in each saga's own stream when it returns
-- BusinessResult::Await).
--
-- One row per PARKED saga instance: (logic_tag, correlation_key,
-- correlation_value) -> stream_id. An awaited domain event finds the instance
-- to wake via CorrelationIndex::find; the $saga.awaiting_cleared marker
-- deletes the row on wake, which is what makes event delivery idempotent
-- (a duplicate delivery finds no row and no-ops).
--
-- The row is written by SagaCorrelationProjector in the SAME transaction as
-- the saga's domain events (the atomic-persist path), so a crash can never
-- leave a saga parked-but-unfindable.
--
-- Deployment note: this migrations directory is embedded by BOTH the legacy
-- postgres crate and postgres-next via sqlx::migrate!. Once this migration
-- is applied to a database, binaries compiled before it will fail
-- run_migrations() (sqlx version-missing check) — rebuild binaries before
-- deploying against a migrated database.

CREATE TABLE IF NOT EXISTS saga_correlation_index (
    logic_tag         TEXT NOT NULL,                     -- DurableBusinessLogic::LOGIC_TAG of the saga type
    correlation_key   TEXT NOT NULL,                     -- Logical correlation dimension (e.g. "order_id")
    correlation_value TEXT NOT NULL,                     -- Instance value the awaited event must carry
    stream_id         TEXT NOT NULL,                     -- The parked saga instance's stream
    deadline          TIMESTAMPTZ,                       -- NULL = no timeout; else the await's absolute deadline
    timeout_tag       TEXT,                              -- Saga-chosen tag identifying which await (paired with deadline)
    awaiting_since    TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- One row per parked instance per correlation; idempotent projection
    -- upserts (re-await of the same key by the same instance) key on this.
    -- Two DIFFERENT instances on the same (logic_tag, key, value) is a
    -- modeling error (a correlation value must identify ONE parked instance);
    -- SagaCorrelationProjector fails loud on a stream_id mismatch rather than
    -- silently orphaning one instance.
    PRIMARY KEY (logic_tag, correlation_key, correlation_value),

    -- deadline and timeout_tag are set together or not at all (the marker
    -- writes both iff the await carries a deadline). ExpiredAwait.timeout_tag
    -- is non-optional, so this pairing is a DB-level invariant, not a hope.
    CONSTRAINT saga_correlation_deadline_tag_paired
        CHECK ((deadline IS NULL) = (timeout_tag IS NULL))
);

-- Timeout sweeps are O(due): partial index over rows that carry a deadline.
CREATE INDEX IF NOT EXISTS idx_saga_correlation_deadline
    ON saga_correlation_index(logic_tag, deadline) WHERE deadline IS NOT NULL;

-- Comments for documentation
COMMENT ON TABLE saga_correlation_index IS 'Query projection of the saga await/wake correlation index (one row per parked instance; deleted on wake)';
COMMENT ON COLUMN saga_correlation_index.logic_tag IS 'Stable saga type tag; scopes correlations so two saga types may reuse a key name';
COMMENT ON COLUMN saga_correlation_index.correlation_key IS 'Logical correlation dimension the awaited event carries (e.g. order_id)';
COMMENT ON COLUMN saga_correlation_index.correlation_value IS 'Instance value that routes the awaited event to THIS parked saga';
COMMENT ON COLUMN saga_correlation_index.stream_id IS 'Event stream of the parked saga instance to resume on wake';
COMMENT ON COLUMN saga_correlation_index.deadline IS 'Absolute await deadline; NULL when the await has no timeout';
COMMENT ON COLUMN saga_correlation_index.timeout_tag IS 'Saga-chosen tag distinguishing which await timed out (paired with deadline)';
