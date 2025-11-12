//! Audit Event Framework
//!
//! Provides structured audit logging for security-critical operations.
//!
//! # Features
//!
//! - Structured audit events with rich metadata
//! - Pluggable backends (in-memory, database, file, syslog, cloud)
//! - Query and search capabilities
//! - Compliance support (GDPR, SOC2, HIPAA)
//!
//! # Current Implementation (Phase 8.4)
//!
//! This phase provides the core audit framework with `InMemoryAuditLogger` for
//! development and testing. Production backends (PostgreSQL, file-based) are planned
//! for future phases.
//!
//! # Future Work
//!
//! - Cryptographic integrity (event signatures, Merkle trees)
//! - PostgreSQL backend for persistent storage
//! - File-based backend with log rotation
//! - Syslog and cloud provider integrations
//!
//! # Example
//!
//! ```
//! use composable_rust_agent_patterns::audit::{
//!     AuditEvent, AuditEventType, AuditLogger, InMemoryAuditLogger,
//! };
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let logger = InMemoryAuditLogger::new();
//!
//! // Log authentication event
//! let event = AuditEvent::authentication(
//!     "user@example.com",
//!     "Login",
//!     true,
//! )
//! .with_source_ip("192.168.1.100")
//! .with_user_agent("Mozilla/5.0...");
//!
//! logger.log(event).await?;
//! # Ok(())
//! # }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Audit event type categorization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    /// Authentication events (login, logout, MFA)
    Authentication,
    /// Authorization events (permission checks, access denied)
    Authorization,
    /// Data access events (read, write, delete)
    DataAccess,
    /// Configuration changes
    Configuration,
    /// Security events (suspicious activity, rate limiting)
    Security,
    /// LLM interactions (prompt injection attempts, content policy violations)
    LlmInteraction,
}

impl fmt::Display for AuditEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authentication => write!(f, "authentication"),
            Self::Authorization => write!(f, "authorization"),
            Self::DataAccess => write!(f, "data_access"),
            Self::Configuration => write!(f, "configuration"),
            Self::Security => write!(f, "security"),
            Self::LlmInteraction => write!(f, "llm_interaction"),
        }
    }
}

/// Audit event severity
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Informational (routine operations)
    Info,
    /// Warning (unusual but not critical)
    Warning,
    /// Error (failed operations)
    Error,
    /// Critical (security incidents, data breaches)
    Critical,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => write!(f, "info"),
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

/// Structured audit event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Unique event ID
    pub id: String,
    /// Event timestamp (RFC3339)
    pub timestamp: String,
    /// Event type
    pub event_type: AuditEventType,
    /// Severity level
    pub severity: Severity,
    /// Actor (user ID, service account, API key ID)
    pub actor: String,
    /// Action performed (e.g., "login", "read_document", "update_config")
    pub action: String,
    /// Resource affected (e.g., "user:123", "document:456")
    pub resource: Option<String>,
    /// Outcome (success/failure)
    pub success: bool,
    /// Error message (if failed)
    pub error_message: Option<String>,
    /// Source IP address
    pub source_ip: Option<String>,
    /// User agent
    pub user_agent: Option<String>,
    /// Session ID
    pub session_id: Option<String>,
    /// Request ID (for correlation)
    pub request_id: Option<String>,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

