//! Context Management (Phase 8.3)
//!
//! This module provides context window management for agents with long conversations.
//! Includes sliding window, summarization, and token estimation.
//!
//! ## Features
//!
//! - **Sliding Window**: Keep recent N messages
//! - **Summarization**: Compress old context (placeholder for LLM integration)
//! - **Token Estimation**: Approximate token counts for context sizing
//! - **Bounded Growth**: Enforces maximum context size

use composable_rust_anthropic::Message;

/// Context window manager with sliding window and summarization
#[derive(Clone, Debug)]
pub struct ContextWindow {
    /// Maximum number of messages to keep
    max_messages: usize,
    /// Messages in context (bounded)
    messages: Vec<Message>,
    /// Compressed/summarized context (optional)
    summary: Option<String>,
}

impl ContextWindow {
    /// Create new context window with maximum size
    #[must_use]
    pub const fn new(max_messages: usize) -> Self {
        Self {
            max_messages,
            messages: Vec::new(),
            summary: None,
        }
    }

    /// Add message to context (may trigger compression)
    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);

        // Enforce sliding window
        if self.messages.len() > self.max_messages {
            self.messages.remove(0);
        }
    }

    /// Get all messages in current context
    #[must_use]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Get summary (if any)
    #[must_use]
    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    /// Estimate total tokens in context
    #[must_use]
    pub fn estimate_tokens(&self) -> usize {
        use composable_rust_anthropic::ContentBlock;

        // Simple heuristic: ~4 characters per token
        let message_chars: usize = self.messages.iter()
            .flat_map(|m| &m.content)
            .map(|c| match c {
                ContentBlock::Text { text } => text.len(),
                ContentBlock::ToolUse { input, .. } => input.to_string().len(),
                ContentBlock::ToolResult { content, .. } => content.len(),
            })
            .sum();

        let summary_chars = self.summary.as_ref().map_or(0, String::len);

        (message_chars + summary_chars) / 4
    }
}

/// Context manager for agents
#[derive(Clone, Debug)]
pub struct ContextManager {
    /// Context window
    window: ContextWindow,
}

impl ContextManager {
    /// Create new context manager
    #[must_use]
    pub const fn new(max_messages: usize) -> Self {
        Self {
            window: ContextWindow::new(max_messages),
        }
    }

    /// Add message to context
    pub fn add_message(&mut self, message: Message) {
        self.window.add_message(message);
    }

    /// Get context for LLM request
    #[must_use]
    pub fn get_context(&self) -> &[Message] {
        self.window.messages()
    }

    /// Estimate current context size in tokens
    #[must_use]
    pub fn estimate_tokens(&self) -> usize {
        self.window.estimate_tokens()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // Test code can use unwrap/expect
mod tests {
    use super::*;

    #[test]
    fn test_context_window_sliding() {
        let mut window = ContextWindow::new(3);

        window.add_message(Message::user("msg1"));
        window.add_message(Message::user("msg2"));
        window.add_message(Message::user("msg3"));
        assert_eq!(window.messages().len(), 3);

        // Should evict oldest
        window.add_message(Message::user("msg4"));
        assert_eq!(window.messages().len(), 3);
    }

    #[test]
    fn test_context_manager() {
        let mut manager = ContextManager::new(5);

        manager.add_message(Message::user("Hello"));
        assert_eq!(manager.get_context().len(), 1);

        let tokens = manager.estimate_tokens();
        assert!(tokens > 0);
    }
}
