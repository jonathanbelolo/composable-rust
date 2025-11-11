//! Security Reporting and Monitoring
//!
//! Provides security incident tracking, alerting, and dashboard metrics.
//!
//! # Features
//!
//! - Security incident tracking and correlation
//! - Real-time security metrics and dashboards
//! - Alert generation for critical events
//! - Anomaly detection utilities
//! - Security posture reporting
//!
//! # Example
//!
//! ```
//! use composable_rust_agent_patterns::security::{
//!     SecurityMonitor, SecurityIncident, ThreatLevel,
//! };
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let monitor = SecurityMonitor::new();
//!
//! // Report security incident
//! let incident = SecurityIncident::brute_force_attack(
//!     "192.168.1.100",
//!     10,  // Failed attempts
//! );
//!
//! monitor.report_incident(incident).await?;
//!
//! // Get security dashboard
//! let dashboard = monitor.get_dashboard().await?;
//! println!("Active incidents: {}", dashboard.active_incidents);
//! # Ok(())
//! # }
//! ```

use crate::audit::{AuditEvent, AuditEventFilter, AuditEventType, AuditLogger, Severity};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Threat level classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreatLevel {
    /// Low threat (monitoring only)
    Low,
    /// Medium threat (investigation needed)
    Medium,
    /// High threat (immediate attention)
    High,
    /// Critical threat (emergency response)
    Critical,
}

impl fmt::Display for ThreatLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Low => write!(f, "low"),
            Self::Medium => write!(f, "medium"),
            Self::High => write!(f, "high"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

/// Security incident types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentType {
    /// Brute force attack detected
    BruteForceAttack,
    /// Unusual access pattern
    AnomalousAccess,
    /// Privilege escalation attempt
    PrivilegeEscalation,
    /// Data exfiltration suspected
    DataExfiltration,
    /// Prompt injection attack
    PromptInjection,
    /// Rate limit abuse
    RateLimitAbuse,
    /// Unauthorized access attempt
    UnauthorizedAccess,
    /// Configuration tampering
    ConfigurationTampering,
    /// Credential stuffing
    CredentialStuffing,
    /// Session hijacking
    SessionHijacking,
}

impl fmt::Display for IncidentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BruteForceAttack => write!(f, "brute_force_attack"),
            Self::AnomalousAccess => write!(f, "anomalous_access"),
            Self::PrivilegeEscalation => write!(f, "privilege_escalation"),
            Self::DataExfiltration => write!(f, "data_exfiltration"),
            Self::PromptInjection => write!(f, "prompt_injection"),
            Self::RateLimitAbuse => write!(f, "rate_limit_abuse"),
            Self::UnauthorizedAccess => write!(f, "unauthorized_access"),
            Self::ConfigurationTampering => write!(f, "configuration_tampering"),
            Self::CredentialStuffing => write!(f, "credential_stuffing"),
            Self::SessionHijacking => write!(f, "session_hijacking"),
        }
    }
}

/// Security incident status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IncidentStatus {
    /// Incident detected, needs review
    Open,
    /// Under investigation
    Investigating,
    /// Incident resolved
    Resolved,
    /// False positive
    FalsePositive,
}

impl fmt::Display for IncidentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Investigating => write!(f, "investigating"),
            Self::Resolved => write!(f, "resolved"),
            Self::FalsePositive => write!(f, "false_positive"),
        }
    }
}

/// Security incident report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityIncident {
    /// Unique incident ID
    pub id: String,
    /// Timestamp (RFC3339)
    pub timestamp: String,
    /// Incident type
    pub incident_type: IncidentType,
    /// Threat level
    pub threat_level: ThreatLevel,
    /// Status
    pub status: IncidentStatus,
    /// Source (IP address, user ID, etc.)
    pub source: String,
    /// Description
    pub description: String,
    /// Affected resources
    pub affected_resources: Vec<String>,
    /// Related audit event IDs
    pub related_events: Vec<String>,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

