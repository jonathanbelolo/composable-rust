/**
 * Routing configuration for the ticketing frontend.
 *
 * Uses composable-svelte's routing system which works isomorphically:
 * - Server: Parse URL from request to determine initial state
 * - Client: Sync state with browser history API
 */

import type { Store } from '@composable-svelte/core';
import type { AppDestination } from './types';

// ============================================================================
// Browser History Sync
// ============================================================================

/**
 * Configuration for browser history synchronization.
 */
export interface BrowserHistoryConfig<State, Action, Dest> {
  parse: (path: string) => Dest | null;
  serialize: (state: State) => string;
  destinationToAction: (destination: Dest | null) => Action | null;
}

interface ComposableSvelteHistoryState {
  composableSvelteSync?: boolean;
}

/**
 * Sync browser history with store state.
 *
 * Sets up bidirectional synchronization:
 * 1. Browser back/forward → parse URL → dispatch action
 * 2. State changes → serialize → push URL to browser history
 */
export function syncBrowserHistory<State, Action, Dest>(
  store: Store<State, Action>,
  config: BrowserHistoryConfig<State, Action, Dest>
): () => void {
  // Track the current URL to avoid duplicate pushes
  let currentUrl = window.location.pathname;
  // Flag to prevent loops when handling popstate
  let isHandlingPopState = false;

  // Listen to browser back/forward
  const handlePopState = (_event: PopStateEvent) => {
    isHandlingPopState = true;
    const path = window.location.pathname;
    currentUrl = path;
    const destination = config.parse(path);
    const action = config.destinationToAction(destination);

    if (action) {
      store.dispatch(action);
    }
    // Reset flag after a tick to allow state to settle
    setTimeout(() => {
      isHandlingPopState = false;
    }, 0);
  };

  // Subscribe to state changes and push URL when destination changes
  const unsubscribe = store.subscribe((state: State) => {
    // Don't push URL if we're handling a popstate event (browser back/forward)
    if (isHandlingPopState) {
      return;
    }

    const newUrl = config.serialize(state);
    if (newUrl !== currentUrl) {
      currentUrl = newUrl;
      history.pushState({ composableSvelteSync: true }, '', newUrl);
    }
  });

  // Register event listener
  window.addEventListener('popstate', handlePopState);

  // Return cleanup function
  return () => {
    window.removeEventListener('popstate', handlePopState);
    unsubscribe();
  };
}

// ============================================================================
// Path Matching
// ============================================================================

// Simple path matching utility (matches patterns like 'events/:id')
function matchPath(pattern: string, path: string): Record<string, string> | null {
  // Normalize path - remove leading/trailing slashes
  const normalizedPath = path.replace(/^\/|\/$/g, '');
  const normalizedPattern = pattern.replace(/^\/|\/$/g, '');

  const patternParts = normalizedPattern.split('/');
  const pathParts = normalizedPath.split('/');

  if (patternParts.length !== pathParts.length) {
    return null;
  }

  const params: Record<string, string> = {};

  for (let i = 0; i < patternParts.length; i++) {
    const patternPart = patternParts[i];
    const pathPart = pathParts[i];

    if (patternPart.startsWith(':')) {
      // This is a parameter
      params[patternPart.slice(1)] = pathPart;
    } else if (patternPart !== pathPart) {
      // Literal mismatch
      return null;
    }
  }

  return params;
}

/**
 * Parse URL path to determine destination.
 *
 * Routes (in order of specificity):
 * - /organizer/:id/analytics → Event analytics
 * - /organizer/:id → Edit event
 * - /organizer/create → Create event
 * - /organizer → Organizer events list
 * - /dashboard/payments → My payments
 * - /dashboard/reservations → My reservations
 * - /dashboard → User dashboard
 * - /events/:id/reserve → Reservation flow
 * - /events/:id → Event detail
 * - /events → Events list
 * - /auth/verify → Verify magic link
 * - /auth/login → Login page
 * - / → Home
 */
export function parseDestinationFromURL(path: string): AppDestination {
  const normalizedPath = path.replace(/^\/|\/$/g, '');

  // Organizer routes (most specific first)
  let params = matchPath('organizer/:id/analytics', normalizedPath);
  if (params) {
    return { type: 'eventAnalytics', state: { eventId: params.id } };
  }

  if (normalizedPath === 'organizer/create') {
    return { type: 'createEvent', state: {} };
  }

  params = matchPath('organizer/:id', normalizedPath);
  if (params && params.id !== 'create') {
    return { type: 'editEvent', state: { eventId: params.id } };
  }

  if (normalizedPath === 'organizer') {
    return { type: 'organizerEvents', state: {} };
  }

  // Dashboard routes
  if (normalizedPath === 'dashboard/payments') {
    return { type: 'myPayments', state: {} };
  }

  if (normalizedPath === 'dashboard/reservations') {
    return { type: 'myReservations', state: {} };
  }

  if (normalizedPath === 'dashboard') {
    return { type: 'dashboard', state: {} };
  }

  // Event routes
  params = matchPath('events/:id/reserve', normalizedPath);
  if (params) {
    return { type: 'reserve', state: { eventId: params.id } };
  }

  params = matchPath('events/:id', normalizedPath);
  if (params && params.id !== 'reserve') {
    return { type: 'eventDetail', state: { eventId: params.id } };
  }

  if (normalizedPath === 'events') {
    return { type: 'events', state: {} };
  }

  // Auth routes
  if (normalizedPath === 'auth/verify') {
    return { type: 'verify', state: {} };
  }

  if (normalizedPath === 'auth/login') {
    return { type: 'login', state: {} };
  }

  // Home
  if (normalizedPath === '' || normalizedPath === '/') {
    return { type: 'home', state: {} };
  }

  // Default to home
  return { type: 'home', state: {} };
}

/**
 * Generate URL from destination.
 */
export function destinationURL(destination: AppDestination): string {
  switch (destination.type) {
    case 'home':
      return '/';
    case 'events':
      return '/events';
    case 'eventDetail':
      return `/events/${destination.state.eventId}`;
    case 'reserve':
      return `/events/${destination.state.eventId}/reserve`;
    case 'login':
      return '/auth/login';
    case 'verify':
      return '/auth/verify';
    case 'dashboard':
      return '/dashboard';
    case 'myReservations':
      return '/dashboard/reservations';
    case 'myPayments':
      return '/dashboard/payments';
    case 'organizerEvents':
      return '/organizer';
    case 'createEvent':
      return '/organizer/create';
    case 'editEvent':
      return `/organizer/${destination.state.eventId}`;
    case 'eventAnalytics':
      return `/organizer/${destination.state.eventId}/analytics`;
    default:
      return '/';
  }
}

// URL helpers
export const urls = {
  home: () => '/',
  events: () => '/events',
  eventDetail: (eventId: string) => `/events/${eventId}`,
  reserve: (eventId: string) => `/events/${eventId}/reserve`,
  login: () => '/auth/login',
  verify: () => '/auth/verify',
  dashboard: () => '/dashboard',
  myReservations: () => '/dashboard/reservations',
  myPayments: () => '/dashboard/payments',
  organizerEvents: () => '/organizer',
  createEvent: () => '/organizer/create',
  editEvent: (eventId: string) => `/organizer/${eventId}`,
  eventAnalytics: (eventId: string) => `/organizer/${eventId}/analytics`
};
