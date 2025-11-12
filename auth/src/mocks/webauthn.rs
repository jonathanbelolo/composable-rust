//! Mock `WebAuthn` provider for testing.

use crate::error::Result;
use crate::providers::PasskeyCredential;
use crate::providers::WebAuthnProvider;
use crate::providers::webauthn::{
    WebAuthnAuthenticationResult, WebAuthnChallenge, WebAuthnRegistrationResult,
};
use crate::state::UserId;
use std::future::Future;

/// Mock `WebAuthn` provider.
///
/// Simulates `WebAuthn` operations without actual crypto.
#[derive(Debug, Clone, Default)]
pub struct MockWebAuthnProvider;

impl MockWebAuthnProvider {
    /// Create a new mock `WebAuthn` provider.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl WebAuthnProvider for MockWebAuthnProvider {
    async fn generate_registration_challenge(
        &self,
        _user_id: UserId,
        _username: &str,
        _display_name: &str,
    ) -> Result<WebAuthnChallenge> {
        Ok(WebAuthnChallenge {
            challenge_id: "mock_challenge_id".to_string(),
            challenge: "mock_challenge_bytes".to_string(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        })
    }

    async fn verify_registration(
        &self,
        _challenge_id: &str,
        _attestation_response: &str,
        _expected_origin: &str,
        _expected_rp_id: &str,
    ) -> Result<WebAuthnRegistrationResult> {
        Ok(WebAuthnRegistrationResult {
            credential_id: "mock_credential_id".to_string(),
            public_key: vec![1, 2, 3, 4],
            counter: 0,
        })
    }

    async fn generate_authentication_challenge(
        &self,
        _user_id: UserId,
        _credentials: Vec<PasskeyCredential>,
    ) -> Result<WebAuthnChallenge> {
        Ok(WebAuthnChallenge {
            challenge_id: "mock_auth_challenge_id".to_string(),
            challenge: "mock_auth_challenge_bytes".to_string(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        })
    }

    fn verify_authentication(
        &self,
        _challenge_id: &str,
        _assertion_response: &str,
        credential: &PasskeyCredential,
        _expected_origin: &str,
        _expected_rp_id: &str,
    ) -> impl Future<Output = Result<WebAuthnAuthenticationResult>> + Send {
        let user_id = credential.user_id;
        let device_id = credential.device_id;

        async move {
            Ok(WebAuthnAuthenticationResult {
                user_id,
                device_id,
                counter: credential.counter + 1,
            })
        }
    }

    async fn extract_challenge_from_attestation(
        &self,
        _attestation_response: &str,
    ) -> Result<String> {
        // In a real implementation, this would parse the attestation response
        // and extract the challenge that was embedded by the client.
        // For testing, we return the mock challenge_id.
        Ok("mock_challenge_id".to_string())
    }

    async fn extract_challenge_from_assertion(
        &self,
        _assertion_response: &str,
    ) -> Result<String> {
        // In a real implementation, this would parse the assertion response
        // and extract the challenge that was embedded by the client.
        // For testing, we return the mock auth challenge_id.
        Ok("mock_auth_challenge_id".to_string())
    }
}
