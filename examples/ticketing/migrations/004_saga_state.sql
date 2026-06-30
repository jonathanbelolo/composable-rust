-- Durable saga state (authoritative resume source).
--
-- Written in the SAME transaction as the saga's event append (via
-- PgReservationSagaStateProjector + PostgresEventStore::append_with_projection),
-- so `version` always equals the saga stream's MAX(version) and the row can never
-- drift from the event log. The saga's QueryFetcher reads this to rehydrate state
-- and return `expected_version` for optimistic concurrency.

CREATE TABLE saga_state (
    reservation_id UUID PRIMARY KEY,
    version        BIGINT NOT NULL,            -- == saga stream MAX(version)
    phase          TEXT   NOT NULL,            -- ReservationSagaPhase (serde tag)
    expires_at     TIMESTAMPTZ,                -- drives expire-only recovery / timers
    state          JSONB  NOT NULL,            -- full ReservationSagaState (serde_json)
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Index for the expire-only recovery sweep and the expiration worker.
CREATE INDEX idx_saga_state_resume ON saga_state (expires_at)
    WHERE phase NOT IN ('Completed', 'Failed');
