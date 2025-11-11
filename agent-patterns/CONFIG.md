# Configuration Management Guide

Comprehensive guide to configuring agent systems for different environments.

## Table of Contents

- [Quick Start](#quick-start)
- [Configuration Structure](#configuration-structure)
- [Environment-Based Config](#environment-based-config)
- [Secrets Management](#secrets-management)
- [Validation](#validation)
- [Best Practices](#best-practices)
- [Examples](#examples)

## Quick Start

### Basic Usage

```rust
use composable_rust_agent_patterns::config::AgentConfig;

// Load from CONFIG_ENV environment variable (defaults to development)
let config = AgentConfig::from_env()?;

println!("Environment: {}", config.environment);
println!("LLM Model: {}", config.llm.model);
println!("Log Level: {}", config.observability.log_level);
```

### Explicit Environment

```rust
use composable_rust_agent_patterns::config::{AgentConfig, Environment};

// Load specific environment
let dev_config = AgentConfig::load(Environment::Development)?;
let prod_config = AgentConfig::load(Environment::Production)?;
```

## Configuration Structure

### AgentConfig

The root configuration object containing all subsections:

```rust
pub struct AgentConfig {
    pub environment: Environment,
    pub llm: LlmConfig,
    pub observability: ObservabilityConfig,
    pub resilience: ResilienceConfig,
    pub database: DatabaseConfig,
}
```

### LlmConfig

LLM provider configuration:

```rust
pub struct LlmConfig {
    pub model: String,              // e.g., "claude-sonnet-4-5-20250929"
    pub max_tokens: u32,            // Max tokens per request
    pub temperature: f32,           // 0.0-1.0
    pub timeout_secs: u64,          // API timeout
    pub max_retries: u32,           // Retry attempts
}
```

**Validation**:
- `model` cannot be empty
- `max_tokens` must be > 0
- `temperature` must be between 0.0 and 1.0
- `timeout_secs` must be > 0

### ObservabilityConfig

Tracing, metrics, and logging:

```rust
pub struct ObservabilityConfig {
    pub tracing_enabled: bool,
    pub jaeger_endpoint: Option<String>,  // e.g., "localhost:6831"
    pub metrics_enabled: bool,
    pub metrics_port: u16,
    pub log_level: String,                // trace, debug, info, warn, error
}
```

**Validation**:
- `log_level` must be one of: trace, debug, info, warn, error

### ResilienceConfig

Circuit breakers, rate limiting, bulkheads:

```rust
pub struct ResilienceConfig {
    pub circuit_breaker_threshold: u32,     // Failures before opening
    pub circuit_breaker_timeout_secs: u64,  // Time before half-open
    pub rate_limit_rps: u32,                // Requests per second
    pub bulkhead_max_concurrent: usize,     // Max concurrent requests
}
```

**Validation**:
- All values must be > 0

### DatabaseConfig

Database connection pooling:

```rust
pub struct DatabaseConfig {
    pub url: Option<String>,           // From DATABASE_URL env var
    pub max_connections: u32,
    pub connect_timeout_secs: u64,
    pub idle_timeout_secs: u64,
}
```

**Validation**:
- `max_connections` must be > 0
- `connect_timeout_secs` must be > 0

## Environment-Based Config

### Three Environments

1. **Development** (`dev`, `development`)
   - Verbose logging (debug level)
   - Permissive rate limits
   - Optional database

2. **Staging** (`staging`, `stage`)
   - Moderate logging (info level)
   - Production-like settings
   - Required database

3. **Production** (`prod`, `production`)
   - Minimal logging (warn level)
   - Strict rate limits and circuit breakers
   - Required database

### Environment-Specific Defaults

The configuration system applies sensible defaults based on environment:

| Setting | Development | Staging | Production |
|---------|------------|---------|------------|
| `log_level` | debug | info | warn |
| `rate_limit_rps` | 100 | 50 | 10 |
| `circuit_breaker_threshold` | 5 | 5 | 3 |
| `DATABASE_URL` | Optional | Required | Required |

### Setting Environment

```bash
# Via environment variable
export CONFIG_ENV=production

# Or in Kubernetes
env:
  - name: CONFIG_ENV
    value: "production"
```

```rust
// Or programmatically
let config = AgentConfig::load(Environment::Production)?;
```

## Secrets Management

### Never Commit Secrets

**❌ BAD - Secrets in config file:**
```toml
[database]
url = "postgres://user:password@localhost/db"  # DON'T DO THIS!
```

**✅ GOOD - Secrets from environment:**
```bash
export DATABASE_URL="postgres://user:password@localhost/db"
```

### Required Environment Variables

#### Production/Staging

- `DATABASE_URL` - Database connection string (required)
- `JAEGER_ENDPOINT` - Jaeger tracing endpoint (optional)

#### Development

- All secrets are optional (uses in-memory defaults)

### Loading Secrets

Secrets are automatically loaded during `AgentConfig::load()`:

```rust
// Automatically loads DATABASE_URL and JAEGER_ENDPOINT
let config = AgentConfig::load(Environment::Production)?;

// Access secrets
if let Some(db_url) = &config.database.url {
    println!("Database configured");
}
```

### Kubernetes Secrets

Use Kubernetes secrets for production:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: agent-secrets
type: Opaque
stringData:
  DATABASE_URL: postgres://user:password@postgres:5432/agents
  JAEGER_ENDPOINT: jaeger-agent:6831
```

Mount as environment variables:

```yaml
env:
  - name: DATABASE_URL
    valueFrom:
      secretKeyRef:
        name: agent-secrets
        key: DATABASE_URL
  - name: JAEGER_ENDPOINT
    valueFrom:
      secretKeyRef:
        name: agent-secrets
        key: JAEGER_ENDPOINT
```

## Validation

### Automatic Validation

All configuration is validated during load:

```rust
let config = AgentConfig::load(Environment::Production)?;
// ✅ Config is valid if this succeeds
```

### Validation Errors

Configuration errors provide clear messages:

```rust
match AgentConfig::load(Environment::Production) {
    Ok(config) => { /* use config */ },
    Err(ConfigError::EnvVarNotSet(var)) => {
        eprintln!("Missing required environment variable: {}", var);
    },
    Err(ConfigError::ValidationError(msg)) => {
        eprintln!("Invalid configuration: {}", msg);
    },
    Err(e) => eprintln!("Config error: {}", e),
}
```

### Validation Rules

#### LlmConfig
- ✅ `model` non-empty
- ✅ `max_tokens > 0`
- ✅ `temperature` in 0.0..=1.0
- ✅ `timeout_secs > 0`

#### ObservabilityConfig
- ✅ `log_level` in {trace, debug, info, warn, error}

#### ResilienceConfig
- ✅ All values > 0

#### DatabaseConfig
- ✅ `max_connections > 0`
- ✅ `connect_timeout_secs > 0`

## Best Practices

### 1. Use Environment Variables for Secrets

**Never** commit sensitive data to version control:

```bash
# .env (add to .gitignore)
DATABASE_URL=postgres://localhost/mydb
ANTHROPIC_API_KEY=sk-ant-...
JAEGER_ENDPOINT=localhost:6831
```

### 2. Validate Early

Load and validate configuration at startup:

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Fail fast if config is invalid
    let config = AgentConfig::from_env()?;

    // Rest of application...
    Ok(())
}
```

### 3. Use Type-Safe Accessors

Use helper methods for duration conversions:

```rust
let timeout = config.llm.timeout();  // Duration, not u64
let circuit_timeout = config.resilience.circuit_breaker_timeout();
```

### 4. Environment-Specific Overrides

Keep environment-specific settings minimal:

```rust
// Good: Use defaults with targeted overrides
let mut config = AgentConfig::load(env)?;

if config.is_development() {
    config.llm.max_tokens = 8192;  // Override for dev testing
}
```

### 5. Document Environment Variables

Create a `.env.example` file:

```bash
# Required in production/staging
DATABASE_URL=postgres://user:pass@host:5432/db

# Optional (enables distributed tracing)
JAEGER_ENDPOINT=localhost:6831

# Optional (defaults to "development")
CONFIG_ENV=production
```

### 6. Use Defaults Wisely

The configuration system provides sensible defaults:

```rust
// ✅ Good: Use defaults for most settings
let config = AgentConfig::load(Environment::Development)?;

// ❌ Bad: Over-configuring
let config = AgentConfig {
    environment: Environment::Development,
    llm: LlmConfig {
        model: "claude-sonnet-4-5-20250929".to_string(),
        // ... 50 more lines ...
    },
    // ... painful ...
};
```

### 7. Validate Before Deploy

Run validation checks in CI/CD:

```bash
# Check that production config is valid
CONFIG_ENV=production \
DATABASE_URL=postgres://localhost/test \
cargo test config::tests::test_agent_config_load_production
```

### 8. Use Config Structs, Not Globals

Pass config as dependencies, not global state:

```rust
// ✅ Good: Explicit dependency
struct MyService {
    config: AgentConfig,
}

impl MyService {
    fn new(config: AgentConfig) -> Self {
        Self { config }
    }
}

// ❌ Bad: Global state
lazy_static! {
    static ref CONFIG: AgentConfig = AgentConfig::from_env().unwrap();
}
```

## Examples

### Example 1: Basic Agent Initialization

```rust
use composable_rust_agent_patterns::config::AgentConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration (fails if invalid)
    let config = AgentConfig::from_env()?;

    println!("🚀 Starting agent system");
    println!("   Environment: {}", config.environment);
    println!("   LLM: {}", config.llm.model);
    println!("   Max tokens: {}", config.llm.max_tokens);

    // Use config throughout application
    let llm_timeout = config.llm.timeout();
    let metrics_port = config.observability.metrics_port;

    Ok(())
}
```

### Example 2: Environment-Specific Logic

```rust
use composable_rust_agent_patterns::config::AgentConfig;

let config = AgentConfig::from_env()?;

if config.is_production() {
    // Enable all production features
    enable_strict_validation();
    enable_rate_limiting();
    enable_circuit_breakers();
} else {
    // Development: More permissive
    println!("⚠️  Running in development mode");
}
```

### Example 3: Testing with Custom Config

```rust
use composable_rust_agent_patterns::config::{AgentConfig, Environment};

#[tokio::test]
async fn test_agent_behavior() {
    // Create test-specific config
    let mut config = AgentConfig::load(Environment::Development).unwrap();
    config.llm.max_tokens = 100;  // Small for fast tests
    config.observability.log_level = "debug".to_string();

    // Use in test
    let agent = MyAgent::new(config);
    // ... test assertions ...
}
```

### Example 4: Graceful Degradation

```rust
use composable_rust_agent_patterns::config::AgentConfig;

let config = AgentConfig::from_env()?;

// Tracing is optional
if config.observability.tracing_enabled {
    if let Some(endpoint) = &config.observability.jaeger_endpoint {
        init_tracing(endpoint)?;
        println!("✅ Distributed tracing enabled");
    } else {
        println!("ℹ️  Tracing enabled but no Jaeger endpoint configured");
    }
} else {
    println!("ℹ️  Tracing disabled");
}
```

### Example 5: Configuration in Kubernetes

**ConfigMap** (non-sensitive config):
```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: agent-config
data:
  CONFIG_ENV: "production"
  LOG_LEVEL: "info"
  METRICS_PORT: "9090"
```

**Secret** (sensitive config):
```yaml
apiVersion: v1
kind: Secret
metadata:
  name: agent-secrets
type: Opaque
stringData:
  DATABASE_URL: "postgres://user:pass@postgres:5432/agents"
  ANTHROPIC_API_KEY: "sk-ant-..."
```

**Deployment** (mounting both):
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: agent-system
spec:
  template:
    spec:
      containers:
      - name: agent
        envFrom:
        - configMapRef:
            name: agent-config
        - secretRef:
            name: agent-secrets
```

### Example 6: Configuration Validation in CI

```rust
// tests/config_validation.rs

#[test]
fn test_all_environments_valid() {
    use composable_rust_agent_patterns::config::{AgentConfig, Environment};

    // Development should always work without env vars
    let dev = AgentConfig::load(Environment::Development);
    assert!(dev.is_ok(), "Development config should be valid");

    // Production requires DATABASE_URL
    std::env::set_var("DATABASE_URL", "postgres://localhost/test");
    let prod = AgentConfig::load(Environment::Production);
    assert!(prod.is_ok(), "Production config should be valid with DATABASE_URL");
}
```

## Troubleshooting

### "Environment variable not set: DATABASE_URL"

**Cause**: Required secret missing in production/staging.

**Fix**: Set the environment variable:
```bash
export DATABASE_URL="postgres://user:pass@host:5432/db"
```

### "Configuration validation failed: temperature must be between 0.0 and 1.0"

**Cause**: Invalid configuration value.

**Fix**: Check your configuration values against validation rules.

### "Invalid environment: prodution"

**Cause**: Typo in environment name.

**Fix**: Use one of: `dev`, `development`, `staging`, `stage`, `prod`, `production`

## Additional Resources

- [Kubernetes Deployment Guide](k8s/README.md)
- [Docker Deployment Guide](DOCKER.md)
- [Secrets Management Best Practices](https://12factor.net/config)
- [Configuration Examples](examples/)

## Summary

- ✅ Use `AgentConfig::from_env()` for automatic environment detection
- ✅ Load secrets from environment variables, never config files
- ✅ Validate configuration at startup (fail fast)
- ✅ Use type-safe accessors for durations and other conversions
- ✅ Keep environment-specific overrides minimal
- ✅ Test configuration validation in CI/CD
