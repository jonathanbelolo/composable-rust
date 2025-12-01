/**
 * Seed script for demo data.
 *
 * This script creates events through the backend API, respecting the
 * event-driven architecture:
 *   1. API call → Handler → BusinessLogic
 *   2. Events persisted to EventStore
 *   3. Projections automatically updated
 *
 * Run with: npm run seed
 * Requires: Backend running at BACKEND_URL (default: http://localhost:8080)
 */

const BACKEND_URL = process.env.BACKEND_URL || 'http://localhost:8080';

interface CreateEventRequest {
  title: string;
  description?: string;
  start_time: string;
  venue_name: string;
  capacity: number;
  price: number;
  owner_id?: string;
}

interface CreateEventResponse {
  event_id: string;
  message: string;
}

interface PublishEventResponse {
  event_id: string;
  message: string;
}

// Demo owner ID for seeded events
const DEMO_OWNER_ID = '00000000-0000-0000-0000-000000000001';

// Test user token for authentication (matches integration tests)
// Uses the test-user-{uuid} pattern that bypasses magic link authentication
// when AUTH_EXPOSE_MAGIC_LINKS_FOR_TESTING=true is set on the server.
const TEST_AUTH_TOKEN = `test-user-${DEMO_OWNER_ID}`;

// Sample events to create
const SEED_EVENTS: CreateEventRequest[] = [
  {
    title: 'Summer Music Festival 2025',
    description: 'A three-day outdoor music festival featuring top artists.',
    start_time: addDays(new Date(), 30).toISOString(),
    venue_name: 'Central Park Amphitheater',
    capacity: 2800,
    price: 150.0,
    owner_id: DEMO_OWNER_ID,
  },
  {
    title: 'Tech Conference 2025',
    description: 'Annual technology conference with industry leaders.',
    start_time: addDays(new Date(), 45).toISOString(),
    venue_name: 'Convention Center Hall A',
    capacity: 850,
    price: 299.0,
    owner_id: DEMO_OWNER_ID,
  },
  {
    title: 'Broadway Musical: Hamilton',
    description: 'Award-winning musical about Alexander Hamilton.',
    start_time: addDays(new Date(), 14).toISOString(),
    venue_name: 'Grand Theater',
    capacity: 900,
    price: 175.0,
    owner_id: DEMO_OWNER_ID,
  },
  {
    title: 'Championship Basketball Finals',
    description: 'The ultimate basketball showdown.',
    start_time: addDays(new Date(), 7).toISOString(),
    venue_name: 'Sports Arena',
    capacity: 15050,
    price: 85.0,
    owner_id: DEMO_OWNER_ID,
  },
  {
    title: 'Stand-Up Comedy Night',
    description: 'An evening of laughter with top comedians.',
    start_time: addDays(new Date(), 3).toISOString(),
    venue_name: 'Comedy Club Downtown',
    capacity: 150,
    price: 45.0,
    owner_id: DEMO_OWNER_ID,
  },
  {
    title: 'Classical Symphony Orchestra',
    description: 'Beethoven and Mozart performed by the city symphony.',
    start_time: addDays(new Date(), 21).toISOString(),
    venue_name: 'Philharmonic Hall',
    capacity: 1040,
    price: 95.0,
    owner_id: DEMO_OWNER_ID,
  },
  {
    title: 'Electronic Dance Music Festival',
    description: 'All-night EDM experience with world-famous DJs.',
    start_time: addDays(new Date(), 60).toISOString(),
    venue_name: 'Warehouse District',
    capacity: 3200,
    price: 120.0,
    owner_id: DEMO_OWNER_ID,
  },
  {
    title: 'Food & Wine Festival',
    description: 'Culinary delights from award-winning chefs.',
    start_time: addDays(new Date(), 25).toISOString(),
    venue_name: 'Waterfront Plaza',
    capacity: 1220,
    price: 75.0,
    owner_id: DEMO_OWNER_ID,
  },
];

function addDays(date: Date, days: number): Date {
  const result = new Date(date);
  result.setDate(result.getDate() + days);
  return result;
}

async function createEvent(event: CreateEventRequest): Promise<CreateEventResponse> {
  const response = await fetch(`${BACKEND_URL}/api/v2/events`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${TEST_AUTH_TOKEN}`,
    },
    body: JSON.stringify(event),
  });

  if (!response.ok) {
    const errorText = await response.text();
    throw new Error(`Failed to create event "${event.title}": ${response.status} ${errorText}`);
  }

  return response.json() as Promise<CreateEventResponse>;
}

async function publishEvent(eventId: string): Promise<PublishEventResponse> {
  const response = await fetch(`${BACKEND_URL}/api/v2/events/${eventId}/publish`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${TEST_AUTH_TOKEN}`,
    },
  });

  if (!response.ok) {
    const errorText = await response.text();
    throw new Error(`Failed to publish event ${eventId}: ${response.status} ${errorText}`);
  }

  return response.json() as Promise<PublishEventResponse>;
}

async function seedEvents(): Promise<void> {
  console.log('🌱 Seeding demo events...\n');
  console.log(`Backend URL: ${BACKEND_URL}\n`);

  const createdEvents: Array<{ id: string; title: string }> = [];

  for (const event of SEED_EVENTS) {
    try {
      // Step 1: Create the event
      console.log(`📝 Creating: ${event.title}`);
      console.log(`   Venue: ${event.venue_name} (${event.capacity} seats)`);
      const createResult = await createEvent(event);
      console.log(`   ✓ Created with ID: ${createResult.event_id}`);

      // Step 2: Publish the event so it's visible
      console.log(`   📢 Publishing event...`);
      await publishEvent(createResult.event_id);
      console.log(`   ✓ Published!\n`);

      createdEvents.push({ id: createResult.event_id, title: event.title });
    } catch (error) {
      console.error(`   ✗ Error: ${error instanceof Error ? error.message : error}\n`);
    }
  }

  console.log('\n📊 Summary:');
  console.log(`   Created and published: ${createdEvents.length}/${SEED_EVENTS.length} events`);

  if (createdEvents.length > 0) {
    console.log('\n📋 Created Events:');
    for (const event of createdEvents) {
      console.log(`   - ${event.title} (${event.id})`);
    }
  }

  console.log('\n✅ Seeding complete!');
  console.log('\n💡 These events flow through the event-driven architecture:');
  console.log('   1. HTTP Request → Handler → BusinessLogic.process()');
  console.log('   2. Events persisted to PostgreSQL EventStore');
  console.log('   3. Projector updates read models (events, inventory tables)');
  console.log('   4. Frontend can now query /api/v2/events to see them');
}

// Run the seed script
seedEvents().catch((error) => {
  console.error('Seeding failed:', error);
  process.exit(1);
});
