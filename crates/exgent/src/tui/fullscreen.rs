use std::{
    cmp, env,
    io::{self, IsTerminal, Stdout},
    time::{Duration, Instant},
};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use exgent_core::AgentEvent;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
    Frame, Terminal,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::{AppRuntime, ModelMenuItem, ModelSettingsItem, UsageTotals},
    commands::{parse_command, AppCommand, COMMAND_HELP},
    localization::{tr, Locale, MessageId, LANGUAGE_OPTIONS},
    model_service::{AuthProviderInfo, SubscriptionProviderInfo},
    oauth::{
        finish_anthropic_oauth_flow, finish_github_copilot_device_flow_cancellable,
        normalize_github_domain, parse_authorization_input, start_anthropic_oauth_flow,
        start_github_copilot_device_flow,
    },
    session::SessionInfo,
    settings::{ThemeRgb, ThemeSettings, THEME_PRESETS},
};

type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

const SIDEBAR_MIN_WIDTH: u16 = 100;
const SIDEBAR_WIDTH: u16 = 36;

pub(super) fn run(runtime: &mut AppRuntime) -> io::Result<()> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return super::run_legacy(runtime);
    }

    let mut terminal = enter_terminal()?;
    let _guard = TerminalRestoreGuard;
    let mut app = FullscreenApp::new(runtime);
    app.push_welcome(tr(app.locale, MessageId::WelcomeNote));

    loop {
        draw(&mut terminal, &mut app, runtime)?;
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                match handle_key(&mut app, runtime, key) {
                    UiAction::None => {}
                    UiAction::Quit => return Ok(()),
                    UiAction::RunPrompt(prompt) => {
                        run_prompt(runtime, &mut app, &mut terminal, prompt)?;
                    }
                    UiAction::RunSubscriptionAuth(provider) => {
                        run_subscription_auth(runtime, &mut app, &mut terminal, provider)?;
                    }
                }
            }
            Event::Resize(width, height) => {
                handle_resize(&mut terminal, width, height)?;
            }
            Event::Paste(value) => {
                handle_paste(&mut app, &value);
            }
            _ => {}
        }
    }
}

fn enter_terminal() -> io::Result<TuiTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

struct TerminalRestoreGuard;

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, cursor::Show, LeaveAlternateScreen);
    }
}

