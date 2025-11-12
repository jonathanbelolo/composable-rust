//! Agent reducer with event sourcing
//!
//! Implements the Reducer pattern with full event sourcing:
//! - Commands are validated and produce events
//! - Events are persisted to EventStore
//! - State is reconstructed from events

use crate::environment::ProductionEnvironment;
use crate::types::{AgentAction, AgentEnvironment, AgentState, Effects, Message, Role};
use composable_rust_agent_patterns::audit::{AuditEvent, AuditEventType, AuditLogger};
use composable_rust_agent_patterns::security::SecurityMonitor;
use composable_rust_core::{append_events, async_effect};
use composable_rust_core::effect::Effect;
use composable_rust_core::event::SerializedEvent;
use composable_rust_core::reducer::Reducer;
use composable_rust_core::stream::{StreamId, Version};
use smallvec::smallvec;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Production agent reducer with event sourcing
#[derive(Clone)]
pub struct ProductionAgentReducer<A: AuditLogger + Send + Sync + 'static> {
    /// Audit logger (generic over any AuditLogger implementation)
    audit_logger: Arc<A>,
    /// Security monitor (reserved for future security checks in reducer)
    #[allow(dead_code)]
    security_monitor: Arc<SecurityMonitor>,
}

impl<A: AuditLogger + Send + Sync + 'static> ProductionAgentReducer<A> {
    /// Create new reducer
    #[must_use]
    pub fn new(
        audit_logger: Arc<A>,
        security_monitor: Arc<SecurityMonitor>,
    ) -> Self {
        Self {
            audit_logger,
            security_monitor,
        }
    }

    /// Apply an event to state (for event replay)
    ///
    /// This method reconstructs state from persisted events.
    /// It must be deterministic and idempotent.
    pub fn apply_event(state: &mut AgentState, action: &AgentAction) {
        match action {
            AgentAction::ConversationStarted {
                conversation_id,
                user_id,
                session_id,
                ..
            } => {
                state.conversation_id = Some(conversation_id.clone());
                state.user_id = Some(user_id.clone());
                state.session_id = Some(session_id.clone());
            }
            AgentAction::MessageReceived { content, timestamp } => {
                state.messages.push(Message {
                    role: Role::User,
                    content: content.clone(),
                    timestamp: timestamp.clone(),
                });
            }
            AgentAction::ResponseGenerated { response, timestamp } => {
                state.messages.push(Message {
                    role: Role::Assistant,
                    content: response.clone(),
                    timestamp: timestamp.clone(),
                });
            }
            AgentAction::ToolExecuted { .. } => {
                // Tool execution tracking could be added here
            }
            AgentAction::ConversationEnded { .. } => {
                // Mark conversation as ended (could add a flag to state)
            }
            AgentAction::SecurityEventDetected { .. } => {
                // Security events are logged but don't change conversation state
            }
            AgentAction::ValidationFailed { error } => {
                state.last_error = Some(error.clone());
            }
            // Commands and internal actions don't modify state during replay
            AgentAction::StartConversation { .. }
            | AgentAction::SendMessage { .. }
            | AgentAction::ProcessResponse { .. }
            | AgentAction::ExecuteTool { .. }
            | AgentAction::EndConversation
            | AgentAction::EventPersisted { .. } => {
                // Commands are not applied during event replay
            }
        }
    }

    /// Serialize an action (event) to bytes using bincode
    fn serialize_event(action: &AgentAction) -> Result<SerializedEvent, String> {
        let event_type = action.event_type().to_string();
        let data =
            bincode::serialize(action).map_err(|e| format!("Failed to serialize event: {e}"))?;

        Ok(SerializedEvent::new(event_type, data, None))
    }

    /// Create an EventStore effect to append events
    fn create_append_effect<E: AgentEnvironment>(
        env: &E,
        stream_id: &StreamId,
        expected_version: Option<Version>,
        event: AgentAction,
    ) -> Effect<AgentAction> {
        // Serialize the event
        let serialized_event = match Self::serialize_event(&event) {
            Ok(e) => e,
            Err(error) => {
                error!("Failed to serialize event: {error}");
                return Effect::None;
            }
        };

        append_events! {
            store: Arc::clone(env.event_store()),
            stream: stream_id.as_str(),
            expected_version: expected_version,
            events: vec![serialized_event],
            on_success: |version| Some(AgentAction::EventPersisted {
                event: Box::new(event.clone()),
                version: version.value(),
            }),
            on_error: |error| Some(AgentAction::ValidationFailed {
                error: error.to_string(),
            })
        }
    }

    /// Create stream ID from conversation ID
    fn stream_id(conversation_id: &str) -> StreamId {
        StreamId::new(format!("conversation-{conversation_id}"))
    }

    /// Generate a new conversation ID
    fn generate_conversation_id<E: AgentEnvironment>(env: &E) -> String {
        format!("conv_{}", env.clock().now())
    }

    /// Log audit event (async effect)
    fn log_audit_effect(
        audit_logger: &Arc<A>,
        event_type: AuditEventType,
        actor: String,
        action: String,
        success: bool,
    ) -> Effect<AgentAction> {
        let audit_logger = Arc::clone(audit_logger);
        async_effect! {
            let event = AuditEvent::new(event_type, &actor, &action, success);
            if let Err(e) = audit_logger.log(event).await {
                error!("Failed to log audit event: {}", e);
            }
            None
        }
    }
}

