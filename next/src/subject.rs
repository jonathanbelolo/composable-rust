//! Identity primitives for handler invocations.
//!
//! This module provides the additive identity substrate:
//!
//! - [`Subject`]: who a handler invocation runs on behalf of
//! - [`SubjectId`]: opaque identifier for an authenticated subject
//! - [`InvocationContext`]: per-invocation context carrying the subject,
//!   clock, and correlation metadata
//!
//! # Design Rules
//!
//! - **Borrowed, not owned.** [`InvocationContext`] lives strictly for one
//!   handler invocation; no per-call `Arc` churn.
//! - **No attribute schema lock-in.** Fields like `role` or `tenant_id` are
//!   conventions that domains declare. The framework only guarantees a carrier.
//! - **`Anonymous` is a variant, not `Option<Subject>`.** One type everywhere;
//!   authorization logic always checks "can *this* subject do X?" rather than
//!   "is there a subject at all?"
//! - **`System` is explicit.** Saga-initiated, timer-initiated, and
//!   projection-rebuild work get [`Subject::System`], not
//!   [`Subject::Anonymous`].
//! - **Field list is closed for v1.** Anything new (feature flags, locale,
//!   trace span) needs explicit review.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use crate::Clock;

/// Opaque identifier for an authenticated subject.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SubjectId(String);

impl SubjectId {
    /// Create a subject ID from any string-like value.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The identity on whose behalf a handler invocation runs.
///
/// Resolved by the environment (via `HandlerEnvironment::current_subject`)
/// and threaded through fetch and process by the [`Handler`](crate::Handler).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Subject {
    /// A known, authenticated principal (human or service account).
    Authenticated {
        /// Stable identifier for the principal.
        id: SubjectId,
        /// Free-form attributes (e.g. `role`, `tenant_id`). Conventions are
        /// declared by domains; the framework only carries them.
        attributes: HashMap<String, Value>,
    },
    /// An unauthenticated caller.
    Anonymous,
    /// Framework-initiated work: sagas, timers, projection rebuilds.
    System,
}

impl Subject {
    /// The subject's ID, if authenticated.
    ///
    /// Returns `None` for [`Subject::Anonymous`] and [`Subject::System`].
    #[must_use]
    pub const fn id(&self) -> Option<&SubjectId> {
        if let Self::Authenticated { id, .. } = self {
            Some(id)
        } else {
            None
        }
    }

    /// Whether this is [`Subject::System`].
    #[must_use]
    pub const fn is_system(&self) -> bool {
        matches!(self, Self::System)
    }

    /// Look up an attribute by key.
    ///
    /// Returns `None` for missing keys and for non-authenticated subjects.
    #[must_use]
    pub fn attr(&self, key: &str) -> Option<&Value> {
        if let Self::Authenticated { attributes, .. } = self {
            attributes.get(key)
        } else {
            None
        }
    }
}

/// Spans a single handler invocation.
///
/// Built once per attempt by the [`Handler`](crate::Handler) and passed to
/// both `QueryFetcher::fetch_with_context` and
/// `BusinessLogic::process_with_context`.
pub struct InvocationContext<'a> {
    /// Clock for timestamps (the same clock `process` receives).
    pub clock: &'a dyn Clock,

    /// The subject this invocation runs on behalf of.
    pub subject: &'a Subject,

    /// Correlation ID for tracing across services.
    pub correlation_id: Option<&'a str>,

    /// Causation ID linking to the event that caused this action.
    pub causation_id: Option<&'a str>,

    /// For saga-emitted events: the originating human subject, threaded
    /// from the saga's start through every feedback step. `None` on
    /// non-saga paths.
    pub origin_subject_id: Option<&'a SubjectId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn authenticated() -> Subject {
        let mut attributes = HashMap::new();
        attributes.insert("role".to_string(), json!("admin"));
        Subject::Authenticated {
            id: SubjectId::new("user-123"),
            attributes,
        }
    }

    #[test]
    fn subject_id_new_and_as_str() {
        let id = SubjectId::new("user-123");
        assert_eq!(id.as_str(), "user-123");
        assert_eq!(id, SubjectId::new(String::from("user-123")));
    }

    #[test]
    fn id_returns_some_only_for_authenticated() {
        assert_eq!(authenticated().id(), Some(&SubjectId::new("user-123")));
        assert_eq!(Subject::Anonymous.id(), None);
        assert_eq!(Subject::System.id(), None);
    }

    #[test]
    fn is_system_only_for_system() {
        assert!(Subject::System.is_system());
        assert!(!Subject::Anonymous.is_system());
        assert!(!authenticated().is_system());
    }

    #[test]
    fn attr_returns_value_for_authenticated_and_none_otherwise() {
        let subject = authenticated();
        assert_eq!(subject.attr("role"), Some(&json!("admin")));
        assert_eq!(subject.attr("missing"), None);
        assert_eq!(Subject::Anonymous.attr("role"), None);
        assert_eq!(Subject::System.attr("role"), None);
    }
}
