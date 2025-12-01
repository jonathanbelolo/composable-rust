/**
 * Ticketing Frontend Types
 *
 * Type definitions matching the backend API responses.
 */

// ============================================================================
// API Types (matching backend)
// ============================================================================

/** Full event detail (from GET /api/v2/events/:id) */
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

/** Event summary for list view (from GET /api/v2/events) */
export interface EventSummary {
  id: string;
  name: string;
  owner_id: string;
  venue_name: string;
  date: string;
  status: 'Draft' | 'Published' | 'Cancelled';
  pricing_tiers_count: number;
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

export interface SectionAvailability {
  section: string;
  total_capacity: number;
  available: number;
  reserved: number;
  sold: number;
}

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

export interface User {
  id: string;
  email: string;
}

// ============================================================================
// App State
// ============================================================================

// All destinations must have a `state` property to satisfy the ParserConfig/SerializerConfig constraint
export type AppDestination =
  | { type: 'home'; state: Record<string, never> }
  | { type: 'events'; state: Record<string, never> }
  | { type: 'eventDetail'; state: { eventId: string } }
  | { type: 'reserve'; state: { eventId: string } }
  | { type: 'login'; state: Record<string, never> }
  | { type: 'verify'; state: Record<string, never> }
  | { type: 'dashboard'; state: Record<string, never> }
  | { type: 'myReservations'; state: Record<string, never> }
  | { type: 'myPayments'; state: Record<string, never> }
  | { type: 'organizerEvents'; state: Record<string, never> }
  | { type: 'createEvent'; state: Record<string, never> }
  | { type: 'editEvent'; state: { eventId: string } }
  | { type: 'eventAnalytics'; state: { eventId: string } };

// Helper to create destinations without needing to specify empty state
export const Destination = {
  home: (): AppDestination => ({ type: 'home', state: {} }),
  events: (): AppDestination => ({ type: 'events', state: {} }),
  eventDetail: (eventId: string): AppDestination => ({ type: 'eventDetail', state: { eventId } }),
  reserve: (eventId: string): AppDestination => ({ type: 'reserve', state: { eventId } }),
  login: (): AppDestination => ({ type: 'login', state: {} }),
  verify: (): AppDestination => ({ type: 'verify', state: {} }),
  dashboard: (): AppDestination => ({ type: 'dashboard', state: {} }),
  myReservations: (): AppDestination => ({ type: 'myReservations', state: {} }),
  myPayments: (): AppDestination => ({ type: 'myPayments', state: {} }),
  organizerEvents: (): AppDestination => ({ type: 'organizerEvents', state: {} }),
  createEvent: (): AppDestination => ({ type: 'createEvent', state: {} }),
  editEvent: (eventId: string): AppDestination => ({ type: 'editEvent', state: { eventId } }),
  eventAnalytics: (eventId: string): AppDestination => ({ type: 'eventAnalytics', state: { eventId } })
};

export interface AppState {
  // Current destination (routing)
  destination: AppDestination;

  // Auth state
  auth: AuthState;

  // Events
  events: EventSummary[];
  selectedEvent: Event | null;
  availability: SectionAvailability[] | null;
  eventsLoading: boolean;
  eventsError: string | null;

  // Reservations
  reservations: Reservation[];
  currentReservation: ReservationFlow | null;
  reservationsLoading: boolean;
  reservationsError: string | null;

  // Payments
  payments: Payment[];
  paymentsLoading: boolean;
  paymentsError: string | null;

  // Meta (for SSR)
  meta: MetaState;

  // UI state
  ui: UIState;
}

export interface AuthState {
  user: User | null;
  token: string | null;
  isAuthenticated: boolean;
  isLoading: boolean;
  error: string | null;
  magicLinkSent: boolean;
}

export interface ReservationFlow {
  step: 'select-tickets' | 'confirm' | 'payment' | 'complete';
  eventId: string;
  selectedSeats: SelectedSeat[];
  reservationId: string | null;
  expiresAt: string | null;
}

export interface SelectedSeat {
  seatId: string;
  section: string;
  tierType: string;
  priceCents: number;
}

export interface MetaState {
  title: string;
  description: string;
  canonical?: string;
  ogImage?: string;
}

export interface UIState {
  toasts: Toast[];
  isLoading: boolean;
}

export interface Toast {
  id: string;
  message: string;
  type: 'success' | 'error' | 'info' | 'warning';
}

// ============================================================================
// Initial State
// ============================================================================

export const initialAuthState: AuthState = {
  user: null,
  token: null,
  isAuthenticated: false,
  isLoading: false,
  error: null,
  magicLinkSent: false
};

export const initialState: AppState = {
  destination: { type: 'home', state: {} },
  auth: initialAuthState,
  events: [],
  selectedEvent: null,
  availability: null,
  eventsLoading: false,
  eventsError: null,
  reservations: [],
  currentReservation: null,
  reservationsLoading: false,
  reservationsError: null,
  payments: [],
  paymentsLoading: false,
  paymentsError: null,
  meta: {
    title: 'Ticketing - Event Tickets Made Easy',
    description: 'Find and book tickets for your favorite events'
  },
  ui: {
    toasts: [],
    isLoading: false
  }
};
