use exgent_core::{
    AgentEvent, AppRuntimeHost, AuthProviderInfo, CompatibleModelKind, Locale, ModelMenuItem,
    ModelSettingsItem, SessionInfo, SubscriptionProviderInfo, ThemeSettings, UsageTotals,
};

use super::settings_actions::AuthProviderSettingsItem as AuthProviderItem;

type TuiRuntime = AppRuntimeHost;

#[derive(Clone, Debug)]
pub(super) struct TuiApp {
    pub(super) transcript: Vec<TranscriptItem>,
    pub(super) composer: ComposerState,
    pub(super) overlay: Overlay,
    pub(super) model_label: String,
    pub(super) session_id: String,
    pub(super) cwd: String,
    pub(super) usage: UsageTotals,
    pub(super) model_context_window: Option<u64>,
    pub(super) model_reasoning: bool,
    pub(super) is_running: bool,
    pub(super) runtime_activity: Option<RuntimeActivity>,
    pub(super) show_reasoning: bool,
    pub(super) locale: Locale,
    pub(super) theme: ThemeSettings,
    pub(super) theme_preview: Option<ThemeSettings>,
}

impl TuiApp {
    pub(super) fn new(runtime: &TuiRuntime) -> Self {
        let mut app = Self {
            transcript: Vec::new(),
            composer: ComposerState::default(),
            overlay: Overlay::None,
            model_label: String::new(),
            session_id: String::new(),
            cwd: project_dir_label(runtime),
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

    pub(super) fn refresh_status(&mut self, runtime: &TuiRuntime) {
        self.model_label = runtime.model_label();
        self.session_id = runtime.session_id().to_string();
        self.usage = runtime.usage_totals().clone();
        self.show_reasoning = runtime.prompt_display_enabled();
        self.locale = runtime.locale();
        self.theme = runtime.theme();
        self.cwd = project_dir_label(runtime);
        if let Some(model) = runtime.model_status() {
            self.model_context_window = model.context_window;
            self.model_reasoning = model.reasoning;
        } else {
            self.model_context_window = None;
            self.model_reasoning = false;
        }
    }

    pub(super) fn push_note(&mut self, note: impl Into<String>) {
        self.transcript.push(TranscriptItem::Note(note.into()));
    }

    pub(super) fn push_welcome(&mut self, message: impl Into<String>) {
        self.transcript
            .push(TranscriptItem::Welcome(message.into()));
    }

    pub(super) fn push_error(&mut self, error: impl Into<String>) {
        self.transcript.push(TranscriptItem::Error(error.into()));
    }

    pub(super) fn push_user(&mut self, input: impl Into<String>) {
        self.transcript.push(TranscriptItem::User(input.into()));
    }

    pub(super) fn start_assistant(&mut self) {
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

    pub(super) fn apply_agent_event(&mut self, event: AgentEvent) {
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
            AgentEvent::AssistantToolCalls { .. } => {}
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

    pub(super) fn sync_slash_menu(&mut self) {
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

fn project_dir_label(runtime: &TuiRuntime) -> String {
    runtime
        .project_dir()
        .display()
        .to_string()
        .replace('\\', "/")
}

#[derive(Clone, Debug, Default)]
pub(super) struct ComposerState {
    pub(super) input: String,
    pub(super) history: Vec<String>,
    pub(super) history_index: Option<usize>,
    pub(super) draft: String,
}

#[derive(Clone, Debug)]
pub(super) enum Overlay {
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
    CustomModelKind(CustomModelKindState),
    AddModelForm(AddModelFormState),
    SubscriptionProvider(SubscriptionProviderState),
    AuthProgress(AuthProgressState),
}

#[derive(Clone, Debug)]
pub(super) struct ModelPickerState {
    pub(super) models: Vec<ModelMenuItem>,
    pub(super) selected: usize,
}

#[derive(Clone, Debug)]
pub(super) struct SettingsMenuState {
    pub(super) selected: usize,
}

#[derive(Clone, Debug)]
pub(super) struct AuthSettingsState {
    pub(super) providers: Vec<AuthProviderItem>,
    pub(super) selected: usize,
    pub(super) checked: Vec<bool>,
}

#[derive(Clone, Debug)]
pub(super) struct AuthActionState {
    pub(super) providers: Vec<AuthProviderItem>,
    pub(super) targets: Vec<usize>,
    pub(super) selected: usize,
}

#[derive(Clone, Debug)]
pub(super) struct ModelSettingsState {
    pub(super) models: Vec<ModelSettingsItem>,
    pub(super) selected: usize,
    pub(super) checked: Vec<bool>,
}

#[derive(Clone, Debug)]
pub(super) struct ModelActionState {
    pub(super) models: Vec<ModelSettingsItem>,
    pub(super) targets: Vec<usize>,
    pub(super) selected: usize,
}

#[derive(Clone, Debug)]
pub(super) struct ThemePickerState {
    pub(super) selected: usize,
}

#[derive(Clone, Debug, Default)]
pub(super) struct CustomThemeState {
    pub(super) field: usize,
    pub(super) red: String,
    pub(super) green: String,
    pub(super) blue: String,
}

#[derive(Clone, Debug)]
pub(super) struct LanguagePickerState {
    pub(super) selected: usize,
}

#[derive(Clone, Debug)]
pub(super) struct SessionPickerState {
    pub(super) sessions: Vec<SessionInfo>,
    pub(super) selected: usize,
}

#[derive(Clone, Debug)]
pub(super) struct DebugMenuState {
    pub(super) selected: usize,
}

#[derive(Clone, Debug)]
pub(super) struct DebugPromptState {
    pub(super) selected: usize,
}

#[derive(Clone, Debug)]
pub(super) struct AuthMethodState {
    pub(super) selected: usize,
}

#[derive(Clone, Debug)]
pub(super) struct ApiKeyProviderState {
    pub(super) providers: Vec<AuthProviderInfo>,
    pub(super) selected: usize,
}

#[derive(Clone, Debug)]
pub(super) struct ApiKeyInputState {
    pub(super) provider: String,
    pub(super) value: String,
}

#[derive(Clone, Debug)]
pub(super) struct CustomModelKindState {
    pub(super) selected: usize,
}

#[derive(Clone, Debug)]
pub(super) struct AddModelFormState {
    pub(super) kind: CompatibleModelKind,
    pub(super) field: usize,
    pub(super) provider: String,
    pub(super) model_id: String,
    pub(super) base_url: String,
    pub(super) api_key: String,
}

impl Default for AddModelFormState {
    fn default() -> Self {
        Self::for_kind(CompatibleModelKind::OpenAi)
    }
}

impl AddModelFormState {
    pub(super) fn for_kind(kind: CompatibleModelKind) -> Self {
        Self {
            kind,
            field: 0,
            provider: String::new(),
            model_id: String::new(),
            base_url: kind.default_base_url().to_string(),
            api_key: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SubscriptionProviderState {
    pub(super) providers: Vec<SubscriptionProviderInfo>,
    pub(super) selected: usize,
}

#[derive(Clone, Debug)]
pub(super) struct AuthProgressState {
    pub(super) title: String,
    pub(super) lines: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) enum TranscriptItem {
    Welcome(String),
    User(String),
    Assistant(String),
    Reasoning(String),
    Tool(String),
    Note(String),
    Error(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RuntimeActivity {
    Thinking,
    Tool(String),
}

pub(super) enum UiAction {
    None,
    Quit,
    RunPrompt(String),
    RunSubscriptionAuth(SubscriptionProviderInfo),
}
