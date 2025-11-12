# Composable Svelte Framework - Exploration

**Date**: 2025-11-12
**Explorer**: Claude (Sonnet 4.5)
**Purpose**: Understand Composable Svelte architecture for Phase 10 web frontend integration with Composable Rust ticketing backend

---

## Executive Summary

**Composable Svelte** is a production-ready (420+ tests) TypeScript/Svelte 5 framework that mirrors the **exact same philosophy** as Composable Rust:

- ✅ **Same Core Concepts**: State, Action, Reducer, Effect, Store
- ✅ **Same Patterns**: Unidirectional data flow, pure reducers, effects as values
- ✅ **Same Composition**: Scope operators, reducer nesting, dependency injection
- ✅ **Same Testing**: TestStore with send/receive pattern

**Key Additions for Frontend**:
- 🎯 **Navigation System**: Tree-based navigation (Modal, Sheet, Drawer, Alert)
- 🎯 **SSR/SSG Support**: Server-side rendering with Fastify, state hydration
- 🎯 **Backend Integration**: HTTP API client, WebSocket, Storage, Clock dependencies
- 🎯 **UI Components**: 73+ shadcn-svelte components with reducer-driven patterns
- 🎯 **i18n**: ICU MessageFormat with locale detection

**Perfect Match for Phase 10**:
- Can consume our ticketing HTTP API (35+ endpoints)
- Can handle WebSocket real-time updates
- Can implement SSR for SEO-optimized ticketing pages
- Uses TypeScript (type-safe integration with Rust backend)

---

## Architecture Overview

### Core Philosophy (Identical to Composable Rust)

```typescript
// 1. Define State
interface CounterState {
  count: number;
  isLoading: boolean;
}

// 2. Define Actions
type CounterAction =
  | { type: 'increment' }
  | { type: 'decrement' }
  | { type: 'incrementAsync' }
  | { type: 'incrementCompleted' };

// 3. Pure Reducer: (State, Action, Deps) => [NewState, Effect]
const counterReducer = (
  state: CounterState,
  action: CounterAction,
  deps: {}
): [CounterState, Effect<CounterAction>] => {
  switch (action.type) {
    case 'increment':
      return [{ ...state, count: state.count + 1 }, Effect.none()];

    case 'incrementAsync':
      return [
        { ...state, isLoading: true },
        Effect.run(async (dispatch) => {
          await new Promise(resolve => setTimeout(resolve, 1000));
          dispatch({ type: 'incrementCompleted' });
        })
      ];

    case 'incrementCompleted':
      return [
        { ...state, count: state.count + 1, isLoading: false },
        Effect.none()
      ];
  }
};

// 4. Create Store
const store = createStore({
  initialState: { count: 0, isLoading: false },
  reducer: counterReducer,
  dependencies: {}
});
```

### Effect System (Same as Rust)

```typescript
// Declarative effects as data structures
Effect.none()                          // No side effects
Effect.run(async (dispatch) => {...})  // Async work + dispatch
Effect.fireAndForget(() => {...})      // Fire-and-forget
Effect.batch(...effects)               // Parallel execution
Effect.cancel('id')                    // Cancel in-flight effect
```

**Key Principle**: Effects are **values**, not executions. The Store executes them.

---

## Key Features for Ticketing Frontend

### 1. HTTP API Integration

**Built-in API Client** with effects, retries, caching, interceptors:

