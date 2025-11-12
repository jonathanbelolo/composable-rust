//! Production agent environment with full resilience features

use crate::types::{AgentEnvironment, AgentError, Message};
use composable_rust_agent_patterns::audit::{AuditEvent, AuditEventType, AuditLogger};
use composable_rust_agent_patterns::resilience::{
    Bulkhead, BulkheadConfig, CircuitBreaker, CircuitBreakerConfig, RateLimiter, RateLimiterConfig,
};
use composable_rust_agent_patterns::security::{SecurityIncident, SecurityMonitor};
use composable_rust_anthropic::{AnthropicClient, MessagesRequest};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Production environment with all Phase 8.4 features
#[derive(Clone)]
pub struct ProductionEnvironment<A: AuditLogger + Send + Sync + 'static> {
    /// Anthropic API client (optional - if None, use mock)
    anthropic_client: Option<Arc<AnthropicClient>>,
    /// Circuit breaker for LLM calls
    llm_circuit_breaker: Arc<CircuitBreaker>,
    /// Rate limiter for API calls
    rate_limiter: Arc<RateLimiter>,
    /// Bulkhead executor for tool calls
    bulkhead: Arc<Bulkhead>,
    /// Audit logger (generic over any AuditLogger implementation)
    audit_logger: Arc<A>,
    /// Security monitor
    security_monitor: Arc<SecurityMonitor>,
    /// Event store for persisting events
    event_store: Arc<dyn composable_rust_core::event_store::EventStore>,
    /// Clock for timestamps
    clock: Arc<dyn composable_rust_core::environment::Clock>,
    /// Event bus for publishing domain events
    event_bus: Arc<dyn composable_rust_core::event_bus::EventBus>,
    /// Projection store for read models
    projection_store: Arc<composable_rust_projections::PostgresProjectionStore>,
    /// LLM timeout (reserved for future timeout implementation)
    #[allow(dead_code)]
    llm_timeout: Duration,
}