fn handle_resize(terminal: &mut TuiTerminal, mut width: u16, mut height: u16) -> io::Result<()> {
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

fn draw(
    terminal: &mut TuiTerminal,
    app: &mut FullscreenApp,
    runtime: &AppRuntime,
) -> io::Result<()> {
    app.refresh_status(runtime);
    terminal.draw(|frame| render(frame, app))?;
    Ok(())
}

fn run_prompt(
    runtime: &mut AppRuntime,
    app: &mut FullscreenApp,
    terminal: &mut TuiTerminal,
    prompt: String,
) -> io::Result<()> {
    app.is_running = true;
    app.push_user(prompt.clone());
    app.start_assistant();
    draw(terminal, app, runtime)?;

    let mut prompt_error = None;
    let result = runtime.run_prompt_streaming(&prompt, &mut |event| {
        app.apply_agent_event(event);
        drain_resize_events(terminal);
        let _ = terminal.draw(|frame| render(frame, app));
    });
    if let Err(error) = result {
        prompt_error = Some(error);
    }

    if let Some(error) = prompt_error {
        app.push_error(error);
    }
    app.is_running = false;
    app.runtime_activity = None;
    app.refresh_status(runtime);
    Ok(())
}

fn run_subscription_auth(
    runtime: &mut AppRuntime,
    app: &mut FullscreenApp,
    terminal: &mut TuiTerminal,
    provider: SubscriptionProviderInfo,
) -> io::Result<()> {
    app.overlay = Overlay::None;
    match provider.provider.as_str() {
        "anthropic" => run_anthropic_subscription(runtime, app, terminal, &provider.provider),
        "github-copilot" => {
            run_github_copilot_subscription(runtime, app, terminal, &provider.provider)
        }
        "openai-codex" => {
            app.push_note(format!(
                "subscription login for {} needs callback-server OAuth and is not implemented yet",
                provider.name
            ));
            draw(terminal, app, runtime)
        }
        _ => {
            app.push_error(format!(
                "unknown subscription provider: {}",
                provider.provider
            ));
            draw(terminal, app, runtime)
        }
    }
}

fn run_anthropic_subscription(
    runtime: &mut AppRuntime,
    app: &mut FullscreenApp,
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

    let mut redirect_input = String::new();
    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    set_anthropic_progress(
        app,
        &flow.url,
        &redirect_input,
        "Waiting for browser callback...",
    );
    draw(terminal, app, runtime)?;

    let authorization = loop {
        match flow.poll_callback(Duration::from_millis(0)) {
            Ok(Some(authorization)) => break Some(authorization),
            Ok(None) => {}
            Err(error) => {
                app.overlay = Overlay::None;
                app.push_error(error);
                return draw(terminal, app, runtime);
            }
        }

        if Instant::now() >= deadline {
            app.overlay = Overlay::None;
            app.push_error("Anthropic subscription login timed out");
            return draw(terminal, app, runtime);
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
                set_anthropic_progress(app, &flow.url, &redirect_input, "Redirect URL pasted.");
                draw(terminal, app, runtime)?;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Esc => {
                    app.overlay = Overlay::None;
                    app.push_note("Anthropic subscription login cancelled");
                    return draw(terminal, app, runtime);
                }
                KeyCode::Backspace => {
                    redirect_input.pop();
                    set_anthropic_progress(
                        app,
                        &flow.url,
                        &redirect_input,
                        "Waiting for browser callback...",
                    );
                    draw(terminal, app, runtime)?;
                }
                KeyCode::Enter => match parse_authorization_input(&redirect_input) {
                    Ok(Some(authorization)) => break Some(authorization),
                    Ok(None) => {
                        set_anthropic_progress(
                            app,
                            &flow.url,
                            &redirect_input,
                            "Redirect URL is empty.",
                        );
                        draw(terminal, app, runtime)?;
                    }
                    Err(error) => {
                        set_anthropic_progress(app, &flow.url, &redirect_input, &error);
                        draw(terminal, app, runtime)?;
                    }
                },
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.overlay = Overlay::None;
                    app.push_note("Anthropic subscription login cancelled");
                    return draw(terminal, app, runtime);
                }
                KeyCode::Char(value) => {
                    redirect_input.push(value);
                    set_anthropic_progress(
                        app,
                        &flow.url,
                        &redirect_input,
                        "Waiting for browser callback...",
                    );
                    draw(terminal, app, runtime)?;
                }
                _ => {}
            },
            _ => {}
        }
    };

    let Some(authorization) = authorization else {
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

fn run_github_copilot_subscription(
    runtime: &mut AppRuntime,
    app: &mut FullscreenApp,
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
        let _ = terminal.draw(|frame| render(frame, app));
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
    runtime: &AppRuntime,
    app: &mut FullscreenApp,
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

fn set_anthropic_progress(app: &mut FullscreenApp, url: &str, redirect_input: &str, status: &str) {
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
    set_auth_progress(app, "Anthropic Subscription", lines);
}

fn set_github_progress(
    app: &mut FullscreenApp,
    verification_uri: &str,
    user_code: &str,
    status: &str,
) {
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

fn set_auth_progress(app: &mut FullscreenApp, title: impl Into<String>, lines: Vec<String>) {
    app.overlay = Overlay::AuthProgress(AuthProgressState {
        title: title.into(),
        lines,
    });
}

fn drain_resize_events(terminal: &mut TuiTerminal) {
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

#[derive(Clone, Debug)]
struct FullscreenApp {
    transcript: Vec<TranscriptItem>,
    composer: ComposerState,
    overlay: Overlay,
    model_label: String,
    session_id: String,
    cwd: String,
    usage: UsageTotals,
    model_context_window: Option<u64>,
    model_reasoning: bool,
    is_running: bool,
    runtime_activity: Option<RuntimeActivity>,
    show_reasoning: bool,
    locale: Locale,
    theme: ThemeSettings,
    theme_preview: Option<ThemeSettings>,
}

impl FullscreenApp {
    fn new(runtime: &AppRuntime) -> Self {
        let mut app = Self {
            transcript: Vec::new(),
            composer: ComposerState::default(),
            overlay: Overlay::None,
            model_label: String::new(),
            session_id: String::new(),
            cwd: env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| ".".to_string()),
            usage: UsageTotals::default(),
            model_context_window: None,
            model_reasoning: false,
            is_running: false,
            runtime_activity: None,
            show_reasoning: runtime.prompt_display_enabled(),
            locale: runtime.locale(),
            theme: runtime.theme(),
            theme_preview: None,
        };
        app.refresh_status(runtime);
        app
    }

    fn refresh_status(&mut self, runtime: &AppRuntime) {
        self.model_label = runtime.model_label();
        self.session_id = runtime.session_id().to_string();
        self.usage = runtime.usage_totals().clone();
        self.show_reasoning = runtime.prompt_display_enabled();
        self.locale = runtime.locale();
        self.theme = runtime.theme();
        if let Some(model) = runtime.model_status() {
            self.model_context_window = model.context_window;
            self.model_reasoning = model.reasoning;
        } else {
            self.model_context_window = None;
            self.model_reasoning = false;
        }
    }

    fn push_note(&mut self, note: impl Into<String>) {
        self.transcript.push(TranscriptItem::Note(note.into()));
    }

    fn push_welcome(&mut self, message: impl Into<String>) {
        self.transcript
            .push(TranscriptItem::Welcome(message.into()));
    }

    fn push_error(&mut self, error: impl Into<String>) {
        self.transcript.push(TranscriptItem::Error(error.into()));
    }

    fn push_user(&mut self, input: impl Into<String>) {
        self.transcript.push(TranscriptItem::User(input.into()));
    }

    fn start_assistant(&mut self) {
        self.runtime_activity = Some(RuntimeActivity::Thinking);
        self.transcript
            .push(TranscriptItem::Assistant(String::new()));
    }

    fn append_assistant(&mut self, delta: &str) {
        self.runtime_activity = None;
        match self.transcript.last_mut() {
            Some(TranscriptItem::Assistant(content)) => content.push_str(delta),
            _ => self
                .transcript
                .push(TranscriptItem::Assistant(delta.to_string())),
        }
    }

    fn append_reasoning(&mut self, delta: &str) {
        self.runtime_activity = Some(RuntimeActivity::Thinking);
        if !self.show_reasoning {
            return;
        }
        match self.transcript.last_mut() {
            Some(TranscriptItem::Reasoning(content)) => content.push_str(delta),
            _ => self
                .transcript
                .push(TranscriptItem::Reasoning(delta.to_string())),
        }
    }

    fn apply_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::AgentStart | AgentEvent::MessageStart { .. } => {
                self.runtime_activity = Some(RuntimeActivity::Thinking);
            }
            AgentEvent::AgentEnd => {
                self.runtime_activity = None;
            }
            AgentEvent::MessageDelta { delta } => self.append_assistant(&delta),
            AgentEvent::ReasoningDelta { delta } => self.append_reasoning(&delta),
            AgentEvent::Usage { .. } => {}
            AgentEvent::MessageEnd { .. } => {
                self.runtime_activity = None;
            }
            AgentEvent::ToolCallStart {
                name, arguments, ..
            } => {
                self.runtime_activity = Some(RuntimeActivity::Tool(name.clone()));
                self.transcript
                    .push(TranscriptItem::Tool(format!("{name}: {arguments:?}")));
            }
            AgentEvent::ToolCallEnd {
                name,
                content,
                is_error,
                ..
            } => {
                let status = if is_error { "error" } else { "ok" };
                self.transcript
                    .push(TranscriptItem::Tool(format!("{name} {status}: {content}")));
                self.runtime_activity = Some(RuntimeActivity::Thinking);
            }
            AgentEvent::Error { message } => {
                self.runtime_activity = None;
                self.push_error(message);
            }
        }
    }

    fn sync_slash_menu(&mut self) {
        if matches!(self.overlay, Overlay::ModelPicker(_)) {
            return;
        }
        if self.composer.input.starts_with('/') {
            self.overlay = Overlay::SlashMenu { selected: 0 };
        } else if matches!(self.overlay, Overlay::SlashMenu { .. }) {
            self.overlay = Overlay::None;
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ComposerState {
    input: String,
    history: Vec<String>,
    history_index: Option<usize>,
    draft: String,
}

#[derive(Clone, Debug)]
enum Overlay {
    None,
    SlashMenu { selected: usize },
    ModelPicker(ModelPickerState),
    SettingsMenu(SettingsMenuState),
    AuthSettings(AuthSettingsState),
    AuthAction(AuthActionState),
    ModelSettings(ModelSettingsState),
    ModelAction(ModelActionState),
    ThemePicker(ThemePickerState),
    CustomTheme(CustomThemeState),
    LanguagePicker(LanguagePickerState),
    SessionPicker(SessionPickerState),
    DebugMenu(DebugMenuState),
    DebugPrompt(DebugPromptState),
    AuthMethod(AuthMethodState),
    ApiKeyProvider(ApiKeyProviderState),
    ApiKeyInput(ApiKeyInputState),
    AddModelForm(AddModelFormState),
    SubscriptionProvider(SubscriptionProviderState),
    AuthProgress(AuthProgressState),
}

#[derive(Clone, Debug)]
struct ModelPickerState {
    models: Vec<ModelMenuItem>,
    selected: usize,
}

#[derive(Clone, Debug)]
struct SettingsMenuState {
    selected: usize,
}

#[derive(Clone, Debug)]
struct AuthProviderItem {
    provider: String,
    model_indices: Vec<usize>,
    is_enabled: bool,
    has_built_in: bool,
}

#[derive(Clone, Debug)]
struct AuthSettingsState {
    providers: Vec<AuthProviderItem>,
    selected: usize,
    checked: Vec<bool>,
}

#[derive(Clone, Debug)]
struct AuthActionState {
    providers: Vec<AuthProviderItem>,
    targets: Vec<usize>,
    selected: usize,
}

#[derive(Clone, Debug)]
struct ModelSettingsState {
    models: Vec<ModelSettingsItem>,
    selected: usize,
    checked: Vec<bool>,
}

#[derive(Clone, Debug)]
struct ModelActionState {
    models: Vec<ModelSettingsItem>,
    targets: Vec<usize>,
    selected: usize,
}

#[derive(Clone, Debug)]
struct ThemePickerState {
    selected: usize,
}

#[derive(Clone, Debug)]
struct CustomThemeState {
    field: usize,
    red: String,
    green: String,
    blue: String,
}

impl Default for CustomThemeState {
    fn default() -> Self {
        Self {
            field: 0,
            red: String::new(),
            green: String::new(),
            blue: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct LanguagePickerState {
    selected: usize,
}

#[derive(Clone, Debug)]
struct SessionPickerState {
    sessions: Vec<SessionInfo>,
    selected: usize,
}

#[derive(Clone, Debug)]
struct DebugMenuState {
    selected: usize,
}

#[derive(Clone, Debug)]
struct DebugPromptState {
    selected: usize,
}

#[derive(Clone, Debug)]
struct AuthMethodState {
    selected: usize,
}

#[derive(Clone, Debug)]
struct ApiKeyProviderState {
    providers: Vec<AuthProviderInfo>,
    selected: usize,
}

#[derive(Clone, Debug)]
struct ApiKeyInputState {
    provider: String,
    value: String,
}

#[derive(Clone, Debug)]
struct AddModelFormState {
    field: usize,
    provider: String,
    model_id: String,
    base_url: String,
    api_key: String,
}

impl Default for AddModelFormState {
    fn default() -> Self {
        Self {
            field: 0,
            provider: String::new(),
            model_id: String::new(),
            base_url: "https://api.example.com/v1".to_string(),
            api_key: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct SubscriptionProviderState {
    providers: Vec<SubscriptionProviderInfo>,
    selected: usize,
}

#[derive(Clone, Debug)]
struct AuthProgressState {
    title: String,
    lines: Vec<String>,
}

#[derive(Clone, Debug)]
enum TranscriptItem {
    Welcome(String),
    User(String),
    Assistant(String),
    Reasoning(String),
    Tool(String),
    Note(String),
    Error(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RuntimeActivity {
    Thinking,
    Tool(String),
}

enum UiAction {
    None,
    Quit,
    RunPrompt(String),
    RunSubscriptionAuth(SubscriptionProviderInfo),
}

fn handle_key(app: &mut FullscreenApp, runtime: &mut AppRuntime, key: KeyEvent) -> UiAction {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return UiAction::Quit;
    }

    match app.overlay.clone() {
        Overlay::ModelPicker(mut picker) => handle_model_picker_key(app, runtime, key, &mut picker),
        Overlay::SettingsMenu(mut state) => handle_settings_menu_key(app, runtime, key, &mut state),
        Overlay::AuthSettings(mut state) => handle_auth_settings_key(app, runtime, key, &mut state),
        Overlay::AuthAction(mut state) => handle_auth_action_key(app, runtime, key, &mut state),
        Overlay::ModelSettings(mut state) => {
            handle_model_settings_key(app, runtime, key, &mut state)
        }
        Overlay::ModelAction(mut state) => handle_model_action_key(app, runtime, key, &mut state),
        Overlay::ThemePicker(mut state) => handle_theme_picker_key(app, runtime, key, &mut state),
        Overlay::CustomTheme(mut state) => handle_custom_theme_key(app, runtime, key, &mut state),
        Overlay::LanguagePicker(mut state) => {
            handle_language_picker_key(app, runtime, key, &mut state)
        }
        Overlay::SessionPicker(mut state) => {
            handle_session_picker_key(app, runtime, key, &mut state)
        }
        Overlay::DebugMenu(mut state) => handle_debug_menu_key(app, key, &mut state),
        Overlay::DebugPrompt(mut state) => handle_debug_prompt_key(app, runtime, key, &mut state),
        Overlay::AuthMethod(mut state) => handle_auth_method_key(app, runtime, key, &mut state),
        Overlay::ApiKeyProvider(mut state) => {
            handle_api_key_provider_key(app, runtime, key, &mut state)
        }
        Overlay::ApiKeyInput(mut state) => handle_api_key_input_key(app, runtime, key, &mut state),
        Overlay::AddModelForm(mut state) => {
            handle_add_model_form_key(app, runtime, key, &mut state)
        }
        Overlay::SubscriptionProvider(mut state) => {
            handle_subscription_provider_key(app, key, &mut state)
        }
        Overlay::AuthProgress(_) => UiAction::None,
        Overlay::SlashMenu { selected } => handle_composer_key(app, runtime, key, Some(selected)),
        Overlay::None => handle_composer_key(app, runtime, key, None),
    }
}

fn handle_model_picker_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    picker: &mut ModelPickerState,
) -> UiAction {
    if picker.models.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoModelsAvailable));
        return UiAction::None;
    }

    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            picker.selected = if picker.selected == 0 {
                picker.models.len() - 1
            } else {
                picker.selected - 1
            };
            app.overlay = Overlay::ModelPicker(picker.clone());
        }
        KeyCode::Down => {
            picker.selected = (picker.selected + 1) % picker.models.len();
            app.overlay = Overlay::ModelPicker(picker.clone());
        }
        KeyCode::Enter => {
            let model = &picker.models[picker.selected];
            match runtime.select_model(model.index) {
                Ok(()) => {
                    app.refresh_status(runtime);
                    app.push_note(format!("selected model: {}", runtime.model_label()));
                }
                Err(error) => app.push_error(error),
            }
            app.overlay = Overlay::None;
        }
        _ => {}
    }
    UiAction::None
}

fn handle_settings_menu_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut SettingsMenuState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(3);
            app.overlay = Overlay::SettingsMenu(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % 4;
            app.overlay = Overlay::SettingsMenu(state.clone());
        }
        KeyCode::Enter => match state.selected {
            0 => open_auth_settings_overlay(app, runtime),
            1 => open_model_settings_overlay(app, runtime),
            2 => open_theme_picker_overlay(app, runtime),
            _ => open_language_picker_overlay(app, runtime),
        },
        _ => {}
    }
    UiAction::None
}

fn handle_auth_settings_key(
    app: &mut FullscreenApp,
    _runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut AuthSettingsState,
) -> UiAction {
    if state.providers.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoConfiguredProviders));
        return UiAction::None;
    }

    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            state.selected = state
                .selected
                .checked_sub(1)
                .unwrap_or(state.providers.len() - 1);
            app.overlay = Overlay::AuthSettings(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % state.providers.len();
            app.overlay = Overlay::AuthSettings(state.clone());
        }
        KeyCode::Char(' ') => {
            if let Some(checked) = state.checked.get_mut(state.selected) {
                *checked = !*checked;
            }
            app.overlay = Overlay::AuthSettings(state.clone());
        }
        KeyCode::Enter => {
            let targets = checked_or_focused_indices(&state.checked, state.selected);
            app.overlay = Overlay::AuthAction(AuthActionState {
                providers: state.providers.clone(),
                targets,
                selected: 0,
            });
        }
        _ => {}
    }
    UiAction::None
}

