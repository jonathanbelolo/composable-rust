# Ticketing Frontend Architecture

A comprehensive plan for building the ticketing frontend using **composable-svelte**.

---

## 1. Overview

### 1.1 Technology Stack

| Layer | Technology |
|-------|------------|
| **Framework** | SvelteKit 2.x + Svelte 5 |
| **State Management** | composable-svelte (TCA-inspired) |
| **UI Components** | shadcn-svelte (via composable-svelte) |
| **Styling** | Tailwind CSS 3.x |
| **API Client** | composable-svelte API client |
| **Real-time** | composable-svelte WebSocket client |
| **Testing** | Vitest + TestStore |

### 1.2 Architecture Philosophy

Both backend (composable-rust) and frontend (composable-svelte) share the same TCA architecture:

```
Action → Reducer → (NewState, Effect) → Effect Execution → New Actions
```

This creates a **unified mental model** across the full stack:
- **Backend**: `EventCommand → BusinessLogic → (State, Events) → Handler → HTTP Response`
- **Frontend**: `UserAction → Reducer → (State, Effect) → Effect Execution → UI Update`

---

## 2. Project Structure

```
frontend/
├── src/
│   ├── lib/
│   │   ├── api/                    # API client configuration
│   │   │   ├── client.ts           # Base API client setup
│   │   │   ├── endpoints/          # Typed endpoint definitions
│   │   │   │   ├── auth.ts
│   │   │   │   ├── events.ts
│   │   │   │   ├── reservations.ts
│   │   │   │   ├── payments.ts
│   │   │   │   └── analytics.ts
│   │   │   └── types.ts            # API response types
│   │   │
│   │   ├── features/               # Feature modules (TCA pattern)
│   │   │   ├── auth/
│   │   │   │   ├── state.ts        # AuthState type
│   │   │   │   ├── action.ts       # AuthAction union
│   │   │   │   ├── reducer.ts      # authReducer
│   │   │   │   └── index.ts        # Feature exports
│   │   │   │
│   │   │   ├── events/
│   │   │   │   ├── state.ts
│   │   │   │   ├── action.ts
│   │   │   │   ├── reducer.ts
│   │   │   │   ├── components/     # Feature-specific components
│   │   │   │   │   ├── EventCard.svelte
│   │   │   │   │   ├── EventList.svelte
│   │   │   │   │   └── EventDetail.svelte
│   │   │   │   └── index.ts
│   │   │   │
│   │   │   ├── reservations/
│   │   │   │   ├── state.ts
│   │   │   │   ├── action.ts
│   │   │   │   ├── reducer.ts
│   │   │   │   ├── components/
│   │   │   │   │   ├── SeatSelector.svelte
│   │   │   │   │   ├── ReservationSummary.svelte
│   │   │   │   │   └── CheckoutFlow.svelte
│   │   │   │   └── index.ts
│   │   │   │
│   │   │   ├── payments/
│   │   │   │   ├── state.ts
│   │   │   │   ├── action.ts
│   │   │   │   ├── reducer.ts
│   │   │   │   └── index.ts
│   │   │   │
│   │   │   ├── dashboard/          # User dashboard
│   │   │   │   ├── state.ts
│   │   │   │   ├── action.ts
│   │   │   │   ├── reducer.ts
│   │   │   │   └── index.ts
│   │   │   │
│   │   │   └── organizer/          # Event organizer features
│   │   │       ├── state.ts
│   │   │       ├── action.ts
│   │   │       ├── reducer.ts
│   │   │       └── index.ts
│   │   │
│   │   ├── app/                    # Root app state composition
│   │   │   ├── state.ts            # AppState (composed)
│   │   │   ├── action.ts           # AppAction (union)
│   │   │   ├── reducer.ts          # Root reducer (composed)
│   │   │   ├── store.ts            # Store creation
│   │   │   └── dependencies.ts     # DI container
│   │   │
│   │   ├── components/             # Shared components
│   │   │   ├── layout/
│   │   │   │   ├── Header.svelte
│   │   │   │   ├── Footer.svelte
│   │   │   │   └── Sidebar.svelte
│   │   │   └── common/
│   │   │       ├── LoadingSpinner.svelte
│   │   │       ├── ErrorBoundary.svelte
│   │   │       └── Toast.svelte
│   │   │
│   │   └── utils/                  # Utilities
│   │       ├── formatters.ts       # Date, currency formatters
│   │       └── validators.ts       # Form validation
│   │
│   ├── routes/                     # SvelteKit routes
│   │   ├── +layout.svelte          # Root layout
│   │   ├── +layout.ts              # Root load function
│   │   ├── +page.svelte            # Home page (event listing)
│   │   │
│   │   ├── auth/
│   │   │   ├── login/+page.svelte
│   │   │   └── verify/+page.svelte
│   │   │
│   │   ├── events/
│   │   │   ├── +page.svelte        # Event listing
│   │   │   └── [id]/
│   │   │       ├── +page.svelte    # Event detail
│   │   │       └── reserve/
│   │   │           └── +page.svelte # Reservation flow
│   │   │
│   │   ├── dashboard/
│   │   │   ├── +page.svelte        # User dashboard
│   │   │   ├── reservations/
│   │   │   │   └── +page.svelte
│   │   │   └── payments/
│   │   │       └── +page.svelte
│   │   │
│   │   └── organizer/              # Event organizer routes
│   │       ├── +page.svelte        # My events
│   │       ├── create/
│   │       │   └── +page.svelte    # Create event
│   │       └── [id]/
│   │           ├── +page.svelte    # Edit event
│   │           └── analytics/
│   │               └── +page.svelte
│   │
│   ├── app.html
│   ├── app.css                     # Tailwind imports
│   └── hooks.server.ts             # SvelteKit hooks
│
├── tests/
│   ├── unit/
│   │   ├── features/
│   │   │   ├── auth.test.ts
│   │   │   ├── events.test.ts
│   │   │   └── reservations.test.ts
│   │   └── components/
│   └── integration/
│       └── flows/
│           ├── reservation-flow.test.ts
│           └── checkout-flow.test.ts
│
├── static/
├── plans/
│   └── ARCHITECTURE.md             # This file
├── package.json
├── svelte.config.js
├── tailwind.config.js
├── tsconfig.json
└── vite.config.ts
```

