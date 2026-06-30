-- Idempotency keys for external command initiation.
--
-- Maps a client idempotency key (scope + key) to the deterministic entity id it
-- produced, so a retried request resolves to the same saga stream instead of
-- creating a duplicate. Written in the same transaction as the first
-- ReservationInitiated event for exactly-once initiation.

CREATE TABLE idempotency_keys (
    scope          TEXT NOT NULL,             -- e.g. 'reservations.create'
    idem_key       TEXT NOT NULL,             -- client-supplied Idempotency-Key
    reservation_id UUID NOT NULL,             -- deterministic id produced for this key
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (scope, idem_key)
);
