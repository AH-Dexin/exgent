use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Cooperative cancellation signal shared between the runtime, agent loop, and tools.
///
/// Cancellation is best-effort: callers periodically check [`is_cancelled`] at safe
/// boundaries (between tool rounds, before/after a tool runs, while a long-running
/// tool polls its subprocess), and abort cleanly when set.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_propagates_through_clones() {
        let original = CancelToken::new();
        let clone = original.clone();

        assert!(!original.is_cancelled());
        clone.cancel();
        assert!(original.is_cancelled());

        original.reset();
        assert!(!clone.is_cancelled());
    }
}