---

## 3. State Architecture

### 3.1 Root AppState

```typescript
// src/lib/app/state.ts
import type { AuthState } from '$lib/features/auth';
import type { EventsState } from '$lib/features/events';
import type { ReservationsState } from '$lib/features/reservations';
import type { PaymentsState } from '$lib/features/payments';
import type { DashboardState } from '$lib/features/dashboard';
import type { OrganizerState } from '$lib/features/organizer';
import type { PresentationState } from '@composable-svelte/core';

export interface AppState {
  // Authentication
  auth: AuthState;

  // Feature states
  events: EventsState;
  reservations: ReservationsState;
  payments: PaymentsState;
  dashboard: DashboardState;
  organizer: OrganizerState;

  // Global UI state
  ui: UIState;

  // Navigation presentations (modals, sheets, etc.)
  eventDetail: EventDetailState | null;
  eventDetailPresentation: PresentationState<EventDetailState>;

  checkoutSheet: CheckoutState | null;
  checkoutPresentation: PresentationState<CheckoutState>;
}

export interface UIState {
  toasts: Toast[];
  isLoading: boolean;
  error: AppError | null;
}
```

### 3.2 Feature State Examples

```typescript
// src/lib/features/auth/state.ts
export interface AuthState {
  user: User | null;
  token: string | null;
  isAuthenticated: boolean;
  isLoading: boolean;
  error: AuthError | null;
  magicLinkSent: boolean;
}

// src/lib/features/events/state.ts
export interface EventsState {
  // List view
  events: Event[];
  isLoading: boolean;
  error: EventsError | null;
  pagination: PaginationState;
  filters: EventFilters;

  // Selected event for detail view
  selectedEvent: Event | null;
  availability: SectionAvailability[] | null;
}

// src/lib/features/reservations/state.ts
export interface ReservationsState {
  // Active reservation flow
  currentReservation: ReservationFlow | null;

  // User's reservations
  myReservations: Reservation[];
  isLoading: boolean;
  error: ReservationError | null;
}

export interface ReservationFlow {
  step: 'select-tickets' | 'confirm' | 'payment' | 'complete';
  eventId: string;
  selectedSeats: SelectedSeat[];
  reservationId: string | null;
  expiresAt: Date | null;
}
```

### 3.3 Action Definitions

