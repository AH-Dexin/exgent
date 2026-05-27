use std::time::{Duration, Instant};

/// 120 FPS draw cap, matching the cadence CodeWhale uses for normal motion.
pub(super) const MIN_FRAME_INTERVAL: Duration = Duration::from_nanos(8_333_334);

#[derive(Debug, Default)]
pub(super) struct FrameRateLimiter {
    last_emitted_at: Option<Instant>,
}

impl FrameRateLimiter {
    pub(super) fn time_until_next_draw(&self, now: Instant) -> Option<Duration> {
        let last_emitted_at = self.last_emitted_at?;
        let min_allowed = last_emitted_at
            .checked_add(MIN_FRAME_INTERVAL)
            .unwrap_or(last_emitted_at);
        if min_allowed <= now {
            None
        } else {
            Some(min_allowed - now)
        }
    }

    pub(super) fn mark_emitted(&mut self, emitted_at: Instant) {
        self.last_emitted_at = Some(emitted_at);
    }
}