```typescript
import { createLiveAPI } from '@composable-svelte/core/api';
import { Effect } from '@composable-svelte/core';

// Create API client
const api = createLiveAPI({
  baseURL: 'http://localhost:8080/api',
  interceptors: {
    request: async (config) => {
      // Add auth token from localStorage
      const token = localStorage.getItem('session_token');
      if (token) {
        config.headers.Authorization = `Bearer ${token}`;
      }
      return config;
    },
    response: async (response) => {
      // Handle 401 globally
      if (response.status === 401) {
        // Redirect to login
        window.location.href = '/login';
      }
      return response;
    }
  },
  retry: {
    maxAttempts: 3,
    delayMs: 1000,
    backoff: 'exponential'
  }
});

// In reducer: Make API calls with Effect.api()
interface AppDependencies {
  api: APIClient;
}

const appReducer = (state, action, deps) => {
  switch (action.type) {
    case 'loadEventsRequested':
      return [
        { ...state, isLoading: true },
        Effect.api(
          deps.api,
          { method: 'GET', url: '/events' },
          (response) => ({
            type: 'eventsLoaded',
            events: response.data
          }),
          (error) => ({
            type: 'eventsLoadFailed',
            error: error.message
          })
        )
      ];

    case 'eventsLoaded':
      return [
        { ...state, isLoading: false, events: action.events },
        Effect.none()
      ];

    case 'createReservation':
      return [
        { ...state, isReserving: true },
        Effect.api(
          deps.api,
          {
            method: 'POST',
            url: '/reservations',
            data: {
              event_id: action.eventId,
              section: action.section,
              quantity: action.quantity
            }
          },
          (response) => ({
            type: 'reservationCreated',
            reservation: response.data
          }),
          (error) => ({
            type: 'reservationFailed',
            error: error.message
          })
        )
      ];
  }
};
```

**Perfect for our ticketing API**: Handles authentication, retries, error handling out of the box.

### 2. WebSocket Integration

**Real-time updates** for seat availability:

```typescript
import { createLiveWebSocket } from '@composable-svelte/core/websocket';

// Create WebSocket client
const ws = createLiveWebSocket({
  url: 'ws://localhost:8080/ws?token=${sessionToken}',
  reconnect: {
    enabled: true,
    maxAttempts: 5,
    delayMs: 1000
  },
  heartbeat: {
    enabled: true,
    intervalMs: 30000
  }
});

// In reducer: Subscribe to WebSocket messages
case 'subscribeToEvent':
  return [
    state,
    Effect.run(async (dispatch) => {
      ws.on('message', (data) => {
        const msg = JSON.parse(data);
        if (msg.type === 'event' && msg.action.type === 'SeatsReserved') {
          dispatch({
            type: 'seatsUpdated',
            eventId: msg.action.event_id,
            available: msg.action.available
          });
        }
      });

      ws.on('reconnected', () => {
        dispatch({ type: 'websocketReconnected' });
      });

      await ws.connect();
    })
  ];
```

**Perfect for real-time seat availability**: Automatically handles reconnection, heartbeat, and message dispatching.

### 3. Server-Side Rendering (SSR)

**SEO-optimized ticketing pages** with SSR:

```typescript
// server.ts (Fastify)
import { createStore } from '@composable-svelte/core';
import { renderToHTML } from '@composable-svelte/core/ssr';
import App from './App.svelte';

app.get('/events/:id', async (req, res) => {
  // 1. Load data on server
  const event = await fetch(`http://localhost:8080/api/events/${req.params.id}`)
    .then(res => res.json());

  // 2. Create store with pre-populated data
  const store = createStore({
    initialState: {
      event,
      availability: null,
      isLoading: false
    },
    reducer: eventReducer,
    dependencies: {}  // No API client on server
  });

  // 3. Render to HTML with embedded state
  const html = renderToHTML(App, { store }, {
    head: `
      <title>${event.name} - Composable Ticketing</title>
      <meta name="description" content="${event.description}">
      <link rel="stylesheet" href="/assets/index.css">
    `,
    clientScript: '/assets/index.js'
  });

  res.type('text/html').send(html);
});

// client.ts (Browser)
import { hydrateStore } from '@composable-svelte/core/ssr';
import { mount } from 'svelte';

// Read serialized state from script tag
const stateJSON = document.getElementById('__COMPOSABLE_SVELTE_STATE__')?.textContent;

// Hydrate with client dependencies (API, WebSocket)
const store = hydrateStore(stateJSON, {
  reducer: eventReducer,
  dependencies: {
    api: createAPIClient(),
    storage: createLocalStorage()
  }
});

