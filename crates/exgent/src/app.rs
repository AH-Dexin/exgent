use std::path::Path;

use exgent_ai::{ChatMessage, DynamicProvider, Model, TokenUsage};
use exgent_core::{Agent, AgentEvent};

use crate::{
    auth::OAuthCredential,
    cli::CliOptions,
    localization::Locale,
    model_service::{no_model_configured_message, ModelService},
    session::SessionInfo,
    session_service::SessionService,
    settings::ThemeSettings,
    system_prompt::build_system_prompt,
    tools::ToolRegistry,
};

pub use crate::model_service::{
    AddedModelInfo, AuthProviderInfo, ModelMenuItem, ModelSettingsItem, SubscriptionProviderInfo,
};
pub use crate::session_service::MessagePreview;

pub struct AppRuntime {
    agent: Option<Agent<DynamicProvider>>,
    model_service: ModelService,
    session_service: SessionService,
    tools: ToolRegistry,
    usage_totals: UsageTotals,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageTotals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelStatus {
    pub provider: String,
    pub id: String,
    pub reasoning: bool,
    pub context_window: Option<u64>,
}

impl AppRuntime {
    pub fn new(options: CliOptions) -> Result<Self, String> {
        let model_service = ModelService::load(&options)?;
        let agent = model_service
            .current_model_with_provider()
            .ok()
            .map(|(model, provider)| Agent::new(model, provider));
        let session_service = SessionService::create(&options)?;

        let usage_totals = UsageTotals::from_usage(
            &session_service.usage_totals(),
            model_service.current_model(),
        );

        Ok(Self {
            agent,
            model_service,
            session_service,
            tools: ToolRegistry::default_builtin(),
            usage_totals,
        })
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

    pub fn add_openai_compatible_model(
        &mut self,
        provider: &str,
        model_id: &str,
        base_url: &str,
        api_key: &str,
    ) -> Result<AddedModelInfo, String> {
        let info = self
            .model_service
            .add_openai_compatible_model(provider, model_id, base_url, api_key)?;
        self.refresh_current_model()?;
        Ok(info)
    }

    fn refresh_current_model(&mut self) -> Result<(), String> {
        self.agent = Some({
            let (model, provider) = self.model_service.current_model_with_provider()?;
            Agent::new(model, provider)
        });
        Ok(())
    }

    pub fn model_label(&self) -> String {
        self.model_service.model_label()
    }

    pub fn model_status(&self) -> Option<ModelStatus> {
        self.model_service.current_model().map(model_status)
    }

    pub fn usage_totals(&self) -> &UsageTotals {
        &self.usage_totals
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
        self.agent = self
            .model_service
            .current_model_with_provider()
            .ok()
            .map(|(model, provider)| Agent::new(model, provider));
        Ok(())
    }

    pub fn delete_model(&mut self, index: usize) -> Result<String, String> {
        let deleted = self.model_service.delete_model(index)?;
        self.agent = self
            .model_service
            .current_model_with_provider()
            .ok()
            .map(|(model, provider)| Agent::new(model, provider));
        Ok(deleted)
    }

    pub fn select_model(&mut self, index: usize) -> Result<(), String> {
        self.model_service.select_model(index)?;
        self.refresh_current_model()
    }

    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.names()
    }

    pub fn system_prompt(&self) -> String {
        build_system_prompt(
            &self.tool_names(),
            self.selected_model_label().as_deref(),
            "TUI",
        )
    }

    fn selected_model_label(&self) -> Option<String> {
        self.agent
            .as_ref()
            .map(|agent| format!("{}/{}", agent.model().provider, agent.model().id))
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

    pub fn session_message_count(&self) -> usize {
        self.session_service.message_count()
    }

    pub fn session_id(&self) -> &str {
        self.session_service.id()
    }

    pub fn session_path(&self) -> String {
        self.session_service.path()
    }

    pub fn recent_messages(&self, limit: usize) -> Vec<MessagePreview> {
        self.session_service.recent_messages(limit)
    }

    pub fn compact_context(&mut self) -> Result<usize, String> {
        self.session_service.compact_context()
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, String> {
        self.session_service.list_sessions()
    }

    pub fn start_new_session(&mut self) -> Result<(), String> {
        self.session_service.start_new_session()?;
        self.refresh_usage_totals();
        Ok(())
    }

    pub fn open_session(&mut self, path: &Path) -> Result<(), String> {
        self.session_service.open_session(path)?;
        self.refresh_usage_totals();
        Ok(())
    }

    pub fn run_prompt(&mut self, prompt: &str) -> Result<Vec<AgentEvent>, String> {
        let mut events = Vec::new();
        self.run_prompt_streaming(prompt, &mut |event| events.push(event))?;
        Ok(events)
    }

    pub fn run_prompt_streaming<F>(&mut self, prompt: &str, emit: &mut F) -> Result<(), String>
    where
        F: FnMut(AgentEvent),
    {
        let Some(agent) = self.agent.as_ref() else {
            let message = no_model_configured_message().to_string();
            emit(AgentEvent::MessageStart {
                role: "assistant".to_string(),
            });
            emit(AgentEvent::MessageDelta {
                delta: message.clone(),
            });
            emit(AgentEvent::MessageEnd { content: message });
            return Ok(());
        };

        let mut messages = vec![ChatMessage::system(self.system_prompt())];
        messages.extend(self.session_service.chat_messages());
        messages.push(ChatMessage::user(prompt));

        let mut assistant_messages = Vec::new();
        let mut errors = Vec::new();
        let mut turn_usage = UsageTotals::default();
        agent.run_messages_with_tools_streaming(messages, &self.tools, &mut |event| {
            match &event {
                AgentEvent::MessageEnd { content } => {
                    assistant_messages.push(content.clone());
                }
                AgentEvent::Usage { usage } => {
                    turn_usage.add_usage(usage, agent.model());
                }
                AgentEvent::Error { message } => {
                    errors.push(message.clone());
                }
                _ => {}
            }
            emit(event);
        });

        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }

        self.session_service.append_user(prompt)?;
        let last_assistant_index = assistant_messages.len().saturating_sub(1);
        let usage = turn_usage.to_usage();
        for (index, content) in assistant_messages.into_iter().enumerate() {
            if index == last_assistant_index {
                self.session_service
                    .append_assistant_with_usage(content, usage.clone())?;
            } else {
                self.session_service.append_assistant(content)?;
            }
        }
        self.usage_totals.add_totals(&turn_usage);

        Ok(())
    }

    fn refresh_usage_totals(&mut self) {
        self.usage_totals = UsageTotals::from_usage(
            &self.session_service.usage_totals(),
            self.model_service.current_model(),
        );
    }
}

impl UsageTotals {
    fn from_usage(usage: &TokenUsage, model: Option<&Model>) -> Self {
        let mut totals = Self::default();
        if let Some(model) = model {
            totals.add_usage(usage, model);
        } else {
            totals.input = usage.input;
            totals.output = usage.output;
            totals.cache_read = usage.cache_read;
            totals.cache_write = usage.cache_write;
        }
        totals
    }