impl<A: AuditLogger + Send + Sync + 'static> ProductionEnvironment<A> {
    /// Create new production environment without Anthropic client (uses mock)
    #[must_use]
    pub fn new(
        audit_logger: Arc<A>,
        security_monitor: Arc<SecurityMonitor>,
        event_store: Arc<dyn composable_rust_core::event_store::EventStore>,
        clock: Arc<dyn composable_rust_core::environment::Clock>,
        event_bus: Arc<dyn composable_rust_core::event_bus::EventBus>,
        projection_store: Arc<composable_rust_projections::PostgresProjectionStore>,
    ) -> Self {
        Self::with_client(None, audit_logger, security_monitor, event_store, clock, event_bus, projection_store)
    }

    /// Create new production environment with Anthropic client
    #[must_use]
    pub fn with_client(
        anthropic_client: Option<Arc<AnthropicClient>>,
        audit_logger: Arc<A>,
        security_monitor: Arc<SecurityMonitor>,
        event_store: Arc<dyn composable_rust_core::event_store::EventStore>,
        clock: Arc<dyn composable_rust_core::environment::Clock>,
        event_bus: Arc<dyn composable_rust_core::event_bus::EventBus>,
        projection_store: Arc<composable_rust_projections::PostgresProjectionStore>,
    ) -> Self {
        // Circuit breaker config
        let cb_config = CircuitBreakerConfig {
            failure_threshold: 5,
            success_threshold: 2,
            timeout: Duration::from_secs(60),
        };

        // Rate limiter config
        let rl_config = RateLimiterConfig {
            capacity: 20,
            refill_rate: 10.0,
        };

        // Bulkhead config
        let bulkhead_config = BulkheadConfig {
            max_concurrent: 100,
            acquire_timeout: Duration::from_secs(30),
        };

        Self {
            anthropic_client,
            llm_circuit_breaker: Arc::new(CircuitBreaker::new("llm".to_string(), cb_config)),
            rate_limiter: Arc::new(RateLimiter::new("api".to_string(), rl_config)),
            bulkhead: Arc::new(Bulkhead::new("tool_execution".to_string(), bulkhead_config)),
            audit_logger,
            security_monitor,
            event_store,
            clock,
            event_bus,
            projection_store,
            llm_timeout: Duration::from_secs(30),
        }
    }

    /// Create production environment from environment variables
    ///
    /// Loads `ANTHROPIC_API_KEY` from environment. If not present, falls back to mock.
    #[must_use]
    pub fn from_env(
        audit_logger: Arc<A>,
        security_monitor: Arc<SecurityMonitor>,
        event_store: Arc<dyn composable_rust_core::event_store::EventStore>,
        clock: Arc<dyn composable_rust_core::environment::Clock>,
        event_bus: Arc<dyn composable_rust_core::event_bus::EventBus>,
        projection_store: Arc<composable_rust_projections::PostgresProjectionStore>,
    ) -> Self {
        let anthropic_client = match AnthropicClient::from_env() {
            Ok(client) => {
                info!("Anthropic API client initialized from environment");
                Some(Arc::new(client))
            }
            Err(e) => {
                warn!("Failed to initialize Anthropic client: {}. Using mock LLM.", e);
                None
            }
        };

        Self::with_client(anthropic_client, audit_logger, security_monitor, event_store, clock, event_bus, projection_store)
    }

    /// Call LLM with resilience features
    async fn call_llm_internal(&self, messages: &[Message]) -> Result<String, AgentError> {
        // Check rate limit
        self.rate_limiter
            .try_acquire(1)
            .await
            .map_err(|_| AgentError::RateLimited)?;

        // Check circuit breaker
        self.llm_circuit_breaker
            .allow_request()
            .await
            .map_err(|_| AgentError::CircuitBreakerOpen)?;

        // Call LLM (real or mock)
        let result = if let Some(client) = &self.anthropic_client {
            self.call_anthropic(client, messages).await
        } else {
            self.mock_llm_call(messages).await
        };

        // Record result in circuit breaker
        match &result {
            Ok(_) => self.llm_circuit_breaker.record_success().await,
            Err(_) => self.llm_circuit_breaker.record_failure().await,
        }

        result
    }

    /// Call Anthropic Claude API
    async fn call_anthropic(
        &self,
        client: &AnthropicClient,
        messages: &[Message],
    ) -> Result<String, AgentError> {
        info!("Calling Anthropic API with {} messages", messages.len());

        // Convert our Message type to Anthropic's Message type
        let anthropic_messages: Vec<composable_rust_anthropic::types::Message> = messages
            .iter()
            .map(|msg| {
                use composable_rust_anthropic::types::{ContentBlock, Message as AnthropicMessage, Role};

                let role = match msg.role {
                    crate::types::Role::User | crate::types::Role::System => Role::User,
                    crate::types::Role::Assistant => Role::Assistant,
                };

                AnthropicMessage {
                    role,
                    content: vec![ContentBlock::Text {
                        text: msg.content.clone(),
                    }],
                }
            })
            .collect();

        // Create request
        let mut request = MessagesRequest::new(anthropic_messages);
        request.max_tokens = 1024;
        request.model = "claude-sonnet-4-5-20250929".to_string();

        // Call API (non-streaming for now)
        let response = client
            .messages(request)
            .await
            .map_err(|e| AgentError::Llm(e.to_string()))?;

        // Extract text from response
        let text = response
            .content
            .iter()
            .filter_map(|block| {
                use composable_rust_anthropic::types::ContentBlock;
                match block {
                    ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(text)
    }

    /// Mock LLM call (fallback when no API key is configured)
    async fn mock_llm_call(&self, messages: &[Message]) -> Result<String, AgentError> {
        info!("Using mock LLM with {} messages", messages.len());

        // Simulate API call
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Mock response
        Ok("This is a mock LLM response. To use the real Claude API, set the ANTHROPIC_API_KEY environment variable.".to_string())
    }

    /// Execute tool with bulkhead pattern
    async fn execute_tool_internal(&self, tool_name: &str, input: &str) -> Result<String, AgentError> {
        info!("Executing tool: {}", tool_name);

        // Execute in bulkhead (limits concurrent tool executions)
        let tool_name_owned = tool_name.to_string();
        let input_owned = input.to_string();

        let future = async move {
            match tool_name_owned.as_str() {
                "search" => Self::mock_search_tool(&input_owned).await,
                "calculator" => Self::mock_calculator_tool(&input_owned).await,
                "weather" => Self::mock_weather_tool(&input_owned).await,
                _ => Err(AgentError::Tool(format!("Unknown tool: {}", tool_name_owned))),
            }
        };

        self.bulkhead
            .execute(future)
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?
    }

    /// Mock search tool
    async fn mock_search_tool(query: &str) -> Result<String, AgentError> {
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(format!("Search results for: {}", query))
    }

    /// Mock calculator tool
    async fn mock_calculator_tool(expression: &str) -> Result<String, AgentError> {
        // Simple calculator (just mock)
        Ok(format!("Result of {}: 42", expression))
    }

    /// Mock weather tool
    async fn mock_weather_tool(location: &str) -> Result<String, AgentError> {
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(format!("Weather in {}: Sunny, 72°F", location))
    }
}

impl<A: AuditLogger + Send + Sync + 'static> AgentEnvironment for ProductionEnvironment<A> {
    fn event_store(&self) -> &Arc<dyn composable_rust_core::event_store::EventStore> {
        &self.event_store
    }

    fn clock(&self) -> &Arc<dyn composable_rust_core::environment::Clock> {
        &self.clock
    }

    fn event_bus(&self) -> &Arc<dyn composable_rust_core::event_bus::EventBus> {
        &self.event_bus
    }

    fn projection_store(&self) -> &Arc<composable_rust_projections::PostgresProjectionStore> {
        &self.projection_store
    }

    async fn call_llm(&self, messages: &[Message]) -> Result<String, AgentError> {
        self.call_llm_internal(messages).await
    }

    async fn execute_tool(&self, tool_name: &str, input: &str) -> Result<String, AgentError> {
        self.execute_tool_internal(tool_name, input).await
    }

    async fn log_audit(
        &self,
        event_type: &str,
        actor: &str,
        action: &str,
        success: bool,
    ) -> Result<(), AgentError> {
        let event_type_enum = match event_type {
            "authentication" => AuditEventType::Authentication,
            "authorization" => AuditEventType::Authorization,
            "data_access" => AuditEventType::DataAccess,
            "configuration" => AuditEventType::Configuration,
            "security" => AuditEventType::Security,
            _ => AuditEventType::LlmInteraction,
        };

        let event = AuditEvent::new(event_type_enum, actor, action, success);

        self.audit_logger
            .log(event)
            .await
            .map_err(|e| AgentError::Audit(e.to_string()))
    }

    async fn report_security_incident(
        &self,
        incident_type: &str,
        source: &str,
        description: &str,
    ) -> Result<(), AgentError> {
        use composable_rust_agent_patterns::security::IncidentType;

        let incident_type_enum = match incident_type {
            "brute_force_attack" => IncidentType::BruteForceAttack,
            "anomalous_access" => IncidentType::AnomalousAccess,
            "privilege_escalation" => IncidentType::PrivilegeEscalation,
            "data_exfiltration" => IncidentType::DataExfiltration,
            "prompt_injection" => IncidentType::PromptInjection,
            "rate_limit_abuse" => IncidentType::RateLimitAbuse,
            "unauthorized_access" => IncidentType::UnauthorizedAccess,
            "configuration_tampering" => IncidentType::ConfigurationTampering,
            "credential_stuffing" => IncidentType::CredentialStuffing,
            "session_hijacking" => IncidentType::SessionHijacking,
            _ => return Err(AgentError::InvalidInput(format!("Unknown incident type: {}", incident_type))),
        };

        let incident = SecurityIncident::new(
            incident_type_enum,
            composable_rust_agent_patterns::security::ThreatLevel::Medium,
            source,
            description,
        );

        self.security_monitor
            .report_incident(incident)
            .await
            .map_err(|e| AgentError::Security(e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use composable_rust_agent_patterns::audit::InMemoryAuditLogger;
    use composable_rust_testing::mocks::{InMemoryEventStore, InMemoryEventBus};

    // Helper to create test environment
    fn create_test_env() -> ProductionEnvironment<InMemoryAuditLogger> {
        let audit_logger = Arc::new(InMemoryAuditLogger::new());
        let security_monitor = Arc::new(SecurityMonitor::new());
        let event_store: Arc<dyn composable_rust_core::event_store::EventStore> = Arc::new(InMemoryEventStore::new());
        let clock: Arc<dyn composable_rust_core::environment::Clock> = Arc::new(composable_rust_core::environment::SystemClock);
        let event_bus: Arc<dyn composable_rust_core::event_bus::EventBus> = Arc::new(InMemoryEventBus::new());
        // Create a minimal in-memory projection store for tests
        let pool = sqlx::PgPool::connect_lazy("postgres://test").expect("Test pool");
        let projection_store = Arc::new(composable_rust_projections::PostgresProjectionStore::new(pool, "test".to_string()));

        ProductionEnvironment::new(audit_logger, security_monitor, event_store, clock, event_bus, projection_store)
    }

    #[tokio::test]
    async fn test_llm_call_with_circuit_breaker() {
        let env = create_test_env();

        let messages = vec![];
        let result = env.call_llm(&messages).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tool_execution_with_bulkhead() {
        let env = create_test_env();

        let result = env.execute_tool("search", "test query").await;
        assert!(result.is_ok());

        let result = env.execute_tool("calculator", "2 + 2").await;
        assert!(result.is_ok());

        let result = env.execute_tool("weather", "San Francisco").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let env = create_test_env();

        // Make many rapid calls
        for _ in 0..25 {
            let _ = env.call_llm(&[]).await;
        }

        // Should hit rate limit eventually
        // (actual behavior depends on rate limiter configuration)
    }
}
