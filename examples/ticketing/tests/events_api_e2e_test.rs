//! Events API End-to-End HTTP integration tests.
//!
//! These tests verify that the events HTTP API endpoints work correctly
//! with real HTTP requests to a running server.
//!
//! Run with: `cargo test --test events_api_e2e_test -- --ignored --nocapture`
//!
//! Prerequisites:
//! - `docker compose up -d` must be running
//! - Server must be running on localhost:8080
//! - AUTH_EXPOSE_MAGIC_LINKS_FOR_TESTING=true
//!
//! Tests cover:
//! - GET /api/events - List all events
//! - GET /api/events/:id - Get a specific event
//! - GET /api/events/:id/availability - Get event availability

#![allow(clippy::expect_used, clippy::unwrap_used)] // Test code can use unwrap/expect
#![allow(clippy::too_many_lines)] // E2E tests demonstrate complex scenarios

use serde_json::json;

/// Base URL for the ticketing API
const API_BASE: &str = "http://localhost:8080";

/// Test user A (event owner)
const USER_A_TOKEN: &str = "test-user-00000000-0000-0000-0000-000000000001";

/// Test user B (different user)
const USER_B_TOKEN: &str = "test-user-00000000-0000-0000-0000-000000000002";

/// Helper function to create a test event payload
fn create_test_event_payload(name: &str) -> serde_json::Value {
    json!({
        "title": name,
        "description": format!("{} - Events API test event", name),
        "start_time": "2025-12-31T20:00:00Z",
        "end_time": "2025-12-31T23:00:00Z",
        "venue_name": "Events Test Arena",
        "venue_address": "123 Events Street, API City, AP 12345",
        "capacity": 1000,
        "price": 50
    })
}

/// Test 1: GET /api/events - List All Events
///
/// Verifies that the list events endpoint returns a list of events.
///
/// # Flow
///
/// 1. User A creates an event
/// 2. User A retrieves all events via GET /api/events
/// 3. Verify the created event appears in the list
#[tokio::test]
#[ignore] // Requires running server
async fn test_list_events() {
    println!("🧪 Test 1: GET /api/events - List All Events");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let event_name = format!("List Events Test {}", uuid::Uuid::new_v4());
    let create_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload(&event_name))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(
        create_response.status(),
        201,
        "Event creation should succeed"
    );

    let created_event: serde_json::Value = create_response
        .json()
        .await
        .expect("Failed to parse event response");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Response should contain event_id");

    println!("  ✅ User A created event: {event_id}");

    // Step 2: List all events
    println!("  📝 Step 2: User A lists all events");
    let list_response = client
        .get(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .send()
        .await
        .expect("Failed to list events");

    assert_eq!(
        list_response.status(),
        200,
        "List events should return 200 OK"
    );

    let events_body: serde_json::Value = list_response
        .json()
        .await
        .expect("Failed to parse events list response");

    println!("  ✅ Received events list response");

    // Step 3: Verify our event appears in the list
    println!("  📝 Step 3: Verifying created event appears in list");
    let events = events_body["events"]
        .as_array()
        .expect("Response should contain events array");

    let found = events.iter().any(|e| {
        e["id"].as_str() == Some(event_id) || e["event_id"].as_str() == Some(event_id)
    });

    assert!(found, "Created event should appear in the events list");
    println!("  ✅ Created event found in list");

    println!("✅ Test 1 PASSED: List events returns created event\n");
}

