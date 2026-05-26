use exgent_core::{
    AgentEvent, AppRuntimeHost, AuthProviderInfo, CompatibleModelKind, ImageContent, Locale,
    ModelMenuItem, ModelSettingsItem, SessionInfo, SubscriptionProviderInfo, ThemeSettings,
    UsageTotals,
};

use super::settings_actions::AuthProviderSettingsItem as AuthProviderItem;

type TuiRuntime = AppRuntimeHost;

#[derive(Clone, Debug)]
pub(super) struct TuiApp {
    pub(super) transcript: Vec<TranscriptItem>,
    pub(super) transcript_scroll: usize,
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
            transcript_scroll: 0,
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
            show_reasoning: true,
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
        self.show_reasoning = true;
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

    pub(super) fn push_system_prompt(&mut self, prompt: impl Into<String>) {
        self.transcript.push(TranscriptItem::Note(format!(
            "system prompt:\n{}",
            prompt.into()
        )));
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

    pub(super) fn push_user_with_images(&mut self, input: impl Into<String>, image_count: usize) {
        let mut input = input.into();
        if image_count > 0 {
            let note = if image_count == 1 {
                "[1 image]".to_string()
            } else {
                format!("[{image_count} images]")
            };
            if input.is_empty() {
                input = note;
            } else {
                input.push('\n');
                input.push_str(&note);
            }
        }
        self.transcript.push(TranscriptItem::User(input));
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
        if self.composer.images.is_empty()
            && !self.composer.is_multiline()
            && self.composer.input.starts_with('/')
        {
            self.overlay = Overlay::SlashMenu { selected: 0 };
        } else if matches!(self.overlay, Overlay::SlashMenu { .. }) {
            self.overlay = Overlay::None;
        }
    }

    pub(super) fn scroll_transcript_up(&mut self, lines: usize) {
        self.transcript_scroll = self.transcript_scroll.saturating_add(lines);
    }

    pub(super) fn scroll_transcript_down(&mut self, lines: usize) {
        self.transcript_scroll = self.transcript_scroll.saturating_sub(lines);
    }

    pub(super) fn scroll_transcript_to_top(&mut self) {
        self.transcript_scroll = usize::MAX;
    }

    pub(super) fn scroll_transcript_to_bottom(&mut self) {
        self.transcript_scroll = 0;
    }

    pub(super) fn clamp_transcript_scroll(&mut self, max_scroll: usize) {
        self.transcript_scroll = self.transcript_scroll.min(max_scroll);
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
    pub(super) images: Vec<ImageContent>,
    pub(super) history: Vec<String>,
    pub(super) history_index: Option<usize>,
    pub(super) draft: String,
    items: Vec<ComposerItem>,
    cursor: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum ComposerItem {
    Text(char),
    Image(ImageContent),
}

impl ComposerState {
    pub(super) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(super) fn cursor(&self) -> usize {
        self.cursor.min(self.items.len())
    }

    pub(super) fn items(&self) -> &[ComposerItem] {
        &self.items
    }

    pub(super) fn is_multiline(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item, ComposerItem::Text('\n')))
    }

    pub(super) fn clear(&mut self) {
        self.input.clear();
        self.images.clear();
        self.items.clear();
        self.cursor = 0;
    }

    pub(super) fn set_text(&mut self, text: impl AsRef<str>) {
        let text = text.as_ref();
        self.input = text.to_string();
        self.images.clear();
        self.items = text.chars().map(ComposerItem::Text).collect();
        self.cursor = self.items.len();
    }

    pub(super) fn insert_char(&mut self, value: char) {
        let index = self.cursor();
        self.items.insert(index, ComposerItem::Text(value));
        self.cursor = index + 1;
        self.sync_input();
    }

    pub(super) fn insert_str(&mut self, value: &str) {
        let value = value.replace("\r\n", "\n").replace('\r', "\n");
        let items = value.chars().map(ComposerItem::Text).collect::<Vec<_>>();
        if items.is_empty() {
            return;
        }
        let index = self.cursor();
        let len = items.len();
        self.items.splice(index..index, items);
        self.cursor = index + len;
        self.sync_input();
    }

    pub(super) fn insert_image(&mut self, image: ImageContent) {
        let index = self.cursor();
        self.items.insert(index, ComposerItem::Image(image));
        self.cursor = index + 1;
        self.sync_images();
    }

    pub(super) fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let index = self.cursor - 1;
        let removed = self.items.remove(index);
        self.cursor = index;
        match removed {
            ComposerItem::Text(_) => self.sync_input(),
            ComposerItem::Image(_) => self.sync_images(),
        }
        true
    }

    pub(super) fn move_cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub(super) fn move_cursor_right(&mut self) {
        self.cursor = self.cursor.saturating_add(1).min(self.items.len());
    }

    pub(super) fn move_cursor_to_start(&mut self) {
        self.cursor = 0;
    }

    pub(super) fn move_cursor_to_end(&mut self) {
        self.cursor = self.items.len();
    }

    pub(super) fn move_cursor_up(&mut self) {
        self.move_cursor_vertical(-1);
    }

    pub(super) fn move_cursor_down(&mut self) {
        self.move_cursor_vertical(1);
    }

