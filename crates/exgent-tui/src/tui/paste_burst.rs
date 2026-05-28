//! Fallback for terminals that do not deliver bracketed paste reliably.
//!
//! Some terminal hosts turn paste into rapid `Char`/`Enter` key events.
//! This buffers that burst until the input goes idle, so pasted newlines
//! stay in the composer instead of submitting the first line.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const PASTE_BURST_CHAR_INTERVAL: Duration = Duration::from_millis(20);
const PASTE_ENTER_SUPPRESS_WINDOW: Duration = Duration::from_millis(120);
#[cfg(not(windows))]
const PASTE_BURST_ACTIVE_IDLE_TIMEOUT: Duration = Duration::from_millis(12);
#[cfg(windows)]
const PASTE_BURST_ACTIVE_IDLE_TIMEOUT: Duration = Duration::from_millis(60);

#[derive(Clone, Debug, Default)]
pub(super) struct PasteBurst {
    bracketed_paste_seen: bool,
    last_plain_char_time: Option<Instant>,
    pending_first_char: Option<(char, Instant)>,
    buffer: String,
    active: bool,
    newline_until: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PasteBurstAction {
    Handled,
    InsertNewline,
    None,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum FlushResult {
    Paste(String),
    Typed(char),
    None,
}

impl PasteBurst {
    #[cfg(test)]
    pub(super) fn recommended_flush_delay() -> Duration {
        PASTE_BURST_CHAR_INTERVAL + Duration::from_millis(1)
    }

    #[cfg(test)]
    pub(super) fn recommended_active_flush_delay() -> Duration {
        PASTE_BURST_ACTIVE_IDLE_TIMEOUT + Duration::from_millis(1)
    }

    pub(super) fn mark_bracketed_paste_seen(&mut self) {
        self.bracketed_paste_seen = true;
        self.clear_after_explicit_paste();
    }

    pub(super) fn observe_key(&mut self, key: &KeyEvent, now: Instant) -> PasteBurstAction {
        if self.bracketed_paste_seen {
            return PasteBurstAction::None;
        }

        match key.code {
            KeyCode::Char(ch) if is_plain_text_key(key) && !ch.is_control() => {
                self.observe_plain_char(ch, now);
                PasteBurstAction::Handled
            }
            KeyCode::Enter if self.append_newline_if_active(now) => PasteBurstAction::Handled,
            KeyCode::Enter if self.begin_with_pending_newline(now) => PasteBurstAction::Handled,
            KeyCode::Enter if self.newline_should_insert_instead_of_submit(now) => {
                self.extend_newline_window(now);
                PasteBurstAction::InsertNewline
            }
            _ => PasteBurstAction::None,
        }
    }

    pub(super) fn flush_if_due(&mut self, now: Instant) -> FlushResult {
        let Some(last) = self.last_plain_char_time else {
            return FlushResult::None;
        };
        let timeout = if self.is_active_internal() {
            PASTE_BURST_ACTIVE_IDLE_TIMEOUT
        } else {
            PASTE_BURST_CHAR_INTERVAL
        };
        if now.duration_since(last) <= timeout {
            return FlushResult::None;
        }
        self.flush_pending()
    }

    pub(super) fn flush_before_modified_input(&mut self) -> FlushResult {
        self.flush_pending()
    }

    pub(super) fn next_flush_delay(&self, now: Instant) -> Option<Duration> {
        let last = self.last_plain_char_time?;
        let timeout = if self.is_active_internal() {
            PASTE_BURST_ACTIVE_IDLE_TIMEOUT
        } else {
            PASTE_BURST_CHAR_INTERVAL
        };
        Some(timeout.saturating_sub(now.duration_since(last)))
    }

    pub(super) fn clear_after_explicit_paste(&mut self) {
        self.last_plain_char_time = None;
        self.pending_first_char = None;
        self.buffer.clear();
        self.active = false;
        self.newline_until = None;
    }

    fn observe_plain_char(&mut self, ch: char, now: Instant) {
        self.last_plain_char_time = Some(now);

        if self.is_active_internal() {
            self.append_char_to_buffer(ch, now);
            return;
        }

        if let Some((held, held_at)) = self.pending_first_char.take() {
            if now.duration_since(held_at) <= PASTE_BURST_CHAR_INTERVAL {
                self.active = true;
                self.buffer.push(held);
                self.append_char_to_buffer(ch, now);
                return;
            }
            self.pending_first_char = Some((held, held_at));
        }

        self.pending_first_char = Some((ch, now));
    }

    fn append_char_to_buffer(&mut self, ch: char, now: Instant) {
        self.buffer.push(ch);
        self.active = true;
        self.extend_newline_window(now);
    }

    fn append_newline_if_active(&mut self, now: Instant) -> bool {
        if !self.is_active_internal() {
            return false;
        }
        self.buffer.push('\n');
        self.last_plain_char_time = Some(now);
        self.extend_newline_window(now);
        true
    }

    fn begin_with_pending_newline(&mut self, now: Instant) -> bool {
        let Some((held, held_at)) = self.pending_first_char.take() else {
            return false;
        };
        if now.duration_since(held_at) > PASTE_BURST_CHAR_INTERVAL {
            self.pending_first_char = Some((held, held_at));
            return false;
        }
        self.buffer.push(held);
        self.buffer.push('\n');
        self.active = true;
        self.last_plain_char_time = Some(now);
        self.extend_newline_window(now);
        true
    }

    fn newline_should_insert_instead_of_submit(&self, now: Instant) -> bool {
        self.newline_until.is_some_and(|until| now <= until)
    }

    fn extend_newline_window(&mut self, now: Instant) {
        self.newline_until = Some(now + PASTE_ENTER_SUPPRESS_WINDOW);
    }

    fn flush_pending(&mut self) -> FlushResult {
        if self.is_active_internal() {
            self.active = false;
            self.pending_first_char = None;
            self.last_plain_char_time = None;
            let out = std::mem::take(&mut self.buffer);
            return FlushResult::Paste(out);
        }
        if let Some((ch, _)) = self.pending_first_char.take() {
            self.last_plain_char_time = None;
            return FlushResult::Typed(ch);
        }
        FlushResult::None
    }

    fn is_active_internal(&self) -> bool {
        self.active || !self.buffer.is_empty()
    }
}

fn is_plain_text_key(key: &KeyEvent) -> bool {
    !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
        && !key.modifiers.contains(KeyModifiers::SUPER)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
    }

    #[test]
    fn slow_char_flushes_as_typed_input() {
        let mut burst = PasteBurst::default();
        let t0 = Instant::now();

        assert_eq!(
            burst.observe_key(&plain('a'), t0),
            PasteBurstAction::Handled
        );
        assert_eq!(
            burst.flush_if_due(t0 + PasteBurst::recommended_flush_delay()),
            FlushResult::Typed('a')
        );
    }

    #[test]
    fn rapid_chars_buffer_until_idle() {
        let mut burst = PasteBurst::default();
        let t0 = Instant::now();

        assert_eq!(
            burst.observe_key(&plain('a'), t0),
            PasteBurstAction::Handled
        );
        assert_eq!(
            burst.observe_key(&plain('b'), t0 + Duration::from_millis(1)),
            PasteBurstAction::Handled
        );
        assert_eq!(
            burst.flush_if_due(
                t0 + Duration::from_millis(1) + PasteBurst::recommended_active_flush_delay()
            ),
            FlushResult::Paste("ab".to_string())
        );
    }

    #[test]
    fn enter_during_active_burst_stays_in_buffer() {
        let mut burst = PasteBurst::default();
        let t0 = Instant::now();

        let _ = burst.observe_key(&plain('a'), t0);
        let _ = burst.observe_key(&plain('b'), t0 + Duration::from_millis(1));
        assert_eq!(
            burst.observe_key(
                &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                t0 + Duration::from_millis(2)
            ),
            PasteBurstAction::Handled
        );
        assert_eq!(
            burst.flush_if_due(
                t0 + Duration::from_millis(2) + PasteBurst::recommended_active_flush_delay()
            ),
            FlushResult::Paste("ab\n".to_string())
        );
    }

    #[test]
    fn bracketed_paste_disables_raw_paste_heuristic() {
        let mut burst = PasteBurst::default();
        let t0 = Instant::now();

        burst.mark_bracketed_paste_seen();

        assert_eq!(burst.observe_key(&plain('a'), t0), PasteBurstAction::None);
    }
}