impl SecurityIncident {
    /// Create a new security incident
    #[must_use]
    pub fn new(
        incident_type: IncidentType,
        threat_level: ThreatLevel,
        source: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            incident_type,
            threat_level,
            status: IncidentStatus::Open,
            source: source.into(),
            description: description.into(),
            affected_resources: Vec::new(),
            related_events: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Create a brute force attack incident
    #[must_use]
    pub fn brute_force_attack(source: impl Into<String>, failed_attempts: u32) -> Self {
        let mut incident = Self::new(
            IncidentType::BruteForceAttack,
            if failed_attempts > 20 {
                ThreatLevel::High
            } else {
                ThreatLevel::Medium
            },
            source,
            format!("{failed_attempts} failed login attempts detected"),
        );
        incident.metadata.insert("failed_attempts".to_string(), failed_attempts.to_string());
        incident
    }

    /// Create a prompt injection incident
    #[must_use]
    pub fn prompt_injection(source: impl Into<String>, detection_method: impl Into<String>) -> Self {
        let mut incident = Self::new(
            IncidentType::PromptInjection,
            ThreatLevel::High,
            source,
            "Prompt injection attack detected",
        );
        incident.metadata.insert("detection_method".to_string(), detection_method.into());
        incident
    }

    /// Create a privilege escalation incident
    #[must_use]
    pub fn privilege_escalation(source: impl Into<String>, attempted_action: impl Into<String>) -> Self {
        let mut incident = Self::new(
            IncidentType::PrivilegeEscalation,
            ThreatLevel::Critical,
            source,
            "Privilege escalation attempt detected",
        );
        incident.metadata.insert("attempted_action".to_string(), attempted_action.into());
        incident
    }

    /// Add affected resource
    #[must_use]
    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        self.affected_resources.push(resource.into());
        self
    }

    /// Add related audit event
    #[must_use]
    pub fn with_event(mut self, event_id: impl Into<String>) -> Self {
        self.related_events.push(event_id.into());
        self
    }

    /// Add metadata
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Update status
    pub fn set_status(&mut self, status: IncidentStatus) {
        self.status = status;
    }
}

/// Security dashboard metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityDashboard {
    /// Total incidents
    pub total_incidents: usize,
    /// Active incidents (open + investigating)
    pub active_incidents: usize,
    /// Incidents by threat level
    pub incidents_by_threat: HashMap<String, usize>,
    /// Incidents by type
    pub incidents_by_type: HashMap<String, usize>,
    /// Failed authentication attempts (last 24h)
    pub failed_auth_24h: usize,
    /// Unique attackers (last 24h)
    pub unique_attackers_24h: usize,
    /// Top attacking IPs
    pub top_attackers: Vec<AttackerSummary>,
    /// Recent critical incidents
    pub recent_critical: Vec<SecurityIncident>,
}

/// Attacker summary for dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttackerSummary {
    /// Source (IP or user ID)
    pub source: String,
    /// Number of incidents
    pub incident_count: usize,
    /// Highest threat level
    pub max_threat: ThreatLevel,
}

/// Security alert
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityAlert {
    /// Alert ID
    pub id: String,
    /// Timestamp
    pub timestamp: String,
    /// Alert message
    pub message: String,
    /// Severity
    pub severity: Severity,
    /// Related incident
    pub incident_id: Option<String>,
}

impl SecurityAlert {
    /// Create a new security alert
    #[must_use]
    pub fn new(message: impl Into<String>, severity: Severity) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            message: message.into(),
            severity,
            incident_id: None,
        }
    }

    /// Create from incident
    #[must_use]
    pub fn from_incident(incident: &SecurityIncident) -> Self {
        let severity = match incident.threat_level {
            ThreatLevel::Low => Severity::Info,
            ThreatLevel::Medium => Severity::Warning,
            ThreatLevel::High => Severity::Error,
            ThreatLevel::Critical => Severity::Critical,
        };

        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: incident.timestamp.clone(),
            message: format!(
                "{}: {} from {}",
                incident.incident_type, incident.description, incident.source
            ),
            severity,
            incident_id: Some(incident.id.clone()),
        }
    }
}

/// Security monitor
pub struct SecurityMonitor {
    incidents: Arc<RwLock<Vec<SecurityIncident>>>,
    alerts: Arc<RwLock<Vec<SecurityAlert>>>,
}