impl AuditEvent {
    /// Create a new audit event
    #[must_use]
    pub fn new(event_type: AuditEventType, actor: impl Into<String>, action: impl Into<String>, success: bool) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            event_type,
            severity: if success { Severity::Info } else { Severity::Error },
            actor: actor.into(),
            action: action.into(),
            resource: None,
            success,
            error_message: None,
            source_ip: None,
            user_agent: None,
            session_id: None,
            request_id: None,
            metadata: HashMap::new(),
        }
    }

    /// Create an authentication event
    #[must_use]
    pub fn authentication(actor: impl Into<String>, action: impl Into<String>, success: bool) -> Self {
        Self::new(AuditEventType::Authentication, actor, action, success)
    }

    /// Create an authorization event
    #[must_use]
    pub fn authorization(actor: impl Into<String>, action: impl Into<String>, success: bool) -> Self {
        Self::new(AuditEventType::Authorization, actor, action, success)
    }

    /// Create a data access event
    #[must_use]
    pub fn data_access(actor: impl Into<String>, action: impl Into<String>, resource: impl Into<String>, success: bool) -> Self {
        let mut event = Self::new(AuditEventType::DataAccess, actor, action, success);
        event.resource = Some(resource.into());
        event
    }

    /// Create a configuration change event
    #[must_use]
    pub fn configuration(actor: impl Into<String>, action: impl Into<String>, resource: impl Into<String>) -> Self {
        let mut event = Self::new(AuditEventType::Configuration, actor, action, true);
        event.resource = Some(resource.into());
        event.severity = Severity::Warning; // Config changes are notable
        event
    }

    /// Create a security event
    #[must_use]
    pub fn security(actor: impl Into<String>, action: impl Into<String>, severity: Severity) -> Self {
        let mut event = Self::new(AuditEventType::Security, actor, action, false);
        event.severity = severity;
        event
    }

    /// Create an LLM interaction event
    #[must_use]
    pub fn llm_interaction(actor: impl Into<String>, action: impl Into<String>, success: bool) -> Self {
        Self::new(AuditEventType::LlmInteraction, actor, action, success)
    }

    /// Set resource
    #[must_use]
    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        self.resource = Some(resource.into());
        self
    }

    /// Set error message
    #[must_use]
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error_message = Some(error.into());
        self.success = false;
        if self.severity == Severity::Info {
            self.severity = Severity::Error;
        }
        self
    }

    /// Set severity
    #[must_use]
    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    /// Set source IP
    #[must_use]
    pub fn with_source_ip(mut self, ip: impl Into<String>) -> Self {
        self.source_ip = Some(ip.into());
        self
    }

    /// Set user agent
    #[must_use]
    pub fn with_user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = Some(ua.into());
        self
    }

    /// Set session ID
    #[must_use]
    pub fn with_session_id(mut self, session: impl Into<String>) -> Self {
        self.session_id = Some(session.into());
        self
    }

    /// Set request ID
    #[must_use]
    pub fn with_request_id(mut self, request: impl Into<String>) -> Self {
        self.request_id = Some(request.into());
        self
    }

    /// Add metadata
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Convert to JSON string
    ///
    /// # Errors
    ///
    /// Returns error if serialization fails
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Convert to pretty JSON string
    ///
    /// # Errors
    ///
    /// Returns error if serialization fails
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Audit event filter for queries
#[derive(Debug, Clone, Default)]
pub struct AuditEventFilter {
    /// Filter by event type
    pub event_type: Option<AuditEventType>,
    /// Filter by actor
    pub actor: Option<String>,
    /// Filter by action
    pub action: Option<String>,
    /// Filter by resource
    pub resource: Option<String>,
    /// Filter by success/failure
    pub success: Option<bool>,
    /// Filter by minimum severity
    pub min_severity: Option<Severity>,
    /// Filter by time range (start)
    pub start_time: Option<chrono::DateTime<chrono::Utc>>,
    /// Filter by time range (end)
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
    /// Maximum results
    pub limit: Option<usize>,
}

impl AuditEventFilter {
    /// Create a new empty filter
    #[must_use]
    pub const fn new() -> Self {
        Self {
            event_type: None,
            actor: None,
            action: None,
            resource: None,
            success: None,
            min_severity: None,
            start_time: None,
            end_time: None,
            limit: None,
        }
    }

    /// Filter by event type
    #[must_use]
    pub const fn event_type(mut self, event_type: AuditEventType) -> Self {
        self.event_type = Some(event_type);
        self
    }

    /// Filter by actor
    #[must_use]
    pub fn actor(mut self, actor: String) -> Self {
        self.actor = Some(actor);
        self
    }

    /// Filter by action
    #[must_use]
    pub fn action(mut self, action: String) -> Self {
        self.action = Some(action);
        self
    }

    /// Filter by success
    #[must_use]
    pub const fn success(mut self, success: bool) -> Self {
        self.success = Some(success);
        self
    }

    /// Filter by minimum severity
    #[must_use]
    pub const fn min_severity(mut self, severity: Severity) -> Self {
        self.min_severity = Some(severity);
        self
    }