fn handle_auth_action_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut AuthActionState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => {
            app.overlay = Overlay::AuthSettings(AuthSettingsState {
                providers: state.providers.clone(),
                selected: state.targets.first().copied().unwrap_or(0),
                checked: vec![false; state.providers.len()],
            });
        }
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(2);
            app.overlay = Overlay::AuthAction(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % 3;
            app.overlay = Overlay::AuthAction(state.clone());
        }
        KeyCode::Enter => {
            let message = match state.selected {
                0 => set_providers_enabled(runtime, &state.providers, &state.targets, true),
                1 => set_providers_enabled(runtime, &state.providers, &state.targets, false),
                2 => remove_providers(runtime, &state.providers, &state.targets),
                _ => Ok(String::new()),
            };
            match message {
                Ok(message) if !message.is_empty() => app.push_note(message),
                Ok(_) => {}
                Err(error) => app.push_error(error),
            }
            app.refresh_status(runtime);
            open_auth_settings_overlay(app, runtime);
        }
        _ => {}
    }
    UiAction::None
}

fn handle_model_settings_key(
    app: &mut FullscreenApp,
    _runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut ModelSettingsState,
) -> UiAction {
    if state.models.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoConfiguredModelsEnableProvider));
        return UiAction::None;
    }

    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            state.selected = state
                .selected
                .checked_sub(1)
                .unwrap_or(state.models.len() - 1);
            app.overlay = Overlay::ModelSettings(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % state.models.len();
            app.overlay = Overlay::ModelSettings(state.clone());
        }
        KeyCode::Char(' ') => {
            if let Some(checked) = state.checked.get_mut(state.selected) {
                *checked = !*checked;
            }
            app.overlay = Overlay::ModelSettings(state.clone());
        }
        KeyCode::Enter => {
            let targets = checked_or_focused_indices(&state.checked, state.selected);
            app.overlay = Overlay::ModelAction(ModelActionState {
                models: state.models.clone(),
                targets,
                selected: 0,
            });
        }
        _ => {}
    }
    UiAction::None
}

fn handle_model_action_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut ModelActionState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => {
            app.overlay = Overlay::ModelSettings(ModelSettingsState {
                models: state.models.clone(),
                selected: state.targets.first().copied().unwrap_or(0),
                checked: vec![false; state.models.len()],
            });
        }
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(1);
            app.overlay = Overlay::ModelAction(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % 2;
            app.overlay = Overlay::ModelAction(state.clone());
        }
        KeyCode::Enter => {
            let message = set_model_items_enabled(
                runtime,
                &state.models,
                &state.targets,
                state.selected == 0,
            );
            match message {
                Ok(message) => app.push_note(message),
                Err(error) => app.push_error(error),
            }
            app.refresh_status(runtime);
            open_model_settings_overlay(app, runtime);
        }
        _ => {}
    }
    UiAction::None
}

fn handle_theme_picker_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut ThemePickerState,
) -> UiAction {
    let item_count = THEME_PRESETS.len() + 1;
    match key.code {
        KeyCode::Esc => {
            app.theme_preview = None;
            app.overlay = Overlay::SettingsMenu(SettingsMenuState { selected: 2 });
        }
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(item_count - 1);
            preview_theme_selection(app, runtime, state.selected);
            app.overlay = Overlay::ThemePicker(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % item_count;
            preview_theme_selection(app, runtime, state.selected);
            app.overlay = Overlay::ThemePicker(state.clone());
        }
        KeyCode::Enter => {
            if let Some(preset) = THEME_PRESETS.get(state.selected).copied() {
                let theme = ThemeSettings::preset(preset);
                match runtime.set_theme(theme.clone()) {
                    Ok(()) => {
                        app.theme_preview = None;
                        app.refresh_status(runtime);
                        app.push_note(
                            tr(app.locale, MessageId::ThemeSaved).replace("{name}", &theme.name),
                        );
                    }
                    Err(error) => app.push_error(error),
                }
                app.overlay = Overlay::None;
            } else {
                app.theme_preview = None;
                app.overlay = Overlay::CustomTheme(CustomThemeState::default());
            }
        }
        _ => {}
    }
    UiAction::None
}

fn handle_custom_theme_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut CustomThemeState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => open_theme_picker_overlay(app, runtime),
        KeyCode::Backspace => {
            active_theme_field_mut(state).pop();
            app.overlay = Overlay::CustomTheme(state.clone());
        }
        KeyCode::Tab | KeyCode::Down => {
            state.field = (state.field + 1) % 3;
            app.overlay = Overlay::CustomTheme(state.clone());
        }
        KeyCode::Up => {
            state.field = state.field.checked_sub(1).unwrap_or(2);
            app.overlay = Overlay::CustomTheme(state.clone());
        }
        KeyCode::Enter => {
            if state.field < 2 {
                state.field += 1;
                app.overlay = Overlay::CustomTheme(state.clone());
                return UiAction::None;
            }
            match parse_custom_theme(state) {
                Ok(theme) => match runtime.set_theme(theme.clone()) {
                    Ok(()) => {
                        app.theme_preview = None;
                        app.refresh_status(runtime);
                        app.push_note(format!(
                            "{}",
                            tr(app.locale, MessageId::ThemeSaved).replace(
                                "{name}",
                                &format!(
                                    "custom rgb({}, {}, {})",
                                    theme.rgb.r, theme.rgb.g, theme.rgb.b
                                ),
                            )
                        ));
                        app.overlay = Overlay::None;
                    }
                    Err(error) => {
                        app.push_error(error);
                        app.overlay = Overlay::CustomTheme(state.clone());
                    }
                },
                Err(error) => {
                    app.push_error(error);
                    app.overlay = Overlay::CustomTheme(state.clone());
                }
            }
        }
        KeyCode::Char(value) if value.is_ascii_digit() => {
            let field = active_theme_field_mut(state);
            if field.len() < 3 {
                field.push(value);
            }
            app.overlay = Overlay::CustomTheme(state.clone());
        }
        _ => {}
    }
    UiAction::None
}

fn handle_language_picker_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut LanguagePickerState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => {
            app.overlay = Overlay::SettingsMenu(SettingsMenuState { selected: 3 });
        }
        KeyCode::Up => {
            state.selected = state
                .selected
                .checked_sub(1)
                .unwrap_or(LANGUAGE_OPTIONS.len().saturating_sub(1));
            app.overlay = Overlay::LanguagePicker(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % LANGUAGE_OPTIONS.len();
            app.overlay = Overlay::LanguagePicker(state.clone());
        }
        KeyCode::Enter => {
            let option = LANGUAGE_OPTIONS[state.selected.min(LANGUAGE_OPTIONS.len() - 1)];
            match runtime.set_locale(option.setting) {
                Ok(()) => {
                    app.refresh_status(runtime);
                    app.push_note(
                        tr(app.locale, MessageId::LanguageSaved)
                            .replace("{name}", option.locale.display_name()),
                    );
                    app.overlay = Overlay::None;
                }
                Err(error) => app.push_error(error),
            }
        }
        _ => {}
    }
    UiAction::None
}

fn handle_session_picker_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut SessionPickerState,
) -> UiAction {
    let item_count = state.sessions.len() + 1;
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(item_count - 1);
            app.overlay = Overlay::SessionPicker(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % item_count;
            app.overlay = Overlay::SessionPicker(state.clone());
        }
        KeyCode::Enter => {
            if state.selected == 0 {
                match runtime.start_new_session() {
                    Ok(()) => {
                        app.transcript.clear();
                        app.refresh_status(runtime);
                        app.push_note(
                            tr(app.locale, MessageId::NewSessionCreated)
                                .replace("{id}", runtime.session_id()),
                        );
                    }
                    Err(error) => app.push_error(error),
                }
            } else if let Some(session) = state.sessions.get(state.selected - 1) {
                match runtime.open_session(&session.path) {
                    Ok(()) => {
                        app.transcript.clear();
                        app.refresh_status(runtime);
                        app.push_note(
                            tr(app.locale, MessageId::SessionLoaded)
                                .replace("{id}", runtime.session_id()),
                        );
                        load_recent_messages(app, runtime);
                    }
                    Err(error) => app.push_error(error),
                }
            }
            app.overlay = Overlay::None;
        }
        _ => {}
    }
    UiAction::None
}

fn handle_debug_menu_key(
    app: &mut FullscreenApp,
    key: KeyEvent,
    state: &mut DebugMenuState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up | KeyCode::Down => {
            state.selected = 0;
            app.overlay = Overlay::DebugMenu(state.clone());
        }
        KeyCode::Enter => {
            app.overlay = Overlay::DebugPrompt(DebugPromptState { selected: 0 });
        }
        _ => {}
    }
    UiAction::None
}

fn handle_debug_prompt_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut DebugPromptState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::DebugMenu(DebugMenuState { selected: 0 }),
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(1);
            app.overlay = Overlay::DebugPrompt(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % 2;
            app.overlay = Overlay::DebugPrompt(state.clone());
        }
        KeyCode::Enter => {
            let enabled = state.selected == 0;
            match runtime.set_prompt_display_enabled(enabled) {
                Ok(()) => app.push_note(if enabled {
                    tr(app.locale, MessageId::DebugPromptEnabled)
                } else {
                    tr(app.locale, MessageId::DebugPromptDisabled)
                }),
                Err(error) => app.push_error(error),
            }
            app.refresh_status(runtime);
            app.overlay = Overlay::None;
        }
        _ => {}
    }
    UiAction::None
}

