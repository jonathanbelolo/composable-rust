/**
 * Fastify server with Server-Side Rendering for Ticketing Frontend.
 *
 * This demonstrates how to use Composable Svelte's SSR capabilities
 * with a modern Node.js framework, proxying API requests to the Rust backend.
 */

import Fastify from 'fastify';
import fastifyStatic from '@fastify/static';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';
import { createStore, renderToHTML } from '@composable-svelte/core';
import App from '../shared/App.svelte';
import { appReducer } from '../shared/reducer';
import type { AppDependencies } from '../shared/reducer';
import { initialState } from '../shared/types';
import type { EventSummary } from '../shared/types';
import { parseDestinationFromURL } from '../shared/routing';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

// Backend API URL (the Rust ticketing server)
const API_BASE_URL = process.env.API_URL || 'http://localhost:8080';

// Create Fastify instance
const app = Fastify({
  logger: {
    level: process.env.NODE_ENV === 'production' ? 'info' : 'debug'
  }
});

// Serve static files (client bundle)
const clientDir = join(__dirname, '../client');
app.register(fastifyStatic, {
  root: clientDir,
  prefix: '/assets/'
});

/**
 * Create mock dependencies for SSR (no API calls during SSR - data is pre-fetched).
 */
function createSSRDependencies(): AppDependencies {
  return {
    api: {
      get: async () => ({ ok: false, error: { message: 'SSR mode - no API calls' } }),
      post: async () => ({ ok: false, error: { message: 'SSR mode - no API calls' } }),
      put: async () => ({ ok: false, error: { message: 'SSR mode - no API calls' } }),
      delete: async () => ({ ok: false, error: { message: 'SSR mode - no API calls' } })
    },
    storage: {
      getItem: () => null,
      setItem: () => {},
      removeItem: () => {}
    }
  };
}

/**
 * Fetch events list from backend API for SSR.
 */
async function fetchEvents(): Promise<EventSummary[]> {
  try {
    const response = await fetch(`${API_BASE_URL}/api/v2/events`);
    if (!response.ok) {
      console.error('Failed to fetch events:', response.status);
      return [];
    }
    const data = await response.json();
    return data.events || [];
  } catch (error) {
    console.error('Error fetching events for SSR:', error);
    return [];
  }
}

/**
 * Main SSR route handler.
 */
async function renderApp(request: any, reply: any) {
  try {
    // 1. Parse URL using router
    const path = request.url;
    const destination = parseDestinationFromURL(path);

    // 2. Compute meta tags and pre-fetch data based on destination
    let meta = initialState.meta;
    let events: EventSummary[] = [];

    if (destination.type === 'home') {
      meta = {
        title: 'Ticketing - Event Tickets Made Easy',
        description: 'Find and book tickets for concerts, sports, theater, and more.',
        canonical: '/'
      };
    } else if (destination.type === 'events') {
      meta = {
        title: 'Browse Events - Ticketing',
        description: 'Discover events near you and book tickets online.',
        canonical: '/events'
      };
      // Pre-fetch events for SSR
      events = await fetchEvents();
    } else if (destination.type === 'login') {
      meta = {
        title: 'Sign In - Ticketing',
        description: 'Sign in to your Ticketing account.',
        canonical: '/auth/login'
      };
    } else if (destination.type === 'dashboard') {
      meta = {
        title: 'Dashboard - Ticketing',
        description: 'Manage your reservations and account.',
        canonical: '/dashboard'
      };
    }

    // 3. Create store with URL-driven state and pre-fetched data
    const store = createStore({
      initialState: {
        ...initialState,
        destination,
        meta,
        events
      },
      reducer: appReducer,
      dependencies: createSSRDependencies()
    });

    // 4. Render component to HTML
    const html = renderToHTML(
      App,
      { store },
      {
        head: `
        <link rel="stylesheet" href="/assets/index.css">
        <meta name="viewport" content="width=device-width, initial-scale=1.0">
        <link rel="icon" type="image/svg+xml" href="/assets/favicon.svg">
      `,
        clientScript: '/assets/index.js'
      }
    );

    // 5. Send response
    reply.type('text/html').send(html);
  } catch (error) {
    request.log.error(error);
    reply.status(500).send({
      error: 'Internal Server Error',
      message: error instanceof Error ? error.message : 'Unknown error'
    });
  }
}

// Register SSR routes - all frontend routes handled by the same handler
app.get('/', renderApp);
app.get('/events', renderApp);
app.get('/events/:id', renderApp);
app.get('/events/:id/reserve', renderApp);
app.get('/auth/login', renderApp);
app.get('/auth/verify', renderApp);
app.get('/auth/magic-link/verify', renderApp);
app.get('/dashboard', renderApp);
app.get('/dashboard/reservations', renderApp);
app.get('/dashboard/payments', renderApp);
app.get('/organizer', renderApp);
app.get('/organizer/create', renderApp);
app.get('/organizer/:id', renderApp);
app.get('/organizer/:id/analytics', renderApp);

/**
 * Proxy helper function to forward requests to the Rust backend.
 */
async function proxyToBackend(request: any, reply: any) {
  try {
    const url = `${API_BASE_URL}${request.url}`;
    const headers: Record<string, string> = {};

    // Forward relevant headers
    if (request.headers.authorization) {
      headers['Authorization'] = request.headers.authorization as string;
    }
    if (request.headers['content-type']) {
      headers['Content-Type'] = request.headers['content-type'] as string;
    }

    const response = await fetch(url, {
      method: request.method,
      headers,
      body: request.method !== 'GET' && request.method !== 'HEAD' ? JSON.stringify(request.body) : undefined
    });

    const data = await response.json();
    reply.status(response.status).send(data);
  } catch (error) {
    request.log.error(error);
    reply.status(502).send({ error: 'Bad Gateway', message: 'Failed to reach backend API' });
  }
}

/**
 * Proxy API requests to the Rust backend.
 */
app.all('/api/*', proxyToBackend);

/**
 * Proxy auth requests to the Rust backend.
 */
app.all('/auth/*', proxyToBackend);

/**
 * Health check endpoint
 */
app.get('/health', async () => {
  return { status: 'ok', timestamp: new Date().toISOString() };
});

/**
 * Start the server
 */
const start = async () => {
  try {
    const port = process.env.PORT ? parseInt(process.env.PORT, 10) : 3000;
    const host = process.env.HOST || '0.0.0.0';

    await app.listen({ port, host });

    console.log('');
    console.log('🎫 Ticketing Frontend SSR Server');
    console.log('');
    console.log(`   Local:   http://localhost:${port}`);
    console.log(`   Network: http://${host}:${port}`);
    console.log(`   API:     ${API_BASE_URL}`);
    console.log('');
    console.log('📝 Press Ctrl+C to stop');
    console.log('');
  } catch (err) {
    app.log.error(err);
    process.exit(1);
  }
};

start();
