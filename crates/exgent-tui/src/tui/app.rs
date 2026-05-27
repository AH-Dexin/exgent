use std::{
    io::{self, IsTerminal},
    sync::{
        mpsc::{self, Receiver, TryRecvError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};
use exgent_core::{tr, AgentEvent, AgentSessionEvent, AppRuntimeHost, CancelToken, MessageId};

use super::auth_flow::run_subscription_auth;
use super::composer_input::handle_paste;
use super::frame_rate_limiter::FrameRateLimiter;
use super::input::handle_key;
use super::mouse_input::handle_mouse;
use super::prompt::handle_running_prompt_key;
use super::render::render;
use super::state::*;
use super::terminal::{
    enter_terminal, handle_resize, set_mouse_capture, TerminalRestoreGuard, TuiTerminal,
};

type TuiRuntime = AppRuntimeHost;

pub(super) fn run(runtime: &mut TuiRuntime) -> io::Result<()> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return super::run_plain(runtime);
    }

    let runtime = Arc::new(Mutex::new(runtime));
    let mut mouse_capture = runtime
        .lock()
        .map(|runtime| runtime.tui_settings().mouse_selection)
        .unwrap_or(true);
    let mut terminal = enter_terminal(mouse_capture)?;
    let _guard = TerminalRestoreGuard;
    let mut app = {
        let runtime = runtime
            .lock()
            .map_err(|_| io::Error::other("runtime lock poisoned"))?;
        TuiApp::new(&runtime)
    };
    app.push_welcome(tr(app.locale, MessageId::WelcomeNote));

    thread::scope(|scope| {
        let mut active_prompt: Option<ActivePrompt> = None;
        let mut frame_rate_limiter = FrameRateLimiter::default();
        let mut needs_draw = true;

        loop {
            if drain_prompt_updates(&mut app, &mut active_prompt, &runtime) {
                needs_draw = true;
            }

            if needs_draw {
                let now = Instant::now();
                let should_wait = app.tui_settings.render_throttle
                    && frame_rate_limiter.time_until_next_draw(now).is_some();
                if !should_wait {
                    draw_shared(&mut terminal, &mut app, &runtime)?;
                    frame_rate_limiter.mark_emitted(Instant::now());
                    sync_mouse_capture(
                        &mut terminal,
                        &mut mouse_capture,
                        app.tui_settings.mouse_selection,
                    )?;
                    needs_draw = false;
                }
            }

            let poll_timeout = if needs_draw {
                frame_rate_limiter
                    .time_until_next_draw(Instant::now())
                    .unwrap_or(Duration::from_millis(1))
                    .min(Duration::from_millis(20))
            } else if app.is_running {
                Duration::from_millis(20)
            } else {
                Duration::from_millis(250)
            };
            if !event::poll(poll_timeout)? {
                continue;
            }

            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if handle_running_cancel_key(&mut app, active_prompt.as_ref(), key) {
                        needs_draw = true;
                        continue;
                    }
                    if app.is_running {
                        handle_running_prompt_key(&mut app, key);
                        needs_draw = true;
                        continue;
                    }

                    let action = {
                        let mut runtime = runtime
                            .lock()
                            .map_err(|_| io::Error::other("runtime lock poisoned"))?;
                        handle_key(&mut app, &mut runtime, key)
                    };
                    match action {
                        UiAction::None => {}
                        UiAction::Quit => return Ok(()),
                        UiAction::RunPrompt { prompt, images } => {
                            app.is_running = true;
                            app.disarm_quit();
                            app.scroll_transcript_to_bottom();
                            if let Ok(runtime) = runtime.try_lock() {
                                if runtime.prompt_display_enabled() {
                                    app.push_system_prompt(runtime.system_prompt());
                                }
                            }
                            if images.is_empty() {
                                app.push_user(prompt.clone());
                            } else {
                                app.push_user_with_images(prompt.clone(), images.len());
                            }
                            app.start_assistant();

                            let cancel = CancelToken::new();
                            let worker_cancel = cancel.clone();
                            let worker_runtime = Arc::clone(&runtime);
                            let (tx, rx) = mpsc::channel();
                            scope.spawn(move || {
                                let mut streaming_error = false;
                                let result = match worker_runtime.lock() {
                                    Ok(mut runtime) => runtime
                                        .run_prompt_events_with_images_cancellable(
                                            &prompt,
                                            &images,
                                            &worker_cancel,
                                            &mut |event| {
                                                if matches!(
                                                    &event,
                                                    AgentSessionEvent::Agent(
                                                        AgentEvent::Error { .. }
                                                    )
                                                ) {
                                                    streaming_error = true;
                                                }
                                                let _ = tx.send(PromptWorkerEvent::Session(event));
                                            },
                                        ),
                                    Err(_) => Err("runtime lock poisoned".to_string()),
                                };
                                let _ = tx.send(PromptWorkerEvent::Finished {
                                    result,
                                    streaming_error,
                                });
                            });
                            active_prompt = Some(ActivePrompt { cancel, rx });
                        }
                        UiAction::RunSubscriptionAuth(provider) => {
                            let mut runtime = runtime
                                .lock()
                                .map_err(|_| io::Error::other("runtime lock poisoned"))?;
                            run_subscription_auth(&mut runtime, &mut app, &mut terminal, provider)?;
                        }
                    }
                    needs_draw = true;
                }
                Event::Resize(width, height) => {
                    handle_resize(&mut terminal, width, height)?;
                    needs_draw = true;
                }
                Event::Paste(value) => {
                    handle_paste(&mut app, &value);
                    needs_draw = true;
                }
                Event::Mouse(mouse) => {
                    let (width, height) = terminal::size()?;
                    let area = ratatui::layout::Rect::new(0, 0, width, height);
                    if handle_mouse(&mut app, area, mouse) {
                        needs_draw = true;
                    }
                }
                _ => {}
            }
        }
    })
}