fn handle_auth_method_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut AuthMethodState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(1);
            app.overlay = Overlay::AuthMethod(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % 2;
            app.overlay = Overlay::AuthMethod(state.clone());
        }
        KeyCode::Enter => {
            if state.selected == 0 {
                open_subscription_provider_overlay(app, runtime);
            } else {
                open_api_key_provider_overlay(app, runtime);
            }
        }
        _ => {}
    }
    UiAction::None
}

fn handle_api_key_provider_key(
    app: &mut FullscreenApp,
    _runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut ApiKeyProviderState,
) -> UiAction {
    let item_count = state.providers.len() + 1;
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::AuthMethod(AuthMethodState { selected: 1 }),
        KeyCode::Up => {
            state.selected = state.selected.checked_sub(1).unwrap_or(item_count - 1);
            app.overlay = Overlay::ApiKeyProvider(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % item_count;
            app.overlay = Overlay::ApiKeyProvider(state.clone());
        }
        KeyCode::Enter => {
            if state.selected == state.providers.len() {
                app.overlay = Overlay::AddModelForm(AddModelFormState::default());
            } else if let Some(provider) = state.providers.get(state.selected) {
                app.overlay = Overlay::ApiKeyInput(ApiKeyInputState {
                    provider: provider.provider.clone(),
                    value: String::new(),
                });
            }
        }
        _ => {}
    }
    UiAction::None
}

fn handle_api_key_input_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut ApiKeyInputState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => open_api_key_provider_overlay(app, runtime),
        KeyCode::Backspace => {
            state.value.pop();
            app.overlay = Overlay::ApiKeyInput(state.clone());
        }
        KeyCode::Enter => {
            let token = state.value.trim();
            let result = if token.is_empty() {
                runtime
                    .remove_auth_token(&state.provider)
                    .map(|_| format!("removed auth token for {}", state.provider))
            } else {
                runtime
                    .set_auth_token(&state.provider, token)
                    .map(|_| format!("saved auth token for {}", state.provider))
            };
            match result {
                Ok(message) => app.push_note(message),
                Err(error) => app.push_error(error),
            }
            app.refresh_status(runtime);
            open_api_key_provider_overlay(app, runtime);
        }
        KeyCode::Char(value) => {
            state.value.push(value);
            app.overlay = Overlay::ApiKeyInput(state.clone());
        }
        _ => {}
    }
    UiAction::None
}

fn handle_add_model_form_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    state: &mut AddModelFormState,
) -> UiAction {
    match key.code {
        KeyCode::Esc => open_api_key_provider_overlay(app, runtime),
        KeyCode::Backspace => {
            active_add_model_field_mut(state).pop();
            app.overlay = Overlay::AddModelForm(state.clone());
        }
        KeyCode::Tab | KeyCode::Down => {
            state.field = (state.field + 1) % 4;
            app.overlay = Overlay::AddModelForm(state.clone());
        }
        KeyCode::Up => {
            state.field = state.field.checked_sub(1).unwrap_or(3);
            app.overlay = Overlay::AddModelForm(state.clone());
        }
        KeyCode::Enter => {
            if state.field < 3 {
                state.field += 1;
                app.overlay = Overlay::AddModelForm(state.clone());
                return UiAction::None;
            }
            if state.provider.trim().is_empty()
                || state.model_id.trim().is_empty()
                || state.base_url.trim().is_empty()
            {
                app.push_error("provider, model id, and base url are required");
                app.overlay = Overlay::AddModelForm(state.clone());
                return UiAction::None;
            }
            match runtime.add_openai_compatible_model(
                state.provider.trim(),
                state.model_id.trim(),
                state.base_url.trim(),
                state.api_key.trim(),
            ) {
                Ok(info) => {
                    app.push_note(format!(
                        "added model {} / {} at index {}",
                        info.provider,
                        info.model_id,
                        info.index + 1
                    ));
                    app.refresh_status(runtime);
                    open_api_key_provider_overlay(app, runtime);
                }
                Err(error) => {
                    app.push_error(error);
                    app.overlay = Overlay::AddModelForm(state.clone());
                }
            }
        }
        KeyCode::Char(value) => {
            active_add_model_field_mut(state).push(value);
            app.overlay = Overlay::AddModelForm(state.clone());
        }
        _ => {}
    }
    UiAction::None
}

fn handle_subscription_provider_key(
    app: &mut FullscreenApp,
    key: KeyEvent,
    state: &mut SubscriptionProviderState,
) -> UiAction {
    if state.providers.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoSubscriptionProviders));
        return UiAction::None;
    }
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Up => {
            state.selected = state
                .selected
                .checked_sub(1)
                .unwrap_or(state.providers.len() - 1);
            app.overlay = Overlay::SubscriptionProvider(state.clone());
        }
        KeyCode::Down => {
            state.selected = (state.selected + 1) % state.providers.len();
            app.overlay = Overlay::SubscriptionProvider(state.clone());
        }
        KeyCode::Enter => {
            let provider = state.providers[state.selected].clone();
            app.overlay = Overlay::None;
            return UiAction::RunSubscriptionAuth(provider);
        }
        _ => {}
    }
    UiAction::None
}

fn handle_composer_key(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    key: KeyEvent,
    slash_selected: Option<usize>,
) -> UiAction {
    if app.is_running {
        return UiAction::None;
    }

    match key.code {
        KeyCode::Esc => {
            app.overlay = Overlay::None;
        }
        KeyCode::Backspace => {
            app.composer.input.pop();
            app.composer.history_index = None;
            app.sync_slash_menu();
        }
        KeyCode::Tab => {
            if let Some(command) = slash_suggestions(&app.composer.input).first() {
                app.composer.input = command.command.to_string();
                app.sync_slash_menu();
            }
        }
        KeyCode::Up if app.composer.input.starts_with('/') => {
            move_slash_selection(app, slash_selected, -1);
        }
        KeyCode::Down if app.composer.input.starts_with('/') => {
            move_slash_selection(app, slash_selected, 1);
        }
        KeyCode::Up => recall_history(app, -1),
        KeyCode::Down => recall_history(app, 1),
        KeyCode::Enter => return submit_input(app, runtime, slash_selected),
        KeyCode::Char(value) => {
            app.composer.input.push(value);
            app.composer.history_index = None;
            app.sync_slash_menu();
        }
        _ => {}
    }
    UiAction::None
}

fn move_slash_selection(app: &mut FullscreenApp, selected: Option<usize>, delta: isize) {
    let suggestions = slash_suggestions(&app.composer.input);
    if suggestions.is_empty() {
        return;
    }
    let selected = selected.unwrap_or(0);
    let next = if delta < 0 {
        selected.checked_sub(1).unwrap_or(suggestions.len() - 1)
    } else {
        (selected + 1) % suggestions.len()
    };
    app.overlay = Overlay::SlashMenu { selected: next };
}

fn recall_history(app: &mut FullscreenApp, delta: isize) {
    if app.composer.history.is_empty() {
        return;
    }
    let len = app.composer.history.len();
    let next = match (app.composer.history_index, delta < 0) {
        (Some(index), true) => index.saturating_sub(1),
        (Some(index), false) if index + 1 < len => index + 1,
        (Some(_), false) => {
            app.composer.history_index = None;
            app.composer.input = app.composer.draft.clone();
            return;
        }
        (None, true) => {
            app.composer.draft = app.composer.input.clone();
            len - 1
        }
        (None, false) => return,
    };
    app.composer.history_index = Some(next);
    app.composer.input = app.composer.history[next].clone();
    app.sync_slash_menu();
}

fn submit_input(
    app: &mut FullscreenApp,
    runtime: &mut AppRuntime,
    slash_selected: Option<usize>,
) -> UiAction {
    let mut input = app.composer.input.trim().to_string();
    if input.is_empty() {
        return UiAction::None;
    }

    if input.starts_with('/')
        && parse_command(&input).is_some_and(|command| matches!(command, AppCommand::Unknown(_)))
    {
        if let Some(index) = slash_selected {
            if let Some(command) = slash_suggestions(&input).get(index) {
                input = command.command.to_string();
            }
        }
    }

    if app.composer.history.last().map(|entry| entry.as_str()) != Some(input.as_str()) {
        app.composer.history.push(input.clone());
    }
    app.composer.history_index = None;
    app.composer.input.clear();
    app.overlay = Overlay::None;

    match parse_command(&input) {
        Some(AppCommand::Quit) => UiAction::Quit,
        Some(AppCommand::Model) => {
            open_model_picker_overlay(app, runtime);
            UiAction::None
        }
        Some(AppCommand::Settings) => {
            app.overlay = Overlay::SettingsMenu(SettingsMenuState { selected: 0 });
            UiAction::None
        }
        Some(AppCommand::SettingsAuth) => {
            open_auth_settings_overlay(app, runtime);
            UiAction::None
        }
        Some(AppCommand::SettingsModel) => {
            open_model_settings_overlay(app, runtime);
            UiAction::None
        }
        Some(AppCommand::SettingsTheme) => {
            open_theme_picker_overlay(app, runtime);
            UiAction::None
        }
        Some(AppCommand::SettingsLanguage) => {
            open_language_picker_overlay(app, runtime);
            UiAction::None
        }
        Some(AppCommand::Session) => {
            open_session_picker_overlay(app, runtime);
            UiAction::None
        }
        Some(AppCommand::Compact) => {
            match runtime.compact_context() {
                Ok(count) => app.push_note(
                    tr(app.locale, MessageId::CompactSuccess)
                        .replace("{count}", &count.to_string()),
                ),
                Err(error) => app.push_error(error),
            }
            UiAction::None
        }
        Some(AppCommand::DebugEnable) => {
            match runtime.set_prompt_display_enabled(true) {
                Ok(()) => app.push_note(tr(app.locale, MessageId::DebugDisplayEnabled)),
                Err(error) => app.push_error(error),
            }
            app.refresh_status(runtime);
            UiAction::None
        }
        Some(AppCommand::DebugDisable) => {
            match runtime.set_prompt_display_enabled(false) {
                Ok(()) => app.push_note(tr(app.locale, MessageId::DebugDisplayDisabled)),
                Err(error) => app.push_error(error),
            }
            app.refresh_status(runtime);
            UiAction::None
        }
        Some(AppCommand::DebugShow) => {
            app.transcript
                .push(TranscriptItem::Note(runtime.system_prompt()));
            UiAction::None
        }
        Some(AppCommand::Auth) => {
            app.overlay = Overlay::AuthMethod(AuthMethodState { selected: 0 });
            UiAction::None
        }
        Some(AppCommand::Debug) => {
            app.overlay = Overlay::DebugMenu(DebugMenuState { selected: 0 });
            UiAction::None
        }
        Some(AppCommand::Unknown(command)) => {
            app.push_error(format!(
                "{}: {command}",
                tr(app.locale, MessageId::UnknownCommand)
            ));
            UiAction::None
        }
        None => UiAction::RunPrompt(input),
    }
}

