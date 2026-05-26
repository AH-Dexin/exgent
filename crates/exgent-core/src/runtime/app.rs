use std::path::{Path, PathBuf};

use exgent_ai::{DynamicProvider, ImageContent, Model};

use crate::{
    agent::{Agent, AgentSession, AgentSessionEvent, SharedAgentHooks},
    auth::OAuthCredential,
    config::{AgentLoopConfig, RuntimeOptions},
    localization::Locale,
    model_service::ModelService,
    models::CompatibleModelKind,
    session::SessionInfo,
    settings::ThemeSettings,
};

#[cfg(test)]
use crate::agent::AgentEvent;
pub use crate::agent::UsageTotals;
pub use crate::model_service::{
    AddedModelInfo, AuthProviderInfo, ModelMenuItem, ModelSettingsItem, SubscriptionProviderInfo,
};
pub use crate::session::MessagePreview;

pub(crate) struct AppRuntime {
    model_service: ModelService,
    agent_session: AgentSession,
    project_dir: PathBuf,
    agent_config: AgentLoopConfig,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelStatus {
    pub provider: String,
    pub id: String,
    pub reasoning: bool,
    pub context_window: Option<u64>,
}

impl AppRuntime {
    pub(crate) fn new(options: RuntimeOptions) -> Result<Self, String> {
        Self::new_in_project_dir(options, current_working_directory())
    }

    pub(crate) fn new_in_project_dir(
        options: RuntimeOptions,
        project_dir: PathBuf,
    ) -> Result<Self, String> {
        let model_service = ModelService::load(&options)?;
        Self::from_new_session(options, model_service, project_dir)
    }

    pub(crate) fn open_existing_session(
        options: RuntimeOptions,
        path: &Path,
    ) -> Result<Self, String> {
        let model_service = ModelService::load(&options)?;
        Self::from_existing_session(options, model_service, path)
    }

    fn from_new_session(
        options: RuntimeOptions,
        model_service: ModelService,
        project_dir: PathBuf,
    ) -> Result<Self, String> {
        let agent_config = options.agent;
        let agent = build_agent(&model_service, agent_config).ok();
        let agent_session = AgentSession::create(
            &options,
            agent,
            model_service.current_model(),
            project_dir.clone(),
        )?;
        let project_dir = agent_session.project_dir().to_path_buf();

        Ok(Self {
            model_service,
            agent_session,
            project_dir,
            agent_config,
        })
    }

    fn from_existing_session(
        options: RuntimeOptions,
        model_service: ModelService,
        session_path: &Path,
    ) -> Result<Self, String> {
        let agent_config = options.agent;
        let agent = build_agent(&model_service, agent_config).ok();
        let agent_session = AgentSession::open_existing(
            &options,
            agent,
            model_service.current_model(),
            session_path,
        )?;
        let project_dir = agent_session.project_dir().to_path_buf();

        Ok(Self {
            model_service,
            agent_session,
            project_dir,
            agent_config,
        })
    }

    #[cfg(test)]
    fn new_with_provider_registry(
        options: RuntimeOptions,
        provider_registry: exgent_ai::ProviderRegistry,
    ) -> Result<Self, String> {
        let model_service = ModelService::load_with_provider_registry(&options, provider_registry)?;
        Self::from_new_session(options, model_service, current_working_directory())
    }

    #[cfg(test)]
    fn open_existing_session_with_provider_registry(
        options: RuntimeOptions,
        path: &Path,
        provider_registry: exgent_ai::ProviderRegistry,
    ) -> Result<Self, String> {
        let model_service = ModelService::load_with_provider_registry(&options, provider_registry)?;
        Self::from_existing_session(options, model_service, path)
    }

    pub fn reload(&mut self, options: &RuntimeOptions) -> Result<(), String> {
        let model_service = ModelService::load(options)?;
        self.model_service = model_service;
        self.agent_config = options.agent;
        self.agent_session
            .set_agent(build_agent(&self.model_service, self.agent_config).ok());
        self.agent_session
            .refresh_usage_totals(self.model_service.current_model());
        Ok(())
    }

    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    pub fn auth_path(&self) -> String {
        self.model_service.auth_path()
    }

