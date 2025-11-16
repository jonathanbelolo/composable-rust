//! Regression test for bincode serialization bug with TokenData.
//!
//! This test demonstrates the bug where `TokenData` cannot be serialized/deserialized
//! with bincode because it contains a `serde_json::Value` field, which requires
//! `deserialize_any()` - a method that bincode does not support.
//!
//! Error: "Bincode does not support the serde::Deserializer::deserialize_any method"
//!
//! This bug prevents magic link verification from working when using `RedisTokenStore`.

use composable_rust_auth::providers::{TokenData, TokenType};
use chrono::{Duration, Utc};

#[test]
#[should_panic(expected = "DeserializeAnyNotSupported")]
fn test_token_data_bincode_serialization_fails() {
    // Create a TokenData with a serde_json::Value in the data field
    let token_data = TokenData::new(
        TokenType::MagicLink,
        "test-token-123".to_string(),
        serde_json::json!({"email": "test@example.com"}),
        Utc::now() + Duration::minutes(10),
    );

    // Serialize with bincode - this works fine
    let serialized = bincode::serialize(&token_data).expect("Failed to serialize");

    // Deserialize with bincode - THIS FAILS with:
    // "Bincode does not support the serde::Deserializer::deserialize_any method"
    //
    // This is because serde_json::Value requires deserialize_any() to determine
    // the type dynamically, but bincode doesn't support self-describing formats.
    let _deserialized: TokenData =
        bincode::deserialize(&serialized).expect("Failed to deserialize");
}

#[test]
fn test_token_data_json_serialization_works() {
    // For comparison: This works fine with serde_json
    let token_data = TokenData::new(
        TokenType::MagicLink,
        "test-token-123".to_string(),
        serde_json::json!({"email": "test@example.com"}),
        Utc::now() + Duration::minutes(10),
    );

    // serde_json supports deserialize_any, so this works
    let serialized = serde_json::to_string(&token_data).expect("Failed to serialize");
    let _deserialized: TokenData =
        serde_json::from_str(&serialized).expect("Failed to deserialize");
}
