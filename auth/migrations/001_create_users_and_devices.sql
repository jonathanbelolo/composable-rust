-- Migration 001: Create users and devices tables
-- This migration establishes the core persistent storage for user accounts and device registry.
-- Devices outlive sessions and are stored in PostgreSQL.

-- Create users table
CREATE TABLE users (
    user_id UUID PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    name TEXT,
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for email lookups (login)
CREATE INDEX idx_users_email ON users(email);

-- Create device type enum
CREATE TYPE device_type AS ENUM ('mobile', 'desktop', 'tablet', 'unknown');

-- Create registered_devices table
CREATE TABLE registered_devices (
    device_id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    device_type device_type NOT NULL DEFAULT 'unknown',
    platform TEXT NOT NULL,
    first_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    user_marked_trusted BOOLEAN NOT NULL DEFAULT FALSE,
    requires_mfa BOOLEAN NOT NULL DEFAULT FALSE,

    -- Passkey/WebAuthn fields
    passkey_credential_id TEXT,
    public_key BYTEA,
    counter BIGINT DEFAULT 0,

    -- Device trust metrics (Phase 6B advanced)
    login_count INTEGER NOT NULL DEFAULT 0,

    CONSTRAINT unique_passkey_credential UNIQUE (passkey_credential_id)
);

-- Indexes for device lookups
CREATE INDEX idx_devices_user_id ON registered_devices(user_id);
CREATE INDEX idx_devices_last_seen ON registered_devices(last_seen DESC);
CREATE INDEX idx_devices_passkey_credential ON registered_devices(passkey_credential_id) WHERE passkey_credential_id IS NOT NULL;

-- Create oauth_links table for account linking
CREATE TABLE oauth_links (
    user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    provider_user_id TEXT NOT NULL,
    provider_email TEXT NOT NULL,
    linked_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (user_id, provider),
    CONSTRAINT unique_provider_user UNIQUE (provider, provider_user_id)
);

-- Index for OAuth provider lookups
CREATE INDEX idx_oauth_links_provider ON oauth_links(provider, provider_user_id);

-- Create passkey_credentials table (separate from devices for flexibility)
CREATE TABLE passkey_credentials (
    credential_id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    device_id UUID NOT NULL REFERENCES registered_devices(device_id) ON DELETE CASCADE,
    public_key BYTEA NOT NULL,
    counter BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used TIMESTAMPTZ,

    CONSTRAINT unique_device_credential UNIQUE (device_id, credential_id)
);

-- Indexes for passkey lookups
CREATE INDEX idx_passkey_user_id ON passkey_credentials(user_id);
CREATE INDEX idx_passkey_device_id ON passkey_credentials(device_id);

-- Add updated_at trigger for users table
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

CREATE TRIGGER update_users_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Comments for documentation
COMMENT ON TABLE users IS 'User accounts - persistent identity across all auth methods';
COMMENT ON TABLE registered_devices IS 'Persistent device registry - outlives sessions, tracks trust levels';
COMMENT ON TABLE oauth_links IS 'OAuth provider account links - maps external identities to users';
COMMENT ON TABLE passkey_credentials IS 'WebAuthn/FIDO2 passkey credentials - hardware-backed authentication';

COMMENT ON COLUMN registered_devices.user_marked_trusted IS 'User explicitly marked this device as trusted';
COMMENT ON COLUMN registered_devices.login_count IS 'Number of successful logins from this device (for trust level calculation)';
COMMENT ON COLUMN passkey_credentials.counter IS 'WebAuthn signature counter for replay protection';
