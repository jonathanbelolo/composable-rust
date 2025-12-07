/**
 * Main app reducer for the ticketing frontend.
 *
 * This is a TCA-style reducer: (state, action, deps) => [newState, effect]
 */

import { Effect } from '@composable-svelte/core';
import type { Reducer, EffectType } from '@composable-svelte/core';
import type {
  AppState,
  AppDestination,
  Event,
  EventSummary,
  Reservation,
  CreateReservationResponse,
  Payment,
  SectionAvailability,
  User,
  SelectedSeat,
  EventFormData,
  CreateEventApiResponse,
  UpdateEventApiResponse
} from './types';
import { initialEventForm } from './types';

// ============================================================================
// Actions
// ============================================================================

export type AppAction =
  // Navigation
  | { type: 'navigate'; destination: AppDestination }

  // Auth
  | { type: 'auth/requestMagicLink'; email: string }
  | { type: 'auth/magicLinkSent'; magicLink?: string }
  | { type: 'auth/verifyToken'; token: string }
  | { type: 'auth/verified'; user: User; token: string }
  | { type: 'auth/failed'; error: string }
  | { type: 'auth/logout' }
  | { type: 'auth/hydrate'; token: string; user: User | null }

  // Events
  | { type: 'events/loadList' }
  | { type: 'events/listLoaded'; events: EventSummary[] }
  | { type: 'events/listFailed'; error: string }
  | { type: 'events/loadDetail'; eventId: string }
  | { type: 'events/detailLoaded'; event: Event; availability: SectionAvailability[] }
  | { type: 'events/detailFailed'; error: string }
  | { type: 'events/availabilityUpdated'; eventId: string; availability: SectionAvailability[] }

  // Reservations
  | { type: 'reservations/startFlow'; eventId: string }
  | { type: 'reservations/selectSeat'; seat: SelectedSeat }
  | { type: 'reservations/deselectSeat'; seatId: string }
  | { type: 'reservations/clearSelection' }
  | { type: 'reservations/create' }
  | { type: 'reservations/created'; response: CreateReservationResponse }
  | { type: 'reservations/createFailed'; error: string }
  | { type: 'reservations/loadMine' }
  | { type: 'reservations/mineLoaded'; reservations: Reservation[] }
  | { type: 'reservations/cancel'; reservationId: string }
  | { type: 'reservations/cancelled'; reservationId: string }
  | { type: 'reservations/expired' }

  // Payments
  | { type: 'payments/process'; reservationId: string; paymentMethod: string }
  | { type: 'payments/succeeded'; payment: Payment }
  | { type: 'payments/failed'; error: string }
  | { type: 'payments/loadHistory' }
  | { type: 'payments/historyLoaded'; payments: Payment[] }

  // UI
  | { type: 'ui/showToast'; message: string; toastType: 'success' | 'error' | 'info' | 'warning' }
  | { type: 'ui/dismissToast'; id: string }
  | { type: 'ui/setLoading'; isLoading: boolean }

  // Organizer
  | { type: 'organizer/loadMyEvents' }
  | { type: 'organizer/myEventsLoaded'; events: EventSummary[] }
  | { type: 'organizer/myEventsFailed'; error: string }
  | { type: 'organizer/updateForm'; field: keyof EventFormData; value: string | number }
  | { type: 'organizer/resetForm' }
  | { type: 'organizer/loadEventForEdit'; eventId: string }
  | { type: 'organizer/eventLoadedForEdit'; event: Event }
  | { type: 'organizer/createEvent' }
  | { type: 'organizer/eventCreated'; eventId: string }
  | { type: 'organizer/createFailed'; error: string }
  | { type: 'organizer/updateEvent'; eventId: string }
  | { type: 'organizer/eventUpdated'; eventId: string }
  | { type: 'organizer/updateFailed'; error: string }
  | { type: 'organizer/publishEvent'; eventId: string }
  | { type: 'organizer/eventPublished'; eventId: string }
  | { type: 'organizer/cancelEvent'; eventId: string }
  | { type: 'organizer/eventCancelled'; eventId: string }
  | { type: 'organizer/deleteEvent'; eventId: string }
  | { type: 'organizer/eventDeleted'; eventId: string };

// ============================================================================
// Dependencies
// ============================================================================

