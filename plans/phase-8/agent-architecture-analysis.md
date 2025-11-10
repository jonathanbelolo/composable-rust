# Phase 8: AI Agents Architecture Analysis

## Executive Summary

This document analyzes how to implement AI agents using Anthropic's Claude API within the composable-rust architecture. After studying Anthropic's agent patterns, tool use API, and design principles, we've identified a near-perfect architectural fit: **agents are reducers, tool use is effects, and multi-agent coordination is EventBus**.

**Key Insight**: Composable-rust's functional architecture provides natural, type-safe, testable abstractions for all seven of Anthropic's agent patterns. The framework we built for business process management is ideal for agentic systems.

## Source Material

This analysis is based on three primary Anthropic resources:

1. **[Building Effective AI Agents](https://www.anthropic.com/research/building-effective-agents)** - Core patterns and design principles
2. **[Tool Use Documentation](https://docs.claude.com/en/docs/agents-and-tools/tool-use/overview)** - API specification and workflows
3. **[Agent Patterns Cookbook](https://github.com/anthropics/anthropic-cookbook/tree/main/patterns/agents)** - Reference implementations

## Anthropic's Seven Agent Patterns

### 1. Augmented LLM (Foundation)
**What**: LLM + retrieval + tools + memory
**When**: Base building block for all agents
**Complexity**: Low

### 2. Prompt Chaining (Workflow)
**What**: Sequential LLM calls, each processing previous outputs
**When**: Tasks decomposable into fixed subtasks
**Complexity**: Low-Medium
**Examples**: Generate outline → write sections → review

### 3. Routing (Workflow)
**What**: Classify inputs and direct to specialized handlers
**When**: Complex tasks with distinct categories
**Complexity**: Low-Medium
**Examples**: Customer support triage, model selection by complexity

### 4. Parallelization (Workflow)
**What**: Concurrent LLM calls with aggregated outputs
**Variants**: Sectioning (independent subtasks), Voting (same task multiple times)
**When**: Speed optimization or multi-perspective validation
**Complexity**: Medium

### 5. Orchestrator-Workers (Workflow)
**What**: Central LLM dynamically decomposes and delegates to workers
**When**: Unpredictable subtask requirements
**Complexity**: Medium-High
**Examples**: Multi-file code changes, research aggregation

### 6. Evaluator-Optimizer (Workflow)
**What**: One LLM generates, another iteratively evaluates and refines
**When**: Clear evaluation criteria exist
**Complexity**: Medium-High
**Examples**: Literary translation, complex search

### 7. Autonomous Agents (Advanced)
**What**: LLMs dynamically plan and execute multi-step tasks with environmental feedback
**When**: Open-ended problems with unpredictable step counts
**Complexity**: High
**Examples**: SWE-bench coding, computer automation

## Anthropic's Design Principles

### 1. Simplicity First
> "Start with simple prompts, optimize them with comprehensive evaluation, and add multi-step agentic systems only when simpler solutions fall short."

**Composable-rust alignment**: Our reducer-first approach encourages starting simple. Complex patterns emerge from composition.

### 2. Transparency
> "Explicitly display agent planning steps and decision-making processes."

**Composable-rust alignment**: All state transitions are explicit actions. Event sourcing provides complete audit trail.

### 3. Agent-Computer Interface (ACI) Design
> "Invest effort in tool documentation and testing equivalent to human-computer interface (HCI) design."

**Composable-rust alignment**: Tools as environment traits with comprehensive testing support built-in.

## Anthropic Tool Use API

### Core Workflow

```
1. Define tools → 2. Claude decides → 3. Execute tool → 4. Return results → 5. Claude continues
```

### Tool Definition Structure

```json
{
  "name": "get_weather",
  "description": "Get the current weather in a given location",
  "input_schema": {
    "type": "object",
    "properties": {
      "location": {
        "type": "string",
        "description": "The city and state, e.g. San Francisco, CA"
      }
    },
    "required": ["location"]
  }
}
```

### Stop Reasons

- `end_turn` - Final response, no tools needed
- `tool_use` - Claude wants to use one or more tools
- `max_tokens` - Hit token limit (continue conversation)

### Multi-Step Patterns

- **Sequential**: Tool output feeds next tool input (one at a time)
- **Parallel**: Independent tools called simultaneously (return all results together)

## Mapping to Composable-Rust Architecture

### 1. Agents as Reducers

**Core Insight**: An agent is a state machine processing a conversation loop.

```rust
/// Agent state encapsulates conversation history and workflow phase
#[derive(Clone, Debug)]
struct AgentState {
    /// Conversation message history
    messages: Vec<Message>,

    /// Current workflow step (for multi-step patterns)
    current_step: Step,

    /// Available tools
    tools: Vec<Tool>,

    /// Optional memory/context
    memory: Option<Memory>,

    /// Workflow-specific state (routing classification, evaluation scores, etc.)
    workflow_state: WorkflowState,
}

/// Agent actions represent all inputs to the conversation loop
#[derive(Clone, Debug)]
enum AgentAction {
    /// User sends a message
    UserMessage { content: String, attachments: Vec<Attachment> },

    /// Claude responds (possibly with tool use requests)
    ClaudeResponse {
        content: Vec<ContentBlock>,
        stop_reason: StopReason,
        tool_uses: Vec<ToolUse>,
        usage: Usage,
    },

    /// Tool execution completed
    ToolResult {
        tool_use_id: String,
        result: Result<String, ToolError>,
    },

    /// Multiple tool results (for parallel execution)
    ToolResults {
        results: Vec<(String, Result<String, ToolError>)>,
    },

    /// Workflow-specific actions
    StepCompleted { step: Step, output: String },
    EvaluationReceived { score: f64, feedback: String },
    WorkerResponse { worker_id: String, output: String },
}
```

**Why this works**:
- State is cloneable (conversation history)
- Actions are explicit (every interaction is visible)
- Reducer is pure logic (decisions about what to do next)
- Effects handle I/O (API calls, tool execution)

### 2. Tool Use as Effects

**Core Insight**: Tool execution is a side effect that returns an action.

```rust
impl Reducer for AgentReducer {
    type State = AgentState;
    type Action = AgentAction;
    type Environment = AgentEnvironment;

    fn reduce(
        &self,
        state: &mut Self::State,
        action: Self::Action,
        env: &Self::Environment,
    ) -> Vec<Effect<Self::Action>> {
        match action {
            AgentAction::UserMessage { content, attachments } => {
                // Add user message to history
                state.messages.push(Message::user(content));

                // Call Claude API
                let messages = state.messages.clone();
                let tools = state.tools.clone();

                vec![Effect::Future(Box::pin(async move {
                    let response = env.claude_client()
                        .messages()
                        .create(MessagesRequest {
                            model: "claude-sonnet-4-5-20250929",
                            messages,
                            tools: Some(tools),
                            max_tokens: 4096,
                        })
                        .await?;

                    Ok(Some(AgentAction::ClaudeResponse {
                        content: response.content,
                        stop_reason: response.stop_reason,
                        tool_uses: extract_tool_uses(&response.content),
                        usage: response.usage,
                    }))
                }))]
            }

            AgentAction::ClaudeResponse { content, tool_uses, stop_reason, .. } => {
                // Add assistant message to history
                state.messages.push(Message::assistant(content));

                match stop_reason {
                    StopReason::ToolUse if !tool_uses.is_empty() => {
                        // Claude wants to use tools - execute them
                        if tool_uses.len() == 1 {
                            // Sequential: single tool
                            let tool_use = tool_uses[0].clone();
                            vec![Effect::Future(Box::pin(async move {
                                let result = env.execute_tool(
                                    &tool_use.name,
                                    &tool_use.input,
                                ).await;

                                Ok(Some(AgentAction::ToolResult {
                                    tool_use_id: tool_use.id,
                                    result,
                                }))
                            }))]
                        } else {
                            // Parallel: multiple tools
                            let futures = tool_uses.into_iter().map(|tool_use| {
                                async move {
                                    let result = env.execute_tool(
                                        &tool_use.name,
                                        &tool_use.input,
                                    ).await;
                                    (tool_use.id, result)
                                }
                            });

                            vec![Effect::Future(Box::pin(async move {
                                let results = futures::future::join_all(futures).await;
                                Ok(Some(AgentAction::ToolResults { results }))
                            }))]
                        }
                    }

                    StopReason::EndTurn => {
                        // Conversation turn complete
                        vec![Effect::None]
                    }

                    _ => vec![Effect::None],
                }
            }

            AgentAction::ToolResult { tool_use_id, result } => {
                // Add tool result to message history
                state.messages.push(Message::tool_result(tool_use_id, result));

                // Continue conversation with Claude
                let messages = state.messages.clone();
                let tools = state.tools.clone();

                vec![Effect::Future(Box::pin(async move {
                    let response = env.claude_client()
                        .messages()
                        .create(MessagesRequest {
                            model: "claude-sonnet-4-5-20250929",
                            messages,
                            tools: Some(tools),
                            max_tokens: 4096,
                        })
                        .await?;

                    Ok(Some(AgentAction::ClaudeResponse {
                        content: response.content,
                        stop_reason: response.stop_reason,
                        tool_uses: extract_tool_uses(&response.content),
                        usage: response.usage,
                    }))
                }))]
            }

            AgentAction::ToolResults { results } => {
                // Add all tool results to message history
                for (tool_use_id, result) in results {
                    state.messages.push(Message::tool_result(tool_use_id, result));
                }

                // Continue conversation with Claude
                let messages = state.messages.clone();
                let tools = state.tools.clone();

                vec![Effect::Future(Box::pin(async move {
                    let response = env.claude_client()
                        .messages()
                        .create(MessagesRequest {
                            model: "claude-sonnet-4-5-20250929",
                            messages,
                            tools: Some(tools),
                            max_tokens: 4096,
                        })
                        .await?;

                    Ok(Some(AgentAction::ClaudeResponse {
                        content: response.content,
                        stop_reason: response.stop_reason,
                        tool_uses: extract_tool_uses(&response.content),
                        usage: response.usage,
                    }))
                }))]
            }

            _ => vec![Effect::None],
        }
    }
}
```

**Why this works**:
- Effects are explicit (every API call and tool execution is visible)
- Effects return actions (feedback loop is natural)
- Effects are composable (parallel, sequential, conditional)
- Effects are testable (mock environment in tests)

### 3. Tools as Environment Traits

**Core Insight**: Tools are injected dependencies, just like Clock or Database.

```rust
/// Tool definition following Anthropic's schema
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Tool {
    name: String,
    description: String,
    input_schema: JsonSchema,
}

/// Agent environment provides Claude API and tool execution
#[async_trait::async_trait]
trait AgentEnvironment: Send + Sync {
    /// Get available tools
    fn tools(&self) -> &[Tool];

    /// Execute a tool by name with JSON input
    async fn execute_tool(&self, name: &str, input: &str) -> Result<String, ToolError>;

    /// Call Claude API
    fn claude_client(&self) -> &dyn ClaudeClient;

    /// Clock for timeouts
    fn clock(&self) -> &dyn Clock;
}

/// Concrete implementation for production
struct ProductionAgentEnvironment {
    tools: Vec<Tool>,
    tool_executors: HashMap<String, Box<dyn ToolExecutor>>,
    claude_client: AnthropicClient,
    clock: SystemClock,
}

/// Mock implementation for testing
struct MockAgentEnvironment {
    tools: Vec<Tool>,
    tool_responses: HashMap<String, String>,
    claude_responses: VecDeque<MessagesResponse>,
    clock: FixedClock,
}

/// Tool executor trait for individual tools
#[async_trait::async_trait]
trait ToolExecutor: Send + Sync {
    async fn execute(&self, input: &str) -> Result<String, ToolError>;
}
```

**Why this works**:
- Tools are type-safe (trait-based)
- Tools are testable (mock implementations)
- Tools are composable (implement ToolExecutor for any tool)
- Tools are discoverable (tools() method lists available tools)

### 4. Agent Patterns as State Machines

Each of Anthropic's seven patterns maps to a different state machine configuration:

#### Pattern 1: Augmented LLM (Base)

```rust
enum BasicAgentStep {
    Idle,
    WaitingForClaude,
    ExecutingTools,
}

// State: current_step tracks conversation phase
// Actions: UserMessage → ClaudeResponse → ToolResult → ClaudeResponse
// Simple feedback loop
```

#### Pattern 2: Prompt Chaining

```rust
enum ChainStep {
    GenerateOutline,
    ValidateOutline,
    WriteSectionOne,
    WriteSectionTwo,
    WriteSectionThree,
    FinalReview,
    Complete,
}

// State: current_step advances through fixed sequence
// Actions: StepCompleted advances to next step
// Gates: validation checks between steps
```

```rust
impl Reducer for ChainAgentReducer {
    fn reduce(&self, state: &mut AgentState, action: AgentAction, env: &Env)
        -> Vec<Effect<AgentAction>>
    {
        match (&state.current_step, action) {
            (ChainStep::GenerateOutline, AgentAction::ClaudeResponse { content, .. }) => {
                // Extract outline from response
                let outline = extract_outline(&content);

                // Validate outline
                if self.validate_outline(&outline) {
                    state.current_step = ChainStep::WriteSectionOne;
                    vec![self.call_claude_for_step(ChainStep::WriteSectionOne, state)]
                } else {
                    // Gate failed - retry
                    vec![self.call_claude_for_step(ChainStep::GenerateOutline, state)]
                }
            }

            (ChainStep::WriteSectionOne, AgentAction::ClaudeResponse { .. }) => {
                state.current_step = ChainStep::WriteSectionTwo;
                vec![self.call_claude_for_step(ChainStep::WriteSectionTwo, state)]
            }

            // ... more steps

            _ => vec![Effect::None],
        }
    }
}
```

#### Pattern 3: Routing

```rust
enum RouteTarget {
    GeneralSupport,
    RefundSpecialist,
    TechnicalSupport,
    Escalation,
}

// State: workflow_state stores classification result
// Actions: UserMessage → classify → route to specialized reducer
// Uses Effect::PublishEvent to route to different agents
```

```rust
impl Reducer for RouterAgentReducer {
    fn reduce(&self, state: &mut AgentState, action: AgentAction, env: &Env)
        -> Vec<Effect<AgentAction>>
    {
        match action {
            AgentAction::UserMessage { content, .. } => {
                // First, classify the message
                vec![Effect::Future(Box::pin(async move {
                    let classification = env.claude_client()
                        .classify_message(&content)
                        .await?;

                    Ok(Some(AgentAction::MessageClassified {
                        category: classification.category,
                        confidence: classification.confidence,
                    }))
                }))]
            }

            AgentAction::MessageClassified { category, confidence } => {
                // Route to appropriate handler
                match category {
                    Category::General => {
                        vec![Effect::PublishEvent(Event::RouteToGeneral { .. })]
                    }
                    Category::Refund if confidence > 0.8 => {
                        vec![Effect::PublishEvent(Event::RouteToRefund { .. })]
                    }
                    Category::Technical => {
                        vec![Effect::PublishEvent(Event::RouteToTechnical { .. })]
                    }
                    _ => {
                        vec![Effect::PublishEvent(Event::RouteToEscalation { .. })]
                    }
                }
            }

            _ => vec![Effect::None],
        }
    }
}
```

#### Pattern 4: Parallelization

```rust
// Sectioning variant: independent subtasks
enum ParallelSection {
    Introduction,
    BodyOne,
    BodyTwo,
    Conclusion,
}

// Voting variant: same task multiple times
struct VotingState {
    responses: Vec<String>,
    required_votes: usize,
}

// Uses Effect::Parallel to execute multiple LLM calls concurrently
```

```rust
impl Reducer for ParallelAgentReducer {
    fn reduce(&self, state: &mut AgentState, action: AgentAction, env: &Env)
        -> Vec<Effect<AgentAction>>
    {
        match action {
            AgentAction::UserMessage { content, .. } => {
                // Decompose into parallel sections
                let sections = self.decompose_task(&content);

                // Create parallel effects for each section
                let effects = sections.into_iter().map(|section| {
                    Effect::Future(Box::pin(async move {
                        let response = env.claude_client()
                            .generate_section(&section)
                            .await?;

                        Ok(Some(AgentAction::SectionCompleted {
                            section: section.id,
                            content: response.content,
                        }))
                    }))
                }).collect();

                vec![Effect::Parallel(effects)]
            }

            AgentAction::SectionCompleted { section, content } => {
                // Collect sections
                state.completed_sections.insert(section, content);

                if state.completed_sections.len() == state.total_sections {
                    // All sections complete - aggregate
                    vec![self.aggregate_sections(state)]
                } else {
                    vec![Effect::None]
                }
            }

            _ => vec![Effect::None],
        }
    }
}
```

#### Pattern 5: Orchestrator-Workers

```rust
struct OrchestratorState {
    pending_subtasks: Vec<Subtask>,
    active_workers: HashMap<WorkerId, Subtask>,
    completed_subtasks: Vec<SubtaskResult>,
}

// Orchestrator dynamically decomposes task
// Workers execute subtasks independently
// Uses EventBus for coordination (like Sagas!)
```

```rust
impl Reducer for OrchestratorReducer {
    fn reduce(&self, state: &mut OrchestratorState, action: OrchestratorAction, env: &Env)
        -> Vec<Effect<OrchestratorAction>>
    {
        match action {
            OrchestratorAction::TaskReceived { description } => {
                // Orchestrator decomposes task dynamically
                vec![Effect::Future(Box::pin(async move {
                    let decomposition = env.claude_client()
                        .decompose_task(&description)
                        .await?;

                    Ok(Some(OrchestratorAction::SubtasksIdentified {
                        subtasks: decomposition.subtasks,
                    }))
                }))]
            }

            OrchestratorAction::SubtasksIdentified { subtasks } => {
                state.pending_subtasks = subtasks.clone();

                // Assign to workers via EventBus
                let effects = subtasks.into_iter().map(|subtask| {
                    Effect::PublishEvent(WorkerEvent::SubtaskAssigned {
                        worker_id: self.assign_worker(),
                        subtask,
                    })
                }).collect();

                effects
            }

            // Cross-aggregate event from worker
            OrchestratorAction::WorkerCompleted { worker_id, result } => {
                state.completed_subtasks.push(result);
                state.active_workers.remove(&worker_id);

                if state.completed_subtasks.len() == state.total_subtasks {
                    // All workers done - synthesize results
                    vec![self.synthesize_results(state)]
                } else {
                    vec![Effect::None]
                }
            }

            _ => vec![Effect::None],
        }
    }
}

// Worker reducer (separate aggregate)
impl Reducer for WorkerReducer {
    fn reduce(&self, state: &mut WorkerState, action: WorkerAction, env: &Env)
        -> Vec<Effect<WorkerAction>>
    {
        match action {
            // Cross-aggregate event from orchestrator
            WorkerAction::SubtaskAssigned { subtask } => {
                state.current_task = Some(subtask.clone());

                // Execute subtask with Claude
                vec![Effect::Future(Box::pin(async move {
                    let result = env.claude_client()
                        .execute_subtask(&subtask)
                        .await?;

                    Ok(Some(WorkerAction::SubtaskCompleted { result }))
                }))]
            }

            WorkerAction::SubtaskCompleted { result } => {
                // Publish completion event back to orchestrator
                vec![Effect::PublishEvent(OrchestratorEvent::WorkerCompleted {
                    worker_id: state.id,
                    result,
                })]
            }

            _ => vec![Effect::None],
        }
    }
}
```

**This is exactly our Saga pattern!** Orchestrator = parent saga, Workers = child sagas. EventBus coordinates them.

#### Pattern 6: Evaluator-Optimizer

```rust
struct EvaluatorState {
    current_draft: Option<String>,
    evaluations: Vec<Evaluation>,
    iteration: usize,
    max_iterations: usize,
}

enum EvalStep {
    Generate,
    Evaluate,
    Refine,
    Complete,
}

// Generator creates draft
// Evaluator scores and provides feedback
// Loop until evaluation meets threshold or max iterations
```

```rust
impl Reducer for EvaluatorOptimizerReducer {
    fn reduce(&self, state: &mut EvaluatorState, action: EvalAction, env: &Env)
        -> Vec<Effect<EvalAction>>
    {
        match (&state.current_step, action) {
            (EvalStep::Generate, EvalAction::UserMessage { prompt }) => {
                // Generate initial draft
                vec![Effect::Future(Box::pin(async move {
                    let draft = env.claude_client()
                        .generate(&prompt)
                        .await?;

                    Ok(Some(EvalAction::DraftGenerated { content: draft }))
                }))]
            }

            (EvalStep::Evaluate, EvalAction::DraftGenerated { content }) => {
                state.current_draft = Some(content.clone());
                state.current_step = EvalStep::Evaluate;

                // Evaluate draft with separate LLM call
                vec![Effect::Future(Box::pin(async move {
                    let evaluation = env.claude_client()
                        .evaluate(&content, &evaluation_criteria)
                        .await?;

                    Ok(Some(EvalAction::EvaluationReceived {
                        score: evaluation.score,
                        feedback: evaluation.feedback,
                    }))
                }))]
            }

            (EvalStep::Evaluate, EvalAction::EvaluationReceived { score, feedback }) => {
                state.evaluations.push(Evaluation { score, feedback: feedback.clone() });
                state.iteration += 1;

                if score >= state.threshold {
                    // Good enough!
                    state.current_step = EvalStep::Complete;
                    vec![Effect::None]
                } else if state.iteration >= state.max_iterations {
                    // Max iterations reached
                    state.current_step = EvalStep::Complete;
                    vec![Effect::None]
                } else {
                    // Refine based on feedback
                    state.current_step = EvalStep::Refine;
                    vec![Effect::Future(Box::pin(async move {
                        let refined = env.claude_client()
                            .refine(&state.current_draft.unwrap(), &feedback)
                            .await?;

                        Ok(Some(EvalAction::DraftGenerated { content: refined }))
                    }))]
                }
            }

            _ => vec![Effect::None],
        }
    }
}
```

#### Pattern 7: Autonomous Agents

```rust
struct AutonomousState {
    goal: String,
    plan: Option<Vec<Step>>,
    executed_steps: Vec<Step>,
    observations: Vec<Observation>,
    max_steps: usize,
}

enum AutonomousStep {
    Planning,
    Executing,
    Observing,
    Replanning,
    AwaitingHumanApproval,
    Complete,
}

// Dynamic planning with environmental feedback
// Human checkpoints for critical decisions
// Stopping conditions based on goal completion or max steps
```

```rust
impl Reducer for AutonomousAgentReducer {
    fn reduce(&self, state: &mut AutonomousState, action: AutonomousAction, env: &Env)
        -> Vec<Effect<AutonomousAction>>
    {
        match (&state.current_step, action) {
            (AutonomousStep::Planning, AutonomousAction::GoalSet { goal }) => {
                state.goal = goal.clone();

                // Plan steps to achieve goal
                vec![Effect::Future(Box::pin(async move {
                    let plan = env.claude_client()
                        .plan_steps(&goal)
                        .await?;

                    Ok(Some(AutonomousAction::PlanGenerated { steps: plan.steps }))
                }))]
            }

            (AutonomousStep::Planning, AutonomousAction::PlanGenerated { steps }) => {
                state.plan = Some(steps.clone());

                // Check if plan needs human approval
                if self.requires_human_approval(&steps) {
                    state.current_step = AutonomousStep::AwaitingHumanApproval;
                    vec![Effect::None] // Wait for human approval action
                } else {
                    state.current_step = AutonomousStep::Executing;
                    vec![self.execute_next_step(state, env)]
                }
            }

            (AutonomousStep::AwaitingHumanApproval, AutonomousAction::HumanApproved) => {
                state.current_step = AutonomousStep::Executing;
                vec![self.execute_next_step(state, env)]
            }

            (AutonomousStep::Executing, AutonomousAction::StepCompleted { step, observation }) => {
                state.executed_steps.push(step);
                state.observations.push(observation.clone());

                // Check stopping conditions
                if self.goal_achieved(&state.observations) {
                    state.current_step = AutonomousStep::Complete;
                    vec![Effect::None]
                } else if state.executed_steps.len() >= state.max_steps {
                    state.current_step = AutonomousStep::Complete;
                    vec![Effect::None]
                } else if self.should_replan(&observation) {
                    // Environment feedback suggests replanning
                    state.current_step = AutonomousStep::Replanning;
                    vec![self.replan(state, env)]
                } else {
                    // Continue with next step
                    vec![self.execute_next_step(state, env)]
                }
            }

            (AutonomousStep::Replanning, AutonomousAction::PlanGenerated { steps }) => {
                state.plan = Some(steps);
                state.current_step = AutonomousStep::Executing;
                vec![self.execute_next_step(state, env)]
            }

            _ => vec![Effect::None],
        }
    }
}
```

### 5. Multi-Agent Coordination via EventBus

**Core Insight**: Multiple agents coordinating = multiple reducers with EventBus (Phase 3 architecture).

```rust
// Agent 1: Research Agent
impl Reducer for ResearchAgentReducer {
    fn reduce(&self, state: &mut ResearchState, action: ResearchAction, env: &Env)
        -> Vec<Effect<ResearchAction>>
    {
        match action {
            ResearchAction::QueryReceived { topic } => {
                // Research the topic
                vec![Effect::Future(Box::pin(async move {
                    let findings = env.research(&topic).await?;
                    Ok(Some(ResearchAction::ResearchCompleted { findings }))
                }))]
            }

            ResearchAction::ResearchCompleted { findings } => {
                // Publish findings to other agents
                vec![Effect::PublishEvent(AgentEvent::ResearchFindings {
                    topic: state.current_topic.clone(),
                    findings,
                })]
            }

            // Cross-aggregate event from Writing Agent
            ResearchAction::MoreInfoRequested { clarification } => {
                // Handle follow-up research request
                vec![self.research_clarification(clarification, env)]
            }

            _ => vec![Effect::None],
        }
    }
}

// Agent 2: Writing Agent
impl Reducer for WritingAgentReducer {
    fn reduce(&self, state: &mut WritingState, action: WritingAction, env: &Env)
        -> Vec<Effect<WritingAction>>
    {
        match action {
            // Cross-aggregate event from Research Agent
            WritingAction::ResearchFindings { topic, findings } => {
                state.research_data.insert(topic.clone(), findings);

                // Check if we have enough data to write
                if self.has_sufficient_data(state) {
                    vec![self.start_writing(state, env)]
                } else {
                    // Request more information
                    vec![Effect::PublishEvent(AgentEvent::MoreInfoRequested {
                        clarification: self.identify_gaps(state),
                    })]
                }
            }

            WritingAction::DraftCompleted { draft } => {
                // Publish draft for review
                vec![Effect::PublishEvent(AgentEvent::DraftReady {
                    content: draft,
                })]
            }

            _ => vec![Effect::None],
        }
    }
}

// Agent 3: Review Agent
impl Reducer for ReviewAgentReducer {
    fn reduce(&self, state: &mut ReviewState, action: ReviewAction, env: &Env)
        -> Vec<Effect<ReviewAction>>
    {
        match action {
            // Cross-aggregate event from Writing Agent
            ReviewAction::DraftReady { content } => {
                vec![Effect::Future(Box::pin(async move {
                    let review = env.review(&content).await?;
                    Ok(Some(ReviewAction::ReviewCompleted { feedback: review }))
                }))]
            }

            ReviewAction::ReviewCompleted { feedback } => {
                // Publish feedback
                vec![Effect::PublishEvent(AgentEvent::ReviewFeedback {
                    feedback,
                })]
            }

            _ => vec![Effect::None],
        }
    }
}
```

**This is exactly our multi-aggregate pattern from Phase 3!** Each agent is an aggregate, EventBus coordinates them.

### 6. Event Sourcing for Agent Audit Trails

**Core Insight**: Every agent interaction is an event. Full observability and replay.

```rust
// Events for audit trail
enum AgentEvent {
    ConversationStarted { agent_id: String, user_id: String, timestamp: DateTime },
    MessageReceived { content: String, timestamp: DateTime },
    ClaudeResponded { content: String, tokens_used: u32, timestamp: DateTime },
    ToolExecuted { tool_name: String, input: String, output: String, timestamp: DateTime },
    StepCompleted { step: String, timestamp: DateTime },
    ConversationEnded { reason: String, timestamp: DateTime },
}

// Store all events
impl EventStore for PostgresAgentStore {
    async fn append(&self, agent_id: &str, events: &[AgentEvent]) -> Result<()> {
        // Store events in database
        // Full conversation history preserved
        // Can replay entire conversation
        // Can audit all tool uses
        // Can analyze token usage
    }

    async fn load(&self, agent_id: &str) -> Result<Vec<AgentEvent>> {
        // Load all events for agent
        // Replay to reconstruct state
    }
}

// Replay for debugging
fn replay_conversation(events: &[AgentEvent]) -> AgentState {
    events.iter().fold(AgentState::default(), |mut state, event| {
        state.apply_event(event);
        state
    })
}
```

**Benefits**:
- Full audit trail (compliance, debugging)
- Conversation replay (reproduce issues)
- Cost tracking (token usage per conversation)
- Performance analysis (response times, tool execution times)
- User behavior analysis (what questions lead to tool use)

### 7. Testing with Mocks

**Core Insight**: Environment traits make agent testing trivial.

```rust
#[tokio::test]
async fn test_agent_uses_correct_tool() {
    // Arrange: Mock environment
    let mut mock_env = MockAgentEnvironment::new();
    mock_env.expect_claude_response(MessagesResponse {
        content: vec![ContentBlock::ToolUse {
            id: "tool_123".to_string(),
            name: "get_weather".to_string(),
            input: r#"{"location": "San Francisco"}"#.to_string(),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage { input_tokens: 100, output_tokens: 50 },
    });
    mock_env.expect_tool_result("tool_123", Ok("72°F, sunny".to_string()));

    let reducer = AgentReducer::new();
    let mut state = AgentState::default();

    // Act: User asks about weather
    let effects = reducer.reduce(
        &mut state,
        AgentAction::UserMessage {
            content: "What's the weather in San Francisco?".to_string(),
            attachments: vec![],
        },
        &mock_env,
    );

    // Assert: Claude called with correct parameters
    assert_eq!(effects.len(), 1);
    assert!(matches!(effects[0], Effect::Future(_)));

    // Execute effect (calls mock)
    let action = execute_effect(effects[0], &mock_env).await.unwrap().unwrap();

    // Assert: Tool use requested
    assert!(matches!(action, AgentAction::ClaudeResponse {
        stop_reason: StopReason::ToolUse,
        tool_uses: tools,
        ..
    } if tools.len() == 1 && tools[0].name == "get_weather"));

    // Act: Process tool use
    let effects = reducer.reduce(&mut state, action, &mock_env);

    // Execute tool effect
    let action = execute_effect(effects[0], &mock_env).await.unwrap().unwrap();

    // Assert: Tool result returned
    assert!(matches!(action, AgentAction::ToolResult {
        tool_use_id: id,
        result: Ok(output),
        ..
    } if id == "tool_123" && output == "72°F, sunny"));
}

#[tokio::test]
async fn test_agent_handles_tool_failure() {
    let mut mock_env = MockAgentEnvironment::new();
    mock_env.expect_tool_result(
        "tool_123",
        Err(ToolError::ExecutionFailed {
            message: "API timeout".to_string()
        }),
    );

    // ... test error handling
}

#[tokio::test]
async fn test_parallel_tool_execution() {
    let mut mock_env = MockAgentEnvironment::new();
    mock_env.expect_claude_response(MessagesResponse {
        content: vec![
            ContentBlock::ToolUse {
                id: "tool_1".to_string(),
                name: "search_web".to_string(),
                input: r#"{"query": "Rust async"}"#.to_string(),
            },
            ContentBlock::ToolUse {
                id: "tool_2".to_string(),
                name: "search_docs".to_string(),
                input: r#"{"query": "Rust async"}"#.to_string(),
            },
        ],
        stop_reason: StopReason::ToolUse,
        usage: Usage { input_tokens: 120, output_tokens: 80 },
    });

    // ... test parallel execution
}
```

**Testing advantages**:
- No API costs (mock Claude responses)
- Deterministic (FixedClock, mock responses)
- Fast (no network calls)
- Comprehensive (test all edge cases)
- Isolated (test each reducer independently)

### 8. Memory and Context Management

**Approaches**:

1. **In-Memory (State)**: Conversation history in `AgentState.messages`
2. **Database (Effect)**: Load/save conversations with `Effect::Database`
3. **MCP Servers (Tool)**: External knowledge via Model Context Protocol tools

```rust
// Approach 1: In-memory (always available)
struct AgentState {
    messages: Vec<Message>, // Full history
    summary: Option<String>, // Compressed for long conversations
}

// Approach 2: Database (persistent)
enum AgentAction {
    LoadConversation { conversation_id: String },
    ConversationLoaded { messages: Vec<Message> },
}

impl Reducer {
    fn reduce(...) {
        match action {
            AgentAction::LoadConversation { conversation_id } => {
                vec![Effect::Database(LoadConversation { id: conversation_id })]
            }
            AgentAction::ConversationLoaded { messages } => {
                state.messages = messages;
                vec![Effect::None]
            }
        }
    }
}

// Approach 3: MCP Servers (external knowledge)
struct MCPTool {
    name: String,
    mcp_server: String,
    description: String,
}

// Tool executor for MCP
impl ToolExecutor for MCPToolExecutor {
    async fn execute(&self, input: &str) -> Result<String> {
        // Call MCP server via standard protocol
        let response = self.mcp_client
            .call_tool(&self.server_name, &self.tool_name, input)
            .await?;

        Ok(response)
    }
}
```

## Implementation Roadmap

### Phase 8.1: Core Agent Infrastructure

**Goal**: Basic agent reducer with Claude API integration

**New Crates**:
- `anthropic/` - Claude API client
  - Message types (Message, ContentBlock, ToolUse, etc.)
  - API client (MessagesAPI, streaming support)
  - Error handling
  - Rate limiting and retries

**Core Types** (`core/src/agent.rs`):
```rust
// Message types
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

pub enum Role {
    User,
    Assistant,
}

pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: String },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
}

// Tool types
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: JsonSchema,
}

pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub input: String,
}

// Stop reasons
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
}

// Agent state trait
pub trait AgentState: Clone + Send + Sync {
    fn messages(&self) -> &[Message];
    fn add_message(&mut self, message: Message);
}

// Agent environment trait
#[async_trait::async_trait]
pub trait AgentEnvironment: Send + Sync {
    fn tools(&self) -> &[Tool];
    async fn execute_tool(&self, name: &str, input: &str) -> Result<String, ToolError>;
    async fn call_claude(&self, request: MessagesRequest) -> Result<MessagesResponse, ClaudeError>;
}
```

**Example**: Basic Q&A agent
```rust
// examples/basic-agent/
// Simple conversational agent with no tools
// Demonstrates message history management
```

**Tests**:
- Message serialization/deserialization
- Mock Claude client
- Basic conversation flow
- Token usage tracking

**Documentation**:
- `docs/agents/getting-started.md`
- `docs/agents/claude-api.md`

### Phase 8.2: Tool Use System

**Goal**: Implement complete tool use protocol

**Features**:
- Tool definition and registration
- Tool execution (sequential and parallel)
- Tool result handling
- Error handling and retries

**Core Types** (`core/src/agent/tools.rs`):
```rust
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, input: &str) -> Result<String, ToolError>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn ToolExecutor>>,
}

impl ToolRegistry {
    pub fn register(&mut self, name: String, executor: Box<dyn ToolExecutor>);
    pub async fn execute(&self, name: &str, input: &str) -> Result<String, ToolError>;
}
```

**Built-in Tools**:
- `get_current_time` - Clock-based tool
- `calculate` - Math expressions
- `search_memory` - Query conversation history

**Example**: Weather agent with API tools
```rust
// examples/weather-agent/
// Agent with weather API tool
// Demonstrates tool use protocol
```

**Tests**:
- Tool registration
- Sequential tool execution
- Parallel tool execution
- Tool error handling
- Mock tool executors

**Documentation**:
- `docs/agents/tool-use.md`
- `docs/agents/custom-tools.md`

### Phase 8.3: Agent Patterns Library

**Goal**: Implement all seven Anthropic patterns

**New Crate**: `agents/` - Pre-built agent patterns

**Patterns**:
1. `BasicAgent` - Augmented LLM (base pattern)
2. `ChainAgent` - Prompt chaining with gates
3. `RouterAgent` - Classification and routing
4. `ParallelAgent` - Concurrent LLM calls
5. `OrchestratorAgent` - Dynamic task decomposition
6. `EvaluatorAgent` - Generate-evaluate-refine loop
7. `AutonomousAgent` - Dynamic planning with checkpoints

**Each Pattern Includes**:
- State struct
- Action enum
- Reducer implementation
- Builder for configuration
- Comprehensive tests
- Example application

**Example Applications**:
```rust
// examples/code-reviewer/
// Uses EvaluatorAgent pattern
// Reviews code and iteratively improves suggestions

// examples/customer-support/
// Uses RouterAgent pattern
// Classifies queries and routes to specialists

// examples/research-system/
// Uses OrchestratorAgent pattern
// Coordinates multiple research workers
```

**Documentation**:
- `docs/agents/patterns.md` - Overview of all patterns
- `docs/agents/pattern-selection.md` - When to use each pattern
- Individual pattern guides

### Phase 8.4: Multi-Agent Coordination

**Goal**: Enable agent-to-agent communication via EventBus

**Features**:
- Agent events (cross-agent communication)
- Agent registry (discover and route to agents)
- Agent orchestration (parent-child coordination)
- Conflict resolution (multiple agents acting on same data)

**Core Types** (`core/src/agent/multi_agent.rs`):
```rust
pub struct AgentEvent {
    pub source_agent: String,
    pub target_agent: Option<String>, // None = broadcast
    pub payload: serde_json::Value,
}

pub struct AgentRegistry {
    agents: HashMap<String, AgentInfo>,
}

pub struct AgentInfo {
    pub id: String,
    pub pattern: AgentPattern,
    pub capabilities: Vec<String>,
    pub event_subscriptions: Vec<String>,
}
```

**Example**: Multi-agent research system
```rust
// examples/multi-agent-research/
// Research agent (gathers information)
// Analysis agent (synthesizes findings)
// Writing agent (creates report)
// Review agent (provides feedback)
// Coordinator agent (orchestrates workflow)
```

**Tests**:
- Agent registration and discovery
- Event publication and subscription
- Agent-to-agent message passing
- Orchestration with multiple agents

**Documentation**:
- `docs/agents/multi-agent.md`
- `docs/agents/coordination-patterns.md`

### Phase 8.5: Memory and Context Management

**Goal**: Persistent conversations and knowledge retrieval

**Features**:
- Conversation persistence (PostgreSQL)
- Conversation summarization (for long contexts)
- Semantic search over history
- MCP server integration (future)

**Core Types** (`core/src/agent/memory.rs`):
```rust
#[async_trait::async_trait]
pub trait ConversationStore: Send + Sync {
    async fn save(&self, conversation_id: &str, messages: &[Message]) -> Result<()>;
    async fn load(&self, conversation_id: &str) -> Result<Vec<Message>>;
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Message>>;
}

pub struct ConversationSummarizer {
    // Summarizes long conversations to fit in context window
}
```

**Example**: Support ticket agent
```rust
// examples/support-agent/
// Loads historical conversation with customer
// Searches past interactions
// Maintains context across sessions
```

**Tests**:
- Conversation save/load
- Conversation search
- Summarization for long contexts

**Documentation**:
- `docs/agents/memory.md`
- `docs/agents/mcp-integration.md` (future)

### Phase 8.6: Production Hardening

**Goal**: Production-ready agent infrastructure

**Features**:
- Rate limiting (Claude API limits)
- Cost tracking and budgets
- Streaming responses
- Timeouts and cancellation
- Observability (tracing, metrics)

**Core Types** (`runtime/src/agent_runtime.rs`):
```rust
pub struct AgentRuntimeConfig {
    pub rate_limit: RateLimitConfig,
    pub timeout: Duration,
    pub max_retries: u32,
    pub cost_budget: Option<CostBudget>,
}

pub struct CostBudget {
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub current_usage: TokenUsage,
}

pub struct AgentMetrics {
    pub conversations_started: Counter,
    pub messages_processed: Counter,
    pub tools_executed: Counter,
    pub tokens_used: Histogram,
    pub response_time: Histogram,
}
```

**Observability**:
```rust
// Tracing spans for each agent operation
#[instrument(skip(state, env))]
fn reduce(...) {
    tracing::info!("Processing action: {:?}", action);
    // ...
}

// Metrics for agent performance
metrics::counter!("agent.messages.processed").increment(1);
metrics::histogram!("agent.response_time").record(duration);
```

**Example**: Production agent deployment
```rust
// examples/production-agent/
// Full production setup
// Rate limiting, cost tracking, monitoring
// Error handling and recovery
// Load testing harness
```

**Tests**:
- Rate limiting enforcement
- Cost budget enforcement
- Timeout handling
- Streaming response handling

**Documentation**:
- `docs/agents/production.md`
- `docs/agents/monitoring.md`
- `docs/agents/cost-optimization.md`

## Architectural Advantages

### 1. Type Safety
- Rust's type system prevents invalid state transitions
- Actions are strongly typed (no stringly-typed messages)
- Tool schemas validated at compile time (via traits)

### 2. Testability
- Mock Claude API (no API costs in tests)
- Mock tools (deterministic execution)
- FixedClock (deterministic timing)
- Event replay (reproduce bugs)

### 3. Observability
- Tracing spans for every operation
- Metrics for performance monitoring
- Event sourcing for audit trails
- Complete conversation history

### 4. Composability
- Agents are reducers (compose with combine_reducers)
- Patterns are building blocks (mix and match)
- Effects are composable (Parallel, Sequential)
- Tools are traits (implement once, use everywhere)

### 5. Performance
- Static dispatch (zero-cost abstractions)
- Streaming responses (low latency)
- Parallel tool execution (throughput)
- Efficient serialization (bincode for events)

### 6. Reliability
- Explicit error handling (Result types)
- Retries and circuit breakers (Effect::Retry)
- Timeouts (Effect::Delay)
- Dead letter queue for failed operations

### 7. Maintainability
- Clear separation of concerns (State, Action, Reducer, Effect)
- Explicit dependencies (Environment trait)
- Comprehensive documentation
- Extensive test coverage

## Comparison with Traditional Approaches

### Traditional (e.g., LangChain, LlamaIndex)

```python
# Implicit state management
agent = Agent(tools=[weather_tool, calculator_tool])
response = agent.run("What's the weather?")  # Where is state?

# Hidden side effects
result = tool.execute(input)  # When does this execute? Can I test it?

# Stringly-typed
agent.add_tool("weather", weather_func)  # Typo = runtime error

# No audit trail
# How did agent decide to use this tool? Can't replay.
```

### Composable-Rust Approach

```rust
// Explicit state management
let mut state = AgentState::default();
let effects = reducer.reduce(&mut state, action, &env);

// Effects as values
let effect = Effect::Future(Box::pin(async move {
    env.execute_tool("weather", input).await
}));
// Effect hasn't executed yet - can inspect, test, compose

// Strongly typed
let tool: Box<dyn ToolExecutor> = Box::new(WeatherTool);
env.register_tool("weather", tool);  // Compile-time checks

// Full audit trail
for event in event_store.load(agent_id) {
    println!("Agent decision: {:?}", event);
}
```

## Open Questions and Future Directions

### 1. MCP Server Integration
**Question**: How to integrate Model Context Protocol servers as tools?

**Approach**: MCP servers are just external tools. Implement `ToolExecutor` that calls MCP protocol.

```rust
struct MCPToolExecutor {
    server_url: String,
    tool_name: String,
}

#[async_trait::async_trait]
impl ToolExecutor for MCPToolExecutor {
    async fn execute(&self, input: &str) -> Result<String> {
        // Call MCP server via HTTP/SSE
        let response = mcp_client::call_tool(
            &self.server_url,
            &self.tool_name,
            input,
        ).await?;

        Ok(response)
    }
}
```

### 2. Streaming Responses
**Question**: How to handle streaming Claude responses in reducer?

**Approach**: Stream is an effect that yields multiple actions.

```rust
Effect::Stream(Box::pin(async_stream::stream! {
    let mut stream = env.claude_client().stream(request).await?;

    while let Some(chunk) = stream.next().await {
        yield AgentAction::ChunkReceived {
            content: chunk.content,
            is_final: chunk.is_final,
        };
    }
}))
```

### 3. Human-in-the-Loop
**Question**: How to implement human approval checkpoints?

**Approach**: State machine with AwaitingApproval step. Action comes from external input.

```rust
enum Step {
    Planning,
    AwaitingApproval,
    Executing,
}

// Reducer waits in AwaitingApproval until HumanApproved action arrives
match (&state.current_step, action) {
    (Step::AwaitingApproval, AgentAction::HumanApproved) => {
        state.current_step = Step::Executing;
        vec![self.execute_next_step(state, env)]
    }
    (Step::AwaitingApproval, AgentAction::HumanRejected { reason }) => {
        state.current_step = Step::Planning;
        vec![self.replan_with_feedback(state, reason, env)]
    }
}
```

### 4. Long-Running Agents
**Question**: How to handle agents that run for hours/days?

**Approach**: Event sourcing + checkpointing. State is reconstructed from events.

```rust
// Save events incrementally
impl EventStore {
    async fn append(&self, agent_id: &str, events: &[AgentEvent]) -> Result<()>;
}

// Resume from checkpoint
async fn resume_agent(agent_id: &str, event_store: &dyn EventStore) -> AgentState {
    let events = event_store.load(agent_id).await?;
    replay_events(&events)
}

// Periodic checkpointing
if state.events_since_checkpoint > 1000 {
    Effect::Database(SaveCheckpoint {
        agent_id: state.id,
        state: state.clone(),
    })
}
```

### 5. Multi-Tenant Isolation
**Question**: How to isolate agents for different users/organizations?

**Approach**: Agent ID includes tenant ID. Environment enforces isolation.

```rust
struct TenantAgentEnvironment {
    tenant_id: String,
    tools: HashMap<String, Box<dyn ToolExecutor>>,
    database: Box<dyn Database>,
}

impl AgentEnvironment for TenantAgentEnvironment {
    async fn execute_tool(&self, name: &str, input: &str) -> Result<String> {
        // Check tenant permissions
        if !self.has_permission(&self.tenant_id, name) {
            return Err(ToolError::PermissionDenied);
        }

        // Execute with tenant isolation
        self.tools.get(name).unwrap().execute(input).await
    }
}
```

### 6. Agent Versioning
**Question**: How to version agents as they evolve?

**Approach**: Version in reducer. Events include schema version.

```rust
struct AgentEvent {
    version: u32,
    payload: serde_json::Value,
}

impl Reducer {
    fn reduce(&self, state: &mut State, action: Action, env: &Env) -> Vec<Effect> {
        match (self.version, action) {
            (1, Action::V1Event { .. }) => { /* handle V1 */ },
            (2, Action::V2Event { .. }) => { /* handle V2 */ },
            _ => { /* migration logic */ },
        }
    }
}
```

## Success Metrics

How do we know Phase 8 is successful?

### Functional Metrics
- [ ] All 7 Anthropic patterns implemented
- [ ] Tool use protocol fully working (sequential + parallel)
- [ ] Multi-agent coordination via EventBus
- [ ] Event sourcing for conversation history
- [ ] Comprehensive test coverage (>90%)

### Quality Metrics
- [ ] Zero clippy warnings
- [ ] All tests passing
- [ ] Documentation complete (patterns, tools, examples)
- [ ] Example applications for each pattern

### Performance Metrics
- [ ] Mock tests run in <100ms (no network)
- [ ] Production agent handles 10+ req/sec
- [ ] Streaming responses <1s latency
- [ ] Parallel tool execution (no sequential blocking)

### Developer Experience Metrics
- [ ] Simple agent in <50 LOC
- [ ] Custom tool in <30 LOC
- [ ] Multi-agent system in <200 LOC
- [ ] Clear error messages for common mistakes

## Conclusion

**Composable-rust is architecturally ideal for building AI agents** because:

1. **Agents are state machines** → Our reducer model is perfect
2. **Tool use is effects** → Our effect system handles it naturally
3. **Multi-agent = EventBus** → We already built this in Phase 3
4. **Audit trails = Event sourcing** → We already built this in Phase 2
5. **Testing = mocks** → Our DI pattern makes it trivial

**The seven Anthropic patterns map cleanly to our architecture**:
- Basic patterns (Augmented LLM, Chaining, Routing) = simple reducers
- Intermediate patterns (Parallelization, Orchestrator) = effect composition + EventBus
- Advanced patterns (Evaluator, Autonomous) = complex state machines

**We can build production-ready agents** with:
- Type safety (Rust's type system prevents bugs)
- Testability (mock everything, no API costs)
- Observability (tracing + metrics + event sourcing)
- Reliability (explicit errors, retries, circuit breakers)
- Performance (static dispatch, streaming, parallel execution)

**Phase 8 will deliver**: A comprehensive, type-safe, testable, production-ready framework for building AI agents that follows Anthropic's best practices and leverages composable-rust's functional architecture.

---

**Next Steps**:
1. Review this analysis for architectural soundness
2. Create detailed implementation plan for Phase 8.1
3. Start with core agent infrastructure (reducer + Claude API)
4. Build out patterns incrementally (8.2 → 8.3 → 8.4 → 8.5 → 8.6)
