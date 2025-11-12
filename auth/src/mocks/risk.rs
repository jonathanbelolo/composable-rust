//! Mock risk calculator for testing.

use crate::actions::AuthLevel;
use crate::error::Result;
use crate::providers::{LoginContext, RiskAssessment, RiskCalculator, RiskFactor, RiskLevel};
use crate::providers::risk::IpLocation;
use std::future::Future;
use std::net::IpAddr;

/// Mock risk calculator.
///
/// Returns fixed risk assessments for testing.
#[derive(Debug, Clone)]
pub struct MockRiskCalculator {
    /// Risk score to return (0.0-1.0).
    pub risk_score: f32,
}

impl MockRiskCalculator {
    /// Create a new mock risk calculator with low risk.
    #[must_use]
    pub const fn new() -> Self {
        Self { risk_score: 0.1 }
    }

    /// Create a mock that returns high risk.
    #[must_use]
    pub const fn high_risk() -> Self {
        Self { risk_score: 0.9 }
    }
}

impl Default for MockRiskCalculator {
    fn default() -> Self {
        Self::new()
    }
}

impl RiskCalculator for MockRiskCalculator {
    fn calculate_login_risk(
        &self,
        _context: &LoginContext,
    ) -> impl Future<Output = Result<RiskAssessment>> + Send {
        let score = self.risk_score;

        async move {
            let level = if score < 0.3 {
                RiskLevel::Low
            } else if score < 0.6 {
                RiskLevel::Medium
            } else if score < 0.8 {
                RiskLevel::High
            } else {
                RiskLevel::Critical
            };

            let recommended_auth_level = match level {
                RiskLevel::Low => AuthLevel::Basic,
                RiskLevel::Medium => AuthLevel::MultiFactor,
                RiskLevel::High | RiskLevel::Critical => AuthLevel::HardwareBacked,
            };

            Ok(RiskAssessment {
                score,
                level,
                factors: vec![RiskFactor {
                    name: "mock_factor".to_string(),
                    weight: score,
                    description: "Mock risk factor for testing".to_string(),
                }],
                recommended_auth_level,
            })
        }
    }

    async fn is_ip_suspicious(
        &self,
        _ip_address: IpAddr,
    ) -> Result<bool> {
        Ok(false)
    }

    async fn get_ip_location(
        &self,
        _ip_address: IpAddr,
    ) -> Result<IpLocation> {
        Ok(IpLocation {
            country: "US".to_string(),
            region: Some("CA".to_string()),
            city: Some("San Francisco".to_string()),
            latitude: 37.7749,
            longitude: -122.4194,
        })
    }

    async fn detect_impossible_travel(
        &self,
        _from_location: &str,
        _to_location: &str,
        _time_delta: chrono::Duration,
    ) -> Result<bool> {
        Ok(false)
    }

    async fn check_credential_breach(
        &self,
        _email: &str,
    ) -> Result<bool> {
        Ok(false)
    }
}