    pub fn models_path(&self) -> String {
        self.model_service.models_path()
    }

    pub fn auth_providers(&self) -> Vec<AuthProviderInfo> {
        self.model_service.auth_providers()
    }

    pub fn subscription_providers(&self) -> Vec<SubscriptionProviderInfo> {
        self.model_service.subscription_providers()
    }

    pub fn set_auth_token(&mut self, provider: &str, token: &str) -> Result<(), String> {
        self.model_service.set_auth_token(provider, token)?;
        self.refresh_current_model()
    }

    pub fn set_oauth_credential(
        &mut self,
        provider: &str,
        credential: OAuthCredential,
    ) -> Result<(), String> {
        self.model_service
            .set_oauth_credential(provider, credential)?;
        self.refresh_current_model()
    }

    pub fn remove_auth_token(&mut self, provider: &str) -> Result<(), String> {
        self.model_service.remove_auth_token(provider)?;
        self.refresh_current_model()
    }

    pub fn add_compatible_model(
        &mut self,
        kind: CompatibleModelKind,
        provider: &str,
        model_id: &str,
        base_url: &str,
        api_key: &str,
    ) -> Result<AddedModelInfo, String> {
        let info = self
            .model_service
            .add_compatible_model(kind, provider, model_id, base_url, api_key)?;
        self.refresh_current_model()?;
        Ok(info)
    }

    fn refresh_current_model(&mut self) -> Result<(), String> {
        self.agent_session
            .set_agent(Some(build_agent(&self.model_service, self.agent_config)?));
        Ok(())
    }

    pub fn model_label(&self) -> String {
        self.model_service.model_label()
    }

    pub fn model_status(&self) -> Option<ModelStatus> {
        self.model_service.current_model().map(model_status)
    }

    pub fn usage_totals(&self) -> &UsageTotals {
        self.agent_session.usage_totals()
    }

    pub fn selectable_models(&self) -> Vec<ModelMenuItem> {
        self.model_service.selectable_models()
    }

    pub fn model_settings_items(&self) -> Vec<ModelSettingsItem> {
        self.model_service.model_settings_items()
    }

    pub fn set_enabled_model_indices(&mut self, enabled_indices: &[usize]) -> Result<(), String> {
        self.model_service
            .set_enabled_model_indices(enabled_indices)?;
        self.agent_session
            .set_agent(build_agent(&self.model_service, self.agent_config).ok());
        Ok(())
    }

    pub fn delete_model(&mut self, index: usize) -> Result<String, String> {
        let deleted = self.model_service.delete_model(index)?;
        self.agent_session
            .set_agent(build_agent(&self.model_service, self.agent_config).ok());
        Ok(deleted)
    }

    pub fn select_model(&mut self, index: usize) -> Result<(), String> {
        self.model_service.select_model(index)?;
        self.refresh_current_model()
    }

    pub fn tool_names(&self) -> Vec<&str> {
        self.agent_session.tool_names()
    }

    pub fn system_prompt(&self) -> String {
        self.agent_session.system_prompt()
    }

    pub fn prompt_display_enabled(&self) -> bool {
        self.model_service.prompt_display_enabled()
    }

    pub fn set_prompt_display_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.model_service.set_prompt_display_enabled(enabled)
    }

    pub fn locale(&self) -> Locale {
        self.model_service.locale()
    }

    pub fn locale_setting(&self) -> &str {
        self.model_service.locale_setting()
    }

    pub fn set_locale(&mut self, locale: &str) -> Result<(), String> {
        self.model_service.set_locale(locale)
    }

    pub fn theme(&self) -> ThemeSettings {
        self.model_service.theme()
    }

