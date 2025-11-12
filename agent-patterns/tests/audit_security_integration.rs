//! Integration tests for audit and security modules

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Test code can use unwrap/expect/panic

use composable_rust_agent_patterns::audit::{AuditEvent, AuditLogger, InMemoryAuditLogger};
use composable_rust_agent_patterns::security::{SecurityMonitor, ThreatLevel};

#[tokio::test]
async fn test_audit_security_integration() {
    let audit_logger = InMemoryAuditLogger::new();
    let security_monitor = SecurityMonitor::new();

    // Log multiple failed authentication attempts from same IP
    for i in 0..10 {
        let event = AuditEvent::authentication(
            format!("user{i}@example.com"),
            "login",
            false,
        )
        .with_source_ip("192.168.1.100");

        audit_logger.log(event).await.unwrap();
    }

    // Analyze audit events for security incidents
    let incident_ids = security_monitor
        .analyze_audit_events(&audit_logger)
        .await
        .unwrap();

    // Should detect brute force attack (> 5 failures from same IP)
    assert_eq!(incident_ids.len(), 1);

    let incident = security_monitor
        .get_incident(&incident_ids[0])
        .await
        .unwrap();

    assert_eq!(incident.source, "192.168.1.100");
    assert!(incident.threat_level >= ThreatLevel::Medium);
}

#[tokio::test]
async fn test_multiple_ips_separate_incidents() {
    let audit_logger = InMemoryAuditLogger::new();
    let security_monitor = SecurityMonitor::new();

    // Log failures from multiple IPs
    for ip_suffix in 100..=102 {
        for i in 0..7 {
            let event = AuditEvent::authentication(
                format!("user{i}@example.com"),
                "login",
                false,
            )
            .with_source_ip(format!("192.168.1.{ip_suffix}"));

            audit_logger.log(event).await.unwrap();
        }
    }

    // Analyze
    let incident_ids = security_monitor
        .analyze_audit_events(&audit_logger)
        .await
        .unwrap();

    // Should detect 3 separate incidents (one per IP)
    assert_eq!(incident_ids.len(), 3);
}

#[tokio::test]
async fn test_below_threshold_no_incident() {
    let audit_logger = InMemoryAuditLogger::new();
    let security_monitor = SecurityMonitor::new();

    // Log only 3 failed attempts (below threshold of 5)
    for i in 0..3 {
        let event = AuditEvent::authentication(
            format!("user{i}@example.com"),
            "login",
            false,
        )
        .with_source_ip("192.168.1.100");

        audit_logger.log(event).await.unwrap();
    }

    // Analyze
    let incident_ids = security_monitor
        .analyze_audit_events(&audit_logger)
        .await
        .unwrap();

    // Should NOT detect incident (below threshold)
    assert_eq!(incident_ids.len(), 0);
}

#[tokio::test]
async fn test_successful_logins_ignored() {
    let audit_logger = InMemoryAuditLogger::new();
    let security_monitor = SecurityMonitor::new();

    // Log successful logins
    for i in 0..10 {
        let event = AuditEvent::authentication(
            format!("user{i}@example.com"),
            "login",
            true,  // Success
        )
        .with_source_ip("192.168.1.100");

        audit_logger.log(event).await.unwrap();
    }

    // Analyze
    let incident_ids = security_monitor
        .analyze_audit_events(&audit_logger)
        .await
        .unwrap();

    // Should NOT detect incident (successful logins)
    assert_eq!(incident_ids.len(), 0);
}

#[tokio::test]
async fn test_dashboard_reflects_incidents() {
    let security_monitor = SecurityMonitor::new();

    // Report various incidents
    security_monitor
        .report_incident(composable_rust_agent_patterns::security::SecurityIncident::brute_force_attack(
            "192.168.1.100",
            10,
        ))
        .await
        .unwrap();

    security_monitor
        .report_incident(composable_rust_agent_patterns::security::SecurityIncident::prompt_injection(
            "user@example.com",
            "pattern_match",
        ))
        .await
        .unwrap();

    // Get dashboard
    let dashboard = security_monitor.get_dashboard().await.unwrap();

    assert_eq!(dashboard.total_incidents, 2);
    assert_eq!(dashboard.active_incidents, 2);
    assert!(dashboard.incidents_by_type.contains_key("brute_force_attack"));
    assert!(dashboard.incidents_by_type.contains_key("prompt_injection"));
}
