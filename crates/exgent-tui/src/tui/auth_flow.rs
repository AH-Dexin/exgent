use std::{
    io,
    time::{Duration, Instant},
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use exgent_core::{
    finish_anthropic_oauth_flow, finish_github_copilot_device_flow_cancellable,
    finish_openai_codex_oauth_flow, normalize_github_domain, parse_authorization_input,
    start_anthropic_oauth_flow, start_github_copilot_device_flow, start_openai_codex_oauth_flow,
    AppRuntimeHost, AuthorizationCode, SubscriptionProviderInfo,
};

use super::app::draw;
use super::formatting::input_view;
use super::state::{AuthProgressState, Overlay, TuiApp};
use super::terminal::{handle_resize, TuiTerminal};

type TuiRuntime = AppRuntimeHost;

struct BrowserAuthorizationPrompt<'a> {
    title: &'a str,
    url: &'a str,
    cancelled_message: &'a str,
    timed_out_message: &'a str,
}

pub(super) fn run_subscription_auth(
    runtime: &mut TuiRuntime,
    app: &mut TuiApp,
    terminal: &mut TuiTerminal,
    provider: SubscriptionProviderInfo,
) -> io::Result<()> {
    app.overlay = Overlay::None;
    match provider.provider.as_str() {
        "anthropic" => run_anthropic_subscription(runtime, app, terminal, &provider.provider),
        "github-copilot" => {
            run_github_copilot_subscription(runtime, app, terminal, &provider.provider)
        }
        "openai-codex" => run_openai_codex_subscription(runtime, app, terminal, &provider.provider),
        _ => {
            app.push_error(format!(
                "unknown subscription provider: {}",
                provider.provider
            ));
            draw(terminal, app, runtime)
        }
    }
}

fn run_openai_codex_subscription(
    runtime: &mut TuiRuntime,
    app: &mut TuiApp,
    terminal: &mut TuiTerminal,
    provider_id: &str,
) -> io::Result<()> {
    let flow = match start_openai_codex_oauth_flow() {
        Ok(flow) => flow,
        Err(error) => {
            app.push_error(error);
            return draw(terminal, app, runtime);
        }
    };

    let Some(authorization) = wait_for_browser_authorization(
        runtime,
        app,
        terminal,
        BrowserAuthorizationPrompt {
            title: "OpenAI Codex Subscription",
            url: &flow.url,
            cancelled_message: "OpenAI Codex subscription login cancelled",
            timed_out_message: "OpenAI Codex subscription login timed out",
        },
        |timeout| flow.poll_callback(timeout),
    )?
    else {
        return Ok(());
    };

    set_auth_progress(
        app,
        "OpenAI Codex Subscription",
        vec!["Exchanging authorization code for tokens...".to_string()],
    );
    draw(terminal, app, runtime)?;

    let credential = match finish_openai_codex_oauth_flow(&flow, authorization) {
        Ok(credential) => credential,
        Err(error) => {
            app.overlay = Overlay::None;
            app.push_error(error);
            return draw(terminal, app, runtime);
        }
    };

    match runtime.set_oauth_credential(provider_id, credential) {
        Ok(()) => {
            app.overlay = Overlay::None;
            app.refresh_status(runtime);
            app.push_note(format!(
                "Logged in to OpenAI Codex. Credentials saved to {}",
                runtime.auth_path()
            ));
        }
        Err(error) => {
            app.overlay = Overlay::None;
            app.push_error(error);
        }
    }
    draw(terminal, app, runtime)
}

fn run_anthropic_subscription(
    runtime: &mut TuiRuntime,
    app: &mut TuiApp,
    terminal: &mut TuiTerminal,
    provider_id: &str,
) -> io::Result<()> {
    let flow = match start_anthropic_oauth_flow() {
        Ok(flow) => flow,
        Err(error) => {
            app.push_error(error);
            return draw(terminal, app, runtime);
        }
    };

    let Some(authorization) = wait_for_browser_authorization(
        runtime,
        app,
        terminal,
        BrowserAuthorizationPrompt {
            title: "Anthropic Subscription",
            url: &flow.url,
            cancelled_message: "Anthropic subscription login cancelled",
            timed_out_message: "Anthropic subscription login timed out",
        },
        |timeout| flow.poll_callback(timeout),
    )?
    else {
        return Ok(());
    };

    set_auth_progress(
        app,
        "Anthropic Subscription",
        vec!["Exchanging authorization code for tokens...".to_string()],
    );
    draw(terminal, app, runtime)?;

    let credential = match finish_anthropic_oauth_flow(&flow, authorization) {
        Ok(credential) => credential,
        Err(error) => {
            app.overlay = Overlay::None;
            app.push_error(error);
            return draw(terminal, app, runtime);
        }
    };

    match runtime.set_oauth_credential(provider_id, credential) {
        Ok(()) => {
            app.overlay = Overlay::None;
            app.refresh_status(runtime);
            app.push_note(format!(
                "Logged in to Anthropic. Credentials saved to {}",
                runtime.auth_path()
            ));
        }
        Err(error) => {
            app.overlay = Overlay::None;
            app.push_error(error);
        }
    }
    draw(terminal, app, runtime)
}

