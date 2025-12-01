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
  Payment,
  SectionAvailability,
  User,
  SelectedSeat
} from './types';

// ============================================================================
// Actions
// ============================================================================

export type AppAction =
  // Navigation
  | { type: 'navigate'; destination: AppDestination }

  // Auth
  | { type: 'auth/requestMagicLink'; email: string }
  | { type: 'auth/magicLinkSent' }
  | { type: 'auth/verifyToken'; token: string }
  | { type: 'auth/verified'; user: User; token: string }
  | { type: 'auth/failed'; error: string }
  | { type: 'auth/logout' }
  | { type: 'auth/hydrate'; token: string }

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
  | { type: 'reservations/created'; reservation: Reservation }
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
  | { type: 'ui/setLoading'; isLoading: boolean };

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
      return [
        { ...state, destination: action.destination },
        Effect.none()
      ];
    }

    // -------------------------------------------------------------------------
    // Auth
    // -------------------------------------------------------------------------
    case 'auth/requestMagicLink': {
      return [
        { ...state, auth: { ...state.auth, isLoading: true, error: null } },
        Effect.run(async (dispatch) => {
          const result = await deps.api.post('/api/v2/auth/magic-link', { email: action.email });
          if (result.ok) {
            dispatch({ type: 'auth/magicLinkSent' });
          } else {
            dispatch({ type: 'auth/failed', error: result.error?.message ?? 'Failed to send magic link' });
          }
        })
      ];
    }

    case 'auth/magicLinkSent': {
      return [
        { ...state, auth: { ...state.auth, isLoading: false, magicLinkSent: true } },
        Effect.none()
      ];
    }

    case 'auth/verifyToken': {
      return [
        { ...state, auth: { ...state.auth, isLoading: true, error: null } },
        Effect.run(async (dispatch) => {
          const result = await deps.api.post<{ user: User; token: string }>('/api/v2/auth/verify', {
            token: action.token
          });
          if (result.ok && result.data) {
            deps.storage.setItem('auth_token', result.data.token);
            dispatch({ type: 'auth/verified', user: result.data.user, token: result.data.token });
          } else {
            dispatch({ type: 'auth/failed', error: result.error?.message ?? 'Invalid token' });
          }
        })
      ];
    }

    case 'auth/verified': {
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
          destination: { type: 'dashboard', state: {} }
        },
        Effect.none()
      ];
    }

    case 'auth/failed': {
      return [
        { ...state, auth: { ...state.auth, isLoading: false, error: action.error } },
        Effect.none()
      ];
    }

    case 'auth/logout': {
      deps.storage.removeItem('auth_token');
      return [
        {
          ...state,
          auth: {
            user: null,
            token: null,
            isAuthenticated: false,
            isLoading: false,
            error: null,
            magicLinkSent: false
          },
          destination: { type: 'home', state: {} }
        },
        Effect.none()
      ];
    }

    case 'auth/hydrate': {
      return [
        { ...state, auth: { ...state.auth, token: action.token, isAuthenticated: true } },
        Effect.none()
      ];
    }

    // -------------------------------------------------------------------------
    // Events
    // -------------------------------------------------------------------------
    case 'events/loadList': {
      return [
        { ...state, eventsLoading: true, eventsError: null },
        Effect.run(async (dispatch) => {
          const result = await deps.api.get<{ events: Event[] }>('/api/v2/events');
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

      return [
        { ...state, reservationsLoading: true },
        Effect.run(async (dispatch) => {
          const result = await deps.api.post<Reservation>('/api/v2/reservations', {
            event_id: eventId,
            seats: selectedSeats.map((s) => ({
              seat_id: s.seatId,
              section: s.section,
              tier_type: s.tierType
            }))
          });

          if (result.ok && result.data) {
            dispatch({ type: 'reservations/created', reservation: result.data });
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
      return [
        {
          ...state,
          reservationsLoading: false,
          currentReservation: {
            ...state.currentReservation,
            step: 'payment',
            reservationId: action.reservation.id,
            expiresAt: action.reservation.expires_at
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
          const result = await deps.api.get<{ reservations: Reservation[] }>('/api/v2/my-reservations');
          if (result.ok && result.data) {
            dispatch({ type: 'reservations/mineLoaded', reservations: result.data.reservations });
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
      return [
        { ...state, paymentsLoading: true },
        Effect.run(async (dispatch) => {
          const result = await deps.api.get<{ payments: Payment[] }>('/api/v2/my-payments');
          if (result.ok && result.data) {
            dispatch({ type: 'payments/historyLoaded', payments: result.data.payments });
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

    default:
      return [state, Effect.none()];
  }
};
