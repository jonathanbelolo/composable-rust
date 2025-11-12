-- Authentication database schema
-- Supports magic links, OAuth, and passkeys (WebAuthn)

-- Users table
CREATE TABLE users (
    id UUID PRIMARY KEY,
    email VARCHAR(255) NOT NULL UNIQUE,
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    name VARCHAR(255),
    role VARCHAR(50) NOT NULL DEFAULT 'customer',  -- customer, admin
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_users_email ON users(email);
CREATE INDEX idx_users_role ON users(role);

-- Sessions table (currently stored in Redis, but schema for reference/backup)
-- Note: In production, sessions are in Redis for speed
-- This table can be used for session persistence backup or analytics
CREATE TABLE sessions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_accessed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ip_address INET,
    user_agent TEXT
);

CREATE INDEX idx_sessions_user ON sessions(user_id);
CREATE INDEX idx_sessions_expires ON sessions(expires_at);

-- Magic link tokens (for passwordless authentication)
-- Tokens are also stored in Redis with TTL, this is for audit trail
CREATE TABLE magic_link_tokens (
    token UUID PRIMARY KEY,
    email VARCHAR(255) NOT NULL,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,  -- NULL if user doesn't exist yet
    expires_at TIMESTAMPTZ NOT NULL,
    used BOOLEAN NOT NULL DEFAULT FALSE,
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ip_address INET
);

CREATE INDEX idx_magic_tokens_email ON magic_link_tokens(email);
CREATE INDEX idx_magic_tokens_user ON magic_link_tokens(user_id);
CREATE INDEX idx_magic_tokens_expires ON magic_link_tokens(expires_at);

-- OAuth accounts (for GitHub, Google, etc.)
CREATE TABLE oauth_accounts (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider VARCHAR(50) NOT NULL,  -- github, google
    provider_user_id VARCHAR(255) NOT NULL,  -- External user ID from OAuth provider
    provider_email VARCHAR(255),
    access_token_encrypted BYTEA,  -- Encrypted OAuth access token
    refresh_token_encrypted BYTEA,  -- Encrypted OAuth refresh token
    token_expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(provider, provider_user_id)
);

CREATE INDEX idx_oauth_user ON oauth_accounts(user_id);
CREATE INDEX idx_oauth_provider ON oauth_accounts(provider, provider_user_id);

-- WebAuthn credentials (for passkeys)
CREATE TABLE webauthn_credentials (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential_id BYTEA NOT NULL UNIQUE,  -- WebAuthn credential ID
    public_key BYTEA NOT NULL,             -- Public key for verification
    sign_count BIGINT NOT NULL DEFAULT 0,  -- Counter for replay protection
    device_name VARCHAR(255),              -- User-friendly device name
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_webauthn_user ON webauthn_credentials(user_id);
CREATE INDEX idx_webauthn_credential ON webauthn_credentials(credential_id);

-- WebAuthn challenges (for registration/authentication flow)
-- Stored in Redis with 5-minute TTL, this is for audit trail
CREATE TABLE webauthn_challenges (
    challenge BYTEA PRIMARY KEY,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    email VARCHAR(255),  -- For registration flow (user doesn't exist yet)
    challenge_type VARCHAR(50) NOT NULL,  -- registration, authentication
    expires_at TIMESTAMPTZ NOT NULL,
    used BOOLEAN NOT NULL DEFAULT FALSE,
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_webauthn_challenges_expires ON webauthn_challenges(expires_at);
CREATE INDEX idx_webauthn_challenges_user ON webauthn_challenges(user_id);

-- Devices table (for tracking authenticated devices)
CREATE TABLE devices (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_name VARCHAR(255),
    device_type VARCHAR(50),  -- browser, mobile, desktop
    browser VARCHAR(100),
    os VARCHAR(100),
    ip_address INET,
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_devices_user ON devices(user_id);
CREATE INDEX idx_devices_last_seen ON devices(last_seen_at);

-- Rate limiting tracking (for anti-abuse)
-- Also stored in Redis, this is for historical analysis
CREATE TABLE rate_limit_violations (
    id BIGSERIAL PRIMARY KEY,
    ip_address INET NOT NULL,
    endpoint VARCHAR(255) NOT NULL,
    request_count INT NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    window_end TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_rate_limit_ip ON rate_limit_violations(ip_address);
CREATE INDEX idx_rate_limit_window ON rate_limit_violations(window_start, window_end);

-- Audit log (for security events)
CREATE TABLE audit_log (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    event_type VARCHAR(100) NOT NULL,  -- login, logout, password_change, etc.
    ip_address INET,
    user_agent TEXT,
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_user ON audit_log(user_id);
CREATE INDEX idx_audit_event_type ON audit_log(event_type);
CREATE INDEX idx_audit_created_at ON audit_log(created_at);
