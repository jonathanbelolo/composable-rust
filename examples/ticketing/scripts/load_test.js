/**
 * K6 Load Test for Ticketing System
 *
 * This script performs realistic load testing of the ticketing API:
 * - 100 concurrent users at peak
 * - Target: 10 reservations/second sustained
 * - Duration: 5 minutes total
 * - Thresholds: p95 < 500ms, error rate < 1%
 *
 * Prerequisites:
 * - Server running on localhost:8080
 * - AUTH_TEST_TOKEN environment variable set on server
 * - Test event pre-created (or use dynamic event creation)
 *
 * Run with:
 *   k6 run scripts/load_test.js
 *
 * Run with detailed output:
 *   k6 run --out json=results.json scripts/load_test.js
 *
 * Analyze results:
 *   k6 run --summary-export=summary.json scripts/load_test.js
 */

import http from 'k6/http';
import { check, sleep, group } from 'k6';
import { Rate, Trend, Counter } from 'k6/metrics';

// ============================================================================
// Configuration
// ============================================================================

const BASE_URL = 'http://localhost:8080';

// Test users (using multi-user test token feature)
// Each virtual user gets a unique user ID
const getUserToken = (userId) => `test-user-00000000-0000-0000-0000-00000000${userId.toString().padStart(4, '0')}`;

// Pre-created test event ID (you can create one via API before load test)
// Or dynamically create events (commented out below)
const TEST_EVENT_ID = __ENV.TEST_EVENT_ID || null;

// ============================================================================
// Test Configuration
// ============================================================================

export let options = {
  // Load profile: Ramp up to 100 users over 5 minutes
  stages: [
    { duration: '1m', target: 50 },   // Warm up: 0 → 50 users
    { duration: '3m', target: 100 },  // Peak load: 50 → 100 users
    { duration: '1m', target: 0 },    // Cool down: 100 → 0 users
  ],

  // Performance thresholds (SLAs)
  thresholds: {
    // Response time SLA
    'http_req_duration': [
      'p(95)<500',  // 95th percentile under 500ms
      'p(99)<1000', // 99th percentile under 1 second
      'avg<300',    // Average under 300ms
    ],

    // Error rate SLA
    'http_req_failed': [
      'rate<0.01',  // Less than 1% errors
    ],

    // Custom metrics thresholds
    'reservation_success_rate': [
      'rate>0.99',  // More than 99% success rate
    ],
    'reservation_duration': [
      'p(95)<600',  // Reservation flow under 600ms
    ],
  },

  // Other options
  noConnectionReuse: false, // Reuse connections for efficiency
  userAgent: 'K6LoadTest/1.0',
};

// ============================================================================
// Custom Metrics
// ============================================================================

let reservationSuccessRate = new Rate('reservation_success_rate');
let reservationDuration = new Trend('reservation_duration');
let reservationFailures = new Counter('reservation_failures');
let insufficientInventoryErrors = new Counter('insufficient_inventory_errors');

// ============================================================================
// Setup (runs once before all VUs)
// ============================================================================

export function setup() {
  console.log('🚀 Starting load test setup...');

  // Create a shared test event if not provided
  if (!TEST_EVENT_ID) {
    console.log('📦 Creating test event for load testing...');

    const token = getUserToken(0); // Use first test user for setup

    const eventPayload = JSON.stringify({
      title: 'Load Test Event',
      description: 'Event for k6 load testing',
      start_time: '2025-12-31T20:00:00Z',
      end_time: '2025-12-31T23:00:00Z',
      venue_name: 'Load Test Arena',
      venue_address: '123 Test Street, Test City, TS 12345',
    });

    const eventRes = http.post(
      `${BASE_URL}/api/events`,
      eventPayload,
      {
        headers: {
          'Authorization': `Bearer ${token}`,
          'Content-Type': 'application/json',
        },
      }
    );

    if (eventRes.status === 201) {
      const eventData = JSON.parse(eventRes.body);
      const eventId = eventData.event_id;
      console.log(`✅ Test event created: ${eventId}`);

      // Wait for event initialization
      sleep(2);

      return { eventId: eventId };
    } else {
      console.error(`❌ Failed to create test event: ${eventRes.status}`);
      console.error(eventRes.body);
      return { eventId: null };
    }
  }

  console.log(`✅ Using pre-configured event: ${TEST_EVENT_ID}`);
  return { eventId: TEST_EVENT_ID };
}

// ============================================================================
// Main Test Scenario (runs for each VU)
// ============================================================================