impl<A: AuditLogger + Send + Sync + 'static> Reducer for ProductionAgentReducer<A> {
    type State = AgentState;
    type Action = AgentAction;
    type Environment = ProductionEnvironment<A>;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> Effects {
        match action {
            // ========== Commands ==========
            AgentAction::StartConversation {
                user_id,
                session_id,
            } => {
                info!("Starting conversation for user: {}", user_id);

                // Validate: check if conversation already started
                if state.conversation_id.is_some() {
                    warn!("Conversation already started");
                    let validation_failed = AgentAction::ValidationFailed {
                        error: "Conversation already started".to_string(),
                    };
                    Self::apply_event(state, &validation_failed);
                    return smallvec![async_effect! { Some(validation_failed) }];
                }

                // Generate conversation ID
                let conversation_id = Self::generate_conversation_id(env);

                // Create event
                let event = AgentAction::ConversationStarted {
                    conversation_id: conversation_id.clone(),
                    user_id: user_id.clone(),
                    session_id: session_id.clone(),
                    timestamp: env.clock().now().to_string(),
                };

                // Apply event optimistically to state for immediate read
                Self::apply_event(state, &event);
                // Optimistically update version (will be corrected by EventPersisted)
                state.version = state.version.map(|v| Version::new(v.value() + 1)).or(Some(Version::new(1)));

                // Create stream ID
                let stream_id = Self::stream_id(&conversation_id);

                // Append event to EventStore
                let append_effect = Self::create_append_effect(env, &stream_id, None, event);

                // Log audit event
                let audit_effect = Self::log_audit_effect(
                    &self.audit_logger,
                    AuditEventType::LlmInteraction,
                    user_id.clone(),
                    format!("start_conversation:{}:{}", conversation_id, session_id),
                    true,
                );

                smallvec![append_effect, audit_effect]
            }

            AgentAction::SendMessage { content, source_ip } => {
                info!("Sending message");

                // Validate: check if conversation started
                let conversation_id = match state.conversation_id.clone() {
                    Some(id) => id,
                    None => {
                        warn!("No active conversation");
                        let validation_failed = AgentAction::ValidationFailed {
                            error: "No active conversation".to_string(),
                        };
                        Self::apply_event(state, &validation_failed);
                        return smallvec![async_effect! { Some(validation_failed) }];
                    }
                };

                // Create event
                let event = AgentAction::MessageReceived {
                    content: content.clone(),
                    timestamp: env.clock().now().to_string(),
                };

                // Apply event optimistically to state for immediate read
                Self::apply_event(state, &event);
                // Optimistically increment version
                state.version = state.version.map(|v| Version::new(v.value() + 1));

                // Create stream ID
                let stream_id = Self::stream_id(&conversation_id);

                // Append event to EventStore (use old version before increment)
                let expected_version = state.version.map(|v| Version::new(v.value() - 1));
                let append_effect =
                    Self::create_append_effect(env, &stream_id, expected_version, event);

                // Log audit event
                let actor = state
                    .user_id
                    .as_ref()
                    .map_or_else(|| "unknown".to_string(), std::clone::Clone::clone);
                let action_desc = format!(
                    "send_message:{}:{}",
                    conversation_id,
                    source_ip.as_deref().unwrap_or("unknown")
                );
                let audit_effect = Self::log_audit_effect(
                    &self.audit_logger,
                    AuditEventType::LlmInteraction,
                    actor,
                    action_desc,
                    true,
                );

                smallvec![append_effect, audit_effect]
            }

            AgentAction::ProcessResponse { response } => {
                info!("Processing LLM response");

                // Validate: check if conversation started
                let conversation_id = match state.conversation_id.clone() {
                    Some(id) => id,
                    None => {
                        warn!("No active conversation");
                        let validation_failed = AgentAction::ValidationFailed {
                            error: "No active conversation".to_string(),
                        };
                        Self::apply_event(state, &validation_failed);
                        return smallvec![async_effect! { Some(validation_failed) }];
                    }
                };

                // Create event
                let event = AgentAction::ResponseGenerated {
                    response: response.clone(),
                    timestamp: env.clock().now().to_string(),
                };

                // Apply event optimistically to state for immediate read
                Self::apply_event(state, &event);
                // Optimistically increment version
                state.version = state.version.map(|v| Version::new(v.value() + 1));

                // Create stream ID
                let stream_id = Self::stream_id(&conversation_id);

                // Append event to EventStore (use old version before increment)
                let expected_version = state.version.map(|v| Version::new(v.value() - 1));
                smallvec![Self::create_append_effect(env, &stream_id, expected_version, event)]
            }

            AgentAction::ExecuteTool { tool_name, input } => {
                info!("Executing tool: {} (not yet implemented in event-sourced version)", tool_name);

                // Validate: check if conversation started
                let Some(ref _conversation_id) = state.conversation_id else {
                    warn!("No active conversation");
                    let validation_failed = AgentAction::ValidationFailed {
                        error: "No active conversation".to_string(),
                    };
                    Self::apply_event(state, &validation_failed);
                    return smallvec![async_effect! { Some(validation_failed) }];
                };

                // TODO: Implement tool execution with proper event sourcing
                // For now, just acknowledge without executing
                warn!("Tool execution ({}) not yet implemented in event-sourced reducer", tool_name);
                warn!("Input: {}", input);

                smallvec![Effect::None]
            }

            AgentAction::EndConversation => {
                info!("Ending conversation");

                // Validate: check if conversation started
                let Some(ref conversation_id) = state.conversation_id else {
                    warn!("No active conversation");
                    let validation_failed = AgentAction::ValidationFailed {
                        error: "No active conversation".to_string(),
                    };
                    Self::apply_event(state, &validation_failed);
                    return smallvec![async_effect! { Some(validation_failed) }];
                };

                // Create event
                let event = AgentAction::ConversationEnded {
                    timestamp: env.clock().now().to_string(),
                };

                // Create stream ID
                let stream_id = Self::stream_id(conversation_id);

                // Append event to EventStore
                smallvec![Self::create_append_effect(env, &stream_id, state.version, event)]
            }

            // ========== Events (from event replay or EventPersisted) ==========
            AgentAction::ConversationStarted { .. }
            | AgentAction::MessageReceived { .. }
            | AgentAction::ResponseGenerated { .. }
            | AgentAction::ToolExecuted { .. }
            | AgentAction::ConversationEnded { .. }
            | AgentAction::SecurityEventDetected { .. } => {
                // Apply event to state
                Self::apply_event(state, &action);

                // Track version during event replay
                state.version = match state.version {
                    None => Some(Version::new(1)),
                    Some(v) => Some(v.next()),
                };

                smallvec![Effect::None]
            }

            AgentAction::EventPersisted { event, version } => {
                // Event was already applied optimistically, just update version to persisted value
                // This ensures version stays in sync with EventStore
                state.version = Some(Version::new(version));

                // Publish event to event bus for cross-aggregate communication
                let serialized_event = match Self::serialize_event(&event) {
                    Ok(e) => e,
                    Err(error) => {
                        error!("Failed to serialize event for publishing: {error}");
                        return smallvec![Effect::None];
                    }
                };

                use composable_rust_core::effect::{Effect, EventBusOperation};
                let publish_effect = Effect::PublishEvent(EventBusOperation::Publish {
                    event_bus: Arc::clone(env.event_bus()),
                    topic: "agent-events".to_string(),
                    event: serialized_event,
                    on_success: Box::new(|()| {
                        info!("Event published to event bus successfully");
                        None
                    }),
                    on_error: Box::new(|error| {
                        error!("Failed to publish event to event bus: {}", error);
                        None
                    }),
                });

                smallvec![publish_effect]
            }

            AgentAction::ValidationFailed { ref error } => {
                error!("Validation failed: {}", error);
                Self::apply_event(state, &action);
                smallvec![Effect::None]
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    use super::*;
    use composable_rust_agent_patterns::audit::InMemoryAuditLogger;
    use composable_rust_agent_patterns::security::SecurityMonitor;
    use composable_rust_core::environment::{Clock, SystemClock};
    use composable_rust_testing::mocks::InMemoryEventStore;
    use crate::environment::ProductionEnvironment;

    #[tokio::test]
    async fn test_start_conversation() {
        let audit_logger = Arc::new(InMemoryAuditLogger::new());
        let security_monitor = Arc::new(SecurityMonitor::new());
        let event_store: Arc<dyn composable_rust_core::event_store::EventStore> =
            Arc::new(InMemoryEventStore::new());
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);

        let event_bus: Arc<dyn composable_rust_core::event_bus::EventBus> = Arc::new(composable_rust_testing::mocks::InMemoryEventBus::new());
        let pool = sqlx::PgPool::connect_lazy("postgres://test").expect("Test pool");
        let projection_store = Arc::new(composable_rust_projections::PostgresProjectionStore::new(pool, "test".to_string()));

        let env = ProductionEnvironment::new(
            audit_logger.clone(),
            security_monitor,
            event_store,
            clock,
            event_bus,
            projection_store,
        );

        let reducer = ProductionAgentReducer::new(audit_logger, Arc::new(SecurityMonitor::new()));
        let mut state = AgentState::new();

        let action = AgentAction::StartConversation {
            user_id: "test_user".to_string(),
            session_id: "test_session".to_string(),
        };

        let effects = reducer.reduce(&mut state, action, &env);

        // Should return append effect and audit effect
        assert_eq!(effects.len(), 2);
    }
}
