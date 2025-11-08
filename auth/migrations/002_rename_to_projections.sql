-- Migration 002: Rename tables to projections and add device_trust_level
--
-- This migration clarifies that user/device tables are projections (read models)
-- built from events, not the source of truth. The event store is the source of truth.
--
-- Architecture:
-- - Event Store (events table in PostgreSQL) = Source of Truth
-- - Projection Tables (*_projection) = Read-optimized views built from events

-- Create device_trust_level enum
CREATE TYPE device_trust_level AS ENUM (
    'unknown',        -- New device, no history
    'recognized',     -- Seen before, minimal history
    'familiar',       -- Regular usage pattern, same location
    'trusted',        -- Passkey registered or long history
    'highly_trusted'  -- Admin-approved or corporate device
);

-- Rename users table to users_projection
ALTER TABLE users RENAME TO users_projection;
ALTER INDEX idx_users_email RENAME TO idx_users_projection_email;
ALTER TRIGGER update_users_updated_at ON users_projection RENAME TO update_users_projection_updated_at;

-- Rename registered_devices to devices_projection and add trust_level
ALTER TABLE registered_devices RENAME TO devices_projection;
ALTER INDEX idx_devices_user_id RENAME TO idx_devices_projection_user_id;
ALTER INDEX idx_devices_last_seen RENAME TO idx_devices_projection_last_seen;
ALTER INDEX idx_devices_passkey_credential RENAME TO idx_devices_projection_passkey_credential;

-- Replace user_marked_trusted with trust_level
ALTER TABLE devices_projection
    DROP COLUMN user_marked_trusted,
    DROP COLUMN requires_mfa,
    ADD COLUMN trust_level device_trust_level NOT NULL DEFAULT 'unknown';

-- Rename oauth_links to oauth_links_projection
ALTER TABLE oauth_links RENAME TO oauth_links_projection;
ALTER INDEX idx_oauth_links_provider RENAME TO idx_oauth_links_projection_provider;

-- Rename passkey_credentials to passkeys_projection
ALTER TABLE passkey_credentials RENAME TO passkeys_projection;
ALTER INDEX idx_passkey_user_id RENAME TO idx_passkeys_projection_user_id;
ALTER INDEX idx_passkey_device_id RENAME TO idx_passkeys_projection_device_id;

-- Add index on trust_level for queries
CREATE INDEX idx_devices_projection_trust_level ON devices_projection(trust_level);

-- Update comments
COMMENT ON TABLE users_projection IS 'User projection - read model built from UserRegistered events';
COMMENT ON TABLE devices_projection IS 'Device projection - read model built from DeviceRegistered and DeviceAccessed events';
COMMENT ON TABLE oauth_links_projection IS 'OAuth links projection - read model built from OAuthAccountLinked events';
COMMENT ON TABLE passkeys_projection IS 'Passkey projection - read model built from PasskeyRegistered events';

COMMENT ON COLUMN devices_projection.trust_level IS 'Progressive trust level calculated from usage patterns';
COMMENT ON COLUMN devices_projection.login_count IS 'Number of successful logins from this device (incremented by DeviceAccessed events)';
COMMENT ON COLUMN passkeys_projection.counter IS 'WebAuthn signature counter for replay protection (updated by PasskeyUsed events)';

-- Add registered_at column to passkeys_projection for consistency
ALTER TABLE passkeys_projection
    DROP COLUMN created_at,
    ADD COLUMN registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
