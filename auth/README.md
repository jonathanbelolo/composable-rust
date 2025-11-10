# composable-rust-auth

**Production-ready authentication framework for Composable Rust.**

## Overview

Complete authentication system with magic links, OAuth 2.0, and passkeys/WebAuthn, built on the Composable Rust architecture.

## Installation

```toml
[dependencies]
composable-rust-auth = { path = "../auth" }
composable-rust-runtime = { path = "../runtime" }
axum = "0.7"
```

## Features

- ✅ **Magic Link** - Passwordless authentication via email
- ✅ **OAuth 2.0** - Google, GitHub providers
- ✅ **Passkeys/WebAuthn** - Biometric authentication
- ✅ **Email Providers** - SMTP (production), Console (development)
- ✅ **Rate Limiting** - Redis-based rate limiter
- ✅ **Risk Scoring** - Anomaly detection
- ✅ **Session Management** - Secure session handling

## Quick Start

### Magic Link Authentication

```rust
use composable_rust_auth::{
    AuthEnvironment, AuthReducer, AuthState, AuthAction,
    providers::{SmtpEmailProvider, ConsoleEmailProvider},
};
use composable_rust_runtime::Store;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Email provider
    let email_provider = SmtpEmailProvider::new(
        "smtp.gmail.com".into(),
        587,
        std::env::var("SMTP_USERNAME")?,
        std::env::var("SMTP_PASSWORD")?,
        "noreply@example.com".into(),
        "My App".into(),
    )?;

    // Auth environment
    let auth_env = AuthEnvironment::new(
        oauth_provider,
        email_provider,
        webauthn_provider,
        session_store,
        token_store,
        user_repository,
        device_repository,
        risk_calculator,
        oauth_token_store,
        challenge_store,
        rate_limiter,
        event_store,
    );

    // Auth store
    let auth_store = Store::new(
        AuthState::default(),
        AuthReducer::new(),
        auth_env,
    );

    // Initiate magic link
    auth_store.send(AuthAction::InitiateMagicLink {
        email: "user@example.com".to_string(),
    }).await?;

    // User clicks link, verify token
    auth_store.send(AuthAction::VerifyMagicLink {
        token: "token-from-email".to_string(),
    }).await?;

    Ok(())
}
```

## Email Providers

### SMTP (Production)

```rust
use composable_rust_auth::providers::SmtpEmailProvider;

let email_provider = SmtpEmailProvider::new(
    "smtp.gmail.com".into(),
    587,
    env::var("SMTP_USERNAME")?,
    env::var("SMTP_PASSWORD")?,
    "noreply@myapp.com".into(),
    "MyApp".into(),
)?;
```

Supports:
- Gmail (App Passwords)
- AWS SES
- SendGrid
- Postmark
- Any SMTP server

### Console (Development)

```rust
use composable_rust_auth::providers::ConsoleEmailProvider;

let email_provider = ConsoleEmailProvider::new();
// Emails logged to console with box-drawing characters
```

Perfect for:
- Local development
- Integration tests
- CI/CD pipelines

## OAuth 2.0

### Google

```rust
use composable_rust_auth::oauth::GoogleOAuthProvider;

let oauth_provider = GoogleOAuthProvider::new(
    client_id,
    client_secret,
    redirect_uri,
)?;

// Initiate OAuth flow
auth_store.send(AuthAction::InitiateOAuth {
    provider: "google".into(),
}).await?;

// Handle callback
auth_store.send(AuthAction::HandleOAuthCallback {
    provider: "google".into(),
    code: authorization_code,
}).await?;
```

### GitHub

```rust
use composable_rust_auth::oauth::GitHubOAuthProvider;

let oauth_provider = GitHubOAuthProvider::new(
    client_id,
    client_secret,
    redirect_uri,
)?;
```

## Passkeys/WebAuthn

```rust
use composable_rust_auth::webauthn::WebAuthnProvider;

// Register passkey
auth_store.send(AuthAction::InitiatePasskeyRegistration {
    user_id: "user-123".into(),
}).await?;

// Authenticate with passkey
auth_store.send(AuthAction::InitiatePasskeyAuthentication {
    user_id: "user-123".into(),
}).await?;
```

## Rate Limiting

```rust
use composable_rust_auth::stores::RateLimiterRedis;

let rate_limiter = RateLimiterRedis::new(
    redis_client,
    100,  // 100 requests
    Duration::from_secs(60),  // per minute
);
```

## Security Features

- ✅ **Token expiration** - Configurable TTL
- ✅ **CSRF protection** - State parameter for OAuth
- ✅ **Rate limiting** - Prevent brute force
- ✅ **Risk scoring** - Anomaly detection (device, IP, velocity)
- ✅ **Secure sessions** - HTTP-only, Secure, SameSite cookies
- ✅ **Audit logging** - All auth events logged

