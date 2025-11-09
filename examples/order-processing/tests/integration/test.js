#!/usr/bin/env node

/**
 * Integration tests for Order Processing HTTP API
 *
 * Tests the full order lifecycle:
 * 1. Place order
 * 2. Get order status
 * 3. Ship order
 * 4. Cancel order (should fail - already shipped)
 *
 * Prerequisites:
 * - Server must be running on http://localhost:3000
 * - Run with: cargo run --bin order-processing --features http
 */

const API_BASE = 'http://localhost:3000/api/v1';
const HEALTH_URL = 'http://localhost:3000/health';

// ANSI color codes for output
const colors = {
  reset: '\x1b[0m',
  green: '\x1b[32m',
  red: '\x1b[31m',
  yellow: '\x1b[33m',
  cyan: '\x1b[36m',
};

function log(message, color = 'reset') {
  console.log(`${colors[color]}${message}${colors.reset}`);
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(`Assertion failed: ${message}`);
  }
}

async function request(method, path, body = null) {
  const url = path.startsWith('http') ? path : `${API_BASE}${path}`;
  const options = {
    method,
    headers: {
      'Content-Type': 'application/json',
      'X-Correlation-ID': crypto.randomUUID(),
    },
  };

  if (body) {
    options.body = JSON.stringify(body);
  }

  log(`\n→ ${method} ${path}`, 'cyan');
  if (body) {
    log(`  Body: ${JSON.stringify(body, null, 2)}`, 'cyan');
  }

  const response = await fetch(url, options);
  const text = await response.text();
  let data = null;

  try {
    data = text ? JSON.parse(text) : null;
  } catch (e) {
    data = text;
  }

  log(`← ${response.status} ${response.statusText}`, response.ok ? 'green' : 'red');
  if (data) {
    log(`  Response: ${JSON.stringify(data, null, 2)}`, 'green');
  }

  return { response, data };
}

async function testHealthCheck() {
  log('\n=== Test: Health Check ===', 'yellow');

  const { response, data } = await request('GET', HEALTH_URL);

  assert(response.ok, 'Health check should return 200');
  assert(data === 'ok', 'Health check should return "ok"');

  log('✓ Health check passed', 'green');
}

async function testPlaceOrder() {
  log('\n=== Test: Place Order ===', 'yellow');

  const orderRequest = {
    customer_id: 'cust-test-123',
    items: [
      {
        product_id: 'prod-widget-a',
        name: 'Premium Widget A',
        quantity: 2,
        unit_price_cents: 2500,
      },
      {
        product_id: 'prod-widget-b',
        name: 'Deluxe Widget B',
        quantity: 1,
        unit_price_cents: 5000,
      },
    ],
  };

  const { response, data } = await request('POST', '/orders', orderRequest);

  assert(response.status === 201, 'Should return 201 Created');
  assert(data.order_id, 'Response should include order_id');
  assert(data.status === 'Placed', 'Order status should be Placed');
  assert(data.total_cents === 10000, 'Total should be 10000 cents ($100)');
  assert(data.placed_at, 'Response should include placed_at timestamp');

  log('✓ Order placed successfully', 'green');

  return data.order_id;
}

async function testGetOrder(orderId) {
  log('\n=== Test: Get Order ===', 'yellow');

  const { response, data } = await request('GET', `/orders/${orderId}`);

  assert(response.ok, 'Should return 200 OK');
  assert(data.order_id === orderId, 'Order ID should match');
  assert(data.customer_id === 'cust-test-123', 'Customer ID should match');
  assert(data.status === 'Placed', 'Status should be Placed');
  assert(data.items.length === 2, 'Should have 2 items');
  assert(data.total_cents === 10000, 'Total should be 10000 cents');

  log('✓ Order retrieved successfully', 'green');
}

async function testGetNonexistentOrder() {
  log('\n=== Test: Get Nonexistent Order ===', 'yellow');

  const { response, data } = await request('GET', '/orders/order-nonexistent');

  assert(response.status === 404, 'Should return 404 Not Found');
  assert(data.message, 'Response should include error message');

  log('✓ Nonexistent order handled correctly', 'green');
}