impl SecurityMonitor {
    /// Create a new security monitor
    #[must_use]
    pub fn new() -> Self {
        Self {
            incidents: Arc::new(RwLock::new(Vec::new())),
            alerts: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Report a security incident
    pub async fn report_incident(&self, incident: SecurityIncident) -> Result<String, SecurityError> {
        let incident_id = incident.id.clone();

        // Generate alert for high/critical incidents
        if incident.threat_level >= ThreatLevel::High {
            let alert = SecurityAlert::from_incident(&incident);
            self.alerts.write().await.push(alert);
        }

        self.incidents.write().await.push(incident);
        Ok(incident_id)
    }

    /// Get incident by ID
    pub async fn get_incident(&self, id: &str) -> Option<SecurityIncident> {
        self.incidents
            .read()
            .await
            .iter()
            .find(|i| i.id == id)
            .cloned()
    }

    /// Update incident status
    pub async fn update_incident_status(
        &self,
        id: &str,
        status: IncidentStatus,
    ) -> Result<(), SecurityError> {
        let mut incidents = self.incidents.write().await;
        if let Some(incident) = incidents.iter_mut().find(|i| i.id == id) {
            incident.set_status(status);
            Ok(())
        } else {
            Err(SecurityError::IncidentNotFound(id.to_string()))
        }
    }

    /// Get active incidents
    pub async fn get_active_incidents(&self) -> Vec<SecurityIncident> {
        self.incidents
            .read()
            .await
            .iter()
            .filter(|i| matches!(i.status, IncidentStatus::Open | IncidentStatus::Investigating))
            .cloned()
            .collect()
    }

    /// Get incidents by threat level
    pub async fn get_incidents_by_threat(&self, min_level: ThreatLevel) -> Vec<SecurityIncident> {
        self.incidents
            .read()
            .await
            .iter()
            .filter(|i| i.threat_level >= min_level)
            .cloned()
            .collect()
    }

    /// Get recent alerts
    pub async fn get_recent_alerts(&self, limit: usize) -> Vec<SecurityAlert> {
        let alerts = self.alerts.read().await;
        let start = alerts.len().saturating_sub(limit);
        alerts[start..].to_vec()
    }

    /// Get security dashboard
    pub async fn get_dashboard(&self) -> Result<SecurityDashboard, SecurityError> {
        let incidents = self.incidents.read().await;

        let total_incidents = incidents.len();
        let active_incidents = incidents
            .iter()
            .filter(|i| matches!(i.status, IncidentStatus::Open | IncidentStatus::Investigating))
            .count();

        // Group by threat level
        let mut incidents_by_threat: HashMap<String, usize> = HashMap::new();
        for incident in incidents.iter() {
            *incidents_by_threat
                .entry(incident.threat_level.to_string())
                .or_insert(0) += 1;
        }

        // Group by type
        let mut incidents_by_type: HashMap<String, usize> = HashMap::new();
        for incident in incidents.iter() {
            *incidents_by_type
                .entry(incident.incident_type.to_string())
                .or_insert(0) += 1;
        }

        // Count failed auth (simplified - would use timestamp filtering in production)
        let failed_auth_24h = incidents
            .iter()
            .filter(|i| i.incident_type == IncidentType::BruteForceAttack)
            .count();

        // Unique attackers
        let unique_sources: std::collections::HashSet<_> =
            incidents.iter().map(|i| i.source.clone()).collect();
        let unique_attackers_24h = unique_sources.len();

        // Top attackers
        let mut attacker_counts: HashMap<String, (usize, ThreatLevel)> = HashMap::new();
        for incident in incidents.iter() {
            let entry = attacker_counts
                .entry(incident.source.clone())
                .or_insert((0, ThreatLevel::Low));
            entry.0 += 1;
            entry.1 = entry.1.max(incident.threat_level);
        }

        let mut top_attackers: Vec<AttackerSummary> = attacker_counts
            .into_iter()
            .map(|(source, (count, max_threat))| AttackerSummary {
                source,
                incident_count: count,
                max_threat,
            })
            .collect();
        top_attackers.sort_by(|a, b| b.incident_count.cmp(&a.incident_count));
        top_attackers.truncate(10);

        // Recent critical incidents
        let recent_critical: Vec<SecurityIncident> = incidents
            .iter()
            .filter(|i| i.threat_level == ThreatLevel::Critical)
            .rev()
            .take(5)
            .cloned()
            .collect();

        Ok(SecurityDashboard {
            total_incidents,
            active_incidents,
            incidents_by_threat,
            incidents_by_type,
            failed_auth_24h,
            unique_attackers_24h,
            top_attackers,
            recent_critical,
        })
    }

    /// Analyze audit events for security incidents
    pub async fn analyze_audit_events<L: AuditLogger>(
        &self,
        audit_logger: &L,
    ) -> Result<Vec<String>, SecurityError> {
        let mut incident_ids = Vec::new();

        // Detect brute force attacks
        let failed_auth_filter = AuditEventFilter::new()
            .event_type(AuditEventType::Authentication)
            .success(false)
            .limit(1000);

        let failed_auth = audit_logger
            .query(failed_auth_filter)
            .await
            .map_err(|e| SecurityError::AuditQueryError(e.to_string()))?;

        // Group by source IP
        let mut ip_failures: HashMap<String, Vec<AuditEvent>> = HashMap::new();
        for event in failed_auth {
            if let Some(ip) = &event.source_ip {
                ip_failures.entry(ip.clone()).or_default().push(event);
            }
        }

        // Report incidents for IPs with > 5 failures
        for (ip, failures) in ip_failures {
            if failures.len() > 5 {
                let incident = SecurityIncident::brute_force_attack(&ip, failures.len() as u32);
                let id = self.report_incident(incident).await?;
                incident_ids.push(id);
            }
        }

        Ok(incident_ids)
    }

    /// Clear all incidents (for testing)
    #[cfg(test)]
    pub async fn clear(&self) {
        self.incidents.write().await.clear();
        self.alerts.write().await.clear();
    }
}

impl Default for SecurityMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Security error
#[derive(Debug)]
pub enum SecurityError {
    /// Incident not found
    IncidentNotFound(String),
    /// Failed to query audit events
    AuditQueryError(String),
    /// Failed to generate report
    ReportError(String),
}

impl fmt::Display for SecurityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncidentNotFound(id) => write!(f, "Incident not found: {id}"),
            Self::AuditQueryError(msg) => write!(f, "Audit query error: {msg}"),
            Self::ReportError(msg) => write!(f, "Report error: {msg}"),
        }
    }
}