```typescript
// src/lib/features/events/action.ts
export type EventsAction =
  // List operations
  | { type: 'events/loadList' }
  | { type: 'events/listLoaded'; events: Event[]; pagination: Pagination }
  | { type: 'events/listFailed'; error: EventsError }

  // Single event
  | { type: 'events/loadDetail'; eventId: string }
  | { type: 'events/detailLoaded'; event: Event; availability: SectionAvailability[] }
  | { type: 'events/detailFailed'; error: EventsError }

  // Filters
  | { type: 'events/setFilter'; filter: Partial<EventFilters> }
  | { type: 'events/clearFilters' }

  // Pagination
  | { type: 'events/loadPage'; page: number }

  // Real-time updates
  | { type: 'events/availabilityUpdated'; eventId: string; availability: SectionAvailability[] };

// src/lib/features/reservations/action.ts
export type ReservationsAction =
  // Start reservation
  | { type: 'reservations/startFlow'; eventId: string }
  | { type: 'reservations/cancelFlow' }

  // Seat selection
  | { type: 'reservations/selectSeat'; seat: SelectedSeat }
  | { type: 'reservations/deselectSeat'; seatId: string }
  | { type: 'reservations/clearSelection' }

  // Create reservation
  | { type: 'reservations/create' }
  | { type: 'reservations/created'; reservation: Reservation }
  | { type: 'reservations/createFailed'; error: ReservationError }

  // Expiration
  | { type: 'reservations/expired' }

  // User's reservations
  | { type: 'reservations/loadMine' }
  | { type: 'reservations/mineLoaded'; reservations: Reservation[] }
  | { type: 'reservations/cancel'; reservationId: string }
  | { type: 'reservations/cancelled'; reservationId: string };
```

### 3.4 Reducer with Effects

```typescript
// src/lib/features/events/reducer.ts
import { Effect } from '@composable-svelte/core';
import type { Reducer } from '@composable-svelte/core';
import type { EventsState } from './state';
import type { EventsAction } from './action';
import type { Dependencies } from '$lib/app/dependencies';

export const eventsReducer: Reducer<EventsState, EventsAction, Dependencies> = (
  state,
  action,
  deps
) => {
  switch (action.type) {
    case 'events/loadList': {
      return [
        { ...state, isLoading: true, error: null },
        Effect.run(async (dispatch) => {
          const result = await deps.api.get('/api/v2/events', {
            params: state.filters
          });

          if (result.ok) {
            dispatch({
              type: 'events/listLoaded',
              events: result.data.events,
              pagination: result.data.pagination
            });
          } else {
            dispatch({
              type: 'events/listFailed',
              error: { code: 'LOAD_FAILED', message: result.error.message }
            });
          }
        })
      ];
    }

    case 'events/listLoaded': {
      return [
        {
          ...state,
          events: action.events,
          pagination: { ...state.pagination, ...action.pagination },
          isLoading: false
        },
        Effect.none()
      ];
    }

    case 'events/loadDetail': {
      return [
        { ...state, isLoading: true, selectedEvent: null, availability: null },
        Effect.batch(
          // Load event details
          Effect.run(async (dispatch) => {
            const eventResult = await deps.api.get(`/api/v2/events/${action.eventId}`);
            const availResult = await deps.api.get(`/api/v2/events/${action.eventId}/availability`);

            if (eventResult.ok && availResult.ok) {
              dispatch({
                type: 'events/detailLoaded',
                event: eventResult.data,
                availability: availResult.data.sections
              });
            } else {
              dispatch({
                type: 'events/detailFailed',
                error: { code: 'LOAD_FAILED', message: 'Failed to load event' }
              });
            }
          }),
          // Subscribe to real-time availability updates
          Effect.subscription('availability-updates', (dispatch) => {
            const unsubscribe = deps.websocket.subscribe(
              `events.${action.eventId}.availability`,
              (data) => {
                dispatch({
                  type: 'events/availabilityUpdated',
                  eventId: action.eventId,
                  availability: data.availability
                });
              }
            );
            return unsubscribe;
          })
        )
      ];
    }

    // ... more cases

    default:
      return [state, Effect.none()];
  }
};
```

---

## 4. Reducer Composition

### 4.1 Root Reducer

