//! Keyboard shortcut predicates shared by TUI input handlers.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Paste-from-clipboard: `Cmd+V` on macOS, `Ctrl+V` on Linux/Windows, or
/// the legacy raw `\u{16}` byte some terminals emit for Ctrl+V.
///
/// On Windows, `Alt+V` is also accepted as a fallback because some terminal
/// hosts consume `Ctrl+V` for their own text paste before the TUI can see it.
pub(super) fn is_paste_shortcut(key: &KeyEvent) -> bool {
    let is_v = matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V'));
    let is_legacy_ctrl_v = matches!(key.code, KeyCode::Char('\u{16}'));
    if !is_v && !is_legacy_ctrl_v {
        return false;
    }

    if is_legacy_ctrl_v {
        return true;
    }

    if key.modifiers.contains(KeyModifiers::SUPER) {
        return true;
    }

    if cfg!(target_os = "windows")
        && key.modifiers.contains(KeyModifiers::ALT)
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::SUPER)
    {
        return true;
    }

    key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_shortcut_accepts_ctrl_v_and_cmd_v() {
        let ctrl_v = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL);
        let ctrl_shift_v = KeyEvent::new(
            KeyCode::Char('V'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        let cmd_v = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::SUPER);

        assert!(is_paste_shortcut(&ctrl_v));
        assert!(is_paste_shortcut(&ctrl_shift_v));
        assert!(is_paste_shortcut(&cmd_v));
    }

    #[test]
    fn paste_shortcut_accepts_legacy_ctrl_v_byte() {
        let legacy = KeyEvent::new(KeyCode::Char('\u{16}'), KeyModifiers::NONE);

        assert!(is_paste_shortcut(&legacy));
    }

    #[test]
    fn paste_shortcut_handles_alt_v_by_platform() {
        let alt_v = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT);
        let plain_v = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE);

        assert_eq!(is_paste_shortcut(&alt_v), cfg!(target_os = "windows"));
        assert!(!is_paste_shortcut(&plain_v));
    }
}
