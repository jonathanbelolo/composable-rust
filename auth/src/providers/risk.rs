//! Risk calculator trait.

use crate::error::Result;
use super::{RiskAssessment, LoginContext};

/// Risk calculator.
///
/// This trait abstracts over risk assessment for authentication.
///
/// # Implementation Notes
///
/// - Analyze IP address (geolocation, VPN detection, known bad actors)
/// - Check device fingerprint
/// - Detect impossible travel
/// - Check for leaked credentials
/// - Calculate risk score (0.0-1.0)
/// - Return recommended authentication level
///
/// # Advanced Features (Phase 6B/6C)
///
/// This is part of the risk-based adaptive authentication feature.
pub trait RiskCalculator: Send + Sync {
    /// Calculate login risk score.
    ///
    /// # Returns
    ///
    /// Risk assessment with score, level, factors, and recommended auth level.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Network request fails (IP geolocation, etc.)
    /// - Database query fails
    fn calculate_login_risk(
        &self,
        context: &LoginContext,
    ) -> impl std::future::Future<Output = Result<RiskAssessment>> + Send;

    /// Check if IP address is suspicious.
    ///
    /// # Returns
    ///
    /// `true` if IP is suspicious (VPN, Tor, known bad actor, etc.).
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    fn is_ip_suspicious(
        &self,
        ip_address: std::net::IpAddr,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Get IP geolocation.
    ///
    /// # Returns
    ///
    /// Country code, region, city, and coordinates.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    fn get_ip_location(
        &self,
        ip_address: std::net::IpAddr,
    ) -> impl std::future::Future<Output = Result<IpLocation>> + Send;

    /// Detect impossible travel.
    ///
    /// Checks if the user could have physically traveled between
    /// two locations in the given time period.
    ///
    /// # Returns
    ///
    /// `true` if travel is impossible (speed > 900 km/h).
    ///
    /// # Errors
    ///
    /// Returns error if calculation fails.
    fn detect_impossible_travel(
        &self,
        from_location: &str,
        to_location: &str,
        time_delta: chrono::Duration,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Check if credentials have been leaked.
    ///
    /// Uses `HaveIBeenPwned` API or similar.
    ///
    /// # Returns
    ///
    /// `true` if credentials appear in known breaches.
    ///
    /// # Errors
    ///
    /// Returns error if network request fails.
    fn check_credential_breach(
        &self,
        email: &str,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;
}

/// IP geolocation information.
#[derive(Debug, Clone, PartialEq)]
pub struct IpLocation {
    /// Country code (ISO 3166-1 alpha-2).
    pub country: String,

    /// Region/state.
    pub region: Option<String>,

    /// City.
    pub city: Option<String>,

    /// Latitude.
    pub latitude: f64,

    /// Longitude.
    pub longitude: f64,
}