```typescript
// src/lib/app/reducer.ts
import { integrate, scope, ifLet } from '@composable-svelte/core';
import { authReducer } from '$lib/features/auth';
import { eventsReducer } from '$lib/features/events';
import { reservationsReducer } from '$lib/features/reservations';
import { paymentsReducer } from '$lib/features/payments';
import { dashboardReducer } from '$lib/features/dashboard';
import { organizerReducer } from '$lib/features/organizer';
import { uiReducer } from './ui-reducer';

export const appReducer = integrate()
  .with('auth', authReducer)
  .with('events', eventsReducer)
  .with('reservations', reservationsReducer)
  .with('payments', paymentsReducer)
  .with('dashboard', dashboardReducer)
  .with('organizer', organizerReducer)
  .with('ui', uiReducer)
  // Navigation presentations
  .with('eventDetail', ifLet(
    (state) => state.eventDetail,
    eventDetailReducer
  ))
  .with('checkoutSheet', ifLet(
    (state) => state.checkoutSheet,
    checkoutReducer
  ))
  .build();
```

### 4.2 Store Creation

```typescript
// src/lib/app/store.ts
import { createStore } from '@composable-svelte/core';
import { appReducer } from './reducer';
import { createDependencies } from './dependencies';
import { initialAppState } from './state';

export function createAppStore() {
  const dependencies = createDependencies();

  return createStore({
    initialState: initialAppState,
    reducer: appReducer,
    dependencies
  });
}

export type AppStore = ReturnType<typeof createAppStore>;
```

---

## 5. Dependencies

### 5.1 Dependency Container

```typescript
// src/lib/app/dependencies.ts
import { createAPIClient } from '@composable-svelte/core';
import { createLiveWebSocket } from '@composable-svelte/core';
import { createSystemClock, createLocalStorage } from '@composable-svelte/core';

export interface Dependencies {
  api: ReturnType<typeof createAPIClient>;
  websocket: ReturnType<typeof createLiveWebSocket>;
  clock: ReturnType<typeof createSystemClock>;
  storage: ReturnType<typeof createLocalStorage>;
}

export function createDependencies(): Dependencies {
  const storage = createLocalStorage();

  const api = createAPIClient({
    baseURL: import.meta.env.VITE_API_URL || 'http://localhost:8080',
    interceptors: {
      request: async (config) => {
        // Add auth token from storage
        const token = storage.get('auth_token');
        if (token) {
          config.headers = {
            ...config.headers,
            Authorization: `Bearer ${token}`
          };
        }
        return config;
      },
      error: async (error) => {
        // Handle 401 - redirect to login
        if (error.status === 401) {
          storage.remove('auth_token');
          window.location.href = '/auth/login';
        }
        return error;
      }
    }
  });

  const websocket = createLiveWebSocket({
    url: import.meta.env.VITE_WS_URL || 'ws://localhost:8080/ws',
    reconnect: true,
    heartbeat: { interval: 30000 }
  });

  return {
    api,
    websocket,
    clock: createSystemClock(),
    storage
  };
}
```

---

## 6. Navigation & Routing

### 6.1 Route Structure

| Route | Description | Auth Required |
|-------|-------------|---------------|
| `/` | Home / Event listing | No |
| `/events` | Browse all events | No |
| `/events/[id]` | Event detail | No |
| `/events/[id]/reserve` | Reservation flow | Yes |
| `/auth/login` | Magic link login | No |
| `/auth/verify` | Verify magic link | No |
| `/dashboard` | User dashboard | Yes |
| `/dashboard/reservations` | My reservations | Yes |
| `/dashboard/payments` | Payment history | Yes |
| `/organizer` | My events (organizer) | Yes |
| `/organizer/create` | Create event | Yes |
| `/organizer/[id]` | Edit event | Yes |
| `/organizer/[id]/analytics` | Event analytics | Yes |

### 6.2 Navigation Presentations

```typescript
// Modal for event quick view from listing
eventDetail: EventDetailState | null;
eventDetailPresentation: PresentationState<EventDetailState>;

// Sheet for checkout flow
checkoutSheet: CheckoutState | null;
checkoutPresentation: PresentationState<CheckoutState>;

// Alert for confirmations
confirmationAlert: ConfirmationState | null;
```

### 6.3 SvelteKit Integration

```typescript
// src/routes/+layout.ts
import { browser } from '$app/environment';
import { createAppStore } from '$lib/app/store';

export const load = async () => {
  // Create store on server and client
  const store = createAppStore();

  // Hydrate auth state from storage on client
  if (browser) {
    const token = localStorage.getItem('auth_token');
    if (token) {
      store.dispatch({ type: 'auth/hydrate', token });
    }
  }

  return { store };
};
```

---

## 7. API Integration

### 7.1 Typed Endpoints

