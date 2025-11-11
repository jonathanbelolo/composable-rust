# Security Monitoring and Incident Response Guide

Comprehensive guide to security monitoring, incident tracking, and threat response.

## Table of Contents

- [Overview](#overview)
- [Quick Start](#quick-start)
- [Incident Types](#incident-types)
- [Threat Levels](#threat-levels)
- [Reporting Incidents](#reporting-incidents)
- [Security Dashboard](#security-dashboard)
- [Alerting](#alerting)
- [Incident Response](#incident-response)
- [Integration with Audit Logs](#integration-with-audit-logs)
- [Best Practices](#best-practices)
- [Examples](#examples)

## Overview

The security monitoring framework provides:
- **Incident tracking** - Structured security incident management
- **Threat classification** - 4-level threat assessment (Low → Critical)
- **Real-time dashboards** - Operational security visibility
- **Automated alerting** - Immediate notification of critical events
- **Audit integration** - Correlation with audit trail
- **Anomaly detection** - Automatic pattern recognition

### Security Posture

Monitor these key metrics:
- Active security incidents
- Failed authentication attempts
- Unique attackers
- Privilege escalation attempts
- Data access anomalies
- LLM-specific threats (prompt injection, jailbreaking)

### Phase 8.4 Implementation Scope

**Current Implementation**: In-memory storage with `SecurityMonitor`
**Intended Use**: Development, testing, proof-of-concept deployments
**Production Readiness**: Core framework complete, persistence layer planned for future phases

**Known Limitations in Phase 8.4**:
- Incidents stored in memory (lost on restart)
- No distributed querying across multiple instances
- Dashboard metrics use simplified time windows (not actual 24h filtering)
- Basic threat detection algorithms (threshold-based only)

**Production Recommendations**:
- Implement PostgreSQL backend for incident persistence
- Add time-series database for metrics (InfluxDB, TimescaleDB)
- Deploy distributed tracing for correlation
- Integrate with SIEM systems (Splunk, ELK, Datadog)
- Implement advanced ML-based anomaly detection

## Quick Start

### Basic Usage

```rust
use composable_rust_agent_patterns::security::{
    SecurityMonitor, SecurityIncident, ThreatLevel,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let monitor = SecurityMonitor::new();

    // Report brute force attack
    let incident = SecurityIncident::brute_force_attack(
        "192.168.1.100",
        25,  // Failed attempts
    );

    let incident_id = monitor.report_incident(incident).await?;
    println!("Incident reported: {}", incident_id);

    // Get security dashboard
    let dashboard = monitor.get_dashboard().await?;
    println!("Active incidents: {}", dashboard.active_incidents);
    println!("Total incidents: {}", dashboard.total_incidents);

    Ok(())
}
```

## Incident Types

### 10 Security Incident Categories

| Type | Description | Typical Threat Level |
|------|-------------|---------------------|
| `BruteForceAttack` | Multiple failed login attempts | Medium → High |
| `AnomalousAccess` | Unusual access patterns | Low → Medium |
| `PrivilegeEscalation` | Unauthorized permission elevation | Critical |
| `DataExfiltration` | Large data download/export | High → Critical |
| `PromptInjection` | LLM prompt manipulation | High |
| `RateLimitAbuse` | Excessive API requests | Low → Medium |
| `UnauthorizedAccess` | Access without permission | Medium → High |
| `ConfigurationTampering` | Unauthorized config changes | High |
| `CredentialStuffing` | Using stolen credentials | High |
| `SessionHijacking` | Session token theft | Critical |

### Creating Incidents

```rust
use composable_rust_agent_patterns::security::{SecurityIncident, ThreatLevel};

// Brute force
let incident = SecurityIncident::brute_force_attack("192.168.1.100", 15);

// Prompt injection
let incident = SecurityIncident::prompt_injection(
    "user@example.com",
    "pattern_match",
);

// Privilege escalation
let incident = SecurityIncident::privilege_escalation(
    "user@example.com",
    "admin_access",
);

// Generic incident
let incident = SecurityIncident::new(
    IncidentType::DataExfiltration,
    ThreatLevel::Critical,
    "user@example.com",
    "Unusual data export volume detected",
)
.with_resource("database:customers")
.with_metadata("bytes_exported", "10000000");
```

## Threat Levels

### 4-Level Classification

| Level | Priority | Response Time | Examples |
|-------|----------|---------------|----------|
| **Low** | Monitoring | Best effort | Single failed login, minor anomaly |
| **Medium** | Investigation | 24 hours | 5-10 failed logins, unusual access time |
| **High** | Immediate | 1 hour | 20+ failed logins, prompt injection |
| **Critical** | Emergency | 15 minutes | Data breach, privilege escalation, active attack |

### Automatic Threat Assessment

```rust
// Automatic threat level based on failed attempts
let incident = SecurityIncident::brute_force_attack("ip", 25);
// threat_level: High (> 20 attempts)

let incident = SecurityIncident::brute_force_attack("ip", 10);
// threat_level: Medium (≤ 20 attempts)

// Manual threat level
let incident = SecurityIncident::new(
    IncidentType::AnomalousAccess,
    ThreatLevel::Low,  // Explicit
    "user",
    "Access from new location",
);
```

## Reporting Incidents

### Report Flow

1. **Create incident** with type, threat level, source
2. **Add context** (resources, events, metadata)
3. **Report to monitor** - Generates ID, triggers alerts
4. **Auto-correlate** with audit logs

```rust
async fn report_security_incident(
    monitor: &SecurityMonitor,
    incident: SecurityIncident,
) -> Result<String, SecurityError> {
    // Report incident
    let incident_id = monitor.report_incident(incident).await?;

    // High/Critical incidents generate automatic alerts
    // No additional action needed

    Ok(incident_id)
}
```

### Rich Metadata

```rust
let incident = SecurityIncident::prompt_injection(
    "user@example.com",
    "regex_pattern",
)
.with_resource("llm:claude-sonnet")
.with_event("audit_evt_123")  // Link to audit event
.with_metadata("prompt_length", "1024")
.with_metadata("injection_type", "system_override")
.with_metadata("blocked", "true");

monitor.report_incident(incident).await?;
```

## Security Dashboard

### Dashboard Metrics

```rust
let dashboard = monitor.get_dashboard().await?;

// Overall metrics
println!("Total incidents: {}", dashboard.total_incidents);
println!("Active incidents: {}", dashboard.active_incidents);

// Breakdown by threat
for (level, count) in &dashboard.incidents_by_threat {
    println!("{}: {}", level, count);
}

// Breakdown by type
for (incident_type, count) in &dashboard.incidents_by_type {
    println!("{}: {}", incident_type, count);
}

// Time-based metrics
// NOTE: Phase 8.4 uses simplified time windows (all incidents, not actual 24h filtering)
println!("Failed auth (24h): {}", dashboard.failed_auth_24h);
println!("Unique attackers (24h): {}", dashboard.unique_attackers_24h);

// Top threats
for attacker in &dashboard.top_attackers {
    println!(
        "{}: {} incidents, max threat: {}",
        attacker.source,
        attacker.incident_count,
        attacker.max_threat
    );
}

// Recent critical
for incident in &dashboard.recent_critical {
    println!("CRITICAL: {} - {}", incident.incident_type, incident.description);
}
```

### Real-Time Monitoring

```rust
use tokio::time::{sleep, Duration};

async fn security_monitoring_loop(monitor: &SecurityMonitor) {
    loop {
        let dashboard = monitor.get_dashboard().await.unwrap();

        if dashboard.active_incidents > 0 {
            println!("⚠️  {} active incidents", dashboard.active_incidents);

            // Check for critical incidents
            let critical = monitor
                .get_incidents_by_threat(ThreatLevel::Critical)
                .await;

            if !critical.is_empty() {
                println!("🚨 {} CRITICAL incidents!", critical.len());
                // Trigger paging, escalate to security team
            }
        }

        sleep(Duration::from_secs(60)).await;
    }
}
```

## Alerting

### Automatic Alerts

Alerts are **automatically generated** for High and Critical threats:

```rust
// This automatically generates an alert
let incident = SecurityIncident::brute_force_attack("ip", 25); // High
monitor.report_incident(incident).await?;

// Check recent alerts
let alerts = monitor.get_recent_alerts(10).await;
for alert in &alerts {
    println!(
        "[{}] {}: {}",
        alert.severity,
        alert.timestamp,
        alert.message
    );
}
```

### Custom Alerts

```rust
use composable_rust_agent_patterns::security::SecurityAlert;
use composable_rust_agent_patterns::audit::Severity;

// Create custom alert
let alert = SecurityAlert::new(
    "Unusual spike in API traffic detected",
    Severity::Warning,
);

// Alerts are typically generated from incidents automatically,
// but you can create custom alerts as needed
```

### Alert Integration

Integrate with external systems:

```rust
async fn handle_alert(alert: &SecurityAlert) {
    match alert.severity {
        Severity::Critical => {
            // Page on-call engineer
            send_pager_duty(alert).await;
            // Send to Slack #security-alerts
            send_slack_alert(alert).await;
            // Create Jira ticket
            create_jira_ticket(alert).await;
        }
        Severity::Error => {
            // Slack notification
            send_slack_alert(alert).await;
        }
        Severity::Warning => {
            // Log to monitoring dashboard
            log_to_dashboard(alert).await;
        }
        Severity::Info => {
            // No immediate action
        }
    }
}
```

## Incident Response

### Incident Status Workflow

```
Open → Investigating → Resolved / FalsePositive
```

### Managing Incidents

```rust
// Get incident
let incident = monitor.get_incident(&incident_id).await.unwrap();

// Start investigation
monitor.update_incident_status(
    &incident_id,
    IncidentStatus::Investigating,
).await?;

// After investigation
monitor.update_incident_status(
    &incident_id,
    IncidentStatus::Resolved,  // or FalsePositive
).await?;
```

### Active Incident Management

```rust
async fn review_active_incidents(monitor: &SecurityMonitor) {
    let active = monitor.get_active_incidents().await;

    for incident in &active {
        println!("\n=== Incident {} ===", incident.id);
        println!("Type: {}", incident.incident_type);
        println!("Threat: {}", incident.threat_level);
        println!("Source: {}", incident.source);
        println!("Status: {}", incident.status);

        // Review and take action
        match incident.threat_level {
            ThreatLevel::Critical => {
                // Immediate response
                block_ip(&incident.source).await;
                escalate_to_security_team(incident).await;
            }
            ThreatLevel::High => {
                // Investigate within 1 hour
                assign_to_security_analyst(incident).await;
            }
            _ => {
                // Queue for review
                add_to_review_queue(incident).await;
            }
        }
    }
}
```

## Integration with Audit Logs

### Automatic Analysis

```rust
use composable_rust_agent_patterns::audit::{AuditLogger, InMemoryAuditLogger};

async fn analyze_security_threats(
    audit_logger: &dyn AuditLogger,
    monitor: &SecurityMonitor,
) -> Result<(), Box<dyn std::error::Error>> {
    // Analyze audit logs for security patterns
    let incident_ids = monitor.analyze_audit_events(audit_logger).await?;

    println!("Detected {} security incidents from audit logs", incident_ids.len());

    for incident_id in &incident_ids {
        let incident = monitor.get_incident(incident_id).await.unwrap();
        println!("- {}: {}", incident.incident_type, incident.description);
    }

    Ok(())
}
```

### Manual Correlation

```rust
use composable_rust_agent_patterns::audit::{AuditEvent, AuditEventFilter};

async fn correlate_incident_with_audit(
    incident_id: &str,
    monitor: &SecurityMonitor,
    audit_logger: &dyn AuditLogger,
) -> Result<(), Box<dyn std::error::Error>> {
    let incident = monitor.get_incident(incident_id).await.unwrap();

    // Find related audit events
    let filter = AuditEventFilter::new()
        .actor(incident.source.clone())
        .limit(100);

    let events = audit_logger.query(filter).await?;

    println!("Found {} related audit events", events.len());

    // Add event IDs to incident
    for event in &events {
        // In production, update incident with event IDs
        println!("  - {}: {}", event.action, event.timestamp);
    }

    Ok(())
}
```

## Best Practices

### 1. Report Early and Often

```rust
// ✅ Good: Report immediately
async fn handle_failed_login(ip: &str, monitor: &SecurityMonitor) {
    let incident = SecurityIncident::brute_force_attack(ip, 1);
    monitor.report_incident(incident).await.ok();
    // Multiple reports from same IP will be aggregated in dashboard
}

// ❌ Bad: Wait until threshold
// Missing early detection opportunity
```

### 2. Include Rich Context

```rust
// ❌ Bad: Minimal context
let incident = SecurityIncident::new(
    IncidentType::UnauthorizedAccess,
    ThreatLevel::High,
    "user",
    "Access denied",
);

// ✅ Good: Rich context
let incident = SecurityIncident::new(
    IncidentType::UnauthorizedAccess,
    ThreatLevel::High,
    "user@example.com",
    "Attempted to access admin panel without admin role",
)
.with_resource("admin_panel")
.with_event("audit_evt_abc123")
.with_metadata("attempted_action", "view_all_users")
.with_metadata("user_role", "basic_user")
.with_metadata("source_ip", "192.168.1.100");
```

### 3. Set Appropriate Threat Levels

```rust
// Routine failed login
ThreatLevel::Low

// Multiple failed logins (5-10)
ThreatLevel::Medium

// Brute force attack (20+)
ThreatLevel::High

// Data breach, privilege escalation
ThreatLevel::Critical
```

### 4. Regular Dashboard Review

```rust
// Daily security review
async fn daily_security_review(monitor: &SecurityMonitor) {
    let dashboard = monitor.get_dashboard().await.unwrap();

    // Check for trends
    if dashboard.failed_auth_24h > 100 {
        println!("⚠️  High authentication failure rate");
    }

    // Review active incidents
    if dashboard.active_incidents > 10 {
        println!("⚠️  High number of active incidents");
    }

    // Check top attackers
    for attacker in dashboard.top_attackers.iter().take(5) {
        if attacker.incident_count > 10 {
            println!("🚨 Persistent attacker: {}", attacker.source);
            // Consider blocking
        }
    }
}
```

### 5. Automate Response for Known Patterns

```rust
async fn automated_threat_response(
    incident: &SecurityIncident,
    firewall: &Firewall,
) {
    match (&incident.incident_type, incident.threat_level) {
        (IncidentType::BruteForceAttack, ThreatLevel::High) => {
            // Auto-block IP after 20 failed attempts
            firewall.block_ip(&incident.source, Duration::from_secs(3600)).await;
        }
        (IncidentType::PromptInjection, _) => {
            // Rate limit user
            firewall.rate_limit(&incident.source, 10).await;
        }
        (IncidentType::PrivilegeEscalation, _) => {
            // Immediately revoke all sessions
            revoke_all_user_sessions(&incident.source).await;
        }
        _ => {
            // Manual review
        }
    }
}
```

### 6. False Positive Management

```rust
async fn handle_false_positive(
    incident_id: &str,
    monitor: &SecurityMonitor,
) {
    // Mark as false positive
    monitor.update_incident_status(
        incident_id,
        IncidentStatus::FalsePositive,
    ).await.unwrap();

    // Update detection rules to reduce false positives
    // (Implementation depends on your detection system)
}
```

### 7. Incident Retention

```rust
// Keep incidents for compliance
// - Active incidents: Forever
// - Resolved: 90 days minimum (longer for critical)
// - False positives: 30 days

async fn cleanup_old_incidents(monitor: &SecurityMonitor) {
    // In production, implement with database queries
    // Example logic:
    // - Delete false positives > 30 days old
    // - Archive resolved incidents > 90 days old
    // - Keep all critical incidents
}
```

## Examples

### Example 1: Detecting Brute Force Attacks

```rust
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

struct LoginTracker {
    failed_attempts: HashMap<String, Vec<u64>>,  // IP → timestamps
}

impl LoginTracker {
    async fn track_failed_login(
        &mut self,
        ip: &str,
        monitor: &SecurityMonitor,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs();

        // Track attempt
        let attempts = self.failed_attempts.entry(ip.to_string()).or_default();
        attempts.push(now);

        // Remove attempts older than 5 minutes
        attempts.retain(|&t| now - t < 300);

        // Check threshold
        let count = attempts.len();
        if count >= 5 {
            let threat_level = if count > 20 {
                ThreatLevel::High
            } else {
                ThreatLevel::Medium
            };

            let incident = SecurityIncident::new(
                IncidentType::BruteForceAttack,
                threat_level,
                ip,
                format!("{count} failed login attempts in 5 minutes"),
            )
            .with_metadata("time_window", "300")
            .with_metadata("failed_attempts", count.to_string());

            monitor.report_incident(incident).await?;
        }

        Ok(())
    }
}
```

### Example 2: Prompt Injection Detection

```rust
async fn check_prompt_injection(
    user_id: &str,
    prompt: &str,
    monitor: &SecurityMonitor,
) -> Result<(), Box<dyn std::error::Error>> {
    // Simple pattern matching (use ML in production)
    let suspicious_patterns = [
        "ignore previous instructions",
        "you are now",
        "forget all",
        "system:",
        "admin mode",
    ];

    for pattern in &suspicious_patterns {
        if prompt.to_lowercase().contains(pattern) {
            let incident = SecurityIncident::prompt_injection(
                user_id,
                "pattern_match",
            )
            .with_metadata("matched_pattern", pattern)
            .with_metadata("prompt_length", prompt.len().to_string());

            monitor.report_incident(incident).await?;

            return Err("Prompt injection detected".into());
        }
    }

    Ok(())
}
```

### Example 3: Data Exfiltration Detection

```rust
async fn track_data_export(
    user_id: &str,
    bytes_exported: u64,
    monitor: &SecurityMonitor,
) -> Result<(), Box<dyn std::error::Error>> {
    // Threshold: 10MB in single request
    const THRESHOLD: u64 = 10_000_000;

    if bytes_exported > THRESHOLD {
        let incident = SecurityIncident::new(
            IncidentType::DataExfiltration,
            ThreatLevel::High,
            user_id,
            "Unusually large data export detected",
        )
        .with_metadata("bytes_exported", bytes_exported.to_string())
        .with_metadata("threshold", THRESHOLD.to_string());

        monitor.report_incident(incident).await?;

        // Block export
        return Err("Data export blocked: size exceeds threshold".into());
    }

    Ok(())
}
```

### Example 4: Privilege Escalation Detection

```rust
async fn check_privilege_escalation(
    user_id: &str,
    current_role: &str,
    attempted_action: &str,
    required_role: &str,
    monitor: &SecurityMonitor,
) -> Result<(), Box<dyn std::error::Error>> {
    if current_role != required_role {
        let incident = SecurityIncident::privilege_escalation(
            user_id,
            attempted_action,
        )
        .with_metadata("current_role", current_role)
        .with_metadata("required_role", required_role);

        monitor.report_incident(incident).await?;

        return Err("Privilege escalation attempt blocked".into());
    }

    Ok(())
}
```

### Example 5: Security Operations Center (SOC) Dashboard

```rust
async fn soc_dashboard(
    monitor: &SecurityMonitor,
    audit_logger: &dyn AuditLogger,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Security Operations Dashboard ===\n");

    // Overall metrics
    let dashboard = monitor.get_dashboard().await?;
    println!("📊 Overall Status");
    println!("   Total Incidents: {}", dashboard.total_incidents);
    println!("   Active: {} | Failed Auth: {}",
        dashboard.active_incidents,
        dashboard.failed_auth_24h
    );

    // Critical incidents
    println!("\n🚨 Critical Incidents");
    for incident in &dashboard.recent_critical {
        println!("   [{}] {} - {}",
            incident.timestamp,
            incident.incident_type,
            incident.source
        );
    }

    // Top threats
    println!("\n⚠️  Top Threats");
    for (i, attacker) in dashboard.top_attackers.iter().take(5).enumerate() {
        println!("   {}. {} - {} incidents ({})",
            i + 1,
            attacker.source,
            attacker.incident_count,
            attacker.max_threat
        );
    }

    // Recent alerts
    println!("\n🔔 Recent Alerts");
    let alerts = monitor.get_recent_alerts(5).await;
    for alert in &alerts {
        println!("   [{}] {}", alert.severity, alert.message);
    }

    // Incident breakdown
    println!("\n📈 Incident Breakdown");
    for (incident_type, count) in &dashboard.incidents_by_type {
        println!("   {}: {}", incident_type, count);
    }

    Ok(())
}
```

### Example 6: Automated Incident Analysis

```rust
async fn automated_incident_analysis(
    monitor: &SecurityMonitor,
    audit_logger: &dyn AuditLogger,
) -> Result<(), Box<dyn std::error::Error>> {
    // Run periodic analysis
    let detected = monitor.analyze_audit_events(audit_logger).await?;

    println!("Automated analysis detected {} incidents", detected.len());

    for incident_id in &detected {
        let incident = monitor.get_incident(incident_id).await.unwrap();

        // Categorize and respond
        match incident.threat_level {
            ThreatLevel::Critical | ThreatLevel::High => {
                // Immediate action
                println!("🚨 HIGH/CRITICAL: {}", incident.description);
                // Alert security team
                // Block source if applicable
            }
            ThreatLevel::Medium => {
                // Queue for review
                println!("⚠️  MEDIUM: {}", incident.description);
            }
            ThreatLevel::Low => {
                // Log only
                println!("ℹ️  LOW: {}", incident.description);
            }
        }
    }

    Ok(())
}
```

## Troubleshooting

### High False Positive Rate

**Symptoms**: Many incidents marked as false positives

**Solutions**:
1. Adjust threat level thresholds
2. Add IP whitelisting for known services
3. Implement geo-blocking exceptions
4. Use machine learning for better detection

### Missing Incidents

**Symptoms**: Known attacks not detected

**Solutions**:
1. Review audit logging coverage
2. Add more detection patterns
3. Lower threat thresholds temporarily
4. Enable debug logging

### Alert Fatigue

**Symptoms**: Too many alerts, team ignoring them

**Solutions**:
1. Raise thresholds for High/Critical
2. Implement alert aggregation
3. Add cooldown periods
4. Use severity-based routing

## Additional Resources

- [NIST Cybersecurity Framework](https://www.nist.gov/cyberframework)
- [MITRE ATT&CK Framework](https://attack.mitre.org/)
- [OWASP Top 10](https://owasp.org/www-project-top-ten/)
- [CIS Controls](https://www.cisecurity.org/controls)

## Summary

- ✅ Report incidents immediately with rich context
- ✅ Use appropriate threat levels (Low → Critical)
- ✅ Monitor security dashboard daily
- ✅ Automate response for known threats
- ✅ Correlate with audit logs
- ✅ Manage false positives actively
- ✅ Escalate critical incidents within 15 minutes
- ✅ Review and close resolved incidents
