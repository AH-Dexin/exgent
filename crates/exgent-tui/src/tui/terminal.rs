use std::{
    io::{self, Stdout},
    time::Duration,
};

use crossterm::{
    cursor,
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal};

pub(super) type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

pub(super) fn enter_terminal() -> io::Result<TuiTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

pub(super) struct TerminalRestoreGuard;

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, cursor::Show, LeaveAlternateScreen);
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

pub(super) fn drain_resize_events(terminal: &mut TuiTerminal) {
    while event::poll(Duration::from_millis(0)).unwrap_or(false) {
        match event::read() {
            Ok(Event::Resize(width, height)) => {
                let _ = handle_resize(terminal, width, height);
            }
            Ok(_) => break,
            Err(_) => break,
        }
    }
}