export default function (data) {
  // Each virtual user has a unique token based on their VU ID
  const userToken = getUserToken(__VU);
  const eventId = data.eventId;

  if (!eventId) {
    console.error('❌ No event ID available - skipping test');
    return;
  }

  const headers = {
    'Authorization': `Bearer ${userToken}`,
    'Content-Type': 'application/json',
  };

  // ========================================
  // Scenario 1: Create Reservation
  // ========================================
  group('Create Reservation', function () {
    const startTime = Date.now();

    const reservationPayload = JSON.stringify({
      event_id: eventId,
      section: Math.random() > 0.5 ? 'VIP' : 'General', // Mix sections
      quantity: Math.floor(Math.random() * 3) + 1, // 1-3 tickets
      specific_seats: null,
    });

    const reserveRes = http.post(
      `${BASE_URL}/api/reservations`,
      reservationPayload,
      { headers: headers }
    );

    const duration = Date.now() - startTime;
    reservationDuration.add(duration);

    const success = check(reserveRes, {
      'reservation status is 201': (r) => r.status === 201,
      'reservation has ID': (r) => {
        if (r.status === 201) {
          const body = JSON.parse(r.body);
          return body.reservation_id !== undefined;
        }
        return false;
      },
    });

    if (success) {
      reservationSuccessRate.add(1);
    } else {
      reservationSuccessRate.add(0);
      reservationFailures.add(1);

      // Track insufficient inventory errors separately
      if (reserveRes.status === 400 || reserveRes.status === 409) {
        const errorBody = reserveRes.body;
        if (errorBody.includes('Insufficient') || errorBody.includes('inventory')) {
          insufficientInventoryErrors.add(1);
        }
      }

      // Log detailed error for debugging
      if (reserveRes.status >= 500) {
        console.error(`❌ Server error: ${reserveRes.status} - ${reserveRes.body}`);
      }
    }

    // Store reservation ID for potential follow-up actions
    if (reserveRes.status === 201) {
      const reservationData = JSON.parse(reserveRes.body);
      const reservationId = reservationData.reservation_id;

      // ========================================
      // Scenario 2: Query Reservation Status
      // ========================================
      group('Query Reservation', function () {
        const queryRes = http.get(
          `${BASE_URL}/api/reservations/${reservationId}`,
          { headers: headers }
        );

        check(queryRes, {
          'query status is 200': (r) => r.status === 200,
          'query returns reservation data': (r) => {
            if (r.status === 200) {
              const body = JSON.parse(r.body);
              return body.id === reservationId;
            }
            return false;
          },
        });
      });
    }
  });

  // ========================================
  // Scenario 3: Query Event Availability
  // ========================================
  group('Query Availability', function () {
    const section = Math.random() > 0.5 ? 'VIP' : 'General';

    const availabilityRes = http.get(
      `${BASE_URL}/api/events/${eventId}/sections/${section}/availability`,
      { headers: headers }
    );

    check(availabilityRes, {
      'availability status is 200': (r) => r.status === 200,
      'availability has data': (r) => {
        if (r.status === 200) {
          const body = JSON.parse(r.body);
          return body.available !== undefined;
        }
        return false;
      },
    });
  });

  // ========================================
  // Think Time (simulate user behavior)
  // ========================================
  // Random think time between 1-5 seconds
  // This simulates real users browsing, reading, deciding
  const thinkTime = Math.random() * 4 + 1;
  sleep(thinkTime);
}

// ============================================================================
// Teardown (runs once after all VUs complete)
// ============================================================================

export function teardown(data) {
  console.log('🏁 Load test complete');
  console.log(`📊 Test event ID: ${data.eventId}`);
  console.log('');
  console.log('📈 Check detailed metrics above for:');
  console.log('   - p95/p99 latency');
  console.log('   - Error rate');
  console.log('   - Reservation success rate');
  console.log('   - Insufficient inventory errors');
}

// ============================================================================
// Helper Functions
// ============================================================================

/**
 * Custom summary output
 *
 * This function runs at the end to provide a clear summary of results.
 */
export function handleSummary(data) {
  return {
    'stdout': textSummary(data, { indent: '  ', enableColors: true }),
    'summary.json': JSON.stringify(data),
  };
}

/**
 * Simple text summary formatter
 */
function textSummary(data, options) {
  const indent = options?.indent || '';
  const enableColors = options?.enableColors || false;

  let output = '';

  output += `\n${'='.repeat(60)}\n`;
  output += `${indent}📊 LOAD TEST SUMMARY\n`;
  output += `${'='.repeat(60)}\n\n`;

  // Metrics
  const metrics = data.metrics;

  if (metrics) {
    output += `${indent}🎯 Key Metrics:\n`;
    output += `${indent}  Requests:              ${metrics.http_reqs?.values?.count || 0}\n`;
    output += `${indent}  Failed Requests:       ${metrics.http_req_failed?.values?.rate * 100 || 0}%\n`;
    output += `${indent}  Avg Response Time:     ${metrics.http_req_duration?.values?.avg?.toFixed(2) || 0}ms\n`;
    output += `${indent}  p95 Response Time:     ${metrics.http_req_duration?.values['p(95)']?.toFixed(2) || 0}ms\n`;
    output += `${indent}  p99 Response Time:     ${metrics.http_req_duration?.values['p(99)']?.toFixed(2) || 0}ms\n`;
    output += `${indent}  Reservation Success:   ${(metrics.reservation_success_rate?.values?.rate * 100 || 0).toFixed(2)}%\n`;
    output += `${indent}  Inventory Errors:      ${metrics.insufficient_inventory_errors?.values?.count || 0}\n`;
    output += `\n`;

    // Threshold checks
    output += `${indent}✅ Threshold Results:\n`;
    const thresholds = data.root_group?.checks || [];
    thresholds.forEach((check) => {
      const status = check.passes > 0 ? '✅' : '❌';
      output += `${indent}  ${status} ${check.name}\n`;
    });
  }

  output += `\n${'='.repeat(60)}\n\n`;

  return output;
}
