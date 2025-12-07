# Future Improvement: Cookie-Based Authentication for SSR

## Status: Planned

## Current Approach

The ticketing frontend currently uses **localStorage** for storing authentication tokens:

1. User completes magic link verification
2. Backend returns JWT token
3. Client stores token in `localStorage`
4. On page refresh, client reads from `localStorage` and dispatches `auth/hydrate`

### Limitations

- **Flash of unauthenticated UI**: SSR renders without auth state (server can't read localStorage), then client hydrates and shows authenticated state. Users briefly see "Sign In" before the UI updates to show their account.
- **XSS vulnerability**: JavaScript can access localStorage, making tokens stealable via XSS attacks.
- **Extra client code**: Need to manage localStorage read/write and hydration dispatch.

## Proposed Approach: HTTP-Only Cookies

Migrate to **HTTP-only cookies** set by the backend:

1. User completes magic link verification
2. Backend sets `Set-Cookie: session=<token>; HttpOnly; Secure; SameSite=Strict`
3. Browser automatically sends cookie on every request
4. SSR server reads cookie, fetches user info, renders authenticated state immediately

### Benefits

- **No flash**: SSR renders the correct authenticated state from the first byte
- **More secure**: HTTP-only cookies cannot be accessed by JavaScript (XSS-resistant)
- **Simpler client code**: No localStorage management, no hydration dispatch
- **Automatic**: Browser handles cookie transmission

## Required Changes

### 1. Backend (`examples/ticketing/src/`)

- Modify auth handlers to set HTTP-only cookies instead of returning tokens in JSON body
- Add cookie parsing middleware
- Ensure `SameSite=Strict` and `Secure` flags in production

```rust
// In auth handler after successful verification:
let cookie = Cookie::build("session", token)
    .http_only(true)
    .secure(true) // Only send over HTTPS
    .same_site(SameSite::Strict)
    .path("/")
    .max_age(Duration::days(7))
    .finish();

response.headers_mut().insert(
    SET_COOKIE,
    cookie.to_string().parse().unwrap(),
);
```

### 2. SSR Server (`examples/ticketing/frontend/src/server/index.ts`)

- Read session cookie from incoming requests
- Validate token with backend (or decode JWT locally if using shared secret)
- Create store with authenticated initial state

```typescript
async function renderApp(request: any, reply: any) {
  // Read session cookie
  const sessionToken = request.cookies?.session;

  let authState = { isAuthenticated: false, user: null, token: null };

  if (sessionToken) {
    // Validate with backend or decode JWT
    const user = await validateToken(sessionToken);
    if (user) {
      authState = { isAuthenticated: true, user, token: sessionToken };
    }
  }

  const store = createStore({
    initialState: {
      ...initialState,
      auth: authState,
      // ... rest of state
    },
    // ...
  });

  // Render with authenticated state
}
```

### 3. Client (`examples/ticketing/frontend/src/client/index.ts`)

- Remove localStorage-based auth hydration
- Remove `auth/hydrate` action dispatch
- Auth state comes from SSR initial state

### 4. Reducer (`examples/ticketing/frontend/src/shared/reducer.ts`)

- Remove or simplify `auth/hydrate` action
- `auth/verified` can remove localStorage storage (cookie is already set by backend)

## Migration Path

1. Add cookie support to backend alongside existing token response
2. Update SSR server to read cookies and inject auth state
3. Remove client-side localStorage logic
4. Test thoroughly with both approaches during transition
5. Remove legacy token response from backend

## Security Considerations

- **CSRF protection**: With `SameSite=Strict`, CSRF attacks are mitigated
- **HTTPS required**: `Secure` flag ensures cookies only sent over HTTPS
- **Token rotation**: Consider implementing refresh tokens for long sessions
- **Logout**: Must clear cookie on server side (`Set-Cookie` with `Max-Age=0`)