fn wait_for_browser_authorization(
    runtime: &TuiRuntime,
    app: &mut TuiApp,
    terminal: &mut TuiTerminal,
    prompt: BrowserAuthorizationPrompt<'_>,
    mut poll_callback: impl FnMut(Duration) -> Result<Option<AuthorizationCode>, String>,
) -> io::Result<Option<AuthorizationCode>> {
    let mut redirect_input = String::new();
    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    set_redirect_progress(
        app,
        prompt.title,
        prompt.url,
        &redirect_input,
        "Waiting for browser callback...",
    );
    draw(terminal, app, runtime)?;
    let _ = open::that(prompt.url);

    loop {
        match poll_callback(Duration::from_millis(0)) {
            Ok(Some(authorization)) => return Ok(Some(authorization)),
            Ok(None) => {}
            Err(error) => {
                app.overlay = Overlay::None;
                app.push_error(error);
                draw(terminal, app, runtime)?;
                return Ok(None);
            }
        }

        if Instant::now() >= deadline {
            app.overlay = Overlay::None;
            app.push_error(prompt.timed_out_message);
            draw(terminal, app, runtime)?;
            return Ok(None);
        }

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }

        match event::read()? {
            Event::Resize(width, height) => {
                handle_resize(terminal, width, height)?;
                draw(terminal, app, runtime)?;
            }
            Event::Paste(value) => {
                redirect_input.push_str(&value);
                set_redirect_progress(
                    app,
                    prompt.title,
                    prompt.url,
                    &redirect_input,
                    "Redirect URL pasted.",
                );
                draw(terminal, app, runtime)?;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc => {
                    app.overlay = Overlay::None;
                    app.push_note(prompt.cancelled_message);
                    draw(terminal, app, runtime)?;
                    return Ok(None);
                }
                KeyCode::Backspace => {
                    redirect_input.pop();
                    set_redirect_progress(
                        app,
                        prompt.title,
                        prompt.url,
                        &redirect_input,
                        "Waiting for browser callback...",
                    );
                    draw(terminal, app, runtime)?;
                }
                KeyCode::Enter => match parse_authorization_input(&redirect_input) {
                    Ok(Some(authorization)) => return Ok(Some(authorization)),
                    Ok(None) => {
                        set_redirect_progress(
                            app,
                            prompt.title,
                            prompt.url,
                            &redirect_input,
                            "Redirect URL is empty.",
                        );
                        draw(terminal, app, runtime)?;
                    }
                    Err(error) => {
                        set_redirect_progress(
                            app,
                            prompt.title,
                            prompt.url,
                            &redirect_input,
                            &error,
                        );
                        draw(terminal, app, runtime)?;
                    }
                },
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.overlay = Overlay::None;
                    app.push_note(prompt.cancelled_message);
                    draw(terminal, app, runtime)?;
                    return Ok(None);
                }
                KeyCode::Char(value) => {
                    redirect_input.push(value);
                    set_redirect_progress(
                        app,
                        prompt.title,
                        prompt.url,
                        &redirect_input,
                        "Waiting for browser callback...",
                    );
                    draw(terminal, app, runtime)?;
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn run_github_copilot_subscription(
    runtime: &mut TuiRuntime,
    app: &mut TuiApp,
    terminal: &mut TuiTerminal,
    provider_id: &str,
) -> io::Result<()> {
    let Some(enterprise_domain) = prompt_github_enterprise_domain(runtime, app, terminal)? else {
        app.overlay = Overlay::None;
        app.push_note("GitHub Copilot subscription login cancelled");
        return draw(terminal, app, runtime);
    };

    set_auth_progress(
        app,
        "GitHub Copilot Subscription",
        vec!["Requesting device code...".to_string()],
    );
    draw(terminal, app, runtime)?;

    let flow = match start_github_copilot_device_flow(enterprise_domain.as_deref()) {
        Ok(flow) => flow,
        Err(error) => {
            app.overlay = Overlay::None;
            app.push_error(error);
            return draw(terminal, app, runtime);
        }
    };

    set_github_progress(
        app,
        &flow.verification_uri,
        &flow.user_code,
        "Waiting for browser authentication...",
    );
    draw(terminal, app, runtime)?;
    let _ = open::that(&flow.verification_uri);

    let mut auth_io_error = None;
    let credential = match finish_github_copilot_device_flow_cancellable(&flow, &mut |message| {
        set_github_progress(app, &flow.verification_uri, &flow.user_code, message);
        let should_cancel = match poll_auth_cancel_events(terminal) {
            Ok(should_cancel) => should_cancel,
            Err(error) => {
                auth_io_error = Some(error);
                true
            }
        };
        let _ = draw(terminal, app, runtime);
        !should_cancel
    }) {
        Ok(credential) => credential,
        Err(error) => {
            if let Some(error) = auth_io_error {
                return Err(error);
            }
            app.overlay = Overlay::None;
            if error == "device flow cancelled" {
                app.push_note("GitHub Copilot subscription login cancelled");
            } else {
                app.push_error(error);
            }
            return draw(terminal, app, runtime);
        }
    };

    match runtime.set_oauth_credential(provider_id, credential) {
        Ok(()) => {
            app.overlay = Overlay::None;
            app.refresh_status(runtime);
            app.push_note(format!(
                "Logged in to GitHub Copilot. Credentials saved to {}",
                runtime.auth_path()
            ));
        }
        Err(error) => {
            app.overlay = Overlay::None;
            app.push_error(error);
        }
    }
    draw(terminal, app, runtime)
}

fn prompt_github_enterprise_domain(
    runtime: &TuiRuntime,
    app: &mut TuiApp,
    terminal: &mut TuiTerminal,
) -> io::Result<Option<Option<String>>> {
    let mut input = String::new();
    loop {
        set_auth_progress(
            app,
            "GitHub Copilot Subscription",
            vec![
                "Leave enterprise domain blank for github.com.".to_string(),
                "Press Enter to continue, Esc to cancel.".to_string(),
                String::new(),
                format!("enterprise domain: {}", input_view(&input, 64)),
            ],
        );
        draw(terminal, app, runtime)?;

        match event::read()? {
            Event::Resize(width, height) => handle_resize(terminal, width, height)?,
            Event::Paste(value) => input.push_str(value.trim()),
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc => return Ok(None),
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Enter => {
                    if input.trim().is_empty() {
                        return Ok(Some(None));
                    }
                    match normalize_github_domain(&input) {
                        Some(domain) => return Ok(Some(Some(domain))),
                        None => {
                            app.push_error("invalid GitHub Enterprise URL/domain");
                            input.clear();
                        }
                    }
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(None);
                }
                KeyCode::Char(value) => input.push(value),
                _ => {}
            },
            _ => {}
        }
    }
}

fn poll_auth_cancel_events(terminal: &mut TuiTerminal) -> io::Result<bool> {
    let mut should_cancel = false;
    while event::poll(Duration::from_millis(0))? {
        match event::read()? {
            Event::Resize(width, height) => {
                handle_resize(terminal, width, height)?;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc => should_cancel = true,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    should_cancel = true;
                }
                _ => {}
            },
            _ => {}
        }
    }
    Ok(should_cancel)
}

fn set_redirect_progress(
    app: &mut TuiApp,
    title: &str,
    url: &str,
    redirect_input: &str,
    status: &str,
) {
    let mut lines = vec![
        "Open this URL in your browser:".to_string(),
        url.to_string(),
        String::new(),
        status.to_string(),
        "Paste final redirect URL and press Enter if callback does not return.".to_string(),
        "Esc cancels.".to_string(),
    ];
    if !redirect_input.is_empty() {
        lines.push(String::new());
        lines.push(format!("redirect URL: {}", input_view(redirect_input, 72)));
    }
    set_auth_progress(app, title, lines);
}

fn set_github_progress(app: &mut TuiApp, verification_uri: &str, user_code: &str, status: &str) {
    set_auth_progress(
        app,
        "GitHub Copilot Subscription",
        vec![
            "Open this URL in your browser:".to_string(),
            verification_uri.to_string(),
            format!("Enter code: {user_code}"),
            String::new(),
            status.to_string(),
        ],
    );
}

fn set_auth_progress(app: &mut TuiApp, title: impl Into<String>, lines: Vec<String>) {
    app.overlay = Overlay::AuthProgress(AuthProgressState {
        title: title.into(),
        lines,
    });
}
