# Audit Logging Guide

Comprehensive guide to audit logging for security, compliance, and operational visibility.

## Table of Contents

- [Overview](#overview)
- [Quick Start](#quick-start)
- [Event Types](#event-types)
- [Severity Levels](#severity-levels)
- [Logging Events](#logging-events)
- [Querying Events](#querying-events)
- [Backends](#backends)
- [Compliance](#compliance)
- [Best Practices](#best-practices)
- [Examples](#examples)

## Overview

The audit framework provides:
- **Structured logging** for security-critical operations
- **Rich metadata** (actor, resource, IP, user agent, etc.)
- **Query capabilities** for investigation and reporting
- **Pluggable backends** (in-memory, database, syslog, cloud)
- **Compliance support** (GDPR, SOC2, HIPAA)

### When to Audit Log

✅ **Always audit**:
- Authentication (login, logout, MFA)
- Authorization (permission checks, access denied)
- Data access (read, write, delete sensitive data)
- Configuration changes
- Security events (rate limiting, suspicious activity)
- LLM interactions (prompt injection, policy violations)

❌ **Don't audit**:
- Normal business logic
- Non-sensitive queries
- Health checks
- Metrics collection

## Quick Start

### Basic Usage

```rust
use composable_rust_agent_patterns::audit::{
    AuditEvent, AuditLogger, InMemoryAuditLogger,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let logger = InMemoryAuditLogger::new();

    // Log authentication event
    let event = AuditEvent::authentication(
        "user@example.com",
        "login",
        true,
    )
    .with_source_ip("192.168.1.100")
    .with_user_agent("Mozilla/5.0...");

    logger.log(event).await?;

    Ok(())
}
```

## Event Types

### AuditEventType

Six event type categories:

| Type | Description | Examples |
|------|-------------|----------|
| `Authentication` | User authentication | login, logout, MFA verification |
| `Authorization` | Permission checks | access granted/denied, role check |
| `DataAccess` | Data operations | read document, write record, delete file |
| `Configuration` | Settings changes | update config, change permissions |
| `Security` | Security incidents | rate limit, brute force, anomaly |
| `LlmInteraction` | LLM-specific events | prompt injection, content policy |

### Creating Events

```rust
use composable_rust_agent_patterns::audit::AuditEvent;

// Authentication
let login = AuditEvent::authentication("user@example.com", "login", true);

// Authorization
let access = AuditEvent::authorization("user@example.com", "read_document", false)
    .with_error("Permission denied");

// Data access
let read = AuditEvent::data_access(
    "user@example.com",
    "read",
    "document:123",
    true,
);

// Configuration
let config = AuditEvent::configuration(
    "admin@example.com",
    "update_rate_limit",
    "config:rate_limit",
);

// Security
let security = AuditEvent::security(
    "192.168.1.100",
    "brute_force_detected",
    Severity::Critical,
);

// LLM interaction
let llm = AuditEvent::llm_interaction(
    "user@example.com",
    "prompt_injection_detected",
    false,
);
```

## Severity Levels

Four severity levels (ordered):

| Severity | Use Case | Examples |
|----------|----------|----------|
| `Info` | Routine operations | Successful login, normal data access |
| `Warning` | Unusual but not critical | Config change, multiple failed attempts |
| `Error` | Failed operations | Authentication failed, access denied |
| `Critical` | Security incidents | Data breach, privilege escalation, DDoS |

### Setting Severity

```rust
// Automatic severity
let event = AuditEvent::authentication("user", "login", true);
// severity: Info (success)

let event = AuditEvent::authentication("user", "login", false);
// severity: Error (failure)

// Manual severity
let event = AuditEvent::security("ip", "data_breach", Severity::Critical);

// Override severity
let event = AuditEvent::authentication("user", "login", true)
    .with_severity(Severity::Warning);
```

## Logging Events

### Builder Pattern

Rich metadata via builder methods:

```rust
let event = AuditEvent::data_access(
    "user@example.com",
    "read",
    "document:123",
    true,
)
.with_source_ip("192.168.1.100")
.with_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
.with_session_id("sess_abc123")
.with_request_id("req_xyz789")
.with_metadata("document_size", "1024")
.with_metadata("document_type", "pdf");

logger.log(event).await?;
```

### Error Handling

```rust
let event = AuditEvent::authorization("user@example.com", "delete_document", false)
    .with_resource("document:456")
    .with_error("User lacks delete permission")
    .with_severity(Severity::Warning);

logger.log(event).await?;
```

### Contextual Information

Always include:
- **Actor**: Who performed the action (user ID, service account, API key)
- **Action**: What was done (login, read, write, delete)
- **Resource**: What was affected (document:123, user:456)
- **Outcome**: Success or failure
- **Source IP**: Where the request came from
- **Timestamp**: When it happened (automatic)

## Querying Events

### AuditEventFilter

Build complex queries:

```rust
use composable_rust_agent_patterns::audit::AuditEventFilter;

// Query by event type
let filter = AuditEventFilter::new()
    .event_type(AuditEventType::Authentication);

let events = logger.query(filter).await?;

// Query by actor
let filter = AuditEventFilter::new()
    .actor("user@example.com".to_string());

// Query failed operations
let filter = AuditEventFilter::new()
    .success(false);

// Query by minimum severity
let filter = AuditEventFilter::new()
    .min_severity(Severity::Warning);

// Complex query
let filter = AuditEventFilter::new()
    .event_type(AuditEventType::Authentication)
    .actor("user@example.com".to_string())
    .success(false)
    .limit(100);

let events = logger.query(filter).await?;
```

### Get by ID

```rust
let event = logger.get_by_id("event_id_123").await?;

if let Some(event) = event {
    println!("Found event: {}", event.action);
}
```

## Backends

### Phase 8.4 Implementation Status

**Currently Available**: `InMemoryAuditLogger` only
**Planned for Future Phases**: PostgreSQL, File, Syslog, Cloud providers

The Phase 8.4 implementation focuses on the core audit framework with an in-memory
backend suitable for development and testing. Production backends will be added in
future phases based on operational requirements.

### InMemoryAuditLogger

For development and testing:

```rust
use composable_rust_agent_patterns::audit::InMemoryAuditLogger;

let logger = InMemoryAuditLogger::new();

// Log events
logger.log(event).await?;

// Testing helpers
let count = logger.count().await;
let all = logger.all_events().await;
logger.clear().await;
```

**Limitations**:
- Events lost on restart
- No persistence
- Limited to memory size
- Not suitable for production use

**Use for**:
- Development
- Unit tests
- Integration tests
- Proof-of-concept deployments

### Database Backend (Planned)

Production-ready PostgreSQL backend (future work):

```rust
use composable_rust_agent_patterns::audit::PostgresAuditLogger;

let logger = PostgresAuditLogger::new(pool).await?;
logger.log(event).await?;
```

**Features**:
- Persistent storage
- Efficient queries with indexes
- Tamper-evident (append-only)
- Partitioned by date

### File Backend (Planned)

JSON Lines format (future work):

```rust
use composable_rust_agent_patterns::audit::FileAuditLogger;

let logger = FileAuditLogger::new("/var/log/audit.jsonl").await?;
logger.log(event).await?;
```

**Features**:
- Simple text files
- Easy to process with standard tools
- Rotation support
- Compression

## Compliance

### GDPR (General Data Protection Regulation)

Audit logging helps with:
- **Article 30**: Records of processing activities
- **Article 32**: Security measures
- **Article 33**: Breach notification

```rust
// Log data access for GDPR compliance
let event = AuditEvent::data_access(
    user_id,
    "read",
    format!("user_data:{}", subject_id),
    true,
)
.with_metadata("purpose", "customer_support")
.with_metadata("legal_basis", "legitimate_interest");

logger.log(event).await?;
```

### SOC 2 (Service Organization Control 2)

Required audit logs:
- User authentication and authorization
- Data access and modifications
- System configuration changes
- Security incidents

```rust
// CC6.3: Logical and physical access controls
let event = AuditEvent::authorization(
    user_id,
    "access_denied",
    false,
)
.with_resource(resource_id)
.with_error("Insufficient privileges");

logger.log(event).await?;
```

### HIPAA (Health Insurance Portability and Accountability Act)

For healthcare data:
- Log all access to PHI (Protected Health Information)
- Minimum 6-year retention
- Tamper-evident logging

```rust
// Log PHI access
let event = AuditEvent::data_access(
    user_id,
    "read",
    format!("patient_record:{}", patient_id),
    true,
)
.with_metadata("data_type", "phi")
.with_metadata("reason", "treatment");

logger.log(event).await?;
```

## Best Practices

### 1. Log Early, Log Often

```rust
// ✅ Good: Log before performing action
logger.log(AuditEvent::authentication("user", "login_attempt", true)).await?;

match authenticate(credentials).await {
    Ok(session) => {
        logger.log(AuditEvent::authentication("user", "login_success", true)
            .with_session_id(&session.id)).await?;
    }
    Err(e) => {
        logger.log(AuditEvent::authentication("user", "login_failed", false)
            .with_error(&e.to_string())).await?;
    }
}
```

### 2. Include Context

```rust
// ❌ Bad: Minimal context
let event = AuditEvent::authentication("user", "login", true);

// ✅ Good: Rich context
let event = AuditEvent::authentication("user", "login", true)
    .with_source_ip(&request.ip)
    .with_user_agent(&request.user_agent)
    .with_session_id(&session.id)
    .with_request_id(&request.id)
    .with_metadata("mfa_used", "true");
```

### 3. Sanitize Sensitive Data

```rust
// ❌ Bad: Logging passwords
let event = AuditEvent::authentication(email, "login", false)
    .with_metadata("password", password);  // DON'T DO THIS!

// ✅ Good: Log metadata, not secrets
let event = AuditEvent::authentication(email, "login", false)
    .with_metadata("password_length", password.len().to_string());
```

### 4. Use Appropriate Severity

```rust
// Routine operations
AuditEvent::authentication(user, "login", true)  // Info

// Configuration changes
AuditEvent::configuration(admin, "update_settings", resource)  // Warning

// Failed operations
AuditEvent::authentication(user, "login", false)  // Error

// Security incidents
AuditEvent::security(ip, "data_breach", Severity::Critical)  // Critical
```

### 5. Query Performance

```rust
// ✅ Good: Use specific filters
let filter = AuditEventFilter::new()
    .event_type(AuditEventType::Security)
    .min_severity(Severity::Warning)
    .limit(1000);

// ❌ Bad: Loading all events
let all_events = logger.query(AuditEventFilter::new()).await?;  // Slow!
```

### 6. Retention Policy

Implement log retention based on compliance requirements:

```rust
// Example: 90-day retention for general logs
// 7-year retention for financial records
// Lifetime retention for security incidents

// Configure in your backend
let logger = DatabaseAuditLogger::new(pool)
    .with_retention_days(90)
    .with_retention_override(
        AuditEventType::Security,
        None,  // Keep forever
    );
```

### 7. Async Logging

Never block on audit logging:

```rust
// ✅ Good: Fire and forget (in background)
tokio::spawn(async move {
    if let Err(e) = logger.log(event).await {
        eprintln!("Audit log failed: {}", e);
    }
});

// Continue with request handling
```

### 8. Monitoring

Set up alerts for critical events:

```rust
let event = AuditEvent::security(ip, "brute_force", Severity::Critical);

// Log to audit trail
logger.log(event.clone()).await?;

// Also send alert
if event.severity == Severity::Critical {
    alert_on_call_team(&event).await;
}
```

## Examples

### Example 1: Authentication Flow

```rust
async fn handle_login(
    credentials: Credentials,
    request: &HttpRequest,
    logger: &dyn AuditLogger,
) -> Result<Session, AuthError> {
    // Log login attempt
    let mut event = AuditEvent::authentication(
        &credentials.email,
        "login_attempt",
        true,
    )
    .with_source_ip(&request.ip)
    .with_user_agent(&request.user_agent);

    logger.log(event.clone()).await?;

    // Attempt authentication
    match authenticate(&credentials).await {
        Ok(session) => {
            // Log success
            event.action = "login_success".to_string();
            event.session_id = Some(session.id.clone());
            logger.log(event).await?;
            Ok(session)
        }
        Err(e) => {
            // Log failure
            event.action = "login_failed".to_string();
            event.success = false;
            event.error_message = Some(e.to_string());
            event.severity = Severity::Error;
            logger.log(event).await?;
            Err(e)
        }
    }
}
```

### Example 2: Data Access

```rust
async fn read_document(
    user_id: &str,
    document_id: &str,
    logger: &dyn AuditLogger,
) -> Result<Document, Error> {
    // Check permission
    if !has_permission(user_id, document_id, "read").await? {
        let event = AuditEvent::authorization(
            user_id,
            "read_document",
            false,
        )
        .with_resource(document_id)
        .with_error("Permission denied")
        .with_severity(Severity::Warning);

        logger.log(event).await?;
        return Err(Error::PermissionDenied);
    }

    // Read document
    let document = fetch_document(document_id).await?;

    // Log successful access
    let event = AuditEvent::data_access(
        user_id,
        "read",
        document_id,
        true,
    )
    .with_metadata("document_size", document.size.to_string())
    .with_metadata("document_type", &document.mime_type);

    logger.log(event).await?;

    Ok(document)
}
```

### Example 3: Configuration Change

```rust
async fn update_rate_limit(
    admin_id: &str,
    new_limit: u32,
    logger: &dyn AuditLogger,
) -> Result<(), Error> {
    let old_limit = get_current_rate_limit().await?;

    // Update configuration
    set_rate_limit(new_limit).await?;

    // Log configuration change
    let event = AuditEvent::configuration(
        admin_id,
        "update_rate_limit",
        "config:rate_limit",
    )
    .with_metadata("old_value", old_limit.to_string())
    .with_metadata("new_value", new_limit.to_string());

    logger.log(event).await?;

    Ok(())
}
```

### Example 4: Security Event

```rust
async fn check_rate_limit(
    ip: &str,
    logger: &dyn AuditLogger,
) -> Result<(), Error> {
    let requests = get_request_count(ip).await?;

    if requests > RATE_LIMIT {
        // Log security event
        let event = AuditEvent::security(
            ip,
            "rate_limit_exceeded",
            Severity::Warning,
        )
        .with_metadata("request_count", requests.to_string())
        .with_metadata("limit", RATE_LIMIT.to_string());

        logger.log(event).await?;

        return Err(Error::RateLimitExceeded);
    }

    Ok(())
}
```

### Example 5: LLM Interaction

```rust
async fn detect_prompt_injection(
    user_id: &str,
    prompt: &str,
    logger: &dyn AuditLogger,
) -> Result<(), Error> {
    if is_prompt_injection(prompt) {
        let event = AuditEvent::llm_interaction(
            user_id,
            "prompt_injection_detected",
            false,
        )
        .with_severity(Severity::Critical)
        .with_metadata("prompt_length", prompt.len().to_string())
        .with_metadata("detection_method", "regex");

        logger.log(event).await?;

        return Err(Error::PromptInjection);
    }

    Ok(())
}
```

### Example 6: Querying for Investigation

```rust
async fn investigate_failed_logins(
    user_email: &str,
    logger: &dyn AuditLogger,
) -> Result<Vec<AuditEvent>, Error> {
    // Find all failed login attempts in last 24 hours
    let filter = AuditEventFilter::new()
        .event_type(AuditEventType::Authentication)
        .actor(user_email.to_string())
        .success(false);

    let events = logger.query(filter).await?;

    println!("Found {} failed login attempts:", events.len());
    for event in &events {
        println!(
            "  - {} from {} at {}",
            event.action,
            event.source_ip.as_deref().unwrap_or("unknown"),
            event.timestamp
        );
    }

    Ok(events)
}
```

### Example 7: Security Dashboard

```rust
async fn security_dashboard(logger: &dyn AuditLogger) -> Result<(), Error> {
    // Critical security events
    let critical = logger.query(
        AuditEventFilter::new()
            .min_severity(Severity::Critical)
            .limit(10)
    ).await?;

    println!("🚨 Critical Events: {}", critical.len());

    // Failed authentication attempts
    let failed_auth = logger.query(
        AuditEventFilter::new()
            .event_type(AuditEventType::Authentication)
            .success(false)
            .limit(100)
    ).await?;

    println!("🔐 Failed Logins: {}", failed_auth.len());

    // Configuration changes
    let config_changes = logger.query(
        AuditEventFilter::new()
            .event_type(AuditEventType::Configuration)
            .limit(50)
    ).await?;

    println!("⚙️  Config Changes: {}", config_changes.len());

    Ok(())
}
```

## Troubleshooting

### High Volume Issues

If audit logging impacts performance:

1. **Use async logging** (fire and forget)
2. **Batch writes** to database
3. **Use separate database** for audit logs
4. **Implement sampling** for high-frequency events
5. **Archive old logs** to cold storage

### Missing Events

If events aren't being logged:

1. Check logger is initialized
2. Verify `await` on log calls
3. Check for panic/errors in logging code
4. Verify database connectivity
5. Check file permissions (for file backend)

### Query Performance

If queries are slow:

1. Add database indexes on common filters
2. Use specific filters (don't query all events)
3. Add time range filters
4. Use `limit` parameter
5. Consider read replicas

## Additional Resources

- [NIST Audit and Accountability](https://csrc.nist.gov/Projects/audit-and-accountability)
- [OWASP Logging Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html)
- [GDPR Article 30](https://gdpr-info.eu/art-30-gdpr/)
- [SOC 2 Audit Logging Requirements](https://www.aicpa.org/interestareas/frc/assuranceadvisoryservices/sorhome)

## Summary

- ✅ Log all security-critical operations
- ✅ Include rich context (IP, user agent, session ID)
- ✅ Use appropriate severity levels
- ✅ Sanitize sensitive data
- ✅ Query with specific filters
- ✅ Implement retention policies
- ✅ Monitor critical events
- ✅ Test audit logging in CI/CD