```typescript
// src/lib/api/endpoints/events.ts
import type { Event, CreateEventRequest, UpdateEventRequest } from '../types';

export const eventsEndpoints = {
  list: (params?: { page?: number; limit?: number; status?: string }) =>
    ({ method: 'GET', path: '/api/v2/events', params }) as const,

  get: (id: string) =>
    ({ method: 'GET', path: `/api/v2/events/${id}` }) as const,

  create: (data: CreateEventRequest) =>
    ({ method: 'POST', path: '/api/v2/events', body: data }) as const,

  update: (id: string, data: UpdateEventRequest) =>
    ({ method: 'PUT', path: `/api/v2/events/${id}`, body: data }) as const,

  delete: (id: string) =>
    ({ method: 'DELETE', path: `/api/v2/events/${id}` }) as const,

  publish: (id: string) =>
    ({ method: 'POST', path: `/api/v2/events/${id}/publish` }) as const,

  cancel: (id: string) =>
    ({ method: 'POST', path: `/api/v2/events/${id}/cancel` }) as const,

  getAvailability: (id: string) =>
    ({ method: 'GET', path: `/api/v2/events/${id}/availability` }) as const,

  myEvents: () =>
    ({ method: 'GET', path: '/api/v2/my-events' }) as const,
};
```

### 7.2 API Types (matching backend)

```typescript
// src/lib/api/types.ts

// Events
export interface Event {
  id: string;
  title: string;
  description: string | null;
  venue: Venue;
  start_time: string;
  end_time: string | null;
  status: 'draft' | 'published' | 'cancelled';
  pricing_tiers: PricingTier[];
  owner_id: string;
  created_at: string;
  updated_at: string;
}

export interface Venue {
  name: string;
  address: string;
  sections: VenueSection[];
}

export interface VenueSection {
  name: string;
  capacity: number;
  seat_type: 'numbered' | 'general_admission';
}

export interface PricingTier {
  tier_type: 'vip' | 'standard' | 'early_bird' | 'group';
  name: string;
  price_cents: number;
  quantity_available: number | null;
}

// Availability
export interface SectionAvailability {
  section: string;
  total_capacity: number;
  available: number;
  reserved: number;
  sold: number;
}

// Reservations
export interface Reservation {
  id: string;
  event_id: string;
  user_id: string;
  status: 'pending' | 'payment_pending' | 'confirmed' | 'cancelled' | 'expired';
  seats: ReservedSeat[];
  total_amount_cents: number;
  expires_at: string | null;
  created_at: string;
}

export interface ReservedSeat {
  seat_id: string;
  section: string;
  tier_type: string;
  price_cents: number;
}

// Payments
export interface Payment {
  id: string;
  reservation_id: string;
  customer_id: string;
  amount_cents: number;
  status: 'pending' | 'processing' | 'succeeded' | 'failed' | 'refunded';
  payment_method: PaymentMethod;
  created_at: string;
}

export interface PaymentMethod {
  type: 'credit_card' | 'apple_pay' | 'google_pay';
  last_four?: string;
}

// Auth
export interface User {
  id: string;
  email: string;
}
```

---

## 8. Real-time Updates

### 8.1 WebSocket Channels

| Channel | Events | Description |
|---------|--------|-------------|
| `events.{id}.availability` | `availability_updated` | Seat availability changes |
| `reservations.{id}` | `status_changed`, `expired` | Reservation status updates |
| `user.{id}.notifications` | `reservation_confirmed`, `payment_received` | User notifications |

### 8.2 WebSocket Integration

```typescript
// In reducer - subscribe to availability updates
Effect.subscription('availability', (dispatch) => {
  return deps.websocket.subscribe(`events.${eventId}.availability`, (data) => {
    dispatch({
      type: 'events/availabilityUpdated',
      eventId,
      availability: data.sections
    });
  });
});
```

---

## 9. Testing Strategy

### 9.1 Unit Tests (Reducers)

```typescript
// tests/unit/features/events.test.ts
import { describe, it, expect } from 'vitest';
import { createTestStore } from '@composable-svelte/core';
import { eventsReducer, initialEventsState } from '$lib/features/events';
import { createMockAPI } from '@composable-svelte/core';

describe('eventsReducer', () => {
  it('loads event list successfully', async () => {
    const mockAPI = createMockAPI();
    mockAPI.mock('GET', '/api/v2/events', {
      events: [{ id: '1', title: 'Concert' }],
      pagination: { page: 1, total: 1 }
    });

    const store = createTestStore({
      initialState: initialEventsState,
      reducer: eventsReducer,
      dependencies: { api: mockAPI }
    });

    await store.send({ type: 'events/loadList' }, (state) => {
      expect(state.isLoading).toBe(true);
    });

    await store.receive({ type: 'events/listLoaded' }, (state) => {
      expect(state.events).toHaveLength(1);
      expect(state.events[0].title).toBe('Concert');
      expect(state.isLoading).toBe(false);
    });

    store.assertNoPendingActions();
  });
});
```