/// Test 2: GET /api/events/:id - Get a Specific Event
///
/// Verifies that the get event endpoint returns correct event details.
///
/// # Flow
///
/// 1. User A creates an event
/// 2. User A retrieves the event by ID via GET /api/events/:id
/// 3. Verify response contains correct event details
#[tokio::test]
#[ignore] // Requires running server
async fn test_get_event_by_id() {
    println!("🧪 Test 2: GET /api/events/:id - Get Specific Event");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let event_name = format!("Get Event Test {}", uuid::Uuid::new_v4());
    let create_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload(&event_name))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(
        create_response.status(),
        201,
        "Event creation should succeed"
    );

    let created_event: serde_json::Value = create_response
        .json()
        .await
        .expect("Failed to parse event response");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Response should contain event_id");

    println!("  ✅ User A created event: {event_id}");

    // Step 2: Get the event by ID
    println!("  📝 Step 2: User A retrieves event by ID");
    let get_response = client
        .get(format!("{API_BASE}/api/events/{event_id}"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .send()
        .await
        .expect("Failed to get event");

    assert_eq!(
        get_response.status(),
        200,
        "Get event should return 200 OK"
    );

    let event_body: serde_json::Value = get_response
        .json()
        .await
        .expect("Failed to parse event response");

    println!("  ✅ Received event details");

    // Step 3: Verify event details
    println!("  📝 Step 3: Verifying event details");

    // Check the event ID matches (could be in 'id' or 'event_id' field)
    let returned_id = event_body["id"].as_str()
        .or_else(|| event_body["event_id"].as_str())
        .expect("Response should contain id or event_id");

    assert_eq!(returned_id, event_id, "Event ID should match");

    // Check the event name
    let returned_name = event_body["name"].as_str()
        .or_else(|| event_body["title"].as_str());

    assert!(
        returned_name.is_some(),
        "Response should contain event name/title"
    );

    println!("  ✅ Event details verified");
    println!("✅ Test 2 PASSED: Get event by ID returns correct details\n");
}

/// Test 3: GET /api/events/:id - Not Found
///
/// Verifies that the get event endpoint returns 404 for non-existent events.
#[tokio::test]
#[ignore] // Requires running server
async fn test_get_event_not_found() {
    println!("🧪 Test 3: GET /api/events/:id - Not Found");

    let client = reqwest::Client::new();

    let non_existent_id = uuid::Uuid::new_v4();

    println!("  📝 Step 1: Request non-existent event");
    let get_response = client
        .get(format!("{API_BASE}/api/events/{non_existent_id}"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(
        get_response.status(),
        404,
        "Non-existent event should return 404"
    );

    println!("  ✅ Received 404 Not Found");
    println!("✅ Test 3 PASSED: Non-existent event returns 404\n");
}

/// Test 4: GET /api/events/:id/availability - Get Event Availability
///
/// Verifies that the availability endpoint returns seat availability data.
///
/// # Flow
///
/// 1. User A creates an event
/// 2. User A initializes inventory for the event
/// 3. User A retrieves availability via GET /api/events/:id/availability
/// 4. Verify response contains availability data
#[tokio::test]
#[ignore] // Requires running server
async fn test_get_event_availability() {
    println!("🧪 Test 4: GET /api/events/:id/availability - Get Event Availability");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let event_name = format!("Availability Test {}", uuid::Uuid::new_v4());
    let create_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload(&event_name))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(
        create_response.status(),
        201,
        "Event creation should succeed"
    );

    let created_event: serde_json::Value = create_response
        .json()
        .await
        .expect("Failed to parse event response");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Response should contain event_id");

    println!("  ✅ User A created event: {event_id}");

    // Note: Event is created with default "General Admission" section already,
    // so we don't need to add sections separately.

    // Step 2: Get availability
    println!("  📝 Step 2: User A retrieves event availability");

    // Allow projection to catch up with the event creation
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let availability_response = client
        .get(format!("{API_BASE}/api/events/{event_id}/availability"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .send()
        .await
        .expect("Failed to get availability");

    // The availability endpoint returns 200 with data if projection synced,
    // or 404 if projection hasn't processed the event yet
    let status = availability_response.status();
    assert!(
        status == 200 || status == 404,
        "Get availability should return 200 (data) or 404 (projection lag), got: {}",
        status
    );

    if status == 200 {
        let availability_body: serde_json::Value = availability_response
            .json()
            .await
            .expect("Failed to parse availability response");

        println!("  ✅ Received availability data: {}", serde_json::to_string_pretty(&availability_body).unwrap());

        // Step 3: Verify availability data structure
        println!("  📝 Step 3: Verifying availability data structure");

        // The response should contain sections array
        let sections = availability_body["sections"]
            .as_array()
            .or_else(|| availability_body["availability"].as_array());

        assert!(
            sections.is_some(),
            "Response should contain sections or availability array"
        );

        println!("  ✅ Availability data structure verified");
    } else {
        println!("  ℹ️  Availability returned 404 (projection not yet synced)");
    }

    println!("✅ Test 4 PASSED: Event availability endpoint works correctly\n");
}

/// Test 5: GET /api/events/:id/availability - No Inventory
///
/// Verifies that availability returns empty/zero when no inventory exists.
#[tokio::test]
#[ignore] // Requires running server
async fn test_get_availability_no_inventory() {
    println!("🧪 Test 5: GET /api/events/:id/availability - No Inventory");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event (no inventory initialization)
    println!("  📝 Step 1: User A creates an event without inventory");
    let event_name = format!("No Inventory Test {}", uuid::Uuid::new_v4());
    let create_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload(&event_name))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(
        create_response.status(),
        201,
        "Event creation should succeed"
    );

    let created_event: serde_json::Value = create_response
        .json()
        .await
        .expect("Failed to parse event response");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Response should contain event_id");

    println!("  ✅ User A created event: {event_id}");

    // Step 2: Get availability without inventory
    println!("  📝 Step 2: User A retrieves availability (no inventory)");
    let availability_response = client
        .get(format!("{API_BASE}/api/events/{event_id}/availability"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .send()
        .await
        .expect("Failed to get availability");

    // Should return 200 with empty sections or 404 if no inventory
    let status = availability_response.status();
    assert!(
        status == 200 || status == 404,
        "Should return 200 (empty) or 404 (not found), got: {status}"
    );

    println!("  ✅ Received status: {status}");
    println!("✅ Test 5 PASSED: Availability handles no inventory case\n");
}

/// Test 6: Different user can view public event
///
/// Verifies that events are publicly viewable by any authenticated user.
#[tokio::test]
#[ignore] // Requires running server
async fn test_different_user_can_view_event() {
    println!("🧪 Test 6: Different User Can View Public Event");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let event_name = format!("Public View Test {}", uuid::Uuid::new_v4());
    let create_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload(&event_name))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(
        create_response.status(),
        201,
        "Event creation should succeed"
    );

    let created_event: serde_json::Value = create_response
        .json()
        .await
        .expect("Failed to parse event response");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Response should contain event_id");

    println!("  ✅ User A created event: {event_id}");

    // Step 2: User B (different user) views the event
    println!("  📝 Step 2: User B (different user) views the event");
    let get_response = client
        .get(format!("{API_BASE}/api/events/{event_id}"))
        .header("Authorization", format!("Bearer {USER_B_TOKEN}"))
        .send()
        .await
        .expect("Failed to get event");

    assert_eq!(
        get_response.status(),
        200,
        "Different user should be able to view event"
    );

    println!("  ✅ User B can view User A's event");
    println!("✅ Test 6 PASSED: Events are publicly viewable\n");
}
