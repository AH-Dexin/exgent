use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use exgent_core::{
    tr, AgentEvent, AgentSessionEvent, AppRuntimeHost, CancelToken, ImageContent, MessageId,
};

use super::app::draw;
use super::render::render;
use super::state::TuiApp;
use super::terminal::{handle_resize, TuiTerminal};

type TuiRuntime = AppRuntimeHost;
const TRANSCRIPT_SCROLL_LINES: usize = 3;

pub(super) fn run_prompt(
    runtime: &mut TuiRuntime,
    app: &mut TuiApp,
    terminal: &mut TuiTerminal,
    prompt: String,
    images: Vec<ImageContent>,
) -> io::Result<()> {
    app.is_running = true;
    app.scroll_transcript_to_bottom();
    if runtime.prompt_display_enabled() {
        app.push_system_prompt(runtime.system_prompt());
    }
    if images.is_empty() {
        app.push_user(prompt.clone());
    } else {
        app.push_user_with_images(prompt.clone(), images.len());
    }
    app.start_assistant();
    draw(terminal, app, runtime)?;

    // Track whether AgentEvent::Error was already displayed via the event stream.
    // apply_agent_event calls push_error for error events, so we must not call it
    // a second time from the Err return value of run_prompt_events.
    let cancel = CancelToken::new();
    let mut input_watcher = PromptInputWatcher::new(cancel.clone());
    let mut streaming_error = false;
    let mut cancel_requested = false;
    let result = runtime.run_prompt_events_with_images_cancellable(
        &prompt,
        &images,
        &cancel,
        &mut |event| {
            if let AgentSessionEvent::Agent(event) = event {
                if matches!(&event, AgentEvent::Error { .. }) {
                    streaming_error = true;
                }
                app.apply_agent_event(event);
            }
            if drain_prompt_input_events(app, terminal, &mut input_watcher).unwrap_or(false)
                && !cancel_requested
            {
                cancel_requested = true;
                app.push_note(tr(app.locale, MessageId::CancelRequested));
            }
            let _ = terminal.draw(|frame| render(frame, app));
        },
    );
    if let Err(error) = result {
        if !streaming_error {
            app.push_error(error);
        }
    }

    app.is_running = false;
    app.runtime_activity = None;
    app.refresh_status(runtime);
    Ok(())
}

enum PromptInputEvent {
    Cancel,
    Resize(u16, u16),
    ScrollUp(usize),
    ScrollDown(usize),
    ScrollTop,
    ScrollBottom,
}

struct PromptInputWatcher {
    active: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    rx: Receiver<PromptInputEvent>,
}

impl PromptInputWatcher {
    fn new(cancel: CancelToken) -> Self {
        let active = Arc::new(AtomicBool::new(true));
        let thread_active = Arc::clone(&active);
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            while thread_active.load(Ordering::Relaxed) {
                match event::poll(Duration::from_millis(50)) {
                    Ok(true) => match event::read() {
                        Ok(Event::Resize(width, height)) => {
                            let _ = tx.send(PromptInputEvent::Resize(width, height));
                        }
                        Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                            if is_prompt_cancel_key(&key) {
                                cancel.cancel();
                                let _ = tx.send(PromptInputEvent::Cancel);
                            } else {
                                match key.code {
                                    KeyCode::Up if key.modifiers.is_empty() => {
                                        let _ = tx.send(PromptInputEvent::ScrollUp(
                                            TRANSCRIPT_SCROLL_LINES,
                                        ));
                                    }
                                    KeyCode::Down if key.modifiers.is_empty() => {
                                        let _ = tx.send(PromptInputEvent::ScrollDown(
                                            TRANSCRIPT_SCROLL_LINES,
                                        ));
                                    }
                                    KeyCode::PageUp => {
                                        let _ = tx.send(PromptInputEvent::ScrollUp(10));
                                    }
                                    KeyCode::PageDown => {
                                        let _ = tx.send(PromptInputEvent::ScrollDown(10));
                                    }
                                    KeyCode::Home
                                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        let _ = tx.send(PromptInputEvent::ScrollTop);
                                    }
                                    KeyCode::End
                                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        let _ = tx.send(PromptInputEvent::ScrollBottom);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    },
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        });

        Self {
            active,
            handle: Some(handle),
            rx,
        }
    }
}

impl Drop for PromptInputWatcher {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn drain_prompt_input_events(
    app: &mut TuiApp,
    terminal: &mut TuiTerminal,
    watcher: &mut PromptInputWatcher,
) -> io::Result<bool> {
    let mut requested = false;
    loop {
        match watcher.rx.try_recv() {
            Ok(PromptInputEvent::Cancel) => requested = true,
            Ok(PromptInputEvent::Resize(width, height)) => handle_resize(terminal, width, height)?,
            Ok(PromptInputEvent::ScrollUp(lines)) => app.scroll_transcript_up(lines),
            Ok(PromptInputEvent::ScrollDown(lines)) => app.scroll_transcript_down(lines),
            Ok(PromptInputEvent::ScrollTop) => app.scroll_transcript_to_top(),
            Ok(PromptInputEvent::ScrollBottom) => app.scroll_transcript_to_bottom(),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => break,
        }
    }
    Ok(requested)
}

fn is_prompt_cancel_key(key: &crossterm::event::KeyEvent) -> bool {
    key.code == KeyCode::Esc
        || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c'))
}
