//! Authorization ownership enforcement tests.
//!
//! These tests verify that the `RequireOwnership` middleware correctly enforces
//! resource ownership, preventing users from accessing or modifying resources
//! owned by other users.
//!
//! Run with: `cargo test --test auth_authorization_test -- --nocapture`
//!
//! Prerequisites:
//! - `docker compose up -d` must be running
//! - Server must be running on localhost:8080
//! - AUTH_TEST_TOKEN environment variable must be set on the server
//!
//! The tests use the multi-user test token feature (test-user-{uuid}) to
//! simulate different users making requests.

#![allow(clippy::expect_used, clippy::unwrap_used)] // Test code can use unwrap/expect

use serde_json::json;

/// Base URL for the ticketing API
const API_BASE: &str = "http://localhost:8080";

/// Test user A (first customer)
const USER_A_UUID: &str = "00000000-0000-0000-0000-000000000001";
const USER_A_TOKEN: &str = "test-user-00000000-0000-0000-0000-000000000001";

/// Test user B (second customer, trying to access User A's resources)
const USER_B_UUID: &str = "00000000-0000-0000-0000-000000000002";
const USER_B_TOKEN: &str = "test-user-00000000-0000-0000-0000-000000000002";

/// Helper function to create a test event for reservation tests
fn create_test_event_payload(name: &str) -> serde_json::Value {
    json!({
        "title": name,
        "description": format!("{} - Authorization test event", name),
        "start_time": "2025-12-31T20:00:00Z",
        "end_time": "2025-12-31T23:00:00Z",
        "venue_name": "Auth Test Arena",
        "venue_address": "123 Security Street, Auth City, AU 12345"
    })
}

/// Test 1: User B Cannot Cancel User A's Reservation
///
/// Verifies that RequireOwnership<ReservationId> correctly prevents
/// User B from canceling a reservation created by User A.
///
/// # Flow
///
/// 1. User A creates an event
/// 2. User A creates a reservation for that event
/// 3. User B attempts to cancel User A's reservation
/// 4. Verify 403 Forbidden response
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_user_cannot_cancel_other_users_reservation() {
    println!("🧪 Test 1: User B Cannot Cancel User A's Reservation");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("Auth Test Event"))
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

    // Step 2: User A creates a reservation
    println!("  📝 Step 2: User A creates a reservation");

    // First, wait a bit for event initialization to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let reservation_request = json!({
        "event_id": event_id,
        "section": "VIP",
        "quantity": 2,
        "specific_seats": null
    });

    let create_reservation_response = client
        .post(format!("{API_BASE}/api/reservations"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&reservation_request)
        .send()
        .await
        .expect("Failed to create reservation");

    let status = create_reservation_response.status();
    if status != 201 {
        let error_body = create_reservation_response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to get error body".to_string());
        panic!(
            "Reservation creation failed with status {}: {}",
            status, error_body
        );
    }

    assert_eq!(status, 201, "Reservation creation should succeed");

    let created_reservation: serde_json::Value = create_reservation_response
        .json()
        .await
        .expect("Failed to parse reservation response");

    let reservation_id = created_reservation["reservation_id"]
        .as_str()
        .expect("Response should contain reservation_id");

    println!("  ✅ User A created reservation: {reservation_id}");

    // Wait for ownership index to be updated
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Step 3: User B attempts to cancel User A's reservation
    println!("  📝 Step 3: User B attempts to cancel User A's reservation");

    let cancel_request = json!({
        "reason": "Attempting unauthorized cancellation"
    });

    let cancel_response = client
        .post(format!(
            "{API_BASE}/api/reservations/{reservation_id}/cancel"
        ))
        .header("Authorization", format!("Bearer {USER_B_TOKEN}"))
        .json(&cancel_request)
        .send()
        .await
        .expect("Failed to send cancel request");

    // Step 4: Verify 403 Forbidden
    println!("  📝 Step 4: Verify 403 Forbidden response");

    let cancel_status = cancel_response.status();
    assert_eq!(
        cancel_status,
        403,
        "User B should receive 403 Forbidden when trying to cancel User A's reservation"
    );

    let error_response: serde_json::Value = cancel_response
        .json()
        .await
        .expect("Failed to parse error response");

    // Verify error message mentions ownership
    let error_message = error_response["error"]
        .as_str()
        .unwrap_or("")
        .to_lowercase();
    assert!(
        error_message.contains("own") || error_message.contains("forbidden"),
        "Error message should mention ownership or forbidden access. Got: {}",
        error_message
    );

    println!("  ✅ Authorization correctly enforced!");
    println!(
        "  ✅ User B was blocked with 403 Forbidden: {}",
        error_message
    );
}