export interface AppDependencies {
  api: {
    get: <T>(url: string) => Promise<{ ok: boolean; data?: T; error?: { message: string } }>;
    post: <T>(url: string, body?: unknown) => Promise<{ ok: boolean; data?: T; error?: { message: string } }>;
    put: <T>(url: string, body?: unknown) => Promise<{ ok: boolean; data?: T; error?: { message: string } }>;
    delete: <T>(url: string) => Promise<{ ok: boolean; data?: T; error?: { message: string } }>;
  };
  storage: {
    getItem: (key: string) => string | null;
    setItem: (key: string, value: string) => void;
    removeItem: (key: string) => void;
  };
}

// ============================================================================
// Reducer
// ============================================================================

export const appReducer: Reducer<AppState, AppAction, AppDependencies> = (
  state,
  action,
  deps
): readonly [AppState, EffectType<AppAction>] => {
  switch (action.type) {
    // -------------------------------------------------------------------------
    // Navigation
    // -------------------------------------------------------------------------
    case 'navigate': {
      const newState = { ...state, destination: action.destination };
      const destType = action.destination.type;

      // Trigger data loading effects based on destination
      // This keeps all side effect logic in the reducer where it belongs
      if ((destType === 'dashboard' || destType === 'myReservations') && state.auth.isAuthenticated) {
        // Load reservations AND events when navigating to dashboard/myReservations
        // Events are needed to display event names in the reservations list
        return [
          { ...newState, reservationsLoading: true, eventsLoading: true },
          Effect.run(async (dispatch) => {
            // Load both in parallel
            const [reservationsResult, eventsResult] = await Promise.all([
              deps.api.get<{ reservations: Reservation[]; total: number }>('/api/v2/reservations'),
              deps.api.get<{ events: EventSummary[] }>('/api/v2/events')
            ]);

            if (reservationsResult.ok && reservationsResult.data) {
              dispatch({ type: 'reservations/mineLoaded', reservations: reservationsResult.data.reservations });
            } else {
              dispatch({ type: 'reservations/mineLoaded', reservations: [] });
            }

            if (eventsResult.ok && eventsResult.data) {
              dispatch({ type: 'events/listLoaded', events: eventsResult.data.events });
            }
          })
        ];
      }

      if (destType === 'myPayments' && state.auth.isAuthenticated) {
        // Load payment history when navigating to myPayments
        const userId = state.auth.user?.id;
        if (!userId) {
          return [{ ...newState, paymentsLoading: false, payments: [] }, Effect.none()];
        }
        return [
          { ...newState, paymentsLoading: true },
          Effect.run(async (dispatch) => {
            const result = await deps.api.get<{ payments: Payment[]; total: number }>(
              `/api/v2/customers/${userId}/payments`
            );
            if (result.ok && result.data) {
              dispatch({ type: 'payments/historyLoaded', payments: result.data.payments });
            } else {
              dispatch({ type: 'payments/historyLoaded', payments: [] });
            }
          })
        ];
      }

      if (destType === 'organizerEvents' && state.auth.isAuthenticated) {
        // Load organizer events when navigating to organizer page
        return [
          { ...newState, organizer: { ...state.organizer, myEventsLoading: true, myEventsError: null } },
          Effect.run(async (dispatch) => {
            const result = await deps.api.get<{ events: EventSummary[] }>('/api/v2/events');
            if (result.ok && result.data) {
              const userId = state.auth.user?.id;
              const myEvents = userId
                ? result.data.events.filter((e) => e.owner_id === userId)
                : [];
              dispatch({ type: 'organizer/myEventsLoaded', events: myEvents });
            } else {
              dispatch({
                type: 'organizer/myEventsFailed',
                error: 'Failed to load events'
              });
            }
          })
        ];
      }

      return [newState, Effect.none()];
    }

    // -------------------------------------------------------------------------
    // Auth
    // -------------------------------------------------------------------------
    case 'auth/requestMagicLink': {
      return [
        { ...state, auth: { ...state.auth, isLoading: true, error: null, testMagicLink: null } },
        Effect.run(async (dispatch) => {
          interface MagicLinkResponse {
            message: string;
            magic_link_token?: string; // Only present when AUTH_EXPOSE_MAGIC_LINKS_FOR_TESTING=true
          }
          const result = await deps.api.post<MagicLinkResponse>('/auth/magic-link/request', { email: action.email });
          if (result.ok) {
            // Build full magic link URL if token is present (testing mode)
            const token = result.data?.magic_link_token;
            const magicLink = token ? `/auth/magic-link/verify?token=${token}` : undefined;
            dispatch({ type: 'auth/magicLinkSent', magicLink });
          } else {
            dispatch({ type: 'auth/failed', error: result.error?.message ?? 'Failed to send magic link' });
          }
        })
      ];
    }

    case 'auth/magicLinkSent': {
      const toast = {
        id: crypto.randomUUID(),
        message: action.magicLink
          ? 'Magic link generated! Click it below to sign in.'
          : 'Check your email for the magic link!',
        type: 'success' as const
      };
      return [
        {
          ...state,
          auth: {
            ...state.auth,
            isLoading: false,
            magicLinkSent: true,
            testMagicLink: action.magicLink ?? null
          },
          ui: { ...state.ui, toasts: [...state.ui.toasts, toast] }
        },
        Effect.none()
      ];
    }

    case 'auth/verifyToken': {
      return [
        { ...state, auth: { ...state.auth, isLoading: true, error: null } },
        Effect.run(async (dispatch) => {
          // Backend returns: { session_id, session_token, user_id, email, expires_at }
          interface VerifyResponse {
            session_id: string;
            session_token: string;
            user_id: string;
            email: string;
            expires_at: string;
          }
          const result = await deps.api.post<VerifyResponse>('/auth/magic-link/verify', {
            token: action.token
          });
          if (result.ok && result.data) {
            // Store the session token and user info for persistence
            deps.storage.setItem('auth_token', result.data.session_token);
            // Create user object from response - use user_id which persists across sessions
            const user: User = {
              id: result.data.user_id,
              email: result.data.email
            };
            // Also store user info for hydration on page reload
            deps.storage.setItem('auth_user', JSON.stringify(user));
            dispatch({ type: 'auth/verified', user, token: result.data.session_token });
          } else {
            dispatch({ type: 'auth/failed', error: result.error?.message ?? 'Invalid or expired token' });
          }
        })
      ];
    }

    case 'auth/verified': {
      const toast = {
        id: crypto.randomUUID(),
        message: 'Welcome back!',
        type: 'success' as const
      };
      return [
        {
          ...state,
          auth: {
            ...state.auth,
            user: action.user,
            token: action.token,
            isAuthenticated: true,
            isLoading: false,
            error: null
          },
          destination: { type: 'dashboard', state: {} },
          ui: { ...state.ui, toasts: [...state.ui.toasts, toast] }
        },
        Effect.none()
      ];
    }

    case 'auth/failed': {
      const toast = {
        id: crypto.randomUUID(),
        message: action.error,
        type: 'error' as const
      };
      return [
        {
          ...state,
          auth: { ...state.auth, isLoading: false, error: action.error },
          ui: { ...state.ui, toasts: [...state.ui.toasts, toast] }
        },
        Effect.none()
      ];
    }

    case 'auth/logout': {
      deps.storage.removeItem('auth_token');
      deps.storage.removeItem('auth_user');
      return [
        {
          ...state,
          auth: {
            user: null,
            token: null,
            isAuthenticated: false,
            isLoading: false,
            error: null,
            magicLinkSent: false,
            testMagicLink: null
          },
          destination: { type: 'home', state: {} }
        },
        Effect.none()
      ];
    }

    case 'auth/hydrate': {
      const newAuthState = {
        ...state,
        auth: {
          ...state.auth,
          token: action.token,
          user: action.user,
          isAuthenticated: true
        }
      };

      // After hydrating auth, check if current destination needs data loading
      // This handles the page refresh case where navigate action wasn't dispatched
      const destType = state.destination.type;

      if (destType === 'dashboard' || destType === 'myReservations') {
        // Load reservations AND events for dashboard/myReservations pages
        // Events are needed to display event names in the reservations list
        return [
          { ...newAuthState, reservationsLoading: true, eventsLoading: true },
          Effect.run(async (dispatch) => {
            // Load both in parallel
            const [reservationsResult, eventsResult] = await Promise.all([
              deps.api.get<{ reservations: Reservation[]; total: number }>('/api/v2/reservations'),
              deps.api.get<{ events: EventSummary[] }>('/api/v2/events')
            ]);

            if (reservationsResult.ok && reservationsResult.data) {
              dispatch({ type: 'reservations/mineLoaded', reservations: reservationsResult.data.reservations });
            } else {
              dispatch({ type: 'reservations/mineLoaded', reservations: [] });
            }

            if (eventsResult.ok && eventsResult.data) {
              dispatch({ type: 'events/listLoaded', events: eventsResult.data.events });
            }
          })
        ];
      }

      if (destType === 'organizerEvents') {
        // Load organizer events
        return [
          { ...newAuthState, organizer: { ...state.organizer, myEventsLoading: true, myEventsError: null } },
          Effect.run(async (dispatch) => {
            const result = await deps.api.get<{ events: EventSummary[] }>('/api/v2/events');
            if (result.ok && result.data) {
              const userId = action.user?.id;
              const myEvents = userId
                ? result.data.events.filter((e) => e.owner_id === userId)
                : [];
              dispatch({ type: 'organizer/myEventsLoaded', events: myEvents });
            } else {
              dispatch({
                type: 'organizer/myEventsFailed',
                error: 'Failed to load events'
              });
            }
          })
        ];
      }

      if (destType === 'myPayments') {
        // Load payment history
        const userId = action.user?.id;
        if (!userId) {
          return [{ ...newAuthState, paymentsLoading: false, payments: [] }, Effect.none()];
        }
        return [
          { ...newAuthState, paymentsLoading: true },
          Effect.run(async (dispatch) => {
            const result = await deps.api.get<{ payments: Payment[]; total: number }>(
              `/api/v2/customers/${userId}/payments`
            );
            if (result.ok && result.data) {
              dispatch({ type: 'payments/historyLoaded', payments: result.data.payments });
            } else {
              dispatch({ type: 'payments/historyLoaded', payments: [] });
            }
          })
        ];
      }

      return [newAuthState, Effect.none()];
    }

    // -------------------------------------------------------------------------
    // Events
    // -------------------------------------------------------------------------
    case 'events/loadList': {
      return [
        { ...state, eventsLoading: true, eventsError: null },
        Effect.run(async (dispatch) => {
          const result = await deps.api.get<{ events: EventSummary[] }>('/api/v2/events');
          if (result.ok && result.data) {
            dispatch({ type: 'events/listLoaded', events: result.data.events });
          } else {
            dispatch({ type: 'events/listFailed', error: result.error?.message ?? 'Failed to load events' });
          }
        })
      ];
    }

    case 'events/listLoaded': {
      return [
        { ...state, events: action.events, eventsLoading: false },
        Effect.none()
      ];
    }

    case 'events/listFailed': {
      return [
        { ...state, eventsLoading: false, eventsError: action.error },
        Effect.none()
      ];
    }

    case 'events/loadDetail': {
      return [
        { ...state, eventsLoading: true, selectedEvent: null, availability: null },
        Effect.run(async (dispatch) => {
          const [eventResult, availResult] = await Promise.all([
            deps.api.get<Event>(`/api/v2/events/${action.eventId}`),
            deps.api.get<{ sections: SectionAvailability[] }>(`/api/v2/events/${action.eventId}/availability`)
          ]);

          if (eventResult.ok && eventResult.data && availResult.ok && availResult.data) {
            dispatch({
              type: 'events/detailLoaded',
              event: eventResult.data,
              availability: availResult.data.sections
            });
          } else {
            dispatch({
              type: 'events/detailFailed',
              error: eventResult.error?.message ?? availResult.error?.message ?? 'Failed to load event'
            });
          }
        })
      ];
    }

    case 'events/detailLoaded': {
      return [
        {
          ...state,
          selectedEvent: action.event,
          availability: action.availability,
          eventsLoading: false,
          meta: {
            title: `${action.event.title} - Ticketing`,
            description: action.event.description ?? `Get tickets for ${action.event.title}`,
            canonical: `/events/${action.event.id}`
          }
        },
        Effect.none()
      ];
    }

    case 'events/detailFailed': {
      // If user is not authenticated, redirect to login instead of showing error
      if (!state.auth.isAuthenticated) {
        return [
          {
            ...state,
            eventsLoading: false,
            eventsError: null,
            destination: { type: 'login', state: {} }
          },
          Effect.none()
        ];
      }
      return [
        { ...state, eventsLoading: false, eventsError: action.error },
        Effect.none()
      ];
    }

    case 'events/availabilityUpdated': {
      if (state.selectedEvent?.id === action.eventId) {
        return [{ ...state, availability: action.availability }, Effect.none()];
      }
      return [state, Effect.none()];
    }

    // -------------------------------------------------------------------------
    // Reservations
    // -------------------------------------------------------------------------
    case 'reservations/startFlow': {
      return [
        {
          ...state,
          currentReservation: {
            step: 'select-tickets',
            eventId: action.eventId,
            selectedSeats: [],
            reservationId: null,
            expiresAt: null
          }
        },
        Effect.none()
      ];
    }

    case 'reservations/selectSeat': {
      if (!state.currentReservation) return [state, Effect.none()];
      return [
        {
          ...state,
          currentReservation: {
            ...state.currentReservation,
            selectedSeats: [...state.currentReservation.selectedSeats, action.seat]
          }
        },
        Effect.none()
      ];
    }

    case 'reservations/deselectSeat': {
      if (!state.currentReservation) return [state, Effect.none()];
      return [
        {
          ...state,
          currentReservation: {
            ...state.currentReservation,
            selectedSeats: state.currentReservation.selectedSeats.filter((s) => s.seatId !== action.seatId)
          }
        },
        Effect.none()
      ];
    }

    case 'reservations/clearSelection': {
      if (!state.currentReservation) return [state, Effect.none()];
      return [
        {
          ...state,
          currentReservation: {
            ...state.currentReservation,
            selectedSeats: []
          }
        },
        Effect.none()
      ];
    }

    case 'reservations/create': {
      if (!state.currentReservation) return [state, Effect.none()];
      const { eventId, selectedSeats } = state.currentReservation;

      // Get section from first selected seat (all seats in same reservation are same section)
      const section = selectedSeats.length > 0 ? selectedSeats[0].section : 'General';

      return [
        { ...state, reservationsLoading: true },
        Effect.run(async (dispatch) => {
          const result = await deps.api.post<CreateReservationResponse>('/api/v2/reservations', {
            event_id: eventId,
            section,
            quantity: selectedSeats.length
          });

          if (result.ok && result.data) {
            dispatch({ type: 'reservations/created', response: result.data });
          } else {
            dispatch({
              type: 'reservations/createFailed',
              error: result.error?.message ?? 'Failed to create reservation'
            });
          }
        })
      ];
    }

    case 'reservations/created': {
      if (!state.currentReservation) return [state, Effect.none()];
      // The backend saga automatically processes payment synchronously when a reservation
      // is created. The saga completes (including payment) before the HTTP response is sent,
      // so we can skip directly to the complete step.
      return [
        {
          ...state,
          reservationsLoading: false,
          currentReservation: {
            ...state.currentReservation,
            step: 'complete',
            reservationId: action.response.reservation_id,
            expiresAt: null // Expiration is handled server-side
          }
        },
        Effect.none()
      ];
    }

    case 'reservations/createFailed': {
      return [
        { ...state, reservationsLoading: false, reservationsError: action.error },
        Effect.none()
      ];
    }

    case 'reservations/loadMine': {
      return [
        { ...state, reservationsLoading: true },
        Effect.run(async (dispatch) => {
          const result = await deps.api.get<{ reservations: Reservation[]; total: number }>('/api/v2/reservations');
          if (result.ok && result.data) {
            dispatch({ type: 'reservations/mineLoaded', reservations: result.data.reservations });
          } else {
            // Clear loading state even on error
            dispatch({ type: 'reservations/mineLoaded', reservations: [] });
          }
        })
      ];
    }

    case 'reservations/mineLoaded': {
      return [
        { ...state, reservations: action.reservations, reservationsLoading: false },
        Effect.none()
      ];
    }

    case 'reservations/cancel': {
      return [
        { ...state, reservationsLoading: true },
        Effect.run(async (dispatch) => {
          const result = await deps.api.post(`/api/v2/reservations/${action.reservationId}/cancel`);
          if (result.ok) {
            dispatch({ type: 'reservations/cancelled', reservationId: action.reservationId });
          }
        })
      ];
    }

    case 'reservations/cancelled': {
      return [
        {
          ...state,
          reservations: state.reservations.map((r) =>
            r.id === action.reservationId ? { ...r, status: 'cancelled' as const } : r
          ),
          reservationsLoading: false
        },
        Effect.none()
      ];
    }

    case 'reservations/expired': {
      return [
        { ...state, currentReservation: null },
        Effect.none()
      ];
    }

    // -------------------------------------------------------------------------
    // Payments
    // -------------------------------------------------------------------------
    case 'payments/process': {
      return [
        { ...state, paymentsLoading: true },
        Effect.run(async (dispatch) => {
          const result = await deps.api.post<Payment>('/api/v2/payments', {
            reservation_id: action.reservationId,
            payment_method: action.paymentMethod
          });

          if (result.ok && result.data) {
            dispatch({ type: 'payments/succeeded', payment: result.data });
          } else {
            dispatch({ type: 'payments/failed', error: result.error?.message ?? 'Payment failed' });
          }
        })
      ];
    }

    case 'payments/succeeded': {
      if (!state.currentReservation) return [state, Effect.none()];
      return [
        {
          ...state,
          paymentsLoading: false,
          currentReservation: {
            ...state.currentReservation,
            step: 'complete'
          }
        },
        Effect.none()
      ];
    }

    case 'payments/failed': {
      return [
        { ...state, paymentsLoading: false, paymentsError: action.error },
        Effect.none()
      ];
    }

    case 'payments/loadHistory': {
      // Get user ID from auth state to use as customer_id
      const userId = state.auth.user?.id;
      if (!userId) {
        // Not authenticated, can't load payments
        return [
          { ...state, paymentsLoading: false, payments: [] },
          Effect.none()
        ];
      }

      return [
        { ...state, paymentsLoading: true },
        Effect.run(async (dispatch) => {
          const result = await deps.api.get<{ payments: Payment[]; total: number }>(
            `/api/v2/customers/${userId}/payments`
          );
          if (result.ok && result.data) {
            dispatch({ type: 'payments/historyLoaded', payments: result.data.payments });
          } else {
            dispatch({ type: 'payments/historyLoaded', payments: [] });
          }
        })
      ];
    }

    case 'payments/historyLoaded': {
      return [
        { ...state, payments: action.payments, paymentsLoading: false },
        Effect.none()
      ];
    }

    // -------------------------------------------------------------------------
    // UI
    // -------------------------------------------------------------------------
    case 'ui/showToast': {
      const toast = {
        id: crypto.randomUUID(),
        message: action.message,
        type: action.toastType
      };
      return [
        { ...state, ui: { ...state.ui, toasts: [...state.ui.toasts, toast] } },
        Effect.none()
      ];
    }

    case 'ui/dismissToast': {
      return [
        {
          ...state,
          ui: { ...state.ui, toasts: state.ui.toasts.filter((t) => t.id !== action.id) }
        },
        Effect.none()
      ];
    }

    case 'ui/setLoading': {
      return [
        { ...state, ui: { ...state.ui, isLoading: action.isLoading } },
        Effect.none()
      ];
    }

    // -------------------------------------------------------------------------
    // Organizer
    // -------------------------------------------------------------------------
    case 'organizer/loadMyEvents': {
      return [
        {
          ...state,
          organizer: { ...state.organizer, myEventsLoading: true, myEventsError: null }
        },
        Effect.run(async (dispatch) => {
          // Load all events - in production, would filter by owner_id on backend
          const result = await deps.api.get<{ events: EventSummary[] }>('/api/v2/events');
          if (result.ok && result.data) {
            // Filter to only show user's events (by owner_id matching session id)
            const userId = state.auth.user?.id;
            const myEvents = userId
              ? result.data.events.filter((e) => e.owner_id === userId)
              : [];
            dispatch({ type: 'organizer/myEventsLoaded', events: myEvents });
          } else {
            dispatch({
              type: 'organizer/myEventsFailed',
              error: result.error?.message ?? 'Failed to load events'
            });
          }
        })
      ];
    }

    case 'organizer/myEventsLoaded': {
      return [
        {
          ...state,
          organizer: {
            ...state.organizer,
            myEvents: action.events,
            myEventsLoading: false,
            myEventsError: null
          }
        },
        Effect.none()
      ];
    }

    case 'organizer/myEventsFailed': {
      const toast = {
        id: crypto.randomUUID(),
        message: action.error,
        type: 'error' as const
      };
      return [
        {
          ...state,
          organizer: {
            ...state.organizer,
            myEventsLoading: false,
            myEventsError: action.error
          },
          ui: { ...state.ui, toasts: [...state.ui.toasts, toast] }
        },
        Effect.none()
      ];
    }

    case 'organizer/updateForm': {
      return [
        {
          ...state,
          organizer: {
            ...state.organizer,
            eventForm: {
              ...state.organizer.eventForm,
              [action.field]: action.value
            }
          }
        },
        Effect.none()
      ];
    }

    case 'organizer/resetForm': {
      return [
        {
          ...state,
          organizer: {
            ...state.organizer,
            eventForm: initialEventForm,
            formError: null
          }
        },
        Effect.none()
      ];
    }

    case 'organizer/loadEventForEdit': {
      return [
        {
          ...state,
          organizer: { ...state.organizer, formLoading: true, formError: null }
        },
        Effect.run(async (dispatch) => {
          const result = await deps.api.get<Event>(`/api/v2/events/${action.eventId}`);
          if (result.ok && result.data) {
            dispatch({ type: 'organizer/eventLoadedForEdit', event: result.data });
          } else {
            dispatch({
              type: 'organizer/createFailed',
              error: result.error?.message ?? 'Failed to load event'
            });
          }
        })
      ];
    }

    case 'organizer/eventLoadedForEdit': {
      const event = action.event;
      return [
        {
          ...state,
          organizer: {
            ...state.organizer,
            formLoading: false,
            eventForm: {
              title: event.title,
              description: event.description ?? '',
              startTime: event.start_time.slice(0, 16), // Format for datetime-local input
              venueName: event.venue.name,
              capacity: event.venue.sections.reduce((sum, s) => sum + s.capacity, 0),
              price: event.pricing_tiers.length > 0
                ? event.pricing_tiers[0].price_cents / 100
                : 25.0
            }
          }
        },
        Effect.none()
      ];
    }

    case 'organizer/createEvent': {
      const form = state.organizer.eventForm;
      return [
        {
          ...state,
          organizer: { ...state.organizer, formLoading: true, formError: null }
        },
        Effect.run(async (dispatch) => {
          const result = await deps.api.post<CreateEventApiResponse>('/api/v2/events', {
            title: form.title,
            description: form.description || undefined,
            start_time: new Date(form.startTime).toISOString(),
            venue_name: form.venueName,
            capacity: form.capacity,
            price: form.price
          });

          if (result.ok && result.data) {
            dispatch({ type: 'organizer/eventCreated', eventId: result.data.event_id });
          } else {
            dispatch({
              type: 'organizer/createFailed',
              error: result.error?.message ?? 'Failed to create event'
            });
          }
        })
      ];
    }

    case 'organizer/eventCreated': {
      const toast = {
        id: crypto.randomUUID(),
        message: 'Event created successfully!',
        type: 'success' as const
      };
      return [
        {
          ...state,
          organizer: {
            ...state.organizer,
            formLoading: false,
            eventForm: initialEventForm
          },
          destination: { type: 'organizerEvents', state: {} },
          ui: { ...state.ui, toasts: [...state.ui.toasts, toast] }
        },
        Effect.none()
      ];
    }

    case 'organizer/createFailed': {
      const toast = {
        id: crypto.randomUUID(),
        message: action.error,
        type: 'error' as const
      };
      return [
        {
          ...state,
          organizer: {
            ...state.organizer,
            formLoading: false,
            formError: action.error
          },
          ui: { ...state.ui, toasts: [...state.ui.toasts, toast] }
        },
        Effect.none()
      ];
    }

    case 'organizer/updateEvent': {
      const form = state.organizer.eventForm;
      return [
        {
          ...state,
          organizer: { ...state.organizer, formLoading: true, formError: null }
        },
        Effect.run(async (dispatch) => {
          const result = await deps.api.put<UpdateEventApiResponse>(
            `/api/v2/events/${action.eventId}`,
            {
              name: form.title,
              venue_name: form.venueName,
              date: new Date(form.startTime).toISOString()
            }
          );

          if (result.ok && result.data) {
            dispatch({ type: 'organizer/eventUpdated', eventId: result.data.event_id });
          } else {
            dispatch({
              type: 'organizer/updateFailed',
              error: result.error?.message ?? 'Failed to update event'
            });
          }
        })
      ];
    }

    case 'organizer/eventUpdated': {
      const toast = {
        id: crypto.randomUUID(),
        message: 'Event updated successfully!',
        type: 'success' as const
      };
      return [
        {
          ...state,
          organizer: {
            ...state.organizer,
            formLoading: false
          },
          destination: { type: 'organizerEvents', state: {} },
          ui: { ...state.ui, toasts: [...state.ui.toasts, toast] }
        },
        Effect.none()
      ];
    }

    case 'organizer/updateFailed': {
      const toast = {
        id: crypto.randomUUID(),
        message: action.error,
        type: 'error' as const
      };
      return [
        {
          ...state,
          organizer: {
            ...state.organizer,
            formLoading: false,
            formError: action.error
          },
          ui: { ...state.ui, toasts: [...state.ui.toasts, toast] }
        },
        Effect.none()
      ];
    }

    case 'organizer/publishEvent': {
      return [
        { ...state, organizer: { ...state.organizer, myEventsLoading: true } },
        Effect.run(async (dispatch) => {
          const result = await deps.api.post(`/api/v2/events/${action.eventId}/publish`);
          if (result.ok) {
            dispatch({ type: 'organizer/eventPublished', eventId: action.eventId });
          } else {
            dispatch({ type: 'organizer/myEventsFailed', error: 'Failed to publish event' });
          }
        })
      ];
    }

    case 'organizer/eventPublished': {
      const toast = {
        id: crypto.randomUUID(),
        message: 'Event published successfully!',
        type: 'success' as const
      };
      return [
        {
          ...state,
          organizer: {
            ...state.organizer,
            myEventsLoading: false,
            myEvents: state.organizer.myEvents.map((e) =>
              e.id === action.eventId ? { ...e, status: 'published' as const } : e
            )
          },
          ui: { ...state.ui, toasts: [...state.ui.toasts, toast] }
        },
        Effect.none()
      ];
    }

    case 'organizer/cancelEvent': {
      return [
        { ...state, organizer: { ...state.organizer, myEventsLoading: true } },
        Effect.run(async (dispatch) => {
          const result = await deps.api.post(`/api/v2/events/${action.eventId}/cancel`);
          if (result.ok) {
            dispatch({ type: 'organizer/eventCancelled', eventId: action.eventId });
          } else {
            dispatch({ type: 'organizer/myEventsFailed', error: 'Failed to cancel event' });
          }
        })
      ];
    }

    case 'organizer/eventCancelled': {
      const toast = {
        id: crypto.randomUUID(),
        message: 'Event cancelled',
        type: 'info' as const
      };
      return [
        {
          ...state,
          organizer: {
            ...state.organizer,
            myEventsLoading: false,
            myEvents: state.organizer.myEvents.map((e) =>
              e.id === action.eventId ? { ...e, status: 'cancelled' as const } : e
            )
          },
          ui: { ...state.ui, toasts: [...state.ui.toasts, toast] }
        },
        Effect.none()
      ];
    }

    case 'organizer/deleteEvent': {
      return [
        { ...state, organizer: { ...state.organizer, myEventsLoading: true } },
        Effect.run(async (dispatch) => {
          const result = await deps.api.delete(`/api/v2/events/${action.eventId}`);
          if (result.ok) {
            dispatch({ type: 'organizer/eventDeleted', eventId: action.eventId });
          } else {
            dispatch({ type: 'organizer/myEventsFailed', error: 'Failed to delete event' });
          }
        })
      ];
    }

    case 'organizer/eventDeleted': {
      const toast = {
        id: crypto.randomUUID(),
        message: 'Event deleted',
        type: 'info' as const
      };
      return [
        {
          ...state,
          organizer: {
            ...state.organizer,
            myEventsLoading: false,
            myEvents: state.organizer.myEvents.filter((e) => e.id !== action.eventId)
          },
          destination: { type: 'organizerEvents', state: {} },
          ui: { ...state.ui, toasts: [...state.ui.toasts, toast] }
        },
        Effect.none()
      ];
    }

    default:
      return [state, Effect.none()];
  }
};