    pub fn set_theme(&mut self, theme: ThemeSettings) -> Result<(), String> {
        self.model_service.set_theme(theme)
    }

    pub fn keybindings(&self) -> crate::settings::KeyBindings {
        self.model_service.keybindings()
    }

    pub fn set_keybindings(
        &mut self,
        keybindings: crate::settings::KeyBindings,
    ) -> Result<(), String> {
        self.model_service.set_keybindings(keybindings)
    }

    pub fn session_message_count(&self) -> usize {
        self.agent_session.message_count()
    }

    pub fn session_id(&self) -> &str {
        self.agent_session.id()
    }

    pub fn session_path(&self) -> String {
        self.agent_session.path()
    }

    pub fn recent_messages(&self, limit: usize) -> Vec<MessagePreview> {
        self.agent_session.recent_messages(limit)
    }

    pub fn compact_context(&mut self) -> Result<usize, String> {
        self.agent_session.compact_context()
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, String> {
        self.agent_session.list_sessions()
    }

    #[cfg(test)]
    fn run_prompt(&mut self, prompt: &str) -> Result<Vec<AgentEvent>, String> {
        let mut events = Vec::new();
        self.run_prompt_events_cancellable(
            prompt,
            &crate::cancel::CancelToken::new(),
            &mut |event| {
                if let AgentSessionEvent::Agent(event) = event {
                    events.push(event);
                }
            },
        )?;
        Ok(events)
    }

    pub fn set_hooks(&mut self, hooks: SharedAgentHooks) {
        self.agent_session.set_hooks(hooks);
    }

    pub fn run_prompt_events_cancellable<F>(
        &mut self,
        prompt: &str,
        cancel: &crate::cancel::CancelToken,
        emit: &mut F,
    ) -> Result<(), String>
    where
        F: FnMut(AgentSessionEvent),
    {
        self.agent_session
            .run_prompt_events_cancellable(prompt, cancel, emit)
    }

    pub fn run_prompt_events_with_images_cancellable<F>(
        &mut self,
        prompt: &str,
        images: &[ImageContent],
        cancel: &crate::cancel::CancelToken,
        emit: &mut F,
    ) -> Result<(), String>
    where
        F: FnMut(AgentSessionEvent),
    {
        self.agent_session
            .run_prompt_events_with_images_cancellable(prompt, images, cancel, emit)
    }

    pub fn subscribe_to_agent_session(
        &mut self,
        callback: impl FnMut(&AgentSessionEvent) + Send + 'static,
    ) -> usize {
        self.agent_session.subscribe(callback)
    }

    pub fn unsubscribe_from_agent_session(&mut self, id: usize) -> bool {
        self.agent_session.unsubscribe(id)
    }
}

fn build_agent(
    model_service: &ModelService,
    config: crate::config::AgentLoopConfig,
) -> Result<Agent<DynamicProvider>, String> {
    let (model, provider) = model_service.current_model_with_provider()?;
    Ok(Agent::with_config(model, provider, config))
}

fn model_status(model: &Model) -> ModelStatus {
    ModelStatus {
        provider: model.provider.clone(),
        id: model.id.clone(),
        reasoning: model.reasoning.unwrap_or(false),
        context_window: model.context_window,
    }
}

fn current_working_directory() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug)]
    struct LoopWriteProvider;

    impl exgent_ai::ProviderAdapter for LoopWriteProvider {
        fn stream_events(
            &self,
            request: exgent_ai::ProviderRequest,
            emit: &mut dyn FnMut(exgent_ai::ProviderEvent),
        ) {
            emit(exgent_ai::ProviderEvent::Start);
            emit(exgent_ai::ProviderEvent::ToolCall(
                exgent_ai::ToolCall::new(format!("call_{}", request.messages.len()), "write")
                    .with_argument("path", "side-effect.txt")
                    .with_argument("content", "changed"),
            ));
            emit(exgent_ai::ProviderEvent::Done(Box::new(
                exgent_ai::AssistantMessage {
                    model: request.model,
                    content: String::new(),
                },
            )));
        }
    }

    #[test]
    fn prompt_uses_session_history() {
        let dir = test_dir("prompt_uses_session_history");
        let _ = std::fs::remove_dir_all(&dir);
        configure_fake_model(&dir);

        let mut runtime = new_fake_runtime(&dir);

        runtime.run_prompt("hello").unwrap();
        let events = runtime.run_prompt("history count").unwrap();

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::MessageEnd { content } if content == "fake history count: 4"
        )));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompt_persists_tool_result_in_session_history() {
        let dir = test_dir("prompt_persists_tool_result_in_session_history");
        let _ = std::fs::remove_dir_all(&dir);
        configure_fake_model(&dir);

        let mut runtime = new_fake_runtime(&dir);

        let events = runtime.run_prompt("tool read Cargo.toml").unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCallEnd { name, is_error: false, .. } if name == "read"
        )));

        let recent = runtime.recent_messages(4);
        assert_eq!(recent[0].role, "user");
        assert_eq!(recent[1].role, "assistant");
        assert_eq!(recent[1].content, "tool call: read");
        assert_eq!(recent[2].role, "tool");
        assert_eq!(recent[3].role, "assistant");

        let events = runtime.run_prompt("history count").unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::MessageEnd { content } if content == "fake history count: 6"
        )));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompt_persists_multiple_tool_calls_as_one_assistant_turn() {
        let dir = test_dir("prompt_persists_multiple_tool_calls_as_one_assistant_turn");
        let _ = std::fs::remove_dir_all(&dir);
        configure_fake_model(&dir);

        let mut runtime = new_fake_runtime(&dir);

        let events = runtime
            .run_prompt("tool read pair Cargo.toml Cargo.lock")
            .unwrap();
        let read_starts = events
            .iter()
            .filter(
                |event| matches!(event, AgentEvent::ToolCallStart { name, .. } if name == "read"),
            )
            .count();
        assert_eq!(read_starts, 2);

        let recent = runtime.recent_messages(5);
        assert_eq!(recent[0].role, "user");
        assert_eq!(recent[1].role, "assistant");
        assert_eq!(recent[1].content, "tool call: read, read");
        assert_eq!(recent[2].role, "tool");
        assert_eq!(recent[3].role, "tool");
        assert_eq!(recent[4].role, "assistant");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompt_emits_agent_session_events_to_subscribers() {
        use std::sync::{Arc, Mutex};

        let dir = test_dir("prompt_emits_agent_session_events_to_subscribers");
        let _ = std::fs::remove_dir_all(&dir);
        configure_fake_model(&dir);

        let mut runtime = new_fake_runtime(&dir);
        let observed = Arc::new(Mutex::new(Vec::new()));
        let listener_observed = Arc::clone(&observed);
        runtime.subscribe_to_agent_session(move |event| {
            listener_observed.lock().unwrap().push(event.clone());
        });

        runtime.run_prompt("hello").unwrap();

        let observed = observed.lock().unwrap();
        assert!(observed.iter().any(|event| matches!(
            event,
            AgentSessionEvent::Agent(AgentEvent::MessageEnd { content })
                if content == "fake response: hello"
        )));
        assert!(observed
            .iter()
            .any(|event| matches!(event, AgentSessionEvent::TurnCommitted { .. })));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompt_injects_runtime_system_prompt() {
        let dir = test_dir("prompt_injects_runtime_system_prompt");
        let _ = std::fs::remove_dir_all(&dir);
        configure_fake_model(&dir);

        let mut runtime = new_fake_runtime(&dir);

        let events = runtime.run_prompt("system prompt").unwrap();

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::MessageEnd { content }
                if content.contains("date:")
                    && content.contains("Project dir for relative tool paths:")
                    && content.contains("Do not mention it unless asked")
                    && content.contains("Tools:")
        )));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opened_session_restores_project_dir_for_prompt_and_tools() {
        let dir = test_dir("opened_session_restores_project_dir_for_prompt_and_tools");
        let _ = std::fs::remove_dir_all(&dir);
        configure_fake_model(&dir);

        let project_dir = dir.join("project");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(project_dir.join("marker.txt"), "from session cwd").unwrap();

        let options = RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        };
        let session = crate::session::Session::create_default_with_cwd(
            options.config_path.as_deref(),
            &project_dir,
        )
        .unwrap();
        let session_path = session.path().to_path_buf();
        drop(session);

        let mut runtime = AppRuntime::open_existing_session_with_provider_registry(
            options,
            &session_path,
            exgent_ai::ProviderRegistry::builtin().with_dev_providers(),
        )
        .unwrap();

        assert_eq!(runtime.project_dir(), project_dir.as_path());
        assert!(runtime
            .system_prompt()
            .contains(&project_dir.display().to_string().replace('\\', "/")));

        let events = runtime.run_prompt("tool read marker.txt").unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::MessageEnd { content } if content.contains("from session cwd")
        )));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_context_reduces_future_prompt_context() {
        let dir = test_dir("compact_context_reduces_future_prompt_context");
        let _ = std::fs::remove_dir_all(&dir);
        configure_fake_model(&dir);

        let mut runtime = new_fake_runtime(&dir);

        runtime.run_prompt("hello").unwrap();
        let compacted_count = runtime.compact_context().unwrap();
        let events = runtime.run_prompt("history count").unwrap();

        assert_eq!(compacted_count, 2);
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::MessageEnd { content } if content == "fake history count: 3"
        )));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_context_emits_lifecycle_events_to_subscribers() {
        use std::sync::{Arc, Mutex};

        let dir = test_dir("compact_context_emits_lifecycle_events_to_subscribers");
        let _ = std::fs::remove_dir_all(&dir);
        configure_fake_model(&dir);

        let mut runtime = new_fake_runtime(&dir);
        runtime.run_prompt("hello").unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let listener_observed = Arc::clone(&observed);
        runtime.subscribe_to_agent_session(move |event| {
            listener_observed.lock().unwrap().push(event.clone());
        });

        runtime.compact_context().unwrap();

        let observed = observed.lock().unwrap();
        assert!(observed.iter().any(|event| matches!(
            event,
            AgentSessionEvent::CompactionStarted { message_count } if *message_count == 2
        )));
        assert!(observed.iter().any(|event| matches!(
            event,
            AgentSessionEvent::CompactionFinished { compacted_count } if *compacted_count == 2
        )));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_prompt_does_not_commit_session_turn() {
        let dir = test_dir("failed_prompt_does_not_commit_session_turn");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("models.json"),
            r#"{
              "models": [
                {
                  "provider": "test-provider",
                  "id": "test-model",
                  "adapter": "unsupported-test-adapter"
                }
              ]
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("settings.json"),
            r#"{"default_model":{"provider":"test-provider","id":"test-model","adapter":"unsupported-test-adapter"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("auth.json"),
            r#"{"credentials":{"test-provider":{"type":"api_key","key":"test-token"}}}"#,
        )
        .unwrap();

        let mut runtime = AppRuntime::new(RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        })
        .unwrap();

        let error = runtime.run_prompt("will fail").unwrap_err();

        assert!(error.contains("unsupported-test-adapter"));
        assert_eq!(runtime.session_message_count(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_prompt_after_tool_execution_preserves_session_facts() {
        let dir = test_dir("failed_prompt_after_tool_execution_preserves_session_facts");
        let _ = std::fs::remove_dir_all(&dir);
        let project_dir = dir.join("project");
        std::fs::create_dir_all(&project_dir).unwrap();
        configure_loop_write_model(&dir);

        let options = RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        };
        let mut registry = exgent_ai::ProviderRegistry::builtin();
        registry
            .register("loop-write", || {
                exgent_ai::DynamicProvider::new("loop-write", LoopWriteProvider)
            })
            .unwrap();
        let model_service = ModelService::load_with_provider_registry(&options, registry).unwrap();
        let mut runtime =
            AppRuntime::from_new_session(options, model_service, project_dir.clone()).unwrap();

        let error = runtime.run_prompt("write until max rounds").unwrap_err();

        assert!(error.contains("maximum tool rounds reached"));
        assert_eq!(
            std::fs::read_to_string(project_dir.join("side-effect.txt")).unwrap(),
            "changed"
        );
        assert!(runtime.session_message_count() > 0);
        assert!(runtime
            .recent_messages(20)
            .iter()
            .any(|message| message.role == "error"
                && message.content.contains("maximum tool rounds reached")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unconfigured_model_returns_auth_prompt() {
        let dir = test_dir("unconfigured_model_returns_auth_prompt");
        let _ = std::fs::remove_dir_all(&dir);

        let mut runtime = AppRuntime::new(RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        })
        .unwrap();

        let events = runtime.run_prompt("hello").unwrap();

        assert_eq!(events.first(), Some(&AgentEvent::AgentStart));
        assert_eq!(events.last(), Some(&AgentEvent::AgentEnd));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::MessageEnd { content }
                if content == "No model configured. Please enter /auth to configure a model."
        )));
        assert_eq!(runtime.session_message_count(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_config_error_does_not_create_session() {
        let dir = test_dir("startup_config_error_does_not_create_session");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("models.json"), "{").unwrap();

        let error = match AppRuntime::new(RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        }) {
            Ok(_) => panic!("startup should fail for invalid model config"),
            Err(error) => error,
        };

        assert!(error.contains("failed to load models"));
        assert!(!dir.join("sessions").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hooks_block_tool_calls() {
        use crate::agent::{AgentHooks, ToolExecutionResult};
        use exgent_ai::ToolCall;
        use std::sync::Arc;

        let dir = test_dir("hooks_block_tool_calls");
        let _ = std::fs::remove_dir_all(&dir);
        configure_fake_model(&dir);

        let mut runtime = new_fake_runtime(&dir);

        struct BlockReads;
        impl AgentHooks for BlockReads {
            fn before_tool_call(&self, call: &ToolCall) -> Option<ToolExecutionResult> {
                (call.name == "read")
                    .then(|| ToolExecutionResult::error("read blocked by user policy"))
            }
        }
        runtime.set_hooks(Arc::new(BlockReads));

        let events = runtime.run_prompt("tool read Cargo.toml").unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCallEnd { name, is_error: true, content, .. }
                if name == "read" && content == "read blocked by user policy"
        )));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("exgent_app_{name}_{stamp}"))
    }

    fn new_fake_runtime(dir: &std::path::Path) -> AppRuntime {
        AppRuntime::new_with_provider_registry(
            RuntimeOptions {
                config_path: Some(dir.display().to_string()),
                ..RuntimeOptions::default()
            },
            exgent_ai::ProviderRegistry::builtin().with_dev_providers(),
        )
        .unwrap()
    }

    fn configure_fake_model(dir: &std::path::Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("models.json"),
            r#"{
              "models": [
                {
                  "provider": "test-provider",
                  "id": "test-model",
                  "adapter": "fake"
                }
              ]
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("settings.json"),
            r#"{"default_model":{"provider":"test-provider","id":"test-model","adapter":"fake"}}"#,
        )
        .unwrap();
    }

    fn configure_loop_write_model(dir: &std::path::Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("models.json"),
            r#"{
              "models": [
                {
                  "provider": "loop-provider",
                  "id": "loop-model",
                  "adapter": "loop-write"
                }
              ]
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("settings.json"),
            r#"{"default_model":{"provider":"loop-provider","id":"loop-model","adapter":"loop-write"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("auth.json"),
            r#"{"credentials":{"loop-provider":{"type":"api_key","key":"test-token"}}}"#,
        )
        .unwrap();
    }
}
