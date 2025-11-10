# Email Providers Guide

This guide covers how to send emails in composable-rust applications using the built-in email provider abstractions.

## Table of Contents

- [Overview](#overview)
- [Email Provider Trait](#email-provider-trait)
- [Console Email Provider](#console-email-provider-development)
- [SMTP Email Provider](#smtp-email-provider-production)
- [Configuration](#configuration)
- [Integration with Auth](#integration-with-auth)
- [Testing](#testing)
- [Troubleshooting](#troubleshooting)

## Overview

The `composable-rust-auth` crate provides a flexible email abstraction that supports multiple backends:

- **`ConsoleEmailProvider`**: Logs emails to console (development/testing)
- **`SmtpEmailProvider`**: Sends real emails via SMTP (production)

Both implement the same `EmailProvider` trait, enabling easy switching between development and production environments.

### Email Types Supported

All email providers support four email types:

1. **Magic Link** - Passwordless authentication
2. **Password Reset** - Account recovery
3. **Email Verification** - Confirm email ownership
4. **Security Alerts** - Notify users of account activity

## Email Provider Trait

The `EmailProvider` trait defines the interface all email providers must implement:

```rust
use composable_rust_auth::providers::EmailProvider;

pub trait EmailProvider: Send + Sync {
    async fn send_magic_link(
        &self,
        to: &str,
        token: &str,
        base_url: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()>;

    async fn send_password_reset(
        &self,
        to: &str,
        token: &str,
        base_url: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()>;

    async fn send_verification_email(
        &self,
        to: &str,
        token: &str,
        base_url: &str,
    ) -> Result<()>;

    async fn send_security_alert(
        &self,
        to: &str,
        subject: &str,
        message: &str,
    ) -> Result<()>;
}
```

## Console Email Provider (Development)

The `ConsoleEmailProvider` logs emails to the console with beautiful box-drawing characters. Perfect for development and testing where you don't want to send real emails.

### Features

- No configuration required
- Zero external dependencies
- Instant delivery (no network calls)
- Beautiful terminal output
- Logs include all email details

### Usage

```rust
use composable_rust_auth::providers::ConsoleEmailProvider;

// Create the provider (no configuration needed)
let email_provider = ConsoleEmailProvider::new();

// Use it in your environment
let auth_env = AuthEnvironment::new(
    oauth_provider,
    email_provider,  // ← Console provider
    webauthn_provider,
    // ... other dependencies
);
```

### Example Output

When you send a magic link, you'll see:

```
╔══════════════════════════════════════════════════════════════╗
║                   MAGIC LINK EMAIL                           ║
╠══════════════════════════════════════════════════════════════╣
║ To: user@example.com                                         ║
║ Subject: Sign in to your account                             ║
╠══════════════════════════════════════════════════════════════╣
║                                                              ║
║ Click the link below to sign in to your account.            ║
║ This link will expire in 15 minutes.                        ║
║                                                              ║
║ Magic Link:                                                  ║
║ https://app.example.com/auth/verify?token=abc123            ║
║                                                              ║
╚══════════════════════════════════════════════════════════════╝
```

### When to Use

- **Local development**: No email server configuration
- **Integration tests**: Fast, deterministic email verification
- **CI/CD pipelines**: No external service dependencies
- **Debugging**: See exactly what emails would be sent

## SMTP Email Provider (Production)

The `SmtpEmailProvider` sends real emails via SMTP using the [Lettre](https://github.com/lettre/lettre) library. Works with any SMTP server (Gmail, AWS SES, SendGrid, Postmark, etc.).

### Features

- Industry-standard SMTP protocol
- Provider-agnostic (works with any SMTP server)
- Beautiful HTML email templates
- Automatic TLS encryption
- Async email sending (non-blocking)
- Configurable sender details

### Installation

The `lettre` dependency is already included in `composable-rust-auth`:

```toml
[dependencies]
composable-rust-auth = { version = "0.1", features = ["email"] }
```

### Usage

```rust
use composable_rust_auth::providers::SmtpEmailProvider;

// Create SMTP provider with configuration
let email_provider = SmtpEmailProvider::new(
    "smtp.gmail.com".to_string(),           // SMTP server
    587,                                     // Port (587 for TLS, 465 for SSL)
    "noreply@example.com".to_string(),      // SMTP username
    env::var("SMTP_PASSWORD")?,             // SMTP password (from env)
    "noreply@example.com".to_string(),      // From email
    "My App".to_string(),                   // From name
)?;

// Use it in your environment
let auth_env = AuthEnvironment::new(
    oauth_provider,
    email_provider,  // ← SMTP provider
    webauthn_provider,
    // ... other dependencies
);
```

### Email Templates

The SMTP provider includes beautiful HTML templates for all email types:

**Magic Link Email:**
```html
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Sign in to your account</title>
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <div style="max-width: 600px; margin: 0 auto; padding: 20px;">
        <h2 style="color: #2563eb;">Sign in to your account</h2>
        <p>Click the link below to sign in. This link will expire in 15 minutes.</p>
        <p style="margin: 30px 0;">
            <a href="{{magic_link}}"
               style="display: inline-block; background-color: #2563eb;
                      color: white; padding: 12px 24px; text-decoration: none;
                      border-radius: 4px;">
                Sign In
            </a>
        </p>
        <p style="color: #666; font-size: 14px;">
            If you didn't request this email, you can safely ignore it.
        </p>
    </div>
</body>
</html>
```

## Configuration

### Environment-Based Configuration

Use environment variables to configure SMTP settings:

```bash
# .env
SMTP_SERVER=smtp.gmail.com
SMTP_PORT=587
SMTP_USERNAME=noreply@example.com
SMTP_PASSWORD=your-app-password
FROM_EMAIL=noreply@example.com
FROM_NAME=My App
```

Load configuration:

```rust
use std::env;

fn create_email_provider() -> Result<impl EmailProvider> {
    let smtp_server = env::var("SMTP_SERVER")?;
    let smtp_port = env::var("SMTP_PORT")?.parse()?;
    let smtp_username = env::var("SMTP_USERNAME")?;
    let smtp_password = env::var("SMTP_PASSWORD")?;
    let from_email = env::var("FROM_EMAIL")?;
    let from_name = env::var("FROM_NAME")?;

    SmtpEmailProvider::new(
        smtp_server,
        smtp_port,
        smtp_username,
        smtp_password,
        from_email,
        from_name,
    )
}
```

### Environment-Specific Providers

Switch providers based on environment:

```rust
fn create_email_provider() -> Box<dyn EmailProvider> {
    match env::var("ENV").as_deref() {
        Ok("production") => {
            // Production: Use SMTP
            Box::new(SmtpEmailProvider::new(/* ... */).expect("SMTP config"))
        }
        _ => {
            // Development/Staging: Use console
            Box::new(ConsoleEmailProvider::new())
        }
    }
}
```

## Integration with Auth

The email provider is injected into the `AuthEnvironment` and used by auth reducers:

```rust
use composable_rust_auth::{
    AuthEnvironment, AuthReducer, AuthState,
    providers::{SmtpEmailProvider, /* other providers */},
};
use composable_rust_runtime::Store;

// Create email provider
let email_provider = SmtpEmailProvider::new(/* ... */)?;

// Create auth environment with email provider
let auth_env = AuthEnvironment::new(
    oauth_provider,
    email_provider,      // ← Email provider
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

// Create auth store
let auth_store = Store::new(
    AuthState::default(),
    AuthReducer::new(),
    auth_env,
);
```

### How Reducers Use Email

When the auth reducer needs to send an email, it returns an effect:

```rust
// Inside AuthReducer::reduce()
match action {
    AuthAction::InitiateMagicLink { email } => {
        // Reducer generates a token and returns an email effect
        let token = generate_token();
        vec![
            AuthEffect::SendMagicLink {
                email: email.clone(),
                token,
                base_url: "https://app.example.com/verify".to_string(),
                expires_at: Utc::now() + Duration::minutes(15),
            }
        ]
    }
    // ... other actions
}
```

The Store then executes the effect, calling `env.email.send_magic_link(...)`.

## Testing

### Unit Testing with ConsoleEmailProvider

```rust
#[tokio::test]
async fn test_magic_link_flow() {
    // Use console provider for fast, deterministic tests
    let email_provider = ConsoleEmailProvider::new();

    let auth_env = create_test_environment(email_provider);
    let mut state = AuthState::default();

    // Send magic link
    let effects = reducer.reduce(
        &mut state,
        AuthAction::InitiateMagicLink {
            email: "test@example.com".to_string(),
        },
        &auth_env,
    );

    // Verify effect was created
    assert!(matches!(effects[0], AuthEffect::SendMagicLink { .. }));
}
```

### Integration Testing with Mock SMTP

For integration tests, you can use a mock SMTP server:

```rust
// Using mailhog or similar mock SMTP server
let email_provider = SmtpEmailProvider::new(
    "localhost".to_string(),
    1025,  // Mailhog SMTP port
    "test".to_string(),
    "test".to_string(),
    "test@example.com".to_string(),
    "Test App".to_string(),
)?;

// Run tests, verify emails in Mailhog UI (localhost:8025)
```

## Troubleshooting

### Common SMTP Issues

#### Authentication Failed

**Problem**: "SMTP authentication error"

**Solutions**:
1. **Gmail**: Use an [App Password](https://support.google.com/accounts/answer/185833), not your regular password
2. **AWS SES**: Use SMTP credentials (not AWS access keys)
3. **SendGrid**: Use API key as password
4. **Check credentials**: Ensure username/password are correct

#### Connection Timeout

**Problem**: "Connection timeout" or "Connection refused"

**Solutions**:
1. **Firewall**: Ensure port 587 (TLS) or 465 (SSL) is open
2. **Network**: Check if your network blocks SMTP
3. **Server address**: Verify SMTP server hostname is correct
4. **TLS/SSL**: Try different ports (587 for TLS, 465 for SSL)

#### TLS/SSL Errors

**Problem**: "TLS handshake failed"

**Solutions**:
1. **Port mismatch**: Use 587 for STARTTLS, 465 for SSL
2. **Certificate issues**: Ensure system CA certificates are up to date
3. **Self-signed certs**: Not supported by default (production should use valid certs)

#### Rate Limiting

**Problem**: "Too many requests" or similar

**Solutions**:
1. **Gmail**: Limited to 500 emails/day for free accounts
2. **SendGrid/SES**: Check your sending limits
3. **Implement retry**: Add exponential backoff for transient failures

### Debugging Email Delivery

#### Enable Verbose Logging

```rust
// Set RUST_LOG environment variable
// RUST_LOG=composable_rust_auth=debug,lettre=debug

use tracing_subscriber;

tracing_subscriber::fmt()
    .with_env_filter("composable_rust_auth=debug,lettre=debug")
    .init();
```

#### Test Email Connectivity

```rust
// Simple test program to verify SMTP connection
use composable_rust_auth::providers::SmtpEmailProvider;

#[tokio::main]
async fn main() -> Result<()> {
    let provider = SmtpEmailProvider::new(
        "smtp.gmail.com".to_string(),
        587,
        env::var("SMTP_USERNAME")?,
        env::var("SMTP_PASSWORD")?,
        "test@example.com".to_string(),
        "Test".to_string(),
    )?;

    provider.send_magic_link(
        "your-email@example.com",
        "test-token",
        "https://example.com/verify",
        Utc::now() + Duration::minutes(15),
    ).await?;

    println!("Email sent successfully!");
    Ok(())
}
```

## Provider Comparison

| Feature                  | ConsoleEmailProvider | SmtpEmailProvider |
|--------------------------|---------------------|-------------------|
| **Use Case**             | Development/Testing | Production        |
| **Configuration**        | None required       | SMTP credentials  |
| **External Dependencies**| None                | SMTP server       |
| **Delivery Speed**       | Instant             | 1-5 seconds       |
| **Network Required**     | No                  | Yes               |
| **Visible Emails**       | Console output      | Recipient inbox   |
| **HTML Templates**       | No (text only)      | Yes               |
| **Cost**                 | Free                | Varies by provider|

## Best Practices

### Production

1. **Use environment variables** for SMTP credentials (never hardcode)
2. **Enable TLS/SSL** for secure transmission
3. **Use dedicated SMTP services** (SendGrid, AWS SES, Postmark) for reliability
4. **Monitor delivery rates** to catch issues early
5. **Implement retry logic** for transient failures
6. **Set up SPF/DKIM/DMARC** for better deliverability

### Development

1. **Use `ConsoleEmailProvider`** for fast iteration
2. **Check console output** to verify email content
3. **Test all email types** (magic link, password reset, etc.)
4. **Verify token expiration** logic

### Testing

1. **Unit tests**: Use `ConsoleEmailProvider` (fast, no I/O)
2. **Integration tests**: Use mock SMTP server (Mailhog)
3. **End-to-end tests**: Use real SMTP with test accounts

## Next Steps

- **See [Getting Started](./getting-started.md)** for a complete authentication setup
- **See [Saga Patterns](./saga-patterns.md)** for email in workflows
- **See [Error Handling](./error-handling.md)** for email error recovery

## Examples

Complete working examples:

- `examples/auth-demo/` - Full authentication with magic links
- `auth/tests/magic_link_integration.rs` - Magic link email tests