fn sync_mouse_capture(
    terminal: &mut TuiTerminal,
    current: &mut bool,
    desired: bool,
) -> io::Result<()> {
    if *current == desired {
        return Ok(());
    }
    set_mouse_capture(terminal, desired)?;
    *current = desired;
    Ok(())
}

pub(super) fn draw(
    terminal: &mut TuiTerminal,
    app: &mut TuiApp,
    runtime: &TuiRuntime,
) -> io::Result<()> {
    app.refresh_status(runtime);
    terminal.draw(|frame| render(frame, app))?;
    Ok(())
}

fn draw_shared(
    terminal: &mut TuiTerminal,
    app: &mut TuiApp,
    runtime: &Arc<Mutex<&mut TuiRuntime>>,
) -> io::Result<()> {
    if let Ok(runtime) = runtime.try_lock() {
        app.refresh_status(&runtime);
    }
    terminal.draw(|frame| render(frame, app))?;
    Ok(())
}

enum PromptWorkerEvent {
    Session(AgentSessionEvent),
    Finished {
        result: Result<(), String>,
        streaming_error: bool,
    },
}

struct ActivePrompt {
    cancel: CancelToken,
    rx: Receiver<PromptWorkerEvent>,
}

fn drain_prompt_updates(
    app: &mut TuiApp,
    active_prompt: &mut Option<ActivePrompt>,
    runtime: &Arc<Mutex<&mut TuiRuntime>>,
) -> bool {
    let Some(prompt) = active_prompt.as_mut() else {
        return false;
    };

    let mut changed = false;
    loop {
        match prompt.rx.try_recv() {
            Ok(PromptWorkerEvent::Session(AgentSessionEvent::Agent(event))) => {
                app.apply_agent_event(event);
                changed = true;
            }
            Ok(PromptWorkerEvent::Session(_)) => {
                changed = true;
            }
            Ok(PromptWorkerEvent::Finished {
                result,
                streaming_error,
            }) => {
                if let Err(error) = result {
                    if !streaming_error {
                        app.push_error(error);
                    }
                }
                app.finish_reasoning();
                app.is_running = false;
                app.runtime_activity = None;
                active_prompt.take();
                if let Ok(runtime) = runtime.try_lock() {
                    app.refresh_status(&runtime);
                }
                changed = true;
                break;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                app.finish_reasoning();
                app.is_running = false;
                app.runtime_activity = None;
                active_prompt.take();
                changed = true;
                break;
            }
        }
    }
    changed
}

fn handle_running_cancel_key(
    app: &mut TuiApp,
    active_prompt: Option<&ActivePrompt>,
    key: KeyEvent,
) -> bool {
    let is_cancel = key.code == KeyCode::Esc
        || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c'));
    if !app.is_running || !is_cancel {
        return false;
    }
    if let Some(prompt) = active_prompt {
        if !prompt.cancel.is_cancelled() {
            prompt.cancel.cancel();
            app.push_note(tr(app.locale, MessageId::CancelRequested));
        }
    }
    app.finish_reasoning();
    app.runtime_activity = None;
    true
}