// Mount app (picks up from server-rendered HTML)
mount(App, { target: document.body, props: { store } });
```

**SSR Flow**:
1. **Server**: Load data → Create store → Render to HTML → Embed state
2. **Client**: Parse state → Hydrate store → Mount app → Interactive!

**Benefits**:
- ✅ SEO-friendly (search engines see full HTML)
- ✅ Fast initial page load (no client-side data fetch)
- ✅ Progressive enhancement (works without JS)

### 4. Navigation System

**Tree-based navigation** for modals, sheets, drawers:

```typescript
// State pattern
interface EventListState {
  events: Event[];
  selectedEvent: EventDetailState | null;  // Optional child
}

interface EventDetailState {
  eventId: string;
  event: Event;
  reservationForm: ReservationFormState | null;  // Nested child
}

// In reducer: Show modal
case 'eventClicked':
  return [
    {
      ...state,
      selectedEvent: {
        eventId: action.eventId,
        event: findEvent(state.events, action.eventId),
        reservationForm: null
      }
    },
    Effect.none()
  ];

// In Svelte component: Render modal
<script lang="ts">
  import { Modal } from '@composable-svelte/core/navigation-components';
  import { scopeTo } from '@composable-svelte/core/navigation';

  // Scope store to selectedEvent field
  const eventDetailStore = $derived(
    scopeTo($rootStore).into('selectedEvent')
  );
</script>