fn handle_paste(app: &mut FullscreenApp, value: &str) {
    if app.is_running {
        return;
    }

    match &mut app.overlay {
        Overlay::ApiKeyInput(state) => {
            state.value.push_str(value);
        }
        Overlay::AddModelForm(state) => {
            active_add_model_field_mut(state).push_str(value);
        }
        Overlay::CustomTheme(state) => {
            paste_theme_value(state, value);
        }
        Overlay::None | Overlay::SlashMenu { .. } => {
            app.composer.input.push_str(value);
            app.composer.history_index = None;
            app.sync_slash_menu();
        }
        _ => {}
    }
}

fn open_model_picker_overlay(app: &mut FullscreenApp, runtime: &AppRuntime) {
    let models = runtime.selectable_models();
    if models.is_empty() {
        app.push_note(tr(app.locale, MessageId::NoEnabledModelsAvailable));
    } else {
        let selected = models
            .iter()
            .position(|model| model.is_current)
            .unwrap_or(0);
        app.overlay = Overlay::ModelPicker(ModelPickerState { models, selected });
    }
}

fn open_auth_settings_overlay(app: &mut FullscreenApp, runtime: &AppRuntime) {
    let providers = auth_provider_items(&runtime.model_settings_items());
    if providers.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoConfiguredModelsAuthFirst));
        return;
    }
    app.overlay = Overlay::AuthSettings(AuthSettingsState {
        checked: vec![false; providers.len()],
        providers,
        selected: 0,
    });
}

fn open_model_settings_overlay(app: &mut FullscreenApp, runtime: &AppRuntime) {
    let models = enabled_provider_model_settings_items(&runtime.model_settings_items());
    if models.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoConfiguredModelsEnableProvider));
        return;
    }
    app.overlay = Overlay::ModelSettings(ModelSettingsState {
        checked: vec![false; models.len()],
        models,
        selected: 0,
    });
}

fn open_theme_picker_overlay(app: &mut FullscreenApp, runtime: &AppRuntime) {
    let theme = runtime.theme();
    let selected = THEME_PRESETS
        .iter()
        .position(|preset| preset.name == theme.name && preset.rgb == theme.rgb)
        .unwrap_or(THEME_PRESETS.len());
    app.overlay = Overlay::ThemePicker(ThemePickerState { selected });
    preview_theme_selection(app, runtime, selected);
}

fn open_language_picker_overlay(app: &mut FullscreenApp, runtime: &AppRuntime) {
    let locale = runtime.locale();
    let selected = LANGUAGE_OPTIONS
        .iter()
        .position(|option| option.locale == locale)
        .unwrap_or(0);
    app.overlay = Overlay::LanguagePicker(LanguagePickerState { selected });
}

fn open_session_picker_overlay(app: &mut FullscreenApp, runtime: &AppRuntime) {
    match runtime.list_sessions() {
        Ok(mut sessions) => {
            let current = runtime.session_id().to_string();
            sessions.retain(|session| session.id != current);
            app.overlay = Overlay::SessionPicker(SessionPickerState {
                sessions,
                selected: 0,
            });
        }
        Err(error) => app.push_error(error),
    }
}

fn open_subscription_provider_overlay(app: &mut FullscreenApp, runtime: &AppRuntime) {
    let providers = runtime.subscription_providers();
    if providers.is_empty() {
        app.overlay = Overlay::None;
        app.push_note(tr(app.locale, MessageId::NoSubscriptionProviders));
        return;
    }
    app.overlay = Overlay::SubscriptionProvider(SubscriptionProviderState {
        providers,
        selected: 0,
    });
}

fn open_api_key_provider_overlay(app: &mut FullscreenApp, runtime: &AppRuntime) {
    app.overlay = Overlay::ApiKeyProvider(ApiKeyProviderState {
        providers: runtime.auth_providers(),
        selected: 0,
    });
}

fn auth_provider_items(models: &[ModelSettingsItem]) -> Vec<AuthProviderItem> {
    let mut providers = Vec::<AuthProviderItem>::new();
    for model in models {
        if let Some(provider) = providers
            .iter_mut()
            .find(|provider| provider.provider == model.provider)
        {
            provider.model_indices.push(model.index);
            provider.is_enabled |= model.is_enabled;
            provider.has_built_in |= !model.is_custom;
        } else {
            providers.push(AuthProviderItem {
                provider: model.provider.clone(),
                model_indices: vec![model.index],
                is_enabled: model.is_enabled,
                has_built_in: !model.is_custom,
            });
        }
    }
    providers
}

fn enabled_provider_model_settings_items(models: &[ModelSettingsItem]) -> Vec<ModelSettingsItem> {
    models
        .iter()
        .filter(|model| {
            models
                .iter()
                .any(|candidate| candidate.provider == model.provider && candidate.is_enabled)
        })
        .cloned()
        .collect()
}

fn active_add_model_field_mut(state: &mut AddModelFormState) -> &mut String {
    match state.field {
        0 => &mut state.provider,
        1 => &mut state.model_id,
        2 => &mut state.base_url,
        _ => &mut state.api_key,
    }
}

fn active_theme_field_mut(state: &mut CustomThemeState) -> &mut String {
    match state.field {
        0 => &mut state.red,
        1 => &mut state.green,
        _ => &mut state.blue,
    }
}

fn paste_theme_value(state: &mut CustomThemeState, value: &str) {
    let components = value
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .take(3)
        .collect::<Vec<_>>();

    if components.len() >= 3 {
        state.red = components[0].chars().take(3).collect();
        state.green = components[1].chars().take(3).collect();
        state.blue = components[2].chars().take(3).collect();
        return;
    }

    let field = active_theme_field_mut(state);
    for ch in value.chars().filter(|ch| ch.is_ascii_digit()) {
        if field.len() >= 3 {
            break;
        }
        field.push(ch);
    }
}

fn parse_custom_theme(state: &CustomThemeState) -> Result<ThemeSettings, String> {
    let red = parse_rgb_component("red", &state.red)?;
    let green = parse_rgb_component("green", &state.green)?;
    let blue = parse_rgb_component("blue", &state.blue)?;
    Ok(ThemeSettings::custom(ThemeRgb::new(red, green, blue)))
}

fn parse_rgb_component(label: &str, value: &str) -> Result<u8, String> {
    value
        .trim()
        .parse::<u8>()
        .map_err(|_| format!("{label} must be 0-255"))
}

fn preview_theme_selection(app: &mut FullscreenApp, runtime: &AppRuntime, selected: usize) {
    let saved = runtime.theme();
    app.theme_preview = if let Some(preset) = THEME_PRESETS.get(selected).copied() {
        Some(ThemeSettings::preset(preset))
    } else if saved.name == "custom" {
        Some(saved)
    } else {
        None
    };
}

fn checked_or_focused_indices(checked: &[bool], selected: usize) -> Vec<usize> {
    let indices = checked
        .iter()
        .enumerate()
        .filter_map(|(index, checked)| checked.then_some(index))
        .collect::<Vec<_>>();
    if indices.is_empty() {
        vec![selected]
    } else {
        indices
    }
}

fn set_providers_enabled(
    runtime: &mut AppRuntime,
    providers: &[AuthProviderItem],
    targets: &[usize],
    enabled: bool,
) -> Result<String, String> {
    let mut enabled_indices = runtime
        .model_settings_items()
        .into_iter()
        .filter(|model| model.is_enabled)
        .map(|model| model.index)
        .collect::<Vec<_>>();

    if enabled {
        for target in targets {
            for model_index in &providers[*target].model_indices {
                if !enabled_indices.contains(model_index) {
                    enabled_indices.push(*model_index);
                }
            }
        }
    } else {
        for target in targets {
            for model_index in &providers[*target].model_indices {
                enabled_indices.retain(|index| index != model_index);
            }
        }
    }

    runtime.set_enabled_model_indices(&enabled_indices)?;
    Ok(format!(
        "{} provider(s) {}",
        targets.len(),
        if enabled { "enabled" } else { "disabled" }
    ))
}

fn remove_providers(
    runtime: &mut AppRuntime,
    providers: &[AuthProviderItem],
    targets: &[usize],
) -> Result<String, String> {
    let mut messages = Vec::new();
    for target in targets.iter().rev() {
        let provider = &providers[*target];
        if provider.has_built_in {
            if let Some(index) = provider.model_indices.first() {
                messages.push(runtime.delete_model(*index)?);
            }
        } else {
            for index in provider.model_indices.iter().rev() {
                messages.push(runtime.delete_model(*index)?);
            }
        }
    }
    if messages.is_empty() {
        Ok("no providers removed".to_string())
    } else {
        Ok(messages.join("\n"))
    }
}

fn set_model_items_enabled(
    runtime: &mut AppRuntime,
    models: &[ModelSettingsItem],
    targets: &[usize],
    enabled: bool,
) -> Result<String, String> {
    let mut enabled_indices = runtime
        .model_settings_items()
        .into_iter()
        .filter(|model| model.is_enabled)
        .map(|model| model.index)
        .collect::<Vec<_>>();

    if enabled {
        for target in targets {
            let model_index = models[*target].index;
            if !enabled_indices.contains(&model_index) {
                enabled_indices.push(model_index);
            }
        }
    } else {
        for target in targets {
            let model_index = models[*target].index;
            enabled_indices.retain(|index| *index != model_index);
        }
    }

    runtime.set_enabled_model_indices(&enabled_indices)?;
    Ok(format!(
        "{} model(s) {}",
        targets.len(),
        if enabled { "enabled" } else { "disabled" }
    ))
}