    pub(super) fn take_images_and_clear(&mut self) -> Vec<ImageContent> {
        let items = std::mem::take(&mut self.items);
        let images = items
            .into_iter()
            .filter_map(|item| match item {
                ComposerItem::Image(image) => Some(image),
                ComposerItem::Text(_) => None,
            })
            .collect();
        self.input.clear();
        self.images.clear();
        self.cursor = 0;
        images
    }

    fn move_cursor_vertical(&mut self, delta: isize) {
        let lines = self.hard_line_bounds();
        if lines.len() <= 1 {
            return;
        }

        let cursor = self.cursor();
        let Some((line_index, line_start, _line_end)) = lines
            .iter()
            .enumerate()
            .find(|(_, (start, end))| cursor >= *start && cursor <= *end)
            .map(|(index, (start, end))| (index, *start, *end))
        else {
            return;
        };

        let target_index = if delta < 0 {
            line_index.saturating_sub(1)
        } else {
            (line_index + 1).min(lines.len() - 1)
        };
        if target_index == line_index {
            return;
        }

        let column = cursor.saturating_sub(line_start);
        let (target_start, target_end) = lines[target_index];
        self.cursor = target_start + column.min(target_end.saturating_sub(target_start));
    }

    fn hard_line_bounds(&self) -> Vec<(usize, usize)> {
        let mut lines = Vec::new();
        let mut start = 0usize;
        for (index, item) in self.items.iter().enumerate() {
            if matches!(item, ComposerItem::Text('\n')) {
                lines.push((start, index));
                start = index + 1;
            }
        }
        lines.push((start, self.items.len()));
        lines
    }

    fn sync_input(&mut self) {
        self.input.clear();
        for item in &self.items {
            if let ComposerItem::Text(ch) = item {
                self.input.push(*ch);
            }
        }
    }

    fn sync_images(&mut self) {
        self.images.clear();
        for item in &self.items {
            if let ComposerItem::Image(image) = item {
                self.images.push(image.clone());
            }
        }
    }
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
    pub(super) base_url: Option<String>,
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
    RunPrompt {
        prompt: String,
        images: Vec<ImageContent>,
    },
    RunSubscriptionAuth(SubscriptionProviderInfo),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(data: &str) -> ImageContent {
        ImageContent::new(data, "image/png")
    }

    #[test]
    fn composer_treats_images_as_single_cursor_items() {
        let mut composer = ComposerState::default();
        composer.insert_char('a');
        composer.insert_image(image("one"));
        composer.insert_char('b');

        assert_eq!(composer.input, "ab");
        assert_eq!(composer.images, vec![image("one")]);
        assert_eq!(composer.cursor(), 3);

        composer.move_cursor_left();
        assert_eq!(composer.cursor(), 2);
        composer.move_cursor_left();
        assert_eq!(composer.cursor(), 1);

        composer.insert_char('x');
        assert_eq!(composer.input, "axb");
        assert_eq!(composer.cursor(), 2);

        composer.move_cursor_right();
        assert_eq!(composer.cursor(), 3);
        assert!(composer.backspace());
        assert_eq!(composer.input, "axb");
        assert!(composer.images.is_empty());
    }

    #[test]
    fn composer_drains_images_in_visual_order() {
        let mut composer = ComposerState::default();
        composer.insert_image(image("one"));
        composer.insert_char('x');
        composer.insert_image(image("two"));

        assert_eq!(
            composer.take_images_and_clear(),
            vec![image("one"), image("two")]
        );
        assert!(composer.is_empty());
        assert!(composer.input.is_empty());
        assert!(composer.images.is_empty());
    }

    #[test]
    fn composer_preserves_multiline_paste_and_normalizes_crlf() {
        let mut composer = ComposerState::default();
        composer.insert_str("one\r\ntwo\rthree");

        assert_eq!(composer.input, "one\ntwo\nthree");
        assert!(composer.is_multiline());
        assert_eq!(composer.cursor(), "one\ntwo\nthree".chars().count());
    }

    #[test]
    fn composer_moves_vertically_across_pasted_lines() {
        let mut composer = ComposerState::default();
        composer.insert_str("one\ntwo\nthree");

        composer.move_cursor_up();
        assert_eq!(composer.cursor(), "one\ntwo".chars().count());
        composer.move_cursor_up();
        assert_eq!(composer.cursor(), "one".chars().count());
        composer.move_cursor_down();
        assert_eq!(composer.cursor(), "one\ntwo".chars().count());
    }

    #[test]
    fn transcript_scroll_state_moves_and_clamps() {
        let mut app = TuiApp {
            transcript: Vec::new(),
            transcript_scroll: 0,
            composer: ComposerState::default(),
            overlay: Overlay::None,
            model_label: String::new(),
            session_id: String::new(),
            cwd: String::new(),
            usage: UsageTotals::default(),
            model_context_window: None,
            model_reasoning: false,
            is_running: false,
            runtime_activity: None,
            show_reasoning: false,
            locale: Locale::En,
            theme: ThemeSettings::default(),
            theme_preview: None,
        };

        app.scroll_transcript_up(20);
        assert_eq!(app.transcript_scroll, 20);
        app.scroll_transcript_down(7);
        assert_eq!(app.transcript_scroll, 13);
        app.clamp_transcript_scroll(5);
        assert_eq!(app.transcript_scroll, 5);
        app.scroll_transcript_to_bottom();
        assert_eq!(app.transcript_scroll, 0);
    }
}
