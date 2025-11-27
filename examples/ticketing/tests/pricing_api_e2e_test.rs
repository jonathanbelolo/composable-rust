//! Pricing API End-to-End HTTP integration tests.
//!
//! These tests verify that the pricing HTTP API endpoints work correctly
//! with real HTTP requests to a running server.
//!
//! Run with: `cargo test --test pricing_api_e2e_test -- --ignored --nocapture`
//!
//! Prerequisites:
//! - `docker compose up -d` must be running
//! - Server must be running on localhost:8080
//! - AUTH_TEST_TOKEN environment variable must be set on the server
//!
//! The tests use the multi-user test token feature (test-user-{uuid}) to
//! simulate different users making requests.

#![allow(clippy::expect_used, clippy::unwrap_used)] // Test code can use unwrap/expect
#![allow(clippy::too_many_lines)] // E2E tests demonstrate complex scenarios

use serde_json::json;

/// Base URL for the ticketing API
const API_BASE: &str = "http://localhost:8080";

/// Test user A (event owner)
const USER_A_TOKEN: &str = "test-user-00000000-0000-0000-0000-000000000001";

/// Test user B (different user, should not be able to update pricing)
const USER_B_TOKEN: &str = "test-user-00000000-0000-0000-0000-000000000002";

/// Helper function to create a test event payload
fn create_test_event_payload(name: &str) -> serde_json::Value {
    json!({
        "title": name,
        "description": format!("{} - Pricing API test event", name),
        "start_time": "2025-12-31T20:00:00Z",
        "end_time": "2025-12-31T23:00:00Z",
        "venue_name": "Pricing Test Arena",
        "venue_address": "123 Pricing Street, API City, AP 12345",
        "capacity": 2000,
        "price": 50
    })
}

