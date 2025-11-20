//! Payment API End-to-End HTTP integration tests.
//!
//! These tests verify that the payment HTTP API endpoints work correctly
//! with real HTTP requests to a running server.
//!
//! Run with: `cargo test --test payment_api_e2e_test -- --ignored --nocapture`
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

/// Helper function to create a test event for payment tests
fn create_test_event_payload(name: &str) -> serde_json::Value {
    json!({
        "title": name,
        "description": format!("{} - Payment API test event", name),
        "start_time": "2025-12-31T20:00:00Z",
        "end_time": "2025-12-31T23:00:00Z",
        "venue_name": "Payment Test Arena",
        "venue_address": "456 Payment Street, API City, AP 54321"
    })
}

/// Test 1: GET /api/payments/{id} - Retrieve Payment Details
///
/// Verifies that the GET payment endpoint returns correct payment details.
///
/// # Flow
///
/// 1. User A creates an event
/// 2. User A creates a reservation
/// 3. User A processes payment for the reservation
/// 4. User A retrieves payment details via GET /api/payments/{id}
/// 5. Verify response contains correct payment data
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_get_payment_details() {
    println!("🧪 Test 1: GET /api/payments/{{id}} - Retrieve Payment Details");

    let client = reqwest::Client::new();

    // Step 1: User A creates an event
    println!("  📝 Step 1: User A creates an event");
    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("Payment GET Test Event"))
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

    // Wait for event initialization
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Step 2: User A creates a reservation
    println!("  📝 Step 2: User A creates a reservation");

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

    assert_eq!(
        create_reservation_response.status(),
        201,
        "Reservation creation should succeed"
    );

    let created_reservation: serde_json::Value = create_reservation_response
        .json()
        .await
        .expect("Failed to parse reservation response");

    let reservation_id = created_reservation["reservation_id"]
        .as_str()
        .expect("Response should contain reservation_id");

    println!("  ✅ User A created reservation: {reservation_id}");

    // Step 3: User A processes payment
    println!("  📝 Step 3: User A processes payment");

    let payment_request = json!({
        "reservation_id": reservation_id,
        "payment_method": {
            "type": "credit_card",
            "last_four": "4242"
        },
        "amount": 200.00
    });

    let process_payment_response = client
        .post(format!("{API_BASE}/api/payments/process"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&payment_request)
        .send()
        .await
        .expect("Failed to process payment");

    assert_eq!(
        process_payment_response.status(),
        200,
        "Payment processing should succeed"
    );

    let payment_result: serde_json::Value = process_payment_response
        .json()
        .await
        .expect("Failed to parse payment response");

    let payment_id = payment_result["payment_id"]
        .as_str()
        .expect("Response should contain payment_id");

    println!("  ✅ User A processed payment: {payment_id}");

    // Wait for payment projection to update
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Step 4: User A retrieves payment details
    println!("  📝 Step 4: User A retrieves payment details");

    let get_payment_response = client
        .get(format!("{API_BASE}/api/payments/{payment_id}"))
        .send()
        .await
        .expect("Failed to get payment");

    assert_eq!(
        get_payment_response.status(),
        200,
        "GET payment should succeed"
    );

    let payment_details: serde_json::Value = get_payment_response
        .json()
        .await
        .expect("Failed to parse payment details");

    // Verify payment details
    assert_eq!(
        payment_details["id"].as_str().unwrap(),
        payment_id,
        "Payment ID should match"
    );
    assert_eq!(
        payment_details["reservation_id"].as_str().unwrap(),
        reservation_id,
        "Reservation ID should match"
    );
    assert_eq!(
        payment_details["amount"].as_f64().unwrap(),
        200.0,
        "Payment amount should match"
    );
    assert!(
        payment_details["payment_method"]
            .as_str()
            .unwrap()
            .contains("4242"),
        "Payment method should contain last four digits"
    );

    println!("  ✅ Payment details retrieved successfully");
}