fn load_recent_messages(app: &mut FullscreenApp, runtime: &AppRuntime) {
    for message in runtime.recent_messages(20) {
        match message.role.as_str() {
            "user" => app.transcript.push(TranscriptItem::User(message.content)),
            "assistant" => app
                .transcript
                .push(TranscriptItem::Assistant(message.content)),
            _ => app.transcript.push(TranscriptItem::Note(message.content)),
        }
    }
}

fn render(frame: &mut Frame<'_>, app: &mut FullscreenApp) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(composer_height(area.width)),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, vertical[0], app);
    let show_sidebar = area.width >= SIDEBAR_MIN_WIDTH;
    let body_chunks = if show_sidebar {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(SIDEBAR_WIDTH)])
            .split(vertical[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(vertical[1])
    };

    render_transcript(frame, body_chunks[0], app);
    if show_sidebar {
        render_sidebar(frame, body_chunks[1], app);
    }
    render_composer(frame, vertical[2], app);
    render_footer(frame, vertical[3], app);
    render_overlay(frame, area, app);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &FullscreenApp) {
    let line = Line::from(vec![
        Span::styled(
            "Exgent",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("agent", Style::default().fg(Color::Magenta)),
        Span::raw("  "),
        Span::styled(app.model_label.clone(), Style::default().fg(Color::Gray)),
    ]);
    Paragraph::new(line).render(area, frame.buffer_mut());
}

fn render_transcript(frame: &mut Frame<'_>, area: Rect, app: &FullscreenApp) {
    let width = usize::from(area.width).max(1);
    let mut lines = Vec::new();
    for item in &app.transcript {
        let mut item_lines = item_lines(item, width);
        if !item_lines.is_empty() {
            lines.append(&mut item_lines);
            lines.push(Line::raw(""));
        }
    }
    if let Some(activity) = runtime_activity_text(app) {
        lines.extend(styled_wrapped_lines(
            "",
            &activity,
            width,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
        ));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            tr(app.locale, MessageId::EmptyTranscriptHint),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let start = lines.len().saturating_sub(usize::from(area.height));
    let visible = lines.into_iter().skip(start).collect::<Vec<_>>();
    Paragraph::new(Text::from(visible)).render(area, frame.buffer_mut());
}

fn render_sidebar(frame: &mut Frame<'_>, area: Rect, app: &FullscreenApp) {
    let inner_width = usize::from(area.width.saturating_sub(2)).max(1);
    let lines = vec![
        Line::from(Span::styled(
            tr(app.locale, MessageId::SidebarStatus),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarSession),
            truncate_plain(&app.session_id, inner_width)
        )),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarModel),
            truncate_plain(&app.model_label, inner_width)
        )),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarReasoning),
            if app.model_reasoning {
                tr(app.locale, MessageId::ReasoningHigh)
            } else {
                tr(app.locale, MessageId::ReasoningOff)
            }
        )),
        Line::raw(""),
        Line::from(Span::styled(
            tr(app.locale, MessageId::SidebarUsage),
            Style::default().fg(Color::Cyan),
        )),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarInput),
            format_tokens(app.usage.input)
        )),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarOutput),
            format_tokens(app.usage.output)
        )),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarCacheRead),
            format_tokens(app.usage.cache_read)
        )),
        Line::raw(format!(
            "{}: {}",
            tr(app.locale, MessageId::SidebarContext),
            format_context_usage(&app.usage, app.model_context_window)
        )),
        Line::raw(""),
        Line::from(Span::styled(
            tr(app.locale, MessageId::SidebarCommands),
            Style::default().fg(Color::Cyan),
        )),
        Line::raw("/model"),
        Line::raw("/auth"),
        Line::raw("/settings"),
        Line::raw("/compact"),
        Line::raw("/quit"),
    ];
    Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::LEFT)
                .border_set(symbols::border::ROUNDED)
                .border_style(Style::default().fg(theme_color(app))),
        )
        .wrap(Wrap { trim: false })
        .render(area, frame.buffer_mut());
}

fn render_composer(frame: &mut Frame<'_>, area: Rect, app: &FullscreenApp) {
    let block = Block::default()
        .title(tr(app.locale, MessageId::ComposerTitle))
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED)
        .border_style(Style::default().fg(theme_color(app)));
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    let prompt = "> ";
    let input_width = usize::from(inner.width).saturating_sub(UnicodeWidthStr::width(prompt));
    let visible_input = input_view(&app.composer.input, input_width);
    let line = Line::from(vec![
        Span::styled(prompt, Style::default().fg(theme_color(app))),
        Span::raw(visible_input.clone()),
    ]);
    Paragraph::new(line).render(inner, frame.buffer_mut());

    let cursor_x = UnicodeWidthStr::width(prompt)
        .saturating_add(UnicodeWidthStr::width(visible_input.as_str()))
        .min(usize::from(inner.width.saturating_sub(1))) as u16;
    frame.set_cursor_position(Position::new(inner.x.saturating_add(cursor_x), inner.y));
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &FullscreenApp) {
    let width = usize::from(area.width).max(1);
    let left = truncate_plain(&app.cwd, width / 2);
    let right = format_context_usage(&app.usage, app.model_context_window);
    let spacer = " ".repeat(width.saturating_sub(
        UnicodeWidthStr::width(left.as_str()) + UnicodeWidthStr::width(right.as_str()),
    ));
    Paragraph::new(Line::from(vec![
        Span::styled(left, Style::default().fg(Color::DarkGray)),
        Span::raw(spacer),
        Span::styled(right, Style::default().fg(Color::DarkGray)),
    ]))
    .render(area, frame.buffer_mut());
}

fn theme_color(app: &FullscreenApp) -> Color {
    let theme = app.theme_preview.as_ref().unwrap_or(&app.theme);
    Color::Rgb(theme.rgb.r, theme.rgb.g, theme.rgb.b)
}

fn dialog_block<'a>(title: &'a str, accent: Color) -> Block<'a> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED)
        .border_style(Style::default().fg(accent))
}

fn selected_style(accent: Color) -> Style {
    Style::default().fg(accent).add_modifier(Modifier::BOLD)
}

fn render_overlay(frame: &mut Frame<'_>, area: Rect, app: &FullscreenApp) {
    let accent = theme_color(app);
    match &app.overlay {
        Overlay::None => {}
        Overlay::SlashMenu { selected } => render_slash_menu(frame, area, app, *selected),
        Overlay::ModelPicker(picker) => {
            render_model_picker(frame, area, picker, app.locale, accent)
        }
        Overlay::SettingsMenu(state) => {
            render_settings_menu(frame, area, state, app.locale, accent)
        }
        Overlay::AuthSettings(state) => {
            render_auth_settings(frame, area, state, app.locale, accent)
        }
        Overlay::AuthAction(state) => render_action_menu(
            frame,
            area,
            tr(app.locale, MessageId::DialogProviderAction),
            &[
                tr(app.locale, MessageId::ActionEnable),
                tr(app.locale, MessageId::ActionDisable),
                tr(app.locale, MessageId::ActionRemove),
            ],
            state.selected,
            accent,
        ),
        Overlay::ModelSettings(state) => {
            render_model_settings(frame, area, state, app.locale, accent)
        }
        Overlay::ModelAction(state) => render_action_menu(
            frame,
            area,
            tr(app.locale, MessageId::DialogModelAction),
            &[
                tr(app.locale, MessageId::ActionEnable),
                tr(app.locale, MessageId::ActionDisable),
            ],
            state.selected,
            accent,
        ),
        Overlay::ThemePicker(state) => render_theme_picker(frame, area, app, state),
        Overlay::CustomTheme(state) => render_custom_theme_form(frame, area, app, state),
        Overlay::LanguagePicker(state) => render_language_picker(frame, area, app, state),
        Overlay::SessionPicker(state) => {
            render_session_picker(frame, area, state, app.locale, accent)
        }
        Overlay::DebugMenu(state) => render_debug_menu(frame, area, state, app.locale, accent),
        Overlay::DebugPrompt(state) => {
            render_debug_prompt_menu(frame, area, state, app.locale, accent)
        }
        Overlay::AuthMethod(state) => {
            render_auth_method_menu(frame, area, state, app.locale, accent)
        }
        Overlay::ApiKeyProvider(state) => {
            render_api_key_provider_picker(frame, area, state, app.locale, accent)
        }
        Overlay::ApiKeyInput(state) => render_api_key_input(frame, area, state, app.locale, accent),
        Overlay::AddModelForm(state) => {
            render_add_model_form(frame, area, state, app.locale, accent)
        }
        Overlay::SubscriptionProvider(state) => {
            render_subscription_provider_picker(frame, area, state, app.locale, accent)
        }
        Overlay::AuthProgress(state) => render_auth_progress(frame, area, state, accent),
    }
}