/// Test 1: GET /api/events/{id}/pricing - Retrieve Event Pricing
///
/// Verifies that the GET pricing endpoint returns correct pricing details.
///
/// # Flow
///
/// 1. User A creates an event with initial pricing
/// 2. User A retrieves pricing via GET /api/events/{id}/pricing
/// 3. Verify response contains correct pricing tiers
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_get_event_pricing() {
    println!("🧪 Test 1: GET /api/events/{{id}}/pricing - Retrieve Event Pricing");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event with initial pricing");
    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("Pricing GET Test Event"))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(
        create_event_response.status(),
        201,
        "Event creation should succeed"
    );

    let created_event: serde_json::Value = create_event_response
        .json()
        .await
        .expect("Failed to parse event response");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Response should contain event_id");

    println!("  ✅ User A created event: {event_id}");

    // Step 2: Add VIP and Balcony sections to the event
    println!("  📝 Step 2: User A adds VIP and Balcony sections");
    let add_sections_payload = json!({
        "sections": [
            {
                "name": "VIP",
                "capacity": 100,
                "seat_type": "general_admission"
            },
            {
                "name": "Balcony",
                "capacity": 200,
                "seat_type": "general_admission"
            }
        ]
    });

    let add_sections_response = client
        .post(format!("{API_BASE}/api/events/{event_id}/sections"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&add_sections_payload)
        .send()
        .await
        .expect("Failed to add sections");

    assert_eq!(
        add_sections_response.status(),
        200,
        "Add sections should succeed"
    );

    println!("  ✅ Sections added successfully");

    // Step 3: Configure multiple pricing tiers for all sections
    println!("  📝 Step 3: User A configures dynamic pricing tiers for all sections");
    let configure_pricing_payload = json!({
        "pricing_tiers": [
            {
                "tier_type": "EarlyBird",
                "section": "General Admission",
                "price_cents": 4000,
                "available_from": "2025-01-01T00:00:00Z",
                "available_until": "2025-06-01T00:00:00Z"
            },
            {
                "tier_type": "Regular",
                "section": "General Admission",
                "price_cents": 5000,
                "available_from": "2025-06-01T00:00:00Z",
                "available_until": null
            },
            {
                "tier_type": "EarlyBird",
                "section": "VIP",
                "price_cents": 10000,
                "available_from": "2025-01-01T00:00:00Z",
                "available_until": "2025-06-01T00:00:00Z"
            },
            {
                "tier_type": "Regular",
                "section": "VIP",
                "price_cents": 15000,
                "available_from": "2025-06-01T00:00:00Z",
                "available_until": null
            },
            {
                "tier_type": "EarlyBird",
                "section": "Balcony",
                "price_cents": 3000,
                "available_from": "2025-01-01T00:00:00Z",
                "available_until": "2025-06-01T00:00:00Z"
            },
            {
                "tier_type": "Regular",
                "section": "Balcony",
                "price_cents": 4000,
                "available_from": "2025-06-01T00:00:00Z",
                "available_until": null
            }
        ]
    });

    let configure_pricing_response = client
        .patch(format!("{API_BASE}/api/events/{event_id}/pricing"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&configure_pricing_payload)
        .send()
        .await
        .expect("Failed to configure pricing");

    assert_eq!(
        configure_pricing_response.status(),
        200,
        "Configure pricing should succeed"
    );

    println!("  ✅ Pricing configured successfully");

    // Step 4: User A retrieves pricing
    println!("  📝 Step 4: User A retrieves pricing for the event");
    let get_pricing_response = client
        .get(format!("{API_BASE}/api/events/{event_id}/pricing"))
        .send()
        .await
        .expect("Failed to get pricing");

    assert_eq!(
        get_pricing_response.status(),
        200,
        "GET pricing should succeed"
    );

    let pricing_data: serde_json::Value = get_pricing_response
        .json()
        .await
        .expect("Failed to parse pricing response");

    println!("  ✅ Retrieved pricing: {pricing_data}");

    // Verify pricing structure
    let pricing_tiers = pricing_data["pricing_tiers"]
        .as_array()
        .expect("Should contain pricing_tiers array");

    // Should now have 6 pricing tiers (2 per section: EarlyBird and Regular for each)
    assert_eq!(
        pricing_tiers.len(),
        6,
        "Should have 6 pricing tiers (2 per section)"
    );

    // Verify General Admission pricing
    let ga_early_bird = pricing_tiers
        .iter()
        .find(|t| {
            t["tier_type"].as_str() == Some("EarlyBird")
                && t["section"].as_str() == Some("General Admission")
        })
        .expect("Should have General Admission EarlyBird pricing");
    assert_eq!(ga_early_bird["price_cents"].as_u64(), Some(4000));

    let ga_regular = pricing_tiers
        .iter()
        .find(|t| {
            t["tier_type"].as_str() == Some("Regular")
                && t["section"].as_str() == Some("General Admission")
        })
        .expect("Should have General Admission Regular pricing");
    assert_eq!(ga_regular["price_cents"].as_u64(), Some(5000));

    // Verify VIP pricing
    let vip_early_bird = pricing_tiers
        .iter()
        .find(|t| {
            t["tier_type"].as_str() == Some("EarlyBird") && t["section"].as_str() == Some("VIP")
        })
        .expect("Should have VIP EarlyBird pricing");
    assert_eq!(vip_early_bird["price_cents"].as_u64(), Some(10000));

    let vip_regular = pricing_tiers
        .iter()
        .find(|t| {
            t["tier_type"].as_str() == Some("Regular") && t["section"].as_str() == Some("VIP")
        })
        .expect("Should have VIP Regular pricing");
    assert_eq!(vip_regular["price_cents"].as_u64(), Some(15000));

    // Verify Balcony pricing
    let balcony_early_bird = pricing_tiers
        .iter()
        .find(|t| {
            t["tier_type"].as_str() == Some("EarlyBird")
                && t["section"].as_str() == Some("Balcony")
        })
        .expect("Should have Balcony EarlyBird pricing");
    assert_eq!(balcony_early_bird["price_cents"].as_u64(), Some(3000));

    let balcony_regular = pricing_tiers
        .iter()
        .find(|t| {
            t["tier_type"].as_str() == Some("Regular") && t["section"].as_str() == Some("Balcony")
        })
        .expect("Should have Balcony Regular pricing");
    assert_eq!(balcony_regular["price_cents"].as_u64(), Some(4000));

    println!("  ✅ Pricing data is correct for all sections");
}

/// Test 2: PATCH /api/events/{id}/pricing - Update Event Pricing (Success)
///
/// Verifies that the event owner can update pricing tiers.
///
/// # Flow
///
/// 1. User A creates an event
/// 2. User A updates pricing via PATCH /api/events/{id}/pricing
/// 3. Verify updated pricing is returned
/// 4. Verify GET pricing returns updated values
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_update_event_pricing_success() {
    println!("🧪 Test 2: PATCH /api/events/{{id}}/pricing - Update Event Pricing (Success)");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("Pricing Update Test Event"))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(create_event_response.status(), 201);

    let created_event: serde_json::Value = create_event_response
        .json()
        .await
        .expect("Failed to parse event response");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Response should contain event_id");

    println!("  ✅ User A created event: {event_id}");

    // Wait for event initialization
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Step 2: Add VIP and Balcony sections to the event
    println!("  📝 Step 2: User A adds VIP and Balcony sections");
    let add_sections_payload = json!({
        "sections": [
            {
                "name": "VIP",
                "capacity": 100,
                "seat_type": "general_admission"
            },
            {
                "name": "Balcony",
                "capacity": 200,
                "seat_type": "general_admission"
            }
        ]
    });

    let add_sections_response = client
        .post(format!("{API_BASE}/api/events/{event_id}/sections"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&add_sections_payload)
        .send()
        .await
        .expect("Failed to add sections");

    assert_eq!(
        add_sections_response.status(),
        200,
        "Add sections should succeed"
    );

    println!("  ✅ Sections added successfully");

    // Step 3: User A updates pricing with time-based tiers for all sections
    println!("  📝 Step 3: User A configures dynamic pricing tiers for all sections");
    let update_pricing_payload = json!({
        "pricing_tiers": [
            {
                "tier_type": "EarlyBird",
                "section": "VIP",
                "price_cents": 8000,
                "available_from": "2025-01-01T00:00:00Z",
                "available_until": "2025-06-01T00:00:00Z"
            },
            {
                "tier_type": "Regular",
                "section": "VIP",
                "price_cents": 10000,
                "available_from": "2025-06-01T00:00:00Z",
                "available_until": "2025-11-01T00:00:00Z"
            },
            {
                "tier_type": "LastMinute",
                "section": "VIP",
                "price_cents": 12000,
                "available_from": "2025-11-01T00:00:00Z",
                "available_until": null
            },
            {
                "tier_type": "Regular",
                "section": "General Admission",
                "price_cents": 5000,
                "available_from": "2025-01-01T00:00:00Z",
                "available_until": null
            },
            {
                "tier_type": "EarlyBird",
                "section": "Balcony",
                "price_cents": 3000,
                "available_from": "2025-01-01T00:00:00Z",
                "available_until": "2025-06-01T00:00:00Z"
            },
            {
                "tier_type": "Regular",
                "section": "Balcony",
                "price_cents": 4000,
                "available_from": "2025-06-01T00:00:00Z",
                "available_until": null
            }
        ]
    });

    let update_pricing_response = client
        .patch(format!("{API_BASE}/api/events/{event_id}/pricing"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&update_pricing_payload)
        .send()
        .await
        .expect("Failed to update pricing");

    assert_eq!(
        update_pricing_response.status(),
        200,
        "PATCH pricing should succeed"
    );

    let updated_pricing: serde_json::Value = update_pricing_response
        .json()
        .await
        .expect("Failed to parse update response");

    println!("  ✅ Updated pricing: {updated_pricing}");

    // Verify updated pricing structure
    let pricing_tiers = updated_pricing["pricing_tiers"]
        .as_array()
        .expect("Should contain pricing_tiers");

    assert_eq!(
        pricing_tiers.len(),
        6,
        "Should have 6 pricing tiers (3 VIP + 1 General + 2 Balcony)"
    );

    // Step 4: Verify GET pricing returns updated values
    println!("  📝 Step 4: Verify GET pricing returns updated values");
    let get_pricing_response = client
        .get(format!("{API_BASE}/api/events/{event_id}/pricing"))
        .send()
        .await
        .expect("Failed to get updated pricing");

    assert_eq!(get_pricing_response.status(), 200);

    let pricing_data: serde_json::Value = get_pricing_response
        .json()
        .await
        .expect("Failed to parse pricing response");

    let retrieved_tiers = pricing_data["pricing_tiers"]
        .as_array()
        .expect("Should contain pricing_tiers");

    assert_eq!(
        retrieved_tiers.len(),
        6,
        "Should have 6 pricing tiers (3 VIP + 1 General + 2 Balcony)"
    );

    // Verify EarlyBird tier exists
    let early_bird = retrieved_tiers
        .iter()
        .find(|t| t["tier_type"].as_str() == Some("EarlyBird"))
        .expect("Should have EarlyBird tier");

    assert_eq!(early_bird["price_cents"].as_u64(), Some(8000));

    println!("  ✅ Updated pricing persisted correctly");
}

/// Test 3: PATCH /api/events/{id}/pricing - Unauthorized (Non-Owner)
///
/// Verifies that only the event owner can update pricing.
///
/// # Flow
///
/// 1. User A creates an event
/// 2. User B attempts to update pricing (should fail with 403 Forbidden)
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_update_event_pricing_forbidden() {
    println!("🧪 Test 3: PATCH /api/events/{{id}}/pricing - Unauthorized (Non-Owner)");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("Pricing Auth Test Event"))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(create_event_response.status(), 201);

    let created_event: serde_json::Value = create_event_response
        .json()
        .await
        .expect("Failed to parse event response");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Response should contain event_id");

    println!("  ✅ User A created event: {event_id}");

    // Wait for event initialization
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Step 2: User B attempts to update pricing (should fail)
    println!("  📝 Step 2: User B attempts to update pricing (should fail)");
    let update_pricing_payload = json!({
        "pricing_tiers": [
            {
                "tier_type": "Regular",
                "section": "VIP",
                "price_cents": 1,
                "available_from": "2025-01-01T00:00:00Z",
                "available_until": null
            },
            {
                "tier_type": "Regular",
                "section": "General Admission",
                "price_cents": 1,
                "available_from": "2025-01-01T00:00:00Z",
                "available_until": null
            }
        ]
    });

    let forbidden_response = client
        .patch(format!("{API_BASE}/api/events/{event_id}/pricing"))
        .header("Authorization", format!("Bearer {USER_B_TOKEN}"))
        .json(&update_pricing_payload)
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(
        forbidden_response.status(),
        403,
        "User B should be forbidden from updating User A's event pricing"
    );

    println!("  ✅ User B correctly denied access to update pricing");
}

/// Test 4: PATCH /api/events/{id}/pricing - Validation Errors
///
/// Verifies that invalid pricing updates are rejected with proper error messages.
///
/// # Flow
///
/// 1. User A creates an event
/// 2. User A attempts to update with empty pricing tiers (should fail)
/// 3. User A attempts to update with invalid section (should fail)
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_update_event_pricing_validation() {
    println!("🧪 Test 4: PATCH /api/events/{{id}}/pricing - Validation Errors");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("Pricing Validation Test"))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(create_event_response.status(), 201);

    let created_event: serde_json::Value = create_event_response
        .json()
        .await
        .expect("Failed to parse event response");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Response should contain event_id");

    println!("  ✅ User A created event: {event_id}");

    // Wait for event initialization
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Step 2: Attempt to update with empty pricing tiers
    println!("  📝 Step 2: Attempt to update with empty pricing tiers");
    let empty_pricing_payload = json!({
        "pricing_tiers": []
    });

    let empty_response = client
        .patch(format!("{API_BASE}/api/events/{event_id}/pricing"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&empty_pricing_payload)
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(
        empty_response.status(),
        400,
        "Empty pricing tiers should be rejected"
    );

    println!("  ✅ Empty pricing tiers correctly rejected");

    // Step 3: Attempt to update with invalid section
    println!("  📝 Step 3: Attempt to update with invalid section");
    let invalid_section_payload = json!({
        "pricing_tiers": [
            {
                "tier_type": "Regular",
                "section": "NonExistentSection",
                "price_cents": 5000,
                "available_from": "2025-01-01T00:00:00Z",
                "available_until": null
            }
        ]
    });

    let invalid_section_response = client
        .patch(format!("{API_BASE}/api/events/{event_id}/pricing"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&invalid_section_payload)
        .send()
        .await
        .expect("Failed to send request");

    assert!(
        invalid_section_response.status().is_client_error(),
        "Invalid section should be rejected"
    );

    println!("  ✅ Invalid section correctly rejected");
}

/// Test 5: GET /api/events/{id}/pricing - Event Not Found
///
/// Verifies that querying pricing for non-existent event returns 404.
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_get_pricing_event_not_found() {
    println!("🧪 Test 5: GET /api/events/{{id}}/pricing - Event Not Found");

    let client = reqwest::Client::new();

    let non_existent_id = "00000000-0000-0000-0000-000000000000";

    let response = client
        .get(format!("{API_BASE}/api/events/{non_existent_id}/pricing"))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(
        response.status(),
        404,
        "Non-existent event should return 404"
    );

    println!("  ✅ Non-existent event correctly returns 404");
}
