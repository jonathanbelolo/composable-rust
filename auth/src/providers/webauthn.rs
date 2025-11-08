//! WebAuthn/Passkey provider trait.

use crate::error::Result;
use crate::state::{DeviceId, UserId};
use super::PasskeyCredential;

/// WebAuthn provider.
///
/// This trait abstracts over WebAuthn/FIDO2 operations.
///
/// # Implementation Notes
///
/// - Use the `webauthn-rs` crate for WebAuthn protocol
/// - Handle challenge generation and storage (Redis)
/// - Verify attestations and assertions
/// - Manage credential storage (PostgreSQL)
pub trait WebAuthnProvider: Send + Sync {
    /// Generate registration challenge.
    ///
    /// # Returns
    ///
    /// Challenge ID and challenge bytes (base64-encoded).
    ///
    /// # Errors
    ///
    /// Returns error if challenge generation fails.
    async fn generate_registration_challenge(
        &self,
        user_id: UserId,
        username: &str,
        display_name: &str,
    ) -> Result<WebAuthnChallenge>;

    /// Verify registration response.
    ///
    /// # Returns
    ///
    /// Credential ID and public key.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Challenge is invalid or expired
    /// - Attestation verification fails
    /// - Origin or RP ID mismatch
    async fn verify_registration(
        &self,
        challenge_id: &str,
        attestation_response: &str,
        expected_origin: &str,
        expected_rp_id: &str,
    ) -> Result<WebAuthnRegistrationResult>;

    /// Generate authentication challenge.
    ///
    /// # Returns
    ///
    /// Challenge ID, challenge bytes, and allowed credentials.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - User has no credentials
    /// - Challenge generation fails
    async fn generate_authentication_challenge(
        &self,
        user_id: UserId,
        credentials: Vec<PasskeyCredential>,
    ) -> Result<WebAuthnChallenge>;

    /// Verify authentication response.
    ///
    /// # Returns
    ///
    /// User ID, device ID, and new counter value.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Challenge is invalid or expired
    /// - Assertion verification fails
    /// - Signature is invalid
    /// - Counter rollback detected
    async fn verify_authentication(
        &self,
        challenge_id: &str,
        assertion_response: &str,
        credential: &PasskeyCredential,
        expected_origin: &str,
        expected_rp_id: &str,
    ) -> Result<WebAuthnAuthenticationResult>;
}

/// WebAuthn challenge.
#[derive(Debug, Clone, PartialEq)]
pub struct WebAuthnChallenge {
    /// Challenge ID (stored in Redis).
    pub challenge_id: String,

    /// Challenge bytes (base64-encoded).
    pub challenge: String,

    /// Expiration timestamp.
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// WebAuthn registration result.
#[derive(Debug, Clone, PartialEq)]
pub struct WebAuthnRegistrationResult {
    /// Credential ID.
    pub credential_id: String,

    /// Public key (COSE format).
    pub public_key: Vec<u8>,

    /// Initial counter value.
    pub counter: u32,
}

/// WebAuthn authentication result.
#[derive(Debug, Clone, PartialEq)]
pub struct WebAuthnAuthenticationResult {
    /// User ID.
    pub user_id: UserId,

    /// Device ID.
    pub device_id: DeviceId,

    /// New counter value.
    pub counter: u32,
}