impl std::error::Error for SecurityError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_incident_creation() {
        let incident = SecurityIncident::brute_force_attack("192.168.1.100", 10);
        assert_eq!(incident.incident_type, IncidentType::BruteForceAttack);
        assert_eq!(incident.source, "192.168.1.100");
        assert_eq!(incident.status, IncidentStatus::Open);
        assert_eq!(incident.metadata.get("failed_attempts"), Some(&"10".to_string()));
    }

    #[test]
    fn test_prompt_injection_incident() {
        let incident = SecurityIncident::prompt_injection("user@example.com", "pattern_match");
        assert_eq!(incident.incident_type, IncidentType::PromptInjection);
        assert_eq!(incident.threat_level, ThreatLevel::High);
    }

    #[test]
    fn test_incident_builder() {
        let incident = SecurityIncident::new(
            IncidentType::UnauthorizedAccess,
            ThreatLevel::Medium,
            "user@example.com",
            "Unauthorized access attempt",
        )
        .with_resource("document:123")
        .with_event("evt_abc")
        .with_metadata("reason", "invalid_token");

        assert_eq!(incident.affected_resources.len(), 1);
        assert_eq!(incident.related_events.len(), 1);
        assert_eq!(incident.metadata.get("reason"), Some(&"invalid_token".to_string()));
    }

    #[test]
    fn test_threat_level_ordering() {
        assert!(ThreatLevel::Low < ThreatLevel::Medium);
        assert!(ThreatLevel::Medium < ThreatLevel::High);
        assert!(ThreatLevel::High < ThreatLevel::Critical);
    }

    #[test]
    fn test_security_alert_from_incident() {
        let incident = SecurityIncident::brute_force_attack("192.168.1.100", 25);
        let alert = SecurityAlert::from_incident(&incident);

        assert_eq!(alert.severity, Severity::Error); // High threat → Error
        assert!(alert.message.contains("brute_force_attack"));
        assert_eq!(alert.incident_id, Some(incident.id));
    }

    #[tokio::test]
    async fn test_security_monitor() {
        let monitor = SecurityMonitor::new();

        let incident = SecurityIncident::brute_force_attack("192.168.1.100", 10);
        let id = monitor.report_incident(incident).await.unwrap();

        let retrieved = monitor.get_incident(&id).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, id);
    }

    #[tokio::test]
    async fn test_update_incident_status() {
        let monitor = SecurityMonitor::new();

        let incident = SecurityIncident::brute_force_attack("192.168.1.100", 10);
        let id = monitor.report_incident(incident).await.unwrap();

        monitor.update_incident_status(&id, IncidentStatus::Resolved).await.unwrap();

        let updated = monitor.get_incident(&id).await.unwrap();
        assert_eq!(updated.status, IncidentStatus::Resolved);
    }

    #[tokio::test]
    async fn test_get_active_incidents() {
        let monitor = SecurityMonitor::new();

        let incident1 = SecurityIncident::brute_force_attack("192.168.1.100", 10);
        let id1 = monitor.report_incident(incident1).await.unwrap();

        let mut incident2 = SecurityIncident::brute_force_attack("192.168.1.101", 15);
        incident2.set_status(IncidentStatus::Resolved);
        monitor.report_incident(incident2).await.unwrap();

        let active = monitor.get_active_incidents().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, id1);
    }

    #[tokio::test]
    async fn test_get_incidents_by_threat() {
        let monitor = SecurityMonitor::new();

        monitor.report_incident(SecurityIncident::brute_force_attack("ip1", 5)).await.unwrap();
        monitor.report_incident(SecurityIncident::brute_force_attack("ip2", 25)).await.unwrap();
        monitor.report_incident(SecurityIncident::prompt_injection("user1", "method")).await.unwrap();

        let high_threat = monitor.get_incidents_by_threat(ThreatLevel::High).await;
        assert_eq!(high_threat.len(), 2); // 25 attempts → High, prompt injection → High
    }

    #[tokio::test]
    async fn test_security_dashboard() {
        let monitor = SecurityMonitor::new();

        monitor.report_incident(SecurityIncident::brute_force_attack("192.168.1.100", 10)).await.unwrap();
        monitor.report_incident(SecurityIncident::brute_force_attack("192.168.1.100", 15)).await.unwrap();
        monitor.report_incident(SecurityIncident::prompt_injection("user1", "pattern")).await.unwrap();

        let dashboard = monitor.get_dashboard().await.unwrap();

        assert_eq!(dashboard.total_incidents, 3);
        assert_eq!(dashboard.active_incidents, 3);
        assert!(dashboard.incidents_by_type.contains_key("brute_force_attack"));
        assert!(dashboard.incidents_by_type.contains_key("prompt_injection"));
    }

    #[tokio::test]
    async fn test_alert_generation() {
        let monitor = SecurityMonitor::new();

        // Low threat - no alert
        let low_incident = SecurityIncident::brute_force_attack("ip1", 5);
        monitor.report_incident(low_incident).await.unwrap();

        // High threat - generates alert
        let high_incident = SecurityIncident::brute_force_attack("ip2", 25);
        monitor.report_incident(high_incident).await.unwrap();

        let alerts = monitor.get_recent_alerts(10).await;
        assert_eq!(alerts.len(), 1); // Only high threat generates alert
    }

    #[tokio::test]
    async fn test_top_attackers() {
        let monitor = SecurityMonitor::new();

        // Same IP, multiple incidents
        monitor.report_incident(SecurityIncident::brute_force_attack("192.168.1.100", 10)).await.unwrap();
        monitor.report_incident(SecurityIncident::brute_force_attack("192.168.1.100", 15)).await.unwrap();
        monitor.report_incident(SecurityIncident::brute_force_attack("192.168.1.101", 5)).await.unwrap();

        let dashboard = monitor.get_dashboard().await.unwrap();

        assert!(!dashboard.top_attackers.is_empty());
        assert_eq!(dashboard.top_attackers[0].source, "192.168.1.100");
        assert_eq!(dashboard.top_attackers[0].incident_count, 2);
    }
}
