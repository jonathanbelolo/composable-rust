-- Durable saga state for the Event-Inventory saga (authoritative resume source).
--
-- Written in the SAME transaction as the saga's event append (via
-- PgEventInventorySagaStateProjector + PostgresEventStore::append_with_projection),
-- so `version` always equals the saga stream's MAX(version) and the row can never
-- drift from the event log. The saga's QueryFetcher reads this to rehydrate state and
-- return `expected_version` for optimistic concurrency across asynchronous restarts.
--
-- A separate table (keyed by event_id) rather than sharing `saga_state` (keyed by
-- reservation_id) — the two sagas have different correlation keys and state shapes.
-- This saga has no timeout, so there is no expires_at column.

CREATE TABLE saga_state_event_inventory (
    event_id      UUID PRIMARY KEY,
    version       BIGINT NOT NULL,            -- == saga stream MAX(version)
    phase         TEXT   NOT NULL,            -- SagaPhase (serde tag)
    state         JSONB  NOT NULL,            -- full SagaState (serde_json)
    state_version SMALLINT NOT NULL DEFAULT 1, -- guards EVENT_INVENTORY_SAGA_STATE_VERSION
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Non-terminal sagas (for any future recovery sweep / operational queries).
CREATE INDEX idx_saga_state_event_inventory_active
    ON saga_state_event_inventory (phase)
    WHERE phase NOT IN ('Completed', 'CompensationCompleted', 'Failed');
