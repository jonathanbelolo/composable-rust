-- Create audit_logs table for security and compliance tracking
CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY,
    event_type VARCHAR(50) NOT NULL,
    severity VARCHAR(20) NOT NULL,
    actor VARCHAR(255) NOT NULL,
    action VARCHAR(100) NOT NULL,
    resource VARCHAR(255),
    success BOOLEAN NOT NULL,
    error_message TEXT,
    source_ip VARCHAR(45),
    user_agent TEXT,
    session_id VARCHAR(255),
    request_id VARCHAR(255),
    metadata JSONB DEFAULT '{}'::jsonb,
    timestamp TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_audit_actor ON audit_logs(actor);
CREATE INDEX IF NOT EXISTS idx_audit_event_type ON audit_logs(event_type);
CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_logs(timestamp);
CREATE INDEX IF NOT EXISTS idx_audit_session_id ON audit_logs(session_id) WHERE session_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_audit_success ON audit_logs(success);
CREATE INDEX IF NOT EXISTS idx_audit_severity ON audit_logs(severity);

-- Index for recent failures (common security query pattern)
CREATE INDEX IF NOT EXISTS idx_audit_recent_failures
    ON audit_logs(timestamp DESC)
    WHERE success = false;

-- GIN index for metadata JSON queries
CREATE INDEX IF NOT EXISTS idx_audit_metadata ON audit_logs USING GIN (metadata);
