//! One cancel signal per run, shared by every step in it.
//!
//! Cancellation used to reach only the steps that are subprocesses: a `broadcast` was bridged
//! onto an `mpsc` receiver and handed to whatever spawned a process, which then got a kill
//! signal. Everything the app does natively - the terrain render, the elevation and extract
//! downloads, the Valhalla package - had no way to hear it, so pressing Cancel during a terrain
//! build did nothing visible for the next twenty minutes.
//!
//! This is the one token both kinds of step can read: async code awaits [`Cancel::cancelled`],
//! blocking loops poll [`Cancel::is_cancelled`] between units of work, and subprocess steps still
//! get their `mpsc` receiver from [`Cancel::receiver`].

use std::future::Future;
use std::sync::Arc;

use tokio::sync::{mpsc, watch};

/// A cancel flag that can be cloned freely; every clone observes the same signal.
#[derive(Clone)]
pub struct Cancel {
    /// `watch` rather than `Notify`: a waiter that subscribes *after* the signal was sent still
    /// sees it, which a notification would have missed. That race is exactly the one a cancel
    /// arriving between two steps would hit.
    tx: Arc<watch::Sender<bool>>,
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}

impl Cancel {
    pub fn new() -> Self {
        Self { tx: Arc::new(watch::channel(false).0) }
    }

    /// A token that is never cancelled, for callers that have nothing to cancel with.
    pub fn never() -> Self {
        Self::new()
    }

    pub fn cancel(&self) {
        let _ = self.tx.send(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.tx.borrow()
    }

    /// Resolves as soon as the token is cancelled, immediately if it already was.
    pub async fn cancelled(&self) {
        let mut rx = self.tx.subscribe();
        // `wait_for` tests the current value before waiting, so an already-cancelled token
        // returns here rather than hanging until a second `cancel()` that will never come.
        let _ = rx.wait_for(|cancelled| *cancelled).await;
    }

    /// Run `fut` unless the token fires first. `None` means cancelled.
    ///
    /// `biased` so a token that is already set wins over a future that is also ready: the point
    /// is to stop, not to squeeze in one more unit of work.
    pub async fn guard<F: Future>(&self, fut: F) -> Option<F::Output> {
        tokio::select! {
            biased;
            _ = self.cancelled() => None,
            out = fut => Some(out),
        }
    }

    /// A receiver that yields once when the token is cancelled.
    ///
    /// This is the shape the subprocess steps take (`planetiler::run_cancellable`,
    /// `external::run`), and it is why the token is not simply an `AtomicBool`: the spawned task
    /// ends when the signal arrives, and dropping every clone of the token is not mistaken for a
    /// cancellation - which is what once killed builds that had merely finished.
    pub fn receiver(&self) -> mpsc::Receiver<()> {
        let token = self.clone();
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(async move {
            token.cancelled().await;
            let _ = tx.send(()).await;
        });
        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_clone_sees_the_signal() {
        let cancel = Cancel::new();
        let other = cancel.clone();
        assert!(!other.is_cancelled());
        cancel.cancel();
        assert!(other.is_cancelled());
        other.cancelled().await;
    }

    /// The race this type exists for: the signal is sent before anyone waits on it. A
    /// notification would be lost here and the waiter would hang for the rest of the run.
    #[tokio::test]
    async fn waiting_after_the_signal_still_returns() {
        let cancel = Cancel::new();
        cancel.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), cancel.cancelled())
            .await
            .expect("an already-cancelled token must not block");
    }

    #[tokio::test]
    async fn guard_gives_up_on_cancel() {
        let cancel = Cancel::new();
        let signal = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            signal.cancel();
        });
        let out = cancel.guard(tokio::time::sleep(std::time::Duration::from_secs(30))).await;
        assert!(out.is_none(), "the sleep must not have been awaited to completion");
    }

    #[tokio::test]
    async fn the_receiver_fires_once() {
        let cancel = Cancel::new();
        let mut rx = cancel.receiver();
        cancel.cancel();
        assert!(rx.recv().await.is_some());
    }
}
