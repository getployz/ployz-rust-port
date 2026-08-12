use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{error::Error, fmt};

use tokio::sync::Notify;

/// A small cloneable cancellation signal for SSH operations.
#[derive(Clone, Debug, Default)]
pub struct Cancellation {
    state: Arc<State>,
}

#[derive(Debug, Default)]
struct State {
    cancelled: AtomicBool,
    notify: Notify,
}

/// The typed cause attached to operations stopped by [`Cancellation`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CancelledError;

impl fmt::Display for CancelledError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("operation canceled")
    }
}

impl Error for CancelledError {}

impl Cancellation {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        if !self.state.cancelled.swap(true, Ordering::AcqRel) {
            self.state.notify.notify_waiters();
        }
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    pub async fn cancelled(&self) {
        loop {
            let notified = self.state.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancellation_is_sticky_for_existing_and_late_waiters() {
        let cancellation = Cancellation::new();
        let waiter = cancellation.clone();
        let task = tokio::spawn(async move { waiter.cancelled().await });

        cancellation.cancel();
        task.await.unwrap();
        cancellation.cancelled().await;
        assert!(cancellation.is_cancelled());
    }
}