{#if eventDetailStore}
  <Modal store={eventDetailStore}>
    <EventDetail />
  </Modal>
{/if}
```

**Perfect for ticketing flows**: Reservation modal → Payment sheet → Confirmation alert

### 5. Svelte 5 Integration

**Reactive state management** with Svelte 5 runes:

```svelte
<script lang="ts">
  import { store } from './store';

  // ✅ CORRECT: Subscribe to store (reactive)
  const state = $derived($store);

  // Access reactive state
  const isLoading = $derived($store.isLoading);
  const events = $derived($store.events);

  // Local component state (not in store)
  let searchQuery = $state('');

  // Derived local state
  const filteredEvents = $derived(
    events.filter(e =>
      e.name.toLowerCase().includes(searchQuery.toLowerCase())
    )
  );
</script>

<div>
  <!-- Reactive rendering -->
  {#if isLoading}
    <Spinner />
  {:else}
    <input bind:value={searchQuery} placeholder="Search events...">

    {#each filteredEvents as event}
      <EventCard
        {event}
        onclick={() => store.dispatch({ type: 'eventClicked', eventId: event.id })}
      />
    {/each}
  {/if}
</div>
```

**Key Pattern**:
- ✅ Use `$store` to subscribe (reactive)
- ❌ Never use `store.state` (not reactive!)

---

## Project Structure

```
composable-svelte/
├── packages/
│   ├── core/                       # @composable-svelte/core
│   │   ├── src/lib/
│   │   │   ├── store.ts           # Store implementation (Svelte 5 runes)
│   │   │   ├── effect.ts          # Effect system
│   │   │   ├── types.ts           # Core types
│   │   │   ├── composition/       # scope(), combineReducers()
│   │   │   ├── navigation/        # ifLet(), createDestination()
│   │   │   ├── navigation-components/  # Modal, Sheet, Drawer
│   │   │   ├── routing/           # URL routing (path-to-regexp)
│   │   │   ├── api/               # HTTP client (162 tests)
│   │   │   ├── websocket/         # WebSocket client (140 tests)
│   │   │   ├── dependencies/      # Clock, Storage, Cookie (118 tests)
│   │   │   ├── i18n/              # Internationalization (35 tests)
│   │   │   ├── ssr/               # Server-side rendering (45 tests)
│   │   │   ├── components/        # 73+ shadcn-svelte components
│   │   │   └── test/              # TestStore
│   │   └── tests/                 # 420+ tests
│   ├── charts/                    # Chart components
│   ├── chat/                      # Chat components
│   ├── maps/                      # Map components
│   └── media/                     # Media components
├── examples/
│   ├── counter/                   # Simple counter
│   ├── product-gallery/           # Full app with navigation
│   ├── ssr-server/                # SSR with Fastify
│   └── styleguide/                # Component showcase
└── .claude/
    └── skills/
        ├── composable-svelte-frontend.md  # Critical patterns
        ├── composable-svelte-i18n/        # i18n patterns
        └── composable-svelte-ssr/         # SSR patterns
```

---

## How to Integrate with Composable Rust Backend

### Step-by-Step Integration

**1. Create API Client Wrapper**

```typescript
// src/lib/api.ts
import { createLiveAPI } from '@composable-svelte/core/api';

export const ticketingAPI = createLiveAPI({
  baseURL: import.meta.env.VITE_API_URL || 'http://localhost:8080/api',
  interceptors: {
    request: async (config) => {
      // Add session token from localStorage
      const token = localStorage.getItem('session_token');
      if (token) {
        config.headers.Authorization = `Bearer ${token}`;
      }
      return config;
    },
    response: async (response) => {
      // Handle 401 (session expired)
      if (response.status === 401) {
        localStorage.removeItem('session_token');
        window.location.href = '/login';
      }
      return response;
    }
  },
  retry: {
    maxAttempts: 3,
    delayMs: 1000,
    backoff: 'exponential'
  }
});

// Type-safe endpoint definitions
export const endpoints = {
  auth: {
    requestMagicLink: (email: string) => ({
      method: 'POST' as const,
      url: '/auth/magic-link/request',
      data: { email }
    }),
    verifyMagicLink: (token: string) => ({
      method: 'POST' as const,
      url: '/auth/magic-link/verify',
      data: { token }
    })
  },

  events: {
    list: () => ({
      method: 'GET' as const,
      url: '/events'
    }),
    get: (eventId: string) => ({
      method: 'GET' as const,
      url: `/events/${eventId}`
    }),
    getAvailability: (eventId: string) => ({
      method: 'GET' as const,
      url: `/events/${eventId}/availability`
    })
  },

  reservations: {
    create: (data: CreateReservationRequest) => ({
      method: 'POST' as const,
      url: '/reservations',
      data
    }),
    get: (reservationId: string) => ({
      method: 'GET' as const,
      url: `/reservations/${reservationId}`
    }),
    completePayment: (reservationId: string) => ({
      method: 'POST' as const,
      url: `/reservations/${reservationId}/payment`
    })
  }
};
```

**2. Define State & Actions (Mirror Backend Domain)**

```typescript
// src/features/events/types.ts

// State (matches backend projections)
export interface EventListState {
  events: Event[];
  isLoading: boolean;
  error: string | null;
  selectedEvent: EventDetailState | null;
}

export interface Event {
  id: string;
  name: string;
  description: string;
  date: string;
  venue: string;
  status: 'draft' | 'published' | 'cancelled';
}

export interface EventDetailState {
  eventId: string;
  event: Event;
  availability: SeatAvailability | null;
  isLoadingAvailability: boolean;
  reservationForm: ReservationFormState | null;
}

export interface SeatAvailability {
  vip: { available: number; total: number; };
  standard: { available: number; total: number; };
  general: { available: number; total: number; };
}

// Actions (mirror backend commands/events)
export type EventListAction =
  | { type: 'loadEventsRequested' }
  | { type: 'eventsLoaded'; events: Event[] }
  | { type: 'eventsLoadFailed'; error: string }
  | { type: 'eventClicked'; eventId: string }
  | { type: 'eventDetail'; action: EventDetailAction };

export type EventDetailAction =
  | { type: 'loadAvailabilityRequested' }
  | { type: 'availabilityLoaded'; availability: SeatAvailability }
  | { type: 'availabilityLoadFailed'; error: string }
  | { type: 'reserveButtonClicked'; section: string }
  | { type: 'dismiss' };
```

**3. Implement Reducer with API Effects**

```typescript
// src/features/events/reducer.ts
import { Effect } from '@composable-svelte/core';
import { ticketingAPI, endpoints } from '../../lib/api';

export interface EventDependencies {
  api: typeof ticketingAPI;
}

export const eventListReducer = (
  state: EventListState,
  action: EventListAction,
  deps: EventDependencies
): [EventListState, Effect<EventListAction>] => {
  switch (action.type) {
    case 'loadEventsRequested':
      return [
        { ...state, isLoading: true, error: null },
        Effect.api(
          deps.api,
          endpoints.events.list(),
          (response) => ({ type: 'eventsLoaded', events: response.data }),
          (error) => ({ type: 'eventsLoadFailed', error: error.message })
        )
      ];

    case 'eventsLoaded':
      return [
        { ...state, isLoading: false, events: action.events },
        Effect.none()
      ];

    case 'eventsLoadFailed':
      return [
        { ...state, isLoading: false, error: action.error },
        Effect.none()
      ];

    case 'eventClicked': {
      const event = state.events.find(e => e.id === action.eventId);
      if (!event) return [state, Effect.none()];

      return [
        {
          ...state,
          selectedEvent: {
            eventId: action.eventId,
            event,
            availability: null,
            isLoadingAvailability: true,
            reservationForm: null
          }
        },
        // Automatically load availability when opening detail
        Effect.api(
          deps.api,
          endpoints.events.getAvailability(action.eventId),
          (response) => ({
            type: 'eventDetail',
            action: {
              type: 'availabilityLoaded',
              availability: response.data
            }
          }),
          (error) => ({
            type: 'eventDetail',
            action: {
              type: 'availabilityLoadFailed',
              error: error.message
            }
          })
        )
      ];
    }

    // Handle child actions with ifLet
    case 'eventDetail': {
      if (!state.selectedEvent) return [state, Effect.none()];

      const [newDetail, effect] = eventDetailReducer(
        state.selectedEvent,
        action.action,
        deps
      );

      return [
        { ...state, selectedEvent: newDetail },
        effect
      ];
    }
  }
};
```

**4. Create Svelte Components**

```svelte
<!-- src/features/events/EventList.svelte -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { store } from '../../store';
  import { scopeTo } from '@composable-svelte/core/navigation';
  import { Modal } from '@composable-svelte/core/navigation-components';
  import EventDetail from './EventDetail.svelte';

  // Subscribe to store (reactive)
  const events = $derived($store.events);
  const isLoading = $derived($store.isLoading);
  const error = $derived($store.error);

  // Scoped store for modal
  const eventDetailStore = $derived(
    scopeTo($store).into('selectedEvent')
  );

  // Load events on mount
  onMount(() => {
    store.dispatch({ type: 'loadEventsRequested' });
  });
</script>

<div class="event-list">
  <h1>Upcoming Events</h1>

  {#if isLoading}
    <div class="spinner">Loading...</div>
  {:else if error}
    <div class="error">{error}</div>
  {:else}
    <div class="grid">
      {#each events as event}
        <div
          class="event-card"
          onclick={() => store.dispatch({
            type: 'eventClicked',
            eventId: event.id
          })}
        >
          <h3>{event.name}</h3>
          <p>{event.description}</p>
          <p>{new Date(event.date).toLocaleDateString()}</p>
          <p>{event.venue}</p>
        </div>
      {/each}
    </div>
  {/if}

  <!-- Event detail modal -->
  {#if eventDetailStore}
    <Modal store={eventDetailStore}>
      <EventDetail />
    </Modal>
  {/if}
</div>
```

**5. Add WebSocket for Real-Time Updates**

```typescript
// In reducer: Subscribe to WebSocket
case 'subscribeToWebSocket':
  return [
    state,
    Effect.run(async (dispatch) => {
      const ws = createLiveWebSocket({
        url: `ws://localhost:8080/ws?token=${sessionToken}`,
        reconnect: { enabled: true, maxAttempts: 5 },
        heartbeat: { enabled: true, intervalMs: 30000 }
      });

      ws.on('message', (data) => {
        const msg = JSON.parse(data);

        // Handle seat availability updates
        if (msg.type === 'event' && msg.action.type === 'SeatsReserved') {
          dispatch({
            type: 'seatsUpdatedFromWebSocket',
            eventId: msg.action.event_id,
            section: msg.action.section,
            quantity: msg.action.quantity
          });
        }
      });

      ws.on('error', (error) => {
        console.error('WebSocket error:', error);
      });

      await ws.connect();
    })
  ];

case 'seatsUpdatedFromWebSocket':
  // Update availability in real-time
  if (state.selectedEvent?.eventId === action.eventId) {
    const availability = { ...state.selectedEvent.availability };
    availability[action.section].available -= action.quantity;

    return [
      {
        ...state,
        selectedEvent: {
          ...state.selectedEvent,
          availability
        }
      },
      Effect.none()
    ];
  }
  return [state, Effect.none()];
```

**6. Set Up SSR (Optional)**

```typescript
// server/index.ts
import Fastify from 'fastify';
import { createStore } from '@composable-svelte/core';
import { renderToHTML } from '@composable-svelte/core/ssr';
import App from '../client/App.svelte';

const app = Fastify({ logger: true });

app.get('/events/:id', async (req, res) => {
  // 1. Fetch data from backend
  const event = await fetch(
    `http://localhost:8080/api/events/${req.params.id}`
  ).then(res => res.json());

  const availability = await fetch(
    `http://localhost:8080/api/events/${req.params.id}/availability`
  ).then(res => res.json());

  // 2. Create store with SSR state
  const store = createStore({
    initialState: {
      events: [event],
      isLoading: false,
      error: null,
      selectedEvent: {
        eventId: event.id,
        event,
        availability,
        isLoadingAvailability: false,
        reservationForm: null
      }
    },
    reducer: eventListReducer,
    dependencies: {}  // No API client on server
  });

  // 3. Render to HTML
  const html = renderToHTML(App, { store }, {
    head: `
      <title>${event.name} - Composable Ticketing</title>
      <meta name="description" content="${event.description}">
    `,
    clientScript: '/assets/client.js'
  });

  res.type('text/html').send(html);
});

app.listen({ port: 3000 });
```

---

## Testing Strategy

### TestStore (Same as Rust's TestStore)

```typescript
import { createTestStore } from '@composable-svelte/core';

describe('Event List', () => {
  it('should load events', async () => {
    const mockAPI = createMockAPI({
      '/events': {
        data: [
          { id: '1', name: 'Concert', ... }
        ]
      }
    });

    const store = createTestStore({
      initialState: { events: [], isLoading: false, error: null },
      reducer: eventListReducer,
      dependencies: { api: mockAPI }
    });

    // Send action
    await store.send({ type: 'loadEventsRequested' }, (state) => {
      expect(state.isLoading).toBe(true);
    });

    // Receive async response
    await store.receive({ type: 'eventsLoaded', events: [...] }, (state) => {
      expect(state.isLoading).toBe(false);
      expect(state.events).toHaveLength(1);
    });
  });
});
```

---

## Comparison: Composable Rust ↔ Composable Svelte

| Concept | Composable Rust | Composable Svelte |
|---------|----------------|-------------------|
| **Core Types** | `State`, `Action`, `Reducer`, `Effect`, `Store` | ✅ Same |
| **Reducer Signature** | `fn reduce(&self, &mut State, Action, &Env) -> Vec<Effect>` | `(State, Action, Deps) => [NewState, Effect]` |
| **Effect System** | `Effect::None`, `Effect::Future`, `Effect::Batch` | `Effect.none()`, `Effect.run()`, `Effect.batch()` |
| **Dependency Injection** | Trait-based environments | Interface-based dependencies |
| **Store** | `Store::new(state, reducer, env)` | `createStore({ initialState, reducer, dependencies })` |
| **Testing** | `TestStore` with `send()`/`receive()` | ✅ Same API |
| **Composition** | `scope()` for child reducers | ✅ Same |
| **Navigation** | N/A (backend) | Tree-based destinations (Modal, Sheet, Drawer) |
| **Backend Integration** | N/A (is the backend) | HTTP client, WebSocket, Storage |
| **SSR** | N/A (Axum handles HTTP) | Full SSR/SSG with hydration |

**Philosophy Alignment**: 🟢 **100% match** - Same patterns, same mindset, seamless integration

---

## Next Steps for Phase 10 Web Frontend

### Recommended Approach

**Phase 10.17: Composable Svelte Frontend** (15-20 hours)

1. **Setup** (2 hours)
   - Clone composable-svelte as submodule or copy core package
   - Set up Vite + TypeScript + Svelte 5
   - Configure API client with ticketing backend URL

2. **Authentication** (3-4 hours)
   - Magic link flow (request → verify → store token)
   - Session management (localStorage)
   - Protected routes (redirect to login if not authenticated)

3. **Event Browsing** (2-3 hours)
   - Event list page (fetch from `/api/events`)
   - Event detail modal (fetch availability from `/api/events/:id/availability`)
   - Real-time seat updates (WebSocket subscription)

4. **Reservation Flow** (4-5 hours)
   - Reservation form (section, quantity)
   - Create reservation (POST `/api/reservations`)
   - Payment flow (POST `/api/reservations/:id/payment`)
   - Confirmation alert

5. **Admin Features** (2-3 hours)
   - Create event form (POST `/api/events`)
   - Publish/open sales/close sales actions
   - Analytics dashboard (fetch from `/api/events/:id/analytics`)

6. **SSR (Optional)** (2-3 hours)
   - Set up Fastify server
   - Render event pages with SSR
   - Client-side hydration

7. **Testing** (2-3 hours)
   - TestStore for reducer logic
   - Mock API for integration tests
   - E2E tests with Playwright

**Total**: 17-24 hours (without SSR: 15-21 hours)

### File Structure

```
ticketing-web/
├── src/
│   ├── lib/
│   │   ├── api.ts                 # API client wrapper
│   │   ├── websocket.ts           # WebSocket client
│   │   └── auth.ts                # Auth helpers
│   ├── features/
│   │   ├── auth/
│   │   │   ├── types.ts
│   │   │   ├── reducer.ts
│   │   │   └── Login.svelte
│   │   ├── events/
│   │   │   ├── types.ts
│   │   │   ├── reducer.ts
│   │   │   ├── EventList.svelte
│   │   │   └── EventDetail.svelte
│   │   ├── reservations/
│   │   │   ├── types.ts
│   │   │   ├── reducer.ts
│   │   │   └── ReservationForm.svelte
│   │   └── admin/
│   │       ├── types.ts
│   │       ├── reducer.ts
│   │       └── CreateEvent.svelte
│   ├── app/
│   │   ├── App.svelte
│   │   ├── app.types.ts
│   │   └── app.reducer.ts
│   ├── store.ts
│   └── main.ts
├── server/                         # SSR (optional)
│   └── index.ts
└── tests/
    ├── auth.test.ts
    ├── events.test.ts
    └── reservations.test.ts
```

---

## Key Takeaways

1. ✅ **Perfect Architectural Match**: Composable Svelte mirrors Composable Rust exactly
2. ✅ **Production-Ready**: 420+ tests, full SSR/SSG, complete backend integration
3. ✅ **Type-Safe**: TypeScript ensures compile-time correctness
4. ✅ **Seamless Integration**: Built-in HTTP client, WebSocket, auth patterns
5. ✅ **Testable**: TestStore mirrors Rust's testing approach
6. ✅ **Modern**: Svelte 5 runes, Motion One animations, shadcn-svelte components

**Recommendation**: Use Composable Svelte for Phase 10 web frontend. It's a **perfect match** for our Composable Rust backend, and the shared philosophy will make development intuitive and maintainable.

---

## Resources

- **Repository**: `/Users/jonathanbelolo/dev/claude/code/composable-svelte`
- **README**: `/Users/jonathanbelolo/dev/claude/code/composable-svelte/README.md`
- **CLAUDE.md**: `/Users/jonathanbelolo/dev/claude/code/composable-svelte/CLAUDE.md` (33KB, comprehensive)
- **Examples**: `/Users/jonathanbelolo/dev/claude/code/composable-svelte/examples/`
  - `ssr-server/` - Complete SSR example with Fastify
  - `product-gallery/` - Full app with navigation
  - `styleguide/` - 73+ component showcase
- **Skills**: `.claude/skills/composable-svelte-frontend.md` - Critical patterns