/// Test 2: GET /api/payments/my-payments - List User's Payments
///
/// Verifies that the list payments endpoint returns all payments for a user.
///
/// # Flow
///
/// 1. User A creates multiple events and reservations
/// 2. User A processes payments for each reservation
/// 3. User A retrieves their payment list
/// 4. Verify all payments are returned
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_list_user_payments() {
    println!("🧪 Test 2: GET /api/payments/my-payments - List User's Payments");

    let client = reqwest::Client::new();

    // We'll create 2 events and payments to test list functionality
    let mut payment_ids = Vec::new();

    for i in 1..=2 {
        // Create event
        println!("  📝 Creating event {i}");
        let create_event_response = client
            .post(format!("{API_BASE}/api/events"))
            .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
            .json(&create_test_event_payload(&format!("Payment List Test Event {i}")))
            .send()
            .await
            .expect("Failed to create event");

        assert_eq!(create_event_response.status(), 201);

        let created_event: serde_json::Value = create_event_response.json().await.unwrap();
        let event_id = created_event["event_id"].as_str().unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Create reservation
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
            .unwrap();

        assert_eq!(create_reservation_response.status(), 201);

        let created_reservation: serde_json::Value = create_reservation_response.json().await.unwrap();
        let reservation_id = created_reservation["reservation_id"].as_str().unwrap();

        // Process payment
        let payment_request = json!({
            "reservation_id": reservation_id,
            "payment_method": {
                "type": "credit_card",
                "last_four": "4242"
            },
            "amount": 100.00
        });

        let process_payment_response = client
            .post(format!("{API_BASE}/api/payments/process"))
            .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
            .json(&payment_request)
            .send()
            .await
            .unwrap();

        assert_eq!(process_payment_response.status(), 200);

        let payment_result: serde_json::Value = process_payment_response.json().await.unwrap();
        let payment_id = payment_result["payment_id"].as_str().unwrap().to_string();
        payment_ids.push(payment_id);

        println!("  ✅ Created payment {i}");
    }

    // Wait for projections to update
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // List User A's payments
    println!("  📝 Retrieving User A's payment list");

    let list_payments_response = client
        .get(format!("{API_BASE}/api/payments/my-payments"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .send()
        .await
        .expect("Failed to list payments");

    assert_eq!(
        list_payments_response.status(),
        200,
        "List payments should succeed"
    );

    let payment_list: serde_json::Value = list_payments_response
        .json()
        .await
        .expect("Failed to parse payment list");

    // Verify response structure
    assert!(
        payment_list["payments"].is_array(),
        "Response should contain payments array"
    );
    assert!(
        payment_list["total"].is_number(),
        "Response should contain total count"
    );

    let payments_array = payment_list["payments"].as_array().unwrap();

    // Should have at least the 2 payments we just created
    assert!(
        payments_array.len() >= 2,
        "Should have at least 2 payments, got {}",
        payments_array.len()
    );

    println!("  ✅ User A has {} payment(s)", payments_array.len());
}

/// Test 3: POST /api/payments/{id}/refund - Refund Payment
///
/// Verifies that the refund endpoint successfully processes refunds.
///
/// # Flow
///
/// 1. User A creates event, reservation, and payment
/// 2. User A refunds the payment
/// 3. Verify refund succeeds and status is updated
/// 4. Verify cannot refund the same payment twice (idempotency)
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_refund_payment() {
    println!("🧪 Test 3: POST /api/payments/{{id}}/refund - Refund Payment");

    let client = reqwest::Client::new();

    // Step 1: Create event, reservation, and payment (same setup as Test 1)
    println!("  📝 Step 1: Setting up event, reservation, and payment");

    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("Payment Refund Test Event"))
        .send()
        .await
        .unwrap();

    let created_event: serde_json::Value = create_event_response.json().await.unwrap();
    let event_id = created_event["event_id"].as_str().unwrap();

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
        .unwrap();

    let created_reservation: serde_json::Value = create_reservation_response.json().await.unwrap();
    let reservation_id = created_reservation["reservation_id"].as_str().unwrap();

    let payment_request = json!({
        "reservation_id": reservation_id,
        "payment_method": {
            "type": "credit_card",
            "last_four": "4242"
        },
        "amount": 200.00
    });

    let process_payment_response = client
        .post(format!("{API_BASE}/api/payments/process"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&payment_request)
        .send()
        .await
        .unwrap();

    let payment_result: serde_json::Value = process_payment_response.json().await.unwrap();
    let payment_id = payment_result["payment_id"].as_str().unwrap();

    println!("  ✅ Setup complete, payment ID: {payment_id}");

    // Wait for payment to be processed
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Step 2: User A refunds the payment
    println!("  📝 Step 2: User A refunds the payment");

    let refund_request = json!({
        "amount": null, // Full refund
        "reason": "Customer requested refund"
    });

    let refund_response = client
        .post(format!("{API_BASE}/api/payments/{payment_id}/refund"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&refund_request)
        .send()
        .await
        .expect("Failed to refund payment");

    assert_eq!(
        refund_response.status(),
        200,
        "Refund should succeed"
    );

    let refund_result: serde_json::Value = refund_response
        .json()
        .await
        .expect("Failed to parse refund response");

    assert_eq!(
        refund_result["payment_id"].as_str().unwrap(),
        payment_id,
        "Payment ID should match"
    );
    assert_eq!(
        refund_result["refund_amount"].as_f64().unwrap(),
        200.0,
        "Refund amount should match original payment"
    );
    assert!(
        refund_result["message"]
            .as_str()
            .unwrap()
            .contains("successfully"),
        "Should have success message"
    );

    println!("  ✅ Payment refunded successfully");

    // Step 3: Attempt to refund again (should fail - idempotency)
    println!("  📝 Step 3: Attempt to refund again (idempotency check)");

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let second_refund_response = client
        .post(format!("{API_BASE}/api/payments/{payment_id}/refund"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&refund_request)
        .send()
        .await
        .expect("Failed to send second refund request");

    // Should get 400 Bad Request (cannot refund already refunded payment)
    assert_eq!(
        second_refund_response.status(),
        400,
        "Second refund should fail with 400 Bad Request"
    );

    println!("  ✅ Second refund correctly rejected (idempotency enforced)");
}

/// Test 4: POST /api/payments/{id}/refund - Ownership Enforcement
///
/// Verifies that RequireOwnership middleware prevents User B from
/// refunding User A's payment.
///
/// # Flow
///
/// 1. User A creates payment
/// 2. User B attempts to refund User A's payment
/// 3. Verify 403 Forbidden response
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_refund_payment_ownership_enforcement() {
    println!("🧪 Test 4: Refund Payment - Ownership Enforcement");

    let client = reqwest::Client::new();

    // Step 1: User A creates payment
    println!("  📝 Step 1: User A creates payment");

    let create_event_response = client
        .post(format!("{API_BASE}/api/events"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&create_test_event_payload("Refund Ownership Test Event"))
        .send()
        .await
        .unwrap();

    let created_event: serde_json::Value = create_event_response.json().await.unwrap();
    let event_id = created_event["event_id"].as_str().unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

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
        .unwrap();

    let created_reservation: serde_json::Value = create_reservation_response.json().await.unwrap();
    let reservation_id = created_reservation["reservation_id"].as_str().unwrap();

    let payment_request = json!({
        "reservation_id": reservation_id,
        "payment_method": {
            "type": "credit_card",
            "last_four": "4242"
        },
        "amount": 100.00
    });

    let process_payment_response = client
        .post(format!("{API_BASE}/api/payments/process"))
        .header("Authorization", format!("Bearer {USER_A_TOKEN}"))
        .json(&payment_request)
        .send()
        .await
        .unwrap();

    let payment_result: serde_json::Value = process_payment_response.json().await.unwrap();
    let payment_id = payment_result["payment_id"].as_str().unwrap();

    println!("  ✅ User A created payment: {payment_id}");

    // Wait for ownership index to update
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Step 2: User B attempts to refund User A's payment
    println!("  📝 Step 2: User B attempts to refund User A's payment");

    let refund_request = json!({
        "amount": null,
        "reason": "Unauthorized refund attempt"
    });

    let refund_response = client
        .post(format!("{API_BASE}/api/payments/{payment_id}/refund"))
        .header("Authorization", format!("Bearer {USER_B_TOKEN}"))
        .json(&refund_request)
        .send()
        .await
        .expect("Failed to send refund request");

    // Should get 403 Forbidden (ownership violation)
    assert_eq!(
        refund_response.status(),
        403,
        "User B should not be able to refund User A's payment"
    );

    println!("  ✅ Ownership enforcement successful - 403 Forbidden");
}

/// Test 5: GET /api/payments/{id} - Not Found
///
/// Verifies that requesting a non-existent payment returns 404.
#[tokio::test]
#[ignore] // Requires running server with AUTH_TEST_TOKEN set
async fn test_get_nonexistent_payment() {
    println!("🧪 Test 5: GET /api/payments/{{id}} - Not Found");

    let client = reqwest::Client::new();

    let fake_payment_id = "00000000-0000-0000-0000-000000000999";

    let get_payment_response = client
        .get(format!("{API_BASE}/api/payments/{fake_payment_id}"))
        .send()
        .await
        .expect("Failed to send GET request");

    assert_eq!(
        get_payment_response.status(),
        404,
        "Should return 404 Not Found for non-existent payment"
    );

    println!("  ✅ Non-existent payment correctly returns 404");
}