## HTTP Handlers

Integration with `composable-rust-web`:

```rust
use composable_rust_web::auth_handlers;
use axum::{Router, routing::post};

let app = Router::new()
    .route("/auth/magic-link", post(auth_handlers::magic_link::initiate))
    .route("/auth/magic-link/verify", post(auth_handlers::magic_link::verify))
    .route("/auth/oauth/:provider", post(auth_handlers::oauth::initiate))
    .route("/auth/oauth/:provider/callback", post(auth_handlers::oauth::callback))
    .with_state(auth_store);
```

## Configuration

### Environment Variables

```bash
# SMTP Email
SMTP_SERVER=smtp.gmail.com
SMTP_PORT=587
SMTP_USERNAME=noreply@myapp.com
SMTP_PASSWORD=your-app-password
FROM_EMAIL=noreply@myapp.com
FROM_NAME=MyApp

# OAuth (Google)
GOOGLE_CLIENT_ID=your-client-id
GOOGLE_CLIENT_SECRET=your-client-secret
GOOGLE_REDIRECT_URI=https://myapp.com/auth/google/callback

# OAuth (GitHub)
GITHUB_CLIENT_ID=your-client-id
GITHUB_CLIENT_SECRET=your-client-secret
GITHUB_REDIRECT_URI=https://myapp.com/auth/github/callback

# Redis (Rate Limiting)
REDIS_URL=redis://localhost:6379
```

## Testing

```rust
use composable_rust_auth::providers::ConsoleEmailProvider;
use composable_rust_testing::*;

#[tokio::test]
async fn test_magic_link() {
    let auth_env = AuthEnvironment::new(
        mock_oauth_provider(),
        ConsoleEmailProvider::new(),  // Console for tests
        mock_webauthn_provider(),
        // ... other mocks
    );

    let store = Store::new(
        AuthState::default(),
        AuthReducer::new(),
        auth_env,
    );

    // Test magic link flow
    store.send(AuthAction::InitiateMagicLink {
        email: "test@example.com".into(),
    }).await.unwrap();

    // Verify token (extract from console output in real test)
    store.send(AuthAction::VerifyMagicLink {
        token: "test-token".into(),
    }).await.unwrap();
}
```

## Development & QA

### Before Committing Changes

Always run the comprehensive verification script:

```bash
./scripts/verify.sh
```

This script:
1. ✅ Checks database is running
2. ✅ Applies migrations
3. ✅ Validates queries against live schema
4. ✅ Generates sqlx cache (verifies 30+ queries found)
5. ✅ Tests offline compilation mode
6. ✅ Runs all tests
7. ✅ Runs clippy with strict lints

**Critical**: Never skip this step. If rust-analyzer shows errors but this passes, investigate!

### Manual Verification Steps

If you need to verify specific components:

```bash
# 1. Start database
docker run -d --name auth-db -p 5434:5432 \
  -e POSTGRES_PASSWORD=password \
  postgres:16-alpine

# 2. Check schema matches code
DATABASE_URL="postgres://postgres:password@localhost:5434/composable_auth" \
  cargo check --all-features

# 3. Regenerate query cache
DATABASE_URL="postgres://postgres:password@localhost:5434/composable_auth" \
  cargo sqlx prepare -- --all-features

# 4. Verify cache was generated (should show 30+ files)
ls -1 .sqlx/query-*.json | wc -l

# 5. Test offline mode
SQLX_OFFLINE=true cargo check --all-features
```

### IDE Integration

The `.vscode/settings.json` configures rust-analyzer for offline mode:

```json
{
  "rust-analyzer.cargo.extraEnv": {
    "SQLX_OFFLINE": "true"
  }
}
```

**If rust-analyzer shows errors:**
1. Run `./scripts/verify.sh` to see if they're real
2. Restart rust-analyzer (Cmd+Shift+P → "Rust Analyzer: Restart Server")
3. If errors persist after verification passes, check `.sqlx/` has cache files

### Common Issues

**"no queries found" when running `cargo sqlx prepare`:**
- Missing `--all-features` flag
- Wrong working directory
- Feature flags not enabled

**rust-analyzer shows errors but compilation passes:**
- Cache may be stale: run `cargo sqlx prepare` again
- Restart rust-analyzer

**Compilation passes but rust-analyzer shows errors:**
- This is the **dangerous case** - trust the IDE!
- Run verification script to find the real issue
- The IDE validates against schema, compilation might not

## Further Reading

- [Email Providers Guide](../docs/email-providers.md) - Complete email setup
- [Auth Example](../examples/ticketing/) - Full auth integration
- [Getting Started](../docs/getting-started.md) - Framework basics

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
