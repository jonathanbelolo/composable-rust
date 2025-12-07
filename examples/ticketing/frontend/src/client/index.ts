/**
 * Client-side entry point for Ticketing Frontend.
 *
 * This hydrates the server-rendered HTML and makes the application interactive.
 */

// Import CSS for Tailwind and shadcn theme variables
import '../shared/app.css';

import { hydrate as hydrateComponent } from 'svelte';
import { hydrateStore } from '@composable-svelte/core';
import App from '../shared/App.svelte';
import { appReducer } from '../shared/reducer';
import type { AppDependencies } from '../shared/reducer';
import type { AppState } from '../shared/types';
import type { AppAction } from '../shared/reducer';
import { parseDestinationFromURL, destinationURL, syncBrowserHistory } from '../shared/routing';

// API base URL (proxied through the SSR server)
const API_BASE_URL = '';

/**
 * Create API client for browser.
 */
function createAPIClient(): AppDependencies['api'] {
  const getAuthToken = () => localStorage.getItem('auth_token');

  const request = async <T>(
    method: string,
    url: string,
    body?: unknown
  ): Promise<{ ok: boolean; data?: T; error?: { message: string } }> => {
    try {
      const headers: Record<string, string> = {
        'Content-Type': 'application/json'
      };

      const token = getAuthToken();
      if (token) {
        headers['Authorization'] = `Bearer ${token}`;
      }

      const response = await fetch(`${API_BASE_URL}${url}`, {
        method,
        headers,
        body: body ? JSON.stringify(body) : undefined
      });

      const data = await response.json();

      if (!response.ok) {
        return { ok: false, error: { message: data.error || 'Request failed' } };
      }

      return { ok: true, data };
    } catch (error) {
      return { ok: false, error: { message: error instanceof Error ? error.message : 'Network error' } };
    }
  };

  return {
    get: <T>(url: string) => request<T>('GET', url),
    post: <T>(url: string, body?: unknown) => request<T>('POST', url, body),
    put: <T>(url: string, body?: unknown) => request<T>('PUT', url, body),
    delete: <T>(url: string) => request<T>('DELETE', url)
  };
}

/**
 * Create storage client for browser.
 */
function createStorageClient(): AppDependencies['storage'] {
  return {
    getItem: (key: string) => {
      try {
        return localStorage.getItem(key);
      } catch {
        return null;
      }
    },
    setItem: (key: string, value: string) => {
      try {
        localStorage.setItem(key, value);
      } catch {
        // Ignore storage errors
      }
    },
    removeItem: (key: string) => {
      try {
        localStorage.removeItem(key);
      } catch {
        // Ignore storage errors
      }
    }
  };
}

/**
 * Hydrate the application.
 */
async function hydrate() {
  try {
    // 1. Read serialized state from the server
    const stateElement = document.getElementById('__COMPOSABLE_SVELTE_STATE__');

    if (!stateElement || !stateElement.textContent) {
      throw new Error('No hydration data found. Server-side rendering may have failed.');
    }

    // 2. Create client dependencies
    const dependencies: AppDependencies = {
      api: createAPIClient(),
      storage: createStorageClient()
    };

    // 3. Hydrate the store with client dependencies
    const store = hydrateStore<AppState, AppAction, AppDependencies>(stateElement.textContent, {
      reducer: appReducer,
      dependencies
    });

    // 4. Sync browser history with state (URL routing!)
    syncBrowserHistory(store, {
      parse: parseDestinationFromURL,
      serialize: (state: AppState) => destinationURL(state.destination),
      destinationToAction: (dest: AppState['destination'] | null) => {
        if (dest) {
          return { type: 'navigate', destination: dest } as AppAction;
        }
        return { type: 'navigate', destination: { type: 'home', state: {} } } as AppAction;
      }
    });

    // 5. Hydrate the app (reuse existing DOM from SSR)
    const app = hydrateComponent(App, {
      target: document.body,
      props: { store }
    });

    // 6. Hydrate auth state from localStorage AFTER component hydration
    // This must happen after hydrateComponent to avoid hydration mismatches
    const token = localStorage.getItem('auth_token');
    if (token) {
      // Try to restore user info as well
      let user = null;
      const userJson = localStorage.getItem('auth_user');
      if (userJson) {
        try {
          user = JSON.parse(userJson);
        } catch {
          // Invalid JSON, ignore
        }
      }
      store.dispatch({ type: 'auth/hydrate', token, user });
    }

    // Log successful hydration
    console.log('✅ Ticketing frontend hydrated successfully');

    // Cleanup on unmount (for HMR during development)
    if (import.meta.hot) {
      import.meta.hot.dispose(() => {
        app.$destroy?.();
      });
    }
  } catch (error) {
    console.error('❌ Hydration failed:', error);

    // Show error to user
    document.body.innerHTML = `
      <div style="
        display: flex;
        align-items: center;
        justify-content: center;
        min-height: 100vh;
        background: #fee;
        color: #c00;
        font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
        padding: 2rem;
      ">
        <div style="text-align: center;">
          <h1 style="margin-bottom: 1rem;">Application Error</h1>
          <p>${error instanceof Error ? error.message : 'Unknown error'}</p>
          <button
            onclick="window.location.reload()"
            style="
              margin-top: 1rem;
              padding: 0.5rem 1rem;
              background: #c00;
              color: white;
              border: none;
              border-radius: 4px;
              cursor: pointer;
            "
          >
            Reload Page
          </button>
        </div>
      </div>
    `;
  }
}

// Start hydration when DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', hydrate);
} else {
  hydrate();
}