    fn add_usage(&mut self, usage: &TokenUsage, model: &Model) {
        self.input = self.input.saturating_add(usage.input);
        self.output = self.output.saturating_add(usage.output);
        self.cache_read = self.cache_read.saturating_add(usage.cache_read);
        self.cache_write = self.cache_write.saturating_add(usage.cache_write);

        if let Some(cost) = &model.cost {
            self.cost += (usage.input as f64 * cost.input
                + usage.output as f64 * cost.output
                + usage.cache_read as f64 * cost.cache_read
                + usage.cache_write as f64 * cost.cache_write)
                / 1_000_000.0;
        }
    }

    fn add_totals(&mut self, totals: &UsageTotals) {
        self.input = self.input.saturating_add(totals.input);
        self.output = self.output.saturating_add(totals.output);
        self.cache_read = self.cache_read.saturating_add(totals.cache_read);
        self.cache_write = self.cache_write.saturating_add(totals.cache_write);
        self.cost += totals.cost;
    }

    pub fn context_tokens(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    fn to_usage(&self) -> TokenUsage {
        TokenUsage {
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
        }
    }
}

fn model_status(model: &Model) -> ModelStatus {
    ModelStatus {
        provider: model.provider.clone(),
        id: model.id.clone(),
        reasoning: model.reasoning.unwrap_or(false),
        context_window: model.context_window,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_uses_session_history() {
        let dir = test_dir("prompt_uses_session_history");
        let _ = std::fs::remove_dir_all(&dir);
        configure_fake_model(&dir);

        let mut runtime = AppRuntime::new(CliOptions {
            config_path: Some(dir.display().to_string()),
        })
        .unwrap();

        runtime.run_prompt("hello").unwrap();
        let events = runtime.run_prompt("history count").unwrap();

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::MessageEnd { content } if content == "fake history count: 4"
        )));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompt_injects_runtime_system_prompt() {
        let dir = test_dir("prompt_injects_runtime_system_prompt");
        let _ = std::fs::remove_dir_all(&dir);
        configure_fake_model(&dir);

        let mut runtime = AppRuntime::new(CliOptions {
            config_path: Some(dir.display().to_string()),
        })
        .unwrap();

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
    fn compact_context_reduces_future_prompt_context() {
        let dir = test_dir("compact_context_reduces_future_prompt_context");
        let _ = std::fs::remove_dir_all(&dir);
        configure_fake_model(&dir);

        let mut runtime = AppRuntime::new(CliOptions {
            config_path: Some(dir.display().to_string()),
        })
        .unwrap();

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

        let mut runtime = AppRuntime::new(CliOptions {
            config_path: Some(dir.display().to_string()),
        })
        .unwrap();

        let error = runtime.run_prompt("will fail").unwrap_err();

        assert!(error.contains("unsupported-test-adapter"));
        assert_eq!(runtime.session_message_count(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unconfigured_model_returns_auth_prompt() {
        let dir = test_dir("unconfigured_model_returns_auth_prompt");
        let _ = std::fs::remove_dir_all(&dir);

        let mut runtime = AppRuntime::new(CliOptions {
            config_path: Some(dir.display().to_string()),
        })
        .unwrap();

        let events = runtime.run_prompt("hello").unwrap();

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

        let error = match AppRuntime::new(CliOptions {
            config_path: Some(dir.display().to_string()),
        }) {
            Ok(_) => panic!("startup should fail for invalid model config"),
            Err(error) => error,
        };

        assert!(error.contains("failed to load models"));
        assert!(!dir.join("sessions").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("exgent_app_{name}_{stamp}"))
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
}