fn render_slash_menu(frame: &mut Frame<'_>, area: Rect, app: &FullscreenApp, selected: usize) {
    let suggestions = slash_suggestions(&app.composer.input);
    if suggestions.is_empty() {
        return;
    }
    let accent = theme_color(app);
    let width = cmp::min(area.width.saturating_sub(4), 72).max(20);
    let height = cmp::min(suggestions.len() as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = Rect::new(
        area.x.saturating_add(2),
        area.y
            .saturating_add(area.height.saturating_sub(height).saturating_sub(3)),
        width,
        height,
    );
    Clear.render(popup, frame.buffer_mut());
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = suggestions
        .iter()
        .enumerate()
        .take(usize::from(height.saturating_sub(2)))
        .map(|(index, command)| {
            let is_selected = index == selected.min(suggestions.len().saturating_sub(1));
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let label = format!(
                "{:<18} {}",
                command.command,
                tr(app.locale, command.description_id)
            );
            Line::from(Span::styled(truncate_plain(&label, inner_width), style))
        })
        .collect::<Vec<_>>();
    Paragraph::new(Text::from(lines))
        .block(dialog_block("/", accent))
        .render(popup, frame.buffer_mut());
}

fn render_model_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    picker: &ModelPickerState,
    locale: Locale,
    accent: Color,
) {
    let width = cmp::min(area.width.saturating_sub(4), 88).max(28);
    let height = cmp::min(
        picker.models.len() as u16 + 2,
        area.height.saturating_sub(4),
    )
    .max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let selected = picker.selected.min(picker.models.len().saturating_sub(1));
    let start = selected.saturating_sub(body_height.saturating_sub(1));
    let lines = picker
        .models
        .iter()
        .enumerate()
        .skip(start)
        .take(body_height)
        .map(|(index, model)| {
            let is_selected = index == selected;
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let current = if model.is_current { " *" } else { "" };
            let label = format!("{}/{}  {}{}", model.provider, model.id, model.name, current);
            Line::from(Span::styled(
                truncate_plain(&label, usize::from(width.saturating_sub(2))),
                style,
            ))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(tr(locale, MessageId::DialogModel), accent))
        .render(popup, frame.buffer_mut());
}

fn render_settings_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &SettingsMenuState,
    locale: Locale,
    accent: Color,
) {
    let labels = [
        tr(locale, MessageId::SettingsAuth),
        tr(locale, MessageId::SettingsModel),
        tr(locale, MessageId::SettingsTheme),
        tr(locale, MessageId::SettingsLanguage),
    ];
    render_action_menu(
        frame,
        area,
        tr(locale, MessageId::DialogSettings),
        &labels,
        state.selected,
        accent,
    );
}

