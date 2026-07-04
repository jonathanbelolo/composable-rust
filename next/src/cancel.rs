//! Cooperative cancellation for durable saga runs
//!
//! [`CancellationToken`] is a lightweight, clonable cancellation signal built
//! on [`tokio::sync::watch`], with two strengths:
//!
//! - **Abort** ([`cancel`](CancellationToken::cancel)): the durable loop
//!   drops in-flight call futures immediately and returns
//!   `DurableOutcome::Suspended`. The call journal still lists the aborted
//!   calls as outstanding, so a later `resume` re-dispatches exactly them.
//!   Cancel ≡ crash ≡ resumable.
//! - **Drain** ([`drain`](CancellationToken::drain)): the loop stops
//!   *starting* calls but lets in-flight ones complete and journal their
//!   feedback cycles; once nothing is in flight it suspends. Calls the saga
//!   topped up while draining are journaled but not dispatched — `resume`
//!   picks them up. Kinder to expensive calls (nothing half-done is thrown
//!   away) at the cost of waiting for the slowest in-flight call.
//!
//! Signals only escalate: `Active < Drain < Abort`. Draining a cancelled
//! token is a no-op; cancelling a draining token upgrades it.

use std::sync::Arc;
use tokio::sync::watch;

/// Cancellation strength; escalation-only ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CancelState {
    Active,
    Drain,
    Abort,
}

/// A clonable, escalation-only cancellation signal.
///
/// All clones observe the same signal: cancelling or draining any clone
/// affects them all.
///
/// # Example
///
/// ```rust
/// use composable_rust_next::CancellationToken;
///
/// let token = CancellationToken::new();
/// token.drain();
/// assert!(token.is_draining());
/// assert!(!token.is_cancelled()); // drain is not abort
/// token.cancel();
/// assert!(token.is_cancelled()); // upgraded
/// ```
#[derive(Clone)]
pub struct CancellationToken {
    /// Kept alive by every clone so `cancelled()` can never observe a
    /// closed channel (the sender outlives all receivers by construction).
    tx: Arc<watch::Sender<CancelState>>,
    rx: watch::Receiver<CancelState>,
}

impl CancellationToken {
    /// Create a new, active (not cancelled, not draining) token.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(CancelState::Active);
        Self {
            tx: Arc::new(tx),
            rx,
        }
    }

    /// Signal **abort**: in-flight calls are dropped, the run suspends
    /// immediately. Idempotent; upgrades a draining token.
    pub fn cancel(&self) {
        self.tx.send_modify(|state| {
            if *state < CancelState::Abort {
                *state = CancelState::Abort;
            }
        });
    }

    /// Signal **drain**: stop starting calls, let in-flight ones complete
    /// and journal, then suspend. Idempotent; a no-op on an already
    /// cancelled token (abort is stronger).
    ///
    /// A configured `max_call_duration` watchdog still applies while
    /// draining: a hung in-flight call trips `CallStuck` instead of
    /// stalling the drain forever. Escalate with
    /// [`cancel`](Self::cancel) to stop waiting immediately.
    pub fn drain(&self) {
        self.tx.send_modify(|state| {
            if *state < CancelState::Drain {
                *state = CancelState::Drain;
            }
        });
    }

    /// Whether **abort** has been signalled (on any clone).
    ///
    /// `false` while merely draining — abort semantics are unchanged from
    /// before drain existed.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow() == CancelState::Abort
    }

    /// Whether drain **or** abort has been signalled (abort implies drain).
    #[must_use]
    pub fn is_draining(&self) -> bool {
        *self.rx.borrow() >= CancelState::Drain
    }

    /// Resolve when **abort** is signalled (drain does not resolve this).
    ///
    /// Resolves immediately if the token is already cancelled.
    pub async fn cancelled(&self) {
        let mut rx = self.rx.clone();
        // wait_for only errors when the sender is dropped, which cannot
        // happen while `self` (holding `tx`) is borrowed by this future.
        let _ = rx.wait_for(|state| *state == CancelState::Abort).await;
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationToken")
            .field("state", &*self.rx.borrow())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;

    #[tokio::test]
    async fn fresh_token_is_active() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        assert!(!token.is_draining());
    }

    #[tokio::test]
    async fn cancel_is_visible_across_clones() {
        let token = CancellationToken::new();
        let clone = token.clone();
        token.cancel();
        assert!(token.is_cancelled());
        assert!(clone.is_cancelled());
        assert!(clone.is_draining(), "abort implies draining");
    }

    #[tokio::test]
    async fn drain_is_not_abort() {
        let token = CancellationToken::new();
        token.drain();
        assert!(token.is_draining());
        assert!(!token.is_cancelled());
        // cancelled() must NOT resolve on drain.
        assert!(
            token.cancelled().now_or_never().is_none(),
            "drain must not resolve cancelled()"
        );
    }

    #[tokio::test]
    async fn drain_then_cancel_upgrades() {
        let token = CancellationToken::new();
        token.drain();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn cancel_then_drain_stays_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        token.drain();
        assert!(token.is_cancelled(), "drain must not downgrade abort");
    }

    #[tokio::test]
    async fn cancelled_resolves_immediately_when_already_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        token.cancelled().await;
    }

    #[tokio::test]
    async fn cancelled_resolves_when_another_clone_cancels() {
        let token = CancellationToken::new();
        let clone = token.clone();

        let waiter = tokio::spawn(async move {
            clone.cancelled().await;
            true
        });

        token.cancel();
        #[allow(clippy::expect_used)]
        let resolved = waiter.await.expect("waiter task must not panic");
        assert!(resolved);
    }

    #[tokio::test]
    async fn cancel_is_idempotent() {
        let token = CancellationToken::new();
        token.cancel();
        token.cancel();
        assert!(token.is_cancelled());
        token.cancelled().await;
    }
}
