use std::{
    io::{self, Stdout, Write},
    time::Duration,
};

#[cfg(not(windows))]
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{
    cursor,
    event::{
        self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, Event,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal};

pub(super) type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

#[cfg(windows)]
pub(super) fn configure_windows_console_utf8() {
    use windows_sys::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};

    const CP_UTF8: u32 = 65001;
    unsafe {
        let _ = SetConsoleCP(CP_UTF8);
        let _ = SetConsoleOutputCP(CP_UTF8);
    }
}

#[cfg(not(windows))]
pub(super) fn configure_windows_console_utf8() {}

pub(super) fn enter_terminal(enable_mouse_capture: bool) -> io::Result<TuiTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    recover_terminal_modes(&mut stdout, enable_mouse_capture);
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

pub(super) fn set_mouse_capture(terminal: &mut TuiTerminal, enabled: bool) -> io::Result<()> {
    if enabled {
        execute!(terminal.backend_mut(), EnableMouseCapture)
    } else {
        execute!(terminal.backend_mut(), DisableMouseCapture)
    }
}

pub(super) fn recover_terminal_modes<W: Write>(writer: &mut W, enable_mouse_capture: bool) {
    push_keyboard_enhancement_flags(writer);
    let _ = execute!(writer, EnableBracketedPaste);
    if enable_mouse_capture {
        let _ = execute!(writer, EnableMouseCapture);
    }
    let _ = execute!(writer, EnableFocusChange);
    let _ = writer.flush();
}

fn push_keyboard_enhancement_flags<W: Write>(writer: &mut W) {
    #[cfg(windows)]
    {
        let _ = write!(writer, "\x1b[>0u");
    }
    #[cfg(not(windows))]
    {
        let _ = execute!(
            writer,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
}

fn pop_keyboard_enhancement_flags<W: Write>(writer: &mut W) {
    #[cfg(windows)]
    {
        let _ = write!(writer, "\x1b[<1u");
    }
    #[cfg(not(windows))]
    {
        let _ = execute!(writer, PopKeyboardEnhancementFlags);
    }
}

pub(super) struct TerminalRestoreGuard;

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        pop_keyboard_enhancement_flags(&mut stdout);
        let _ = execute!(
            stdout,
            cursor::Show,
            DisableFocusChange,
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
    }
}

pub(super) fn handle_resize(
    terminal: &mut TuiTerminal,
    mut width: u16,
    mut height: u16,
) -> io::Result<()> {
    while event::poll(Duration::from_millis(20))? {
        match event::read()? {
            Event::Resize(next_width, next_height) => {
                width = next_width;
                height = next_height;
            }
            _ => break,
        }
    }

    terminal.resize(Rect::new(0, 0, width, height))?;
    terminal.clear()?;
    Ok(())
}