fn render_auth_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AuthSettingsState,
    locale: Locale,
    accent: Color,
) {
    let width = cmp::min(area.width.saturating_sub(4), 72).max(30);
    let height = cmp::min(
        state.providers.len() as u16 + 2,
        area.height.saturating_sub(4),
    )
    .max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let selected = state.selected.min(state.providers.len().saturating_sub(1));
    let start = selected.saturating_sub(body_height.saturating_sub(1));
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = state
        .providers
        .iter()
        .enumerate()
        .skip(start)
        .take(body_height)
        .map(|(index, provider)| {
            let is_selected = index == selected;
            let checked = state.checked.get(index).copied().unwrap_or(false);
            let style = if is_selected {
                selected_style(accent)
            } else if provider.is_enabled {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let label = format!(
                "{} {} {}",
                if is_selected { ">" } else { " " },
                if checked { "[*]" } else { "[ ]" },
                provider.provider
            );
            Line::from(Span::styled(truncate_plain(&label, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(
            tr(locale, MessageId::DialogSettingsAuth),
            accent,
        ))
        .render(popup, frame.buffer_mut());
}

fn render_model_settings(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &ModelSettingsState,
    locale: Locale,
    accent: Color,
) {
    let (rows, selected_row) = model_settings_rows(state, accent);
    let width = cmp::min(area.width.saturating_sub(4), 88).max(32);
    let height = cmp::min(rows.len() as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let start = selected_row.saturating_sub(body_height.saturating_sub(1));
    let visible = rows
        .into_iter()
        .skip(start)
        .take(body_height)
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(visible))
        .block(dialog_block(
            tr(locale, MessageId::DialogSettingsModel),
            accent,
        ))
        .render(popup, frame.buffer_mut());
}

fn render_action_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    labels: &[&str],
    selected: usize,
    accent: Color,
) {
    let longest = labels.iter().map(|label| label.width()).max().unwrap_or(16);
    let width = cmp::min(area.width.saturating_sub(4), (longest + 8) as u16).max(24);
    let height = cmp::min(labels.len() as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let is_selected = index == selected.min(labels.len().saturating_sub(1));
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let text = format!("{} {label}", if is_selected { ">" } else { " " });
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();
    Paragraph::new(Text::from(lines))
        .block(dialog_block(title, accent))
        .render(popup, frame.buffer_mut());
}

fn render_theme_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &FullscreenApp,
    state: &ThemePickerState,
) {
    let item_count = THEME_PRESETS.len() + 1;
    let accent = theme_color(app);
    let width = cmp::min(area.width.saturating_sub(4), 72).max(36);
    let height = cmp::min(item_count as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let selected = state.selected.min(item_count.saturating_sub(1));
    let start = selected.saturating_sub(body_height.saturating_sub(1));
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = (0..item_count)
        .skip(start)
        .take(body_height)
        .map(|index| {
            let is_selected = index == selected;
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let label = if let Some(preset) = THEME_PRESETS.get(index) {
                let current = if app.theme.name == preset.name && app.theme.rgb == preset.rgb {
                    " *"
                } else {
                    ""
                };
                format!(
                    "{}  rgb({}, {}, {}){}",
                    preset.name, preset.rgb.r, preset.rgb.g, preset.rgb.b, current
                )
            } else {
                let current = if app.theme.name == "custom" { " *" } else { "" };
                format!("{}{}", tr(app.locale, MessageId::CustomRgb), current)
            };
            let text = format!("{} {label}", if is_selected { ">" } else { " " });
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(tr(app.locale, MessageId::DialogTheme), accent))
        .render(popup, frame.buffer_mut());
}

fn render_custom_theme_form(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &FullscreenApp,
    state: &CustomThemeState,
) {
    let accent = theme_color(app);
    let width = cmp::min(area.width.saturating_sub(4), 52).max(32);
    let height = cmp::min(6, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let inner_width = usize::from(width.saturating_sub(2));
    let fields = [
        (tr(app.locale, MessageId::FieldRed), state.red.as_str()),
        (tr(app.locale, MessageId::FieldGreen), state.green.as_str()),
        (tr(app.locale, MessageId::FieldBlue), state.blue.as_str()),
    ];
    let lines = fields
        .iter()
        .enumerate()
        .map(|(index, (label, value))| {
            let is_selected = index == state.field.min(2);
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let value = if value.is_empty() {
                tr(app.locale, MessageId::EmptyValue)
            } else {
                value
            };
            let text = format!(
                "{} {:<5} {}",
                if is_selected { ">" } else { " " },
                label,
                value
            );
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .chain([Line::from(Span::styled(
            tr(app.locale, MessageId::CustomRgbHint),
            Style::default().fg(Color::DarkGray),
        ))])
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(tr(app.locale, MessageId::CustomRgb), accent))
        .render(popup, frame.buffer_mut());
}

fn render_language_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &FullscreenApp,
    state: &LanguagePickerState,
) {
    let accent = theme_color(app);
    let width = cmp::min(area.width.saturating_sub(4), 52).max(28);
    let height = cmp::min(
        LANGUAGE_OPTIONS.len() as u16 + 2,
        area.height.saturating_sub(4),
    )
    .max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let selected = state.selected.min(LANGUAGE_OPTIONS.len().saturating_sub(1));
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = LANGUAGE_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let is_selected = index == selected;
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let current = if app.locale == option.locale {
                " *"
            } else {
                ""
            };
            let text = format!(
                "{} {}{}",
                if is_selected { ">" } else { " " },
                option.label,
                current
            );
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(
            tr(app.locale, MessageId::DialogLanguage),
            accent,
        ))
        .render(popup, frame.buffer_mut());
}

fn render_session_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &SessionPickerState,
    locale: Locale,
    accent: Color,
) {
    let item_count = state.sessions.len() + 1;
    let width = cmp::min(area.width.saturating_sub(4), 96).max(36);
    let height = cmp::min(item_count as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let selected = state.selected.min(item_count.saturating_sub(1));
    let start = selected.saturating_sub(body_height.saturating_sub(1));
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = (0..item_count)
        .skip(start)
        .take(body_height)
        .map(|index| {
            let is_selected = index == selected;
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let label = if index == 0 {
                tr(locale, MessageId::NewSession).to_string()
            } else {
                let session = &state.sessions[index - 1];
                let preview = session
                    .preview
                    .as_deref()
                    .map(|preview| format!("  {preview}"))
                    .unwrap_or_default();
                format!(
                    "{}  {}={}  {}{}",
                    session.id,
                    tr(locale, MessageId::LabelMessages),
                    session.message_count,
                    session.cwd,
                    preview
                )
            };
            let text = format!("{} {label}", if is_selected { ">" } else { " " });
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(tr(locale, MessageId::DialogSession), accent))
        .render(popup, frame.buffer_mut());
}

fn render_debug_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DebugMenuState,
    locale: Locale,
    accent: Color,
) {
    render_action_menu(
        frame,
        area,
        tr(locale, MessageId::DialogDebug),
        &[tr(locale, MessageId::DebugPrompt)],
        state.selected,
        accent,
    );
}

fn render_debug_prompt_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DebugPromptState,
    locale: Locale,
    accent: Color,
) {
    render_action_menu(
        frame,
        area,
        tr(locale, MessageId::DialogDebugPrompt),
        &[
            tr(locale, MessageId::ActionEnable),
            tr(locale, MessageId::ActionDisable),
        ],
        state.selected,
        accent,
    );
}

fn render_auth_method_menu(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AuthMethodState,
    locale: Locale,
    accent: Color,
) {
    render_action_menu(
        frame,
        area,
        tr(locale, MessageId::DialogAuthentication),
        &[
            tr(locale, MessageId::AuthSubscription),
            tr(locale, MessageId::AuthApiKey),
        ],
        state.selected,
        accent,
    );
}

fn render_api_key_provider_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &ApiKeyProviderState,
    locale: Locale,
    accent: Color,
) {
    let item_count = state.providers.len() + 1;
    let width = cmp::min(area.width.saturating_sub(4), 72).max(36);
    let height = cmp::min(item_count as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let selected = state.selected.min(item_count.saturating_sub(1));
    let start = selected.saturating_sub(body_height.saturating_sub(1));
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = (0..item_count)
        .skip(start)
        .take(body_height)
        .map(|index| {
            let is_selected = index == selected;
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let label = if index == state.providers.len() {
                tr(locale, MessageId::AddOpenAiCompatibleModel).to_string()
            } else {
                let provider = &state.providers[index];
                format!(
                    "{}  {}",
                    provider.provider,
                    auth_status_text(locale, provider.has_token)
                )
            };
            let text = format!("{} {label}", if is_selected { ">" } else { " " });
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(tr(locale, MessageId::DialogApiKey), accent))
        .render(popup, frame.buffer_mut());
}

fn render_api_key_input(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &ApiKeyInputState,
    locale: Locale,
    accent: Color,
) {
    let width = cmp::min(area.width.saturating_sub(4), 72).max(36);
    let height = cmp::min(7, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let inner_width = usize::from(width.saturating_sub(2));
    let token = if state.value.is_empty() {
        tr(locale, MessageId::BlankRemovesStoredToken).to_string()
    } else {
        mask_secret(&state.value)
    };
    let lines = [
        format!(
            "{}: {}",
            tr(locale, MessageId::FieldProvider),
            state.provider
        ),
        String::new(),
        format!("token: {token}"),
        String::new(),
        tr(locale, MessageId::EnterSavesEscapeCancels).to_string(),
    ]
    .into_iter()
    .map(|line| Line::raw(truncate_plain(&line, inner_width)))
    .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(
            tr(locale, MessageId::DialogProviderToken),
            accent,
        ))
        .render(popup, frame.buffer_mut());
}

fn render_add_model_form(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AddModelFormState,
    locale: Locale,
    accent: Color,
) {
    let width = cmp::min(area.width.saturating_sub(4), 82).max(40);
    let height = cmp::min(8, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let inner_width = usize::from(width.saturating_sub(2));
    let masked_api_key = mask_secret(&state.api_key);
    let fields = [
        (
            tr(locale, MessageId::FieldProvider),
            state.provider.as_str(),
        ),
        (tr(locale, MessageId::FieldModelId), state.model_id.as_str()),
        (tr(locale, MessageId::FieldBaseUrl), state.base_url.as_str()),
        (tr(locale, MessageId::FieldApiKey), masked_api_key.as_str()),
    ];
    let body_height = usize::from(height.saturating_sub(2));
    let start = state.field.saturating_sub(body_height.saturating_sub(1));
    let lines = fields
        .iter()
        .enumerate()
        .skip(start)
        .take(body_height)
        .map(|(index, (label, value))| {
            let is_selected = index == state.field.min(3);
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let value = if value.is_empty() {
                tr(locale, MessageId::EmptyValue)
            } else {
                value
            };
            let text = format!(
                "{} {:<9} {}",
                if is_selected { ">" } else { " " },
                label,
                value
            );
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(
            tr(locale, MessageId::DialogAddOpenAiModel),
            accent,
        ))
        .render(popup, frame.buffer_mut());
}

fn render_subscription_provider_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &SubscriptionProviderState,
    locale: Locale,
    accent: Color,
) {
    let width = cmp::min(area.width.saturating_sub(4), 72).max(36);
    let height = cmp::min(
        state.providers.len() as u16 + 2,
        area.height.saturating_sub(4),
    )
    .max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let body_height = usize::from(height.saturating_sub(2));
    let selected = state.selected.min(state.providers.len().saturating_sub(1));
    let start = selected.saturating_sub(body_height.saturating_sub(1));
    let inner_width = usize::from(width.saturating_sub(2));
    let lines = state
        .providers
        .iter()
        .enumerate()
        .skip(start)
        .take(body_height)
        .map(|(index, provider)| {
            let is_selected = index == selected;
            let style = if is_selected {
                selected_style(accent)
            } else {
                Style::default()
            };
            let label = format!(
                "{}  {}",
                provider.name,
                auth_status_text(locale, provider.has_subscription)
            );
            let text = format!("{} {label}", if is_selected { ">" } else { " " });
            Line::from(Span::styled(truncate_plain(&text, inner_width), style))
        })
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(
            tr(locale, MessageId::DialogSubscription),
            accent,
        ))
        .render(popup, frame.buffer_mut());
}

fn render_auth_progress(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AuthProgressState,
    accent: Color,
) {
    let width = cmp::min(area.width.saturating_sub(4), 96).max(40);
    let height = cmp::min(state.lines.len() as u16 + 2, area.height.saturating_sub(4)).max(3);
    let popup = centered_rect(area, width, height);
    Clear.render(popup, frame.buffer_mut());

    let inner_width = usize::from(width.saturating_sub(2));
    let body_height = usize::from(height.saturating_sub(2));
    let lines = state
        .lines
        .iter()
        .take(body_height)
        .map(|line| Line::raw(truncate_plain(line, inner_width)))
        .collect::<Vec<_>>();

    Paragraph::new(Text::from(lines))
        .block(dialog_block(&state.title, accent))
        .wrap(Wrap { trim: false })
        .render(popup, frame.buffer_mut());
}

fn model_settings_rows(state: &ModelSettingsState, accent: Color) -> (Vec<Line<'static>>, usize) {
    let mut rows = Vec::new();
    let mut selected_row = 0usize;
    let mut last_provider = "";
    let selected = state.selected.min(state.models.len().saturating_sub(1));

    for (index, model) in state.models.iter().enumerate() {
        if model.provider != last_provider {
            rows.push(Line::from(Span::styled(
                model.provider.clone(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )));
            last_provider = &model.provider;
        }

        let is_selected = index == selected;
        if is_selected {
            selected_row = rows.len();
        }
        let checked = state.checked.get(index).copied().unwrap_or(false);
        let style = if is_selected {
            selected_style(accent)
        } else if model.is_enabled {
            Style::default()
        } else {
            Style::default().fg(Color::DarkGray)
        };
        rows.push(Line::from(Span::styled(
            format!(
                "{} {} {}",
                if is_selected { ">" } else { " " },
                if checked { "[*]" } else { "[ ]" },
                model.id
            ),
            style,
        )));
    }

    (rows, selected_row)
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
    let y = area
        .y
        .saturating_add(area.height.saturating_sub(height) / 2);
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

fn item_lines(item: &TranscriptItem, width: usize) -> Vec<Line<'static>> {
    match item {
        TranscriptItem::Welcome(text) => {
            styled_wrapped_lines("", text, width, Style::default().fg(Color::Gray))
        }
        TranscriptItem::User(text) => styled_wrapped_lines(
            "> ",
            text,
            width,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        TranscriptItem::Assistant(text) => {
            if text.is_empty() {
                Vec::new()
            } else {
                styled_wrapped_lines("", text, width, Style::default().fg(Color::White))
            }
        }
        TranscriptItem::Reasoning(text) => {
            styled_wrapped_lines("thinking ", text, width, Style::default().fg(Color::Yellow))
        }
        TranscriptItem::Tool(text) => {
            styled_wrapped_lines("tool ", text, width, Style::default().fg(Color::Blue))
        }
        TranscriptItem::Note(text) => {
            styled_wrapped_lines("note ", text, width, Style::default().fg(Color::Gray))
        }
        TranscriptItem::Error(text) => {
            styled_wrapped_lines("error ", text, width, Style::default().fg(Color::Red))
        }
    }
}

fn runtime_activity_text(app: &FullscreenApp) -> Option<String> {
    if !app.is_running {
        return None;
    }
    match app.runtime_activity.as_ref()? {
        RuntimeActivity::Thinking => Some(tr(app.locale, MessageId::StatusThinking).to_string()),
        RuntimeActivity::Tool(name) => {
            Some(tr(app.locale, MessageId::RuntimeRunningTool).replace("{tool}", name))
        }
    }
}

fn styled_wrapped_lines(
    prefix: &str,
    text: &str,
    width: usize,
    style: Style,
) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for (paragraph_index, paragraph) in text.split('\n').enumerate() {
        let mut wrapped = wrap_plain(paragraph, width.saturating_sub(prefix.width()).max(1));
        if wrapped.is_empty() {
            wrapped.push(String::new());
        }
        for (index, content) in wrapped.into_iter().enumerate() {
            let marker = if paragraph_index == 0 && index == 0 {
                prefix.to_string()
            } else {
                " ".repeat(prefix.width())
            };
            lines.push(Line::from(vec![
                Span::styled(marker, style),
                Span::styled(content, style),
            ]));
        }
    }
    lines
}

fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0usize;

    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if line_width > 0 && line_width + ch_width > width {
            lines.push(line);
            line = String::new();
            line_width = 0;
        }
        line.push(ch);
        line_width += ch_width;
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

fn slash_suggestions(input: &str) -> Vec<&'static crate::commands::CommandHelp> {
    if !input.starts_with('/') {
        return Vec::new();
    }
    COMMAND_HELP
        .iter()
        .filter(|help| help.command.starts_with(input))
        .filter(|help| !help.command.trim_start_matches('/').contains(' '))
        .collect()
}

fn composer_height(width: u16) -> u16 {
    if width < 50 {
        4
    } else {
        3
    }
}

fn input_view(input: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(input) <= max_width {
        return input.to_string();
    }
    if max_width <= 1 {
        return ".".to_string();
    }
    format!("…{}", suffix_to_width(input, max_width - 1))
}

fn suffix_to_width(text: &str, max_width: usize) -> String {
    let mut chars = Vec::new();
    let mut width = 0usize;
    for ch in text.chars().rev() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        chars.push(ch);
        width += ch_width;
    }
    chars.into_iter().rev().collect()
}

fn truncate_plain(text: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    let mut result = String::new();
    let mut width = 0usize;
    let target = max_width - 3;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > target {
            break;
        }
        result.push(ch);
        width += ch_width;
    }
    result.push_str("...");
    result
}

fn auth_status_text(locale: Locale, is_configured: bool) -> &'static str {
    if is_configured {
        tr(locale, MessageId::AuthConfigured)
    } else {
        tr(locale, MessageId::AuthMissing)
    }
}

fn mask_secret(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    "*".repeat(value.chars().count().clamp(1, 40))
}

fn format_context_usage(totals: &UsageTotals, context_window: Option<u64>) -> String {
    let Some(context_window) = context_window else {
        return "?/? (auto)".to_string();
    };
    if context_window == 0 {
        return "?/? (auto)".to_string();
    }
    let percent = totals.context_tokens() as f64 * 100.0 / context_window as f64;
    format!("{:.1}%/{} (auto)", percent, format_tokens(context_window))
}

fn format_tokens(count: u64) -> String {
    if count < 1_000 {
        return count.to_string();
    }
    if count < 10_000 {
        return format!("{:.1}k", count as f64 / 1_000.0);
    }
    if count < 1_000_000 {
        return format!("{}k", count / 1_000);
    }
    if count < 10_000_000 {
        return format!("{:.1}M", count as f64 / 1_000_000.0);
    }
    format!("{}M", count / 1_000_000)
}