### 9.2 Integration Tests (Flows)

```typescript
// tests/integration/flows/reservation-flow.test.ts
import { describe, it, expect } from 'vitest';
import { createTestStore } from '@composable-svelte/core';
import { appReducer, initialAppState } from '$lib/app';

describe('Reservation Flow', () => {
  it('completes full reservation and payment flow', async () => {
    const store = createTestStore({
      initialState: {
        ...initialAppState,
        auth: { ...initialAppState.auth, isAuthenticated: true, user: mockUser }
      },
      reducer: appReducer,
      dependencies: createMockDependencies()
    });

    // Start reservation
    await store.send({ type: 'reservations/startFlow', eventId: 'event-1' });

    // Select seats
    await store.send({ type: 'reservations/selectSeat', seat: mockSeat });

    // Create reservation
    await store.send({ type: 'reservations/create' });
    await store.receive({ type: 'reservations/created' });

    // Process payment
    await store.send({ type: 'payments/process', paymentMethod: mockPaymentMethod });
    await store.receive({ type: 'payments/succeeded' });

    // Verify final state
    expect(store.state.reservations.currentReservation?.step).toBe('complete');
  });
});
```

---

## 10. Implementation Phases

### Phase 1: Foundation (Week 1)
- [ ] SvelteKit project setup
- [ ] composable-svelte integration
- [ ] Tailwind + shadcn-svelte setup
- [ ] API client configuration
- [ ] Basic routing structure

### Phase 2: Authentication (Week 1)
- [ ] Auth feature module (state, actions, reducer)
- [ ] Magic link request page
- [ ] Magic link verification
- [ ] Auth state persistence
- [ ] Protected route guards

### Phase 3: Event Browsing (Week 2)
- [ ] Events feature module
- [ ] Event listing page with filters
- [ ] Event detail page
- [ ] Availability display
- [ ] Pagination

### Phase 4: Reservation Flow (Week 2-3)
- [ ] Reservations feature module
- [ ] Seat selection UI
- [ ] Reservation creation
- [ ] Timer countdown for expiration
- [ ] Checkout sheet presentation

### Phase 5: Payments (Week 3)
- [ ] Payments feature module
- [ ] Payment form (mock)
- [ ] Payment confirmation
- [ ] Receipt display

### Phase 6: User Dashboard (Week 4)
- [ ] Dashboard feature module
- [ ] My reservations list
- [ ] Payment history
- [ ] Reservation cancellation

### Phase 7: Organizer Features (Week 4-5)
- [ ] Organizer feature module
- [ ] Event creation form
- [ ] Event editing
- [ ] Analytics dashboard

### Phase 8: Real-time & Polish (Week 5)
- [ ] WebSocket integration
- [ ] Real-time availability updates
- [ ] Toast notifications
- [ ] Error handling improvements
- [ ] Loading states
- [ ] Accessibility audit

---

## 11. Environment Configuration

```bash
# .env
VITE_API_URL=http://localhost:8080
VITE_WS_URL=ws://localhost:8080/ws
```

```bash
# .env.production
VITE_API_URL=https://api.ticketing.example.com
VITE_WS_URL=wss://api.ticketing.example.com/ws
```

---

## 12. Key Decisions

1. **SvelteKit over plain Svelte**: Better routing, SSR support, file-based routing
2. **Feature-based structure**: Collocate state/actions/reducer/components per feature
3. **composable-svelte for state**: Same TCA pattern as backend, proven test infrastructure
4. **shadcn-svelte components**: Already integrated in composable-svelte, accessible
5. **Effect-based side effects**: All API calls through Effects for testability
6. **WebSocket for real-time**: Availability updates, reservation status changes

---

## 13. Success Criteria

- [ ] All pages functional with working API integration
- [ ] Full reservation flow works end-to-end
- [ ] Real-time availability updates work
- [ ] 80%+ test coverage on reducers
- [ ] Accessible (WCAG 2.1 AA)
- [ ] Mobile responsive
- [ ] Loading/error states handled gracefully