    /// Set result limit
    #[must_use]
    pub const fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Check if event matches filter
    #[must_use]
    pub fn matches(&self, event: &AuditEvent) -> bool {
        if let Some(event_type) = self.event_type {
            if event.event_type != event_type {
                return false;
            }
        }

        if let Some(ref actor) = self.actor {
            if &event.actor != actor {
                return false;
            }
        }

        if let Some(ref action) = self.action {
            if &event.action != action {
                return false;
            }
        }

        if let Some(ref resource) = self.resource {
            if event.resource.as_ref() != Some(resource) {
                return false;
            }
        }

        if let Some(success) = self.success {
            if event.success != success {
                return false;
            }
        }

        if let Some(min_severity) = self.min_severity {
            if event.severity < min_severity {
                return false;
            }
        }

        true
    }
}

/// Audit logger trait
pub trait AuditLogger: Send + Sync {
    /// Log an audit event
    ///
    /// # Errors
    ///
    /// Returns error if logging fails
    fn log(&self, event: AuditEvent) -> impl std::future::Future<Output = Result<(), AuditError>> + Send;

    /// Query audit events
    ///
    /// # Errors
    ///
    /// Returns error if query fails
    fn query(&self, filter: AuditEventFilter) -> impl std::future::Future<Output = Result<Vec<AuditEvent>, AuditError>> + Send;

    /// Get event by ID
    ///
    /// # Errors
    ///
    /// Returns error if query fails
    fn get_by_id(&self, id: &str) -> impl std::future::Future<Output = Result<Option<AuditEvent>, AuditError>> + Send;
}

/// Audit error
#[derive(Debug)]
pub enum AuditError {
    /// Failed to serialize event
    SerializationError(String),
    /// Failed to store event
    StorageError(String),
    /// Failed to query events
    QueryError(String),
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SerializationError(msg) => write!(f, "Serialization error: {msg}"),
            Self::StorageError(msg) => write!(f, "Storage error: {msg}"),
            Self::QueryError(msg) => write!(f, "Query error: {msg}"),
        }
    }
}

impl std::error::Error for AuditError {}

/// In-memory audit logger (for testing/development)
#[derive(Clone)]
pub struct InMemoryAuditLogger {
    events: Arc<RwLock<Vec<AuditEvent>>>,
}

impl InMemoryAuditLogger {
    /// Create a new in-memory audit logger
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Get all events (for testing)
    pub async fn all_events(&self) -> Vec<AuditEvent> {
        self.events.read().await.clone()
    }

    /// Clear all events (for testing)
    pub async fn clear(&self) {
        self.events.write().await.clear();
    }

    /// Get event count
    pub async fn count(&self) -> usize {
        self.events.read().await.len()
    }
}

impl Default for InMemoryAuditLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditLogger for InMemoryAuditLogger {
    async fn log(&self, event: AuditEvent) -> Result<(), AuditError> {
        self.events.write().await.push(event);
        Ok(())
    }

    async fn query(&self, filter: AuditEventFilter) -> Result<Vec<AuditEvent>, AuditError> {
        let events = self.events.read().await;
        let mut results: Vec<AuditEvent> = events
            .iter()
            .filter(|e| filter.matches(e))
            .cloned()
            .collect();

        if let Some(limit) = filter.limit {
            results.truncate(limit);
        }

        Ok(results)
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<AuditEvent>, AuditError> {
        let events = self.events.read().await;
        Ok(events.iter().find(|e| e.id == id).cloned())
    }
}

/// `PostgreSQL`-backed audit logger (for production)
///
/// Persists audit events to a `PostgreSQL` database for compliance and security monitoring.
///
/// # Schema
///
/// Requires the following table:
///
/// ```sql
/// CREATE TABLE audit_logs (
///     id UUID PRIMARY KEY,
///     event_type VARCHAR(50) NOT NULL,
///     severity VARCHAR(20) NOT NULL,
///     actor VARCHAR(255) NOT NULL,
///     action VARCHAR(100) NOT NULL,
///     resource VARCHAR(255),
///     success BOOLEAN NOT NULL,
///     error_message TEXT,
///     source_ip VARCHAR(45),
///     user_agent TEXT,
///     session_id VARCHAR(255),
///     request_id VARCHAR(255),
///     metadata JSONB,
///     timestamp TIMESTAMPTZ NOT NULL,
///     created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
/// );
/// CREATE INDEX idx_audit_actor ON audit_logs(actor);
/// CREATE INDEX idx_audit_event_type ON audit_logs(event_type);
/// CREATE INDEX idx_audit_timestamp ON audit_logs(timestamp);
/// ```
///
/// # Example
///
/// ```ignore
/// use composable_rust_agent_patterns::audit::{PostgresAuditLogger, AuditLogger, AuditEvent};
/// use sqlx::postgres::PgPool;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let pool = PgPool::connect("postgres://localhost/mydb").await?;
/// let logger = PostgresAuditLogger::new(pool);
///
/// let event = AuditEvent::authentication("user@example.com", "login", true);
/// logger.log(event).await?;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "audit-postgres")]
#[derive(Clone)]
pub struct PostgresAuditLogger {
    pool: sqlx::postgres::PgPool,
}

#[cfg(feature = "audit-postgres")]
impl PostgresAuditLogger {
    /// Create a new `PostgreSQL` audit logger
    #[must_use]
    pub const fn new(pool: sqlx::postgres::PgPool) -> Self {
        Self { pool }
    }

