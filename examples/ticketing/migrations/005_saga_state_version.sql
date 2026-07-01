-- Schema-version guard for the persisted saga-state JSONB.
--
-- `state_version` records the shape of the `state` column so that a binary reading a
-- row written by an incompatible shape fails loudly (with a clear "run a migration"
-- error) instead of a cryptic serde decode error. Bump the matching Rust constant
-- (`RESERVATION_SAGA_STATE_VERSION`) whenever the persisted state shape changes
-- incompatibly, and add a data migration that rewrites existing rows.
ALTER TABLE saga_state
    ADD COLUMN state_version SMALLINT NOT NULL DEFAULT 1;

-- Reservation expiration now flows through `saga_state` (driven by idx_saga_state_resume),
-- so the old inventory-side expiration index on `reservations` is dead weight. Drop it.
DROP INDEX IF EXISTS idx_reservations_expiration;
