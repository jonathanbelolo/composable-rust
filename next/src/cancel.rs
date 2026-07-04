//! Cooperative cancellation for durable saga runs
//!
//! [`CancellationToken`] is a lightweight, clonable cancellation signal built
//! on [`tokio::sync::watch`]. The durable saga loop (`Handler::handle_durable`)
//! checks it between completion cycles: on cancellation the loop stops topping
//! up, aborts in-flight calls (their futures are dropped), and returns
//! `DurableOutcome::Suspended` — the call journal still lists the aborted
//! calls as outstanding, so a later `resume` re-dispatches exactly them.
//! Cancel ≡ crash ≡ resumable.

use std::sync::Arc;
use tokio::sync::watch;

/// A clonable, idempotent cancellation signal.
///
/// All clones observe the same signal: cancelling any clone cancels them all.
///
/// # Example
///
/// ```rust
/// use composable_rust_next::CancellationToken;
///
/// let token = CancellationToken::new();
/// let clone = token.clone();
/// assert!(!clone.is_cancelled());
/// token.cancel();
/// assert!(clone.is_cancelled());
/// ```
#[derive(Clone)]
pub struct CancellationToken {
    /// Kept alive by every clone so `cancelled()` can never observe a
    /// closed channel (the sender outlives all receivers by construction).
    tx: Arc<watch::Sender<bool>>,
    rx: watch::Receiver<bool>,
}

impl CancellationToken {
    /// Create a new, not-yet-cancelled token.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(false);
        Self {
            tx: Arc::new(tx),
            rx,
        }
    }

    /// Signal cancellation. Idempotent: repeated calls are no-ops.
    pub fn cancel(&self) {
        // Send only fails when no receiver exists; every token holds one.
        let _ = self.tx.send(true);
    }

    /// Whether cancellation has been signalled (on any clone).
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow()
    }

    /// Resolve when cancellation is signalled.
    ///
    /// Resolves immediately if the token is already cancelled.
    pub async fn cancelled(&self) {
        let mut rx = self.rx.clone();
        // wait_for only errors when the sender is dropped, which cannot
        // happen while `self` (holding `tx`) is borrowed by this future.
        let _ = rx.wait_for(|cancelled| *cancelled).await;
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
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fresh_token_is_not_cancelled() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[tokio::test]
    async fn cancel_is_visible_across_clones() {
        let token = CancellationToken::new();
        let clone = token.clone();
        token.cancel();
        assert!(token.is_cancelled());
        assert!(clone.is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_resolves_immediately_when_already_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        // Must not hang.
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