    /// Get a reference to the connection pool
    #[must_use]
    pub const fn pool(&self) -> &sqlx::postgres::PgPool {
        &self.pool
    }

    /// Get event count
    ///
    /// # Errors
    ///
    /// Returns error if database query fails
    pub async fn count(&self) -> Result<usize, AuditError> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM audit_logs")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AuditError::QueryError(format!("Failed to count audit events: {e}")))?;

        Ok(row.0 as usize)
    }
}

#[cfg(feature = "audit-postgres")]
impl AuditLogger for PostgresAuditLogger {
    async fn log(&self, event: AuditEvent) -> Result<(), AuditError> {
        let metadata_json = serde_json::to_value(&event.metadata)
            .map_err(|e| AuditError::SerializationError(format!("Failed to serialize metadata: {e}")))?;

        // Parse RFC3339 timestamp string to DateTime<Utc>
        let timestamp = chrono::DateTime::parse_from_rfc3339(&event.timestamp)
            .map_err(|e| AuditError::SerializationError(format!("Invalid timestamp format: {e}")))?
            .with_timezone(&chrono::Utc);

        sqlx::query(
            r"
            INSERT INTO audit_logs (
                id, event_type, severity, actor, action, resource,
                success, error_message, source_ip, user_agent,
                session_id, request_id, metadata, timestamp
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            "
        )
        .bind(&event.id)
        .bind(event.event_type.to_string())
        .bind(event.severity.to_string())
        .bind(&event.actor)
        .bind(&event.action)
        .bind(&event.resource)
        .bind(event.success)
        .bind(&event.error_message)
        .bind(&event.source_ip)
        .bind(&event.user_agent)
        .bind(&event.session_id)
        .bind(&event.request_id)
        .bind(metadata_json)
        .bind(timestamp)
        .execute(&self.pool)
        .await
        .map_err(|e| AuditError::StorageError(format!("Failed to insert audit event: {e}")))?;

        tracing::debug!(
            event_id = %event.id,
            event_type = %event.event_type,
            actor = %event.actor,
            "Audit event logged to PostgreSQL"
        );

        Ok(())
    }

    async fn query(&self, filter: AuditEventFilter) -> Result<Vec<AuditEvent>, AuditError> {
        let mut query_str = String::from("SELECT * FROM audit_logs WHERE 1=1");
        let mut bindings: Vec<String> = Vec::new();

        if let Some(event_type) = filter.event_type {
            bindings.push(event_type.to_string());
            query_str.push_str(&format!(" AND event_type = ${}", bindings.len()));
        }

        if let Some(ref actor) = filter.actor {
            bindings.push(actor.clone());
            query_str.push_str(&format!(" AND actor = ${}", bindings.len()));
        }

        if let Some(ref action) = filter.action {
            bindings.push(action.clone());
            query_str.push_str(&format!(" AND action = ${}", bindings.len()));
        }

        if let Some(ref resource) = filter.resource {
            bindings.push(resource.clone());
            query_str.push_str(&format!(" AND resource = ${}", bindings.len()));
        }

        if let Some(success) = filter.success {
            bindings.push(success.to_string());
            query_str.push_str(&format!(" AND success = ${}", bindings.len()));
        }

        query_str.push_str(" ORDER BY timestamp DESC");

        if let Some(limit) = filter.limit {
            query_str.push_str(&format!(" LIMIT {limit}"));
        }

        // Build query dynamically based on filter
        let mut query = sqlx::query_as::<_, AuditEventRow>(&query_str);

        for binding in &bindings {
            query = query.bind(binding);
        }

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AuditError::QueryError(format!("Failed to query audit events: {e}")))?;

        rows.into_iter()
            .map(|row| row.into_audit_event())
            .collect()
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<AuditEvent>, AuditError> {
        let row = sqlx::query_as::<_, AuditEventRow>("SELECT * FROM audit_logs WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AuditError::QueryError(format!("Failed to get audit event: {e}")))?;

        Ok(row.map(|r| r.into_audit_event()).transpose()?)
    }
}

#[cfg(feature = "audit-postgres")]
#[derive(sqlx::FromRow)]
struct AuditEventRow {
    id: String,
    event_type: String,
    severity: String,
    actor: String,
    action: String,
    resource: Option<String>,
    success: bool,
    error_message: Option<String>,
    source_ip: Option<String>,
    user_agent: Option<String>,
    session_id: Option<String>,
    request_id: Option<String>,
    metadata: serde_json::Value,
    timestamp: String,
}

#[cfg(feature = "audit-postgres")]
impl AuditEventRow {
    fn into_audit_event(self) -> Result<AuditEvent, AuditError> {
        let event_type = match self.event_type.as_str() {
            "authentication" => AuditEventType::Authentication,
            "authorization" => AuditEventType::Authorization,
            "data_access" => AuditEventType::DataAccess,
            "configuration" => AuditEventType::Configuration,
            "security" => AuditEventType::Security,
            "llm_interaction" => AuditEventType::LlmInteraction,
            _ => return Err(AuditError::QueryError(format!("Unknown event type: {}", self.event_type))),
        };

        let severity = match self.severity.as_str() {
            "info" => Severity::Info,
            "warning" => Severity::Warning,
            "error" => Severity::Error,
            "critical" => Severity::Critical,
            _ => return Err(AuditError::QueryError(format!("Unknown severity: {}", self.severity))),
        };

        let metadata: HashMap<String, String> = serde_json::from_value(self.metadata)
            .map_err(|e| AuditError::QueryError(format!("Failed to parse metadata: {e}")))?;

        Ok(AuditEvent {
            id: self.id,
            timestamp: self.timestamp,
            event_type,
            severity,
            actor: self.actor,
            action: self.action,
            resource: self.resource,
            success: self.success,
            error_message: self.error_message,
            source_ip: self.source_ip,
            user_agent: self.user_agent,
            session_id: self.session_id,
            request_id: self.request_id,
            metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event_creation() {
        let event = AuditEvent::authentication("user@example.com", "login", true);
        assert_eq!(event.event_type, AuditEventType::Authentication);
        assert_eq!(event.actor, "user@example.com");
        assert_eq!(event.action, "login");
        assert!(event.success);
        assert_eq!(event.severity, Severity::Info);
    }

    #[test]
    fn test_audit_event_builder() {
        let event = AuditEvent::data_access("user@example.com", "read", "document:123", true)
            .with_source_ip("192.168.1.100")
            .with_user_agent("Mozilla/5.0")
            .with_session_id("session_abc")
            .with_metadata("document_size", "1024");

        assert_eq!(event.source_ip, Some("192.168.1.100".to_string()));
        assert_eq!(event.user_agent, Some("Mozilla/5.0".to_string()));
        assert_eq!(event.session_id, Some("session_abc".to_string()));
        assert_eq!(event.metadata.get("document_size"), Some(&"1024".to_string()));
    }

    #[test]
    fn test_audit_event_with_error() {
        let event = AuditEvent::authentication("user@example.com", "login", true)
            .with_error("Invalid password");

        assert!(!event.success);
        assert_eq!(event.error_message, Some("Invalid password".to_string()));
        assert_eq!(event.severity, Severity::Error);
    }

    #[test]
    fn test_security_event() {
        let event = AuditEvent::security("192.168.1.100", "rate_limit_exceeded", Severity::Warning);

        assert_eq!(event.event_type, AuditEventType::Security);
        assert_eq!(event.severity, Severity::Warning);
        assert!(!event.success);
    }

    #[test]
    fn test_event_serialization() {
        let event = AuditEvent::authentication("user@example.com", "login", true);
        let json = event.to_json().unwrap();
        assert!(json.contains("authentication"));
        assert!(json.contains("user@example.com"));
    }

    #[tokio::test]
    async fn test_in_memory_logger() {
        let logger = InMemoryAuditLogger::new();

        let event1 = AuditEvent::authentication("user1@example.com", "login", true);
        let event2 = AuditEvent::authentication("user2@example.com", "login", false);

        logger.log(event1).await.unwrap();
        logger.log(event2).await.unwrap();

        assert_eq!(logger.count().await, 2);
    }

    #[tokio::test]
    async fn test_audit_query_by_event_type() {
        let logger = InMemoryAuditLogger::new();

        logger.log(AuditEvent::authentication("user1", "login", true)).await.unwrap();
        logger.log(AuditEvent::authorization("user2", "check_permission", true)).await.unwrap();
        logger.log(AuditEvent::authentication("user3", "logout", true)).await.unwrap();

        let filter = AuditEventFilter::new().event_type(AuditEventType::Authentication);
        let results = logger.query(filter).await.unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|e| e.event_type == AuditEventType::Authentication));
    }

    #[tokio::test]
    async fn test_audit_query_by_actor() {
        let logger = InMemoryAuditLogger::new();

        logger.log(AuditEvent::authentication("user1", "login", true)).await.unwrap();
        logger.log(AuditEvent::authentication("user2", "login", true)).await.unwrap();
        logger.log(AuditEvent::authentication("user1", "logout", true)).await.unwrap();

        let filter = AuditEventFilter::new().actor("user1".to_string());
        let results = logger.query(filter).await.unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|e| e.actor == "user1"));
    }

    #[tokio::test]
    async fn test_audit_query_by_success() {
        let logger = InMemoryAuditLogger::new();

        logger.log(AuditEvent::authentication("user1", "login", true)).await.unwrap();
        logger.log(AuditEvent::authentication("user2", "login", false)).await.unwrap();
        logger.log(AuditEvent::authentication("user3", "login", false)).await.unwrap();

        let filter = AuditEventFilter::new().success(false);
        let results = logger.query(filter).await.unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|e| !e.success));
    }

    #[tokio::test]
    async fn test_audit_query_by_severity() {
        let logger = InMemoryAuditLogger::new();

        logger.log(AuditEvent::authentication("user1", "login", true)).await.unwrap(); // Info
        logger.log(AuditEvent::security("user2", "brute_force", Severity::Warning)).await.unwrap();
        logger.log(AuditEvent::security("user3", "data_breach", Severity::Critical)).await.unwrap();

        let filter = AuditEventFilter::new().min_severity(Severity::Warning);
        let results = logger.query(filter).await.unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|e| e.severity >= Severity::Warning));
    }

    #[tokio::test]
    async fn test_audit_query_limit() {
        let logger = InMemoryAuditLogger::new();

        for i in 0..10 {
            logger.log(AuditEvent::authentication(format!("user{i}"), "login", true)).await.unwrap();
        }

        let filter = AuditEventFilter::new().limit(5);
        let results = logger.query(filter).await.unwrap();

        assert_eq!(results.len(), 5);
    }

    #[tokio::test]
    async fn test_get_by_id() {
        let logger = InMemoryAuditLogger::new();

        let event = AuditEvent::authentication("user1", "login", true);
        let event_id = event.id.clone();

        logger.log(event).await.unwrap();

        let retrieved = logger.get_by_id(&event_id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, event_id);

        let not_found = logger.get_by_id("nonexistent").await.unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_filter_matches() {
        let event = AuditEvent::authentication("user1", "login", true)
            .with_severity(Severity::Info);

        let filter = AuditEventFilter::new()
            .event_type(AuditEventType::Authentication)
            .actor("user1".to_string());

        assert!(filter.matches(&event));

        let filter2 = AuditEventFilter::new().actor("user2".to_string());
        assert!(!filter2.matches(&event));
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        assert!(Severity::Error < Severity::Critical);
    }
}