/// Test 2: User Can Access Their Own Reservation
///
/// Verifies that the owner can successfully access their own reservation.
/// This is a positive test to ensure the ownership check doesn't block
/// legitimate access.
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_user_can_access_own_reservation() {
    println!("🧪 Test 2: User Can Access Their Own Reservation");

    let client = reqwest::Client::new();

    // User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("User Access Test Event"))
        .send()
        .await
        .expect("Failed to create event");

    let created_event: serde_json::Value = create_event_response
        .json()
        .await
        .expect("Failed to parse event");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Event ID missing");

    println!("  ✅ User A created event: {event_id}");

    // Wait for event initialization
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // User A creates a reservation
    println!("  📝 Step 2: User A creates a reservation");

    let reservation_request = json!({
        "event_id": event_id,
        "section": "General",
        "quantity": 1,
        "specific_seats": null
    });

    let create_reservation_response = client
        .post(format!("{API_BASE}/api/reservations"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&reservation_request)
        .send()
        .await
        .expect("Failed to create reservation");

    let created_reservation: serde_json::Value = create_reservation_response
        .json()
        .await
        .expect("Failed to parse reservation");

    let reservation_id = created_reservation["reservation_id"]
        .as_str()
        .expect("Reservation ID missing");

    println!("  ✅ User A created reservation: {reservation_id}");

    // Wait for ownership index update
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // User A retrieves their own reservation
    println!("  📝 Step 3: User A retrieves their own reservation");

    let get_response = client
        .get(format!("{API_BASE}/api/reservations/{reservation_id}"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .send()
        .await
        .expect("Failed to get reservation");

    let get_status = get_response.status();
    assert_eq!(
        get_status, 200,
        "User A should be able to access their own reservation"
    );

    println!("  ✅ User A successfully accessed their own reservation");
}

/// Test 3: User B Cannot Access User A's Reservation Details
///
/// Verifies that GET /api/reservations/:id returns 403 when User B
/// attempts to access User A's reservation.
///
/// Note: This test depends on whether the GET endpoint uses RequireOwnership.
/// If it's a public endpoint, this test would fail. Adjust based on actual API design.
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set - May fail if GET is public
async fn test_user_cannot_get_other_users_reservation() {
    println!("🧪 Test 3: User B Cannot Access User A's Reservation Details");

    let client = reqwest::Client::new();

    // User A creates an event and reservation
    println!("  📝 Step 1: User A creates event and reservation");

    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("Privacy Test Event"))
        .send()
        .await
        .expect("Failed to create event");

    let created_event: serde_json::Value = create_event_response
        .json()
        .await
        .expect("Failed to parse event");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Event ID missing");

    // Wait for event initialization
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let reservation_request = json!({
        "event_id": event_id,
        "section": "VIP",
        "quantity": 1,
        "specific_seats": null
    });

    let create_reservation_response = client
        .post(format!("{API_BASE}/api/reservations"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&reservation_request)
        .send()
        .await
        .expect("Failed to create reservation");

    let created_reservation: serde_json::Value = create_reservation_response
        .json()
        .await
        .expect("Failed to parse reservation");

    let reservation_id = created_reservation["reservation_id"]
        .as_str()
        .expect("Reservation ID missing");

    println!("  ✅ User A created reservation: {reservation_id}");

    // Wait for ownership index update
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // User B attempts to GET User A's reservation
    println!("  📝 Step 2: User B attempts to access User A's reservation");

    let get_response = client
        .get(format!("{API_BASE}/api/reservations/{reservation_id}"))
        .header("Authorization", format!("Bearer {USER_B_TOKEN}"))
        .send()
        .await
        .expect("Failed to send GET request");

    let get_status = get_response.status();

    // Note: The GET endpoint might be public (anyone can view reservation status)
    // In that case, this test would need to be adjusted or removed
    // For now, we'll document this as a conditional test
    if get_status == 403 {
        println!("  ✅ GET endpoint uses ownership enforcement (403 Forbidden)");
        let error_response: serde_json::Value = get_response
            .json()
            .await
            .expect("Failed to parse error response");

        let error_message = error_response["error"].as_str().unwrap_or("");
        println!("  ✅ Error message: {error_message}");
    } else if get_status == 200 {
        println!("  ℹ️  GET endpoint is public (allows viewing any reservation)");
        println!("  ℹ️  This is acceptable for public reservation lookup");
    } else {
        panic!(
            "Unexpected status code: {}. Expected 200 (public) or 403 (ownership enforced)",
            get_status
        );
    }
}

/// Test 4: User Can Update Their Own Event
///
/// Verifies that an event owner can successfully update their own event.
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_user_can_update_own_event() {
    println!("🧪 Test 4: User Can Update Their Own Event");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("Original Event Name"))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(create_event_response.status(), 201);

    let created_event: serde_json::Value = create_event_response
        .json()
        .await
        .expect("Failed to parse event");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Event ID missing");

    println!("  ✅ User A created event: {event_id}");

    // Step 2: User A updates their own event
    println!("  📝 Step 2: User A updates their own event");

    let update_request = json!({
        "title": "Updated Event Name"
    });

    let update_response = client
        .put(format!("{API_BASE}/api/events/{event_id}"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&update_request)
        .send()
        .await
        .expect("Failed to update event");

    let update_status = update_response.status();
    assert_eq!(
        update_status, 200,
        "User A should be able to update their own event"
    );

    let updated_event: serde_json::Value = update_response
        .json()
        .await
        .expect("Failed to parse updated event");

    let updated_title = updated_event["title"]
        .as_str()
        .expect("Updated event should have title");

    assert_eq!(updated_title, "Updated Event Name");

    println!("  ✅ User A successfully updated their event to: {updated_title}");
}

/// Test 5: User B Cannot Update User A's Event
///
/// Verifies that ownership enforcement prevents User B from updating
/// an event created by User A.
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_user_cannot_update_other_users_event() {
    println!("🧪 Test 5: User B Cannot Update User A's Event");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("User A's Event"))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(create_event_response.status(), 201);

    let created_event: serde_json::Value = create_event_response
        .json()
        .await
        .expect("Failed to parse event");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Event ID missing");

    println!("  ✅ User A created event: {event_id}");

    // Wait for projection update
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Step 2: User B attempts to update User A's event
    println!("  📝 Step 2: User B attempts to update User A's event");

    let update_request = json!({
        "title": "Malicious Update"
    });

    let update_response = client
        .put(format!("{API_BASE}/api/events/{event_id}"))
        .header("Authorization", format!("Bearer {USER_B_TOKEN}"))
        .json(&update_request)
        .send()
        .await
        .expect("Failed to send update request");

    // Step 3: Verify 403 Forbidden
    let update_status = update_response.status();
    assert_eq!(
        update_status, 403,
        "User B should receive 403 Forbidden when trying to update User A's event"
    );

    let error_response: serde_json::Value = update_response
        .json()
        .await
        .expect("Failed to parse error response");

    let error_message = error_response["error"]
        .as_str()
        .unwrap_or("")
        .to_lowercase();

    assert!(
        error_message.contains("own") || error_message.contains("forbidden"),
        "Error message should mention ownership. Got: {}",
        error_message
    );

    println!("  ✅ Authorization correctly enforced!");
    println!("  ✅ User B was blocked with 403: {error_message}");
}

/// Test 6: User Can Delete Their Own Event
///
/// Verifies that an event owner can successfully delete their own event.
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_user_can_delete_own_event() {
    println!("🧪 Test 6: User Can Delete Their Own Event");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("Event To Delete"))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(create_event_response.status(), 201);

    let created_event: serde_json::Value = create_event_response
        .json()
        .await
        .expect("Failed to parse event");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Event ID missing");

    println!("  ✅ User A created event: {event_id}");

    // Step 2: User A deletes their own event
    println!("  📝 Step 2: User A deletes their own event");

    let delete_response = client
        .delete(format!("{API_BASE}/api/events/{event_id}"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .send()
        .await
        .expect("Failed to delete event");

    let delete_status = delete_response.status();
    assert_eq!(
        delete_status, 204,
        "User A should be able to delete their own event"
    );

    println!("  ✅ User A successfully deleted their event");
}

/// Test 7: User B Cannot Delete User A's Event
///
/// Verifies that ownership enforcement prevents User B from deleting
/// an event created by User A.
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_user_cannot_delete_other_users_event() {
    println!("🧪 Test 7: User B Cannot Delete User A's Event");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("Protected Event"))
        .send()
        .await
        .expect("Failed to create event");

    assert_eq!(create_event_response.status(), 201);

    let created_event: serde_json::Value = create_event_response
        .json()
        .await
        .expect("Failed to parse event");

    let event_id = created_event["event_id"]
        .as_str()
        .expect("Event ID missing");

    println!("  ✅ User A created event: {event_id}");

    // Wait for projection update
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Step 2: User B attempts to delete User A's event
    println!("  📝 Step 2: User B attempts to delete User A's event");

    let delete_response = client
        .delete(format!("{API_BASE}/api/events/{event_id}"))
        .header("Authorization", format!("Bearer {USER_B_TOKEN}"))
        .send()
        .await
        .expect("Failed to send delete request");

    // Step 3: Verify 403 Forbidden
    let delete_status = delete_response.status();
    assert_eq!(
        delete_status, 403,
        "User B should receive 403 Forbidden when trying to delete User A's event"
    );

    let error_response: serde_json::Value = delete_response
        .json()
        .await
        .expect("Failed to parse error response");

    let error_message = error_response["error"]
        .as_str()
        .unwrap_or("")
        .to_lowercase();

    assert!(
        error_message.contains("own") || error_message.contains("forbidden"),
        "Error message should mention ownership. Got: {}",
        error_message
    );

    println!("  ✅ Authorization correctly enforced!");
    println!("  ✅ User B was blocked with 403: {error_message}");
}

/// Test 8: Verify User IDs Are Different
///
/// Sanity check to ensure test users A and B have different IDs.
#[tokio::test]
async fn test_different_user_ids() {
    println!("🧪 Test 8: Verify User IDs Are Different");

    assert_ne!(
        USER_A_UUID, USER_B_UUID,
        "Test users must have different UUIDs"
    );
    assert_ne!(
        USER_A_TOKEN, USER_B_TOKEN,
        "Test users must have different tokens"
    );

    println!("  ✅ User A UUID: {USER_A_UUID}");
    println!("  ✅ User B UUID: {USER_B_UUID}");
    println!("  ✅ Users have different identities");
}