async function testShipOrder(orderId) {
  log('\n=== Test: Ship Order ===', 'yellow');

  const shipRequest = {
    tracking: 'TRACK-ABC123XYZ',
  };

  const { response, data } = await request('POST', `/orders/${orderId}/ship`, shipRequest);

  assert(response.ok, 'Should return 200 OK');
  assert(data.order_id === orderId, 'Order ID should match');
  assert(data.status === 'Shipped', 'Status should be Shipped');
  assert(data.tracking === 'TRACK-ABC123XYZ', 'Tracking number should match');
  assert(data.shipped_at, 'Response should include shipped_at timestamp');

  log('✓ Order shipped successfully', 'green');
}

async function testCancelShippedOrder(orderId) {
  log('\n=== Test: Cancel Shipped Order (Should Fail) ===', 'yellow');

  const cancelRequest = {
    reason: 'Customer changed mind',
  };

  const { response, data } = await request('POST', `/orders/${orderId}/cancel`, cancelRequest);

  assert(response.status === 400, 'Should return 400 Bad Request');
  assert(data.message, 'Response should include error message');
  assert(data.message.includes('Shipped'), 'Error should mention shipped status');

  log('✓ Cancellation validation working correctly', 'green');
}

async function testCancelBeforeShipping() {
  log('\n=== Test: Cancel Order Before Shipping ===', 'yellow');

  // Place a new order
  const orderRequest = {
    customer_id: 'cust-cancel-test',
    items: [
      {
        product_id: 'prod-test',
        name: 'Test Product',
        quantity: 1,
        unit_price_cents: 1000,
      },
    ],
  };

  const { data: placeData } = await request('POST', '/orders', orderRequest);
  const orderId = placeData.order_id;

  log(`  Created order: ${orderId}`, 'cyan');

  // Cancel it
  const cancelRequest = {
    reason: 'Test cancellation',
  };

  const { response, data } = await request('POST', `/orders/${orderId}/cancel`, cancelRequest);

  assert(response.ok, 'Should return 200 OK');
  assert(data.order_id === orderId, 'Order ID should match');
  assert(data.status === 'Cancelled', 'Status should be Cancelled');
  assert(data.reason === 'Test cancellation', 'Reason should match');
  assert(data.cancelled_at, 'Response should include cancelled_at timestamp');

  log('✓ Order cancelled successfully', 'green');
}

async function testShipCancelledOrder() {
  log('\n=== Test: Ship Cancelled Order (Should Fail) ===', 'yellow');

  // Place and immediately cancel an order
  const orderRequest = {
    customer_id: 'cust-ship-cancel-test',
    items: [
      {
        product_id: 'prod-test',
        name: 'Test Product',
        quantity: 1,
        unit_price_cents: 1000,
      },
    ],
  };

  const { data: placeData } = await request('POST', '/orders', orderRequest);
  const orderId = placeData.order_id;

  await request('POST', `/orders/${orderId}/cancel`, { reason: 'Test' });

  // Try to ship the cancelled order
  const shipRequest = {
    tracking: 'TRACK-SHOULD-FAIL',
  };

  const { response, data } = await request('POST', `/orders/${orderId}/ship`, shipRequest);

  assert(response.status === 400, 'Should return 400 Bad Request');
  assert(data.message, 'Response should include error message');

  log('✓ Shipping validation working correctly', 'green');
}

async function runTests() {
  log('\n╔════════════════════════════════════════════════╗', 'cyan');
  log('║   Order Processing Integration Tests          ║', 'cyan');
  log('╚════════════════════════════════════════════════╝', 'cyan');

  try {
    // Check if server is running
    try {
      await fetch(HEALTH_URL);
    } catch (error) {
      throw new Error(
        'Server is not running. Start it with:\n' +
        '  cargo run --bin order-processing --features http'
      );
    }

    await testHealthCheck();

    // Main test flow: Place → Get → Ship → Try to Cancel
    const orderId = await testPlaceOrder();
    await testGetOrder(orderId);
    await testGetNonexistentOrder();
    await testShipOrder(orderId);
    await testCancelShippedOrder(orderId);

    // Additional validation tests
    await testCancelBeforeShipping();
    await testShipCancelledOrder();

    log('\n╔════════════════════════════════════════════════╗', 'green');
    log('║   ✓ All tests passed!                         ║', 'green');
    log('╚════════════════════════════════════════════════╝', 'green');

    process.exit(0);
  } catch (error) {
    log('\n╔════════════════════════════════════════════════╗', 'red');
    log('║   ✗ Test failed!                              ║', 'red');
    log('╚════════════════════════════════════════════════╝', 'red');
    log(`\nError: ${error.message}`, 'red');
    console.error(error);
    process.exit(1);
  }
}

// Run tests
runTests();
