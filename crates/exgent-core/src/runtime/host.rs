use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use crate::{
    agent::{AgentSessionEvent, SharedAgentHooks},
    auth::OAuthCredential,
    cancel::CancelToken,
    config::RuntimeOptions,
    localization::Locale,
    models::CompatibleModelKind,
    session::SessionInfo,
    settings::ThemeSettings,
    ImageContent,
};

use super::app::{
    AddedModelInfo, AppRuntime, AuthProviderInfo, MessagePreview, ModelMenuItem, ModelSettingsItem,
    ModelStatus, SubscriptionProviderInfo, UsageTotals,
};

pub struct AppRuntimeHost {
    options: RuntimeOptions,
    runtime: AppRuntime,
    listeners: Vec<RuntimeHostListener>,
    next_listener_id: usize,
}

type RuntimeHostCallback = Arc<Mutex<Box<dyn FnMut(&AgentSessionEvent) + Send>>>;

struct RuntimeHostListener {
    id: usize,
    runtime_listener_id: usize,
    callback: RuntimeHostCallback,
}

impl AppRuntimeHost {
    pub fn new(options: RuntimeOptions) -> Result<Self, String> {
        Ok(Self {
            runtime: AppRuntime::new(options.clone())?,
            options,
            listeners: Vec::new(),
            next_listener_id: 0,
        })
    }

    pub fn options(&self) -> &RuntimeOptions {
        &self.options
    }

    pub fn reload(&mut self) -> Result<(), String> {
        self.runtime.reload(&self.options)
    }

    pub fn start_new_session(&mut self) -> Result<(), String> {
        let project_dir = self.runtime.project_dir().to_path_buf();
        self.runtime = AppRuntime::new_in_project_dir(self.options.clone(), project_dir)?;
        self.rebind_listeners();
        Ok(())
    }

    pub fn open_session(&mut self, path: &Path) -> Result<(), String> {
        self.runtime = AppRuntime::open_existing_session(self.options.clone(), path)?;
        self.rebind_listeners();
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, String> {
        self.runtime.list_sessions()
    }

    pub fn auth_path(&self) -> String {
        self.runtime.auth_path()
    }

    pub fn models_path(&self) -> String {
        self.runtime.models_path()
    }

    pub fn auth_providers(&self) -> Vec<AuthProviderInfo> {
        self.runtime.auth_providers()
    }

    pub fn subscription_providers(&self) -> Vec<SubscriptionProviderInfo> {
        self.runtime.subscription_providers()
    }

    pub fn set_auth_token(&mut self, provider: &str, token: &str) -> Result<(), String> {
        self.runtime.set_auth_token(provider, token)
    }

    pub fn set_oauth_credential(
        &mut self,
        provider: &str,
        credential: OAuthCredential,
    ) -> Result<(), String> {
        self.runtime.set_oauth_credential(provider, credential)
    }

    pub fn remove_auth_token(&mut self, provider: &str) -> Result<(), String> {
        self.runtime.remove_auth_token(provider)
    }

    pub fn add_compatible_model(
        &mut self,
        kind: CompatibleModelKind,
        provider: &str,
        model_id: &str,
        base_url: &str,
        api_key: &str,
    ) -> Result<AddedModelInfo, String> {
        self.runtime
            .add_compatible_model(kind, provider, model_id, base_url, api_key)
    }

    pub fn model_label(&self) -> String {
        self.runtime.model_label()
    }

    pub fn model_status(&self) -> Option<ModelStatus> {
        self.runtime.model_status()
    }

    pub fn usage_totals(&self) -> &UsageTotals {
        self.runtime.usage_totals()
    }

    pub fn selectable_models(&self) -> Vec<ModelMenuItem> {
        self.runtime.selectable_models()
    }

    pub fn model_settings_items(&self) -> Vec<ModelSettingsItem> {
        self.runtime.model_settings_items()
    }

    pub fn set_enabled_model_indices(&mut self, enabled_indices: &[usize]) -> Result<(), String> {
        self.runtime.set_enabled_model_indices(enabled_indices)
    }

    pub fn delete_model(&mut self, index: usize) -> Result<String, String> {
        self.runtime.delete_model(index)
    }

    pub fn select_model(&mut self, index: usize) -> Result<(), String> {
        self.runtime.select_model(index)
    }

    pub fn tool_names(&self) -> Vec<&str> {
        self.runtime.tool_names()
    }

    pub fn system_prompt(&self) -> String {
        self.runtime.system_prompt()
    }

    pub fn prompt_display_enabled(&self) -> bool {
        self.runtime.prompt_display_enabled()
    }

    pub fn set_prompt_display_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.runtime.set_prompt_display_enabled(enabled)
    }

    pub fn locale(&self) -> Locale {
        self.runtime.locale()
    }

    pub fn locale_setting(&self) -> &str {
        self.runtime.locale_setting()
    }

    pub fn set_locale(&mut self, locale: &str) -> Result<(), String> {
        self.runtime.set_locale(locale)
    }

    pub fn theme(&self) -> ThemeSettings {
        self.runtime.theme()
    }

    pub fn set_theme(&mut self, theme: ThemeSettings) -> Result<(), String> {
        self.runtime.set_theme(theme)
    }

    pub fn keybindings(&self) -> crate::settings::KeyBindings {
        self.runtime.keybindings()
    }

    pub fn set_keybindings(
        &mut self,
        keybindings: crate::settings::KeyBindings,
    ) -> Result<(), String> {
        self.runtime.set_keybindings(keybindings)
    }

    pub fn session_message_count(&self) -> usize {
        self.runtime.session_message_count()
    }

    pub fn session_id(&self) -> &str {
        self.runtime.session_id()
    }

    pub fn session_path(&self) -> String {
        self.runtime.session_path()
    }

    pub fn recent_messages(&self, limit: usize) -> Vec<MessagePreview> {
        self.runtime.recent_messages(limit)
    }

    pub fn compact_context(&mut self) -> Result<usize, String> {
        self.runtime.compact_context()
    }

    pub fn run_prompt_events<F>(&mut self, prompt: &str, emit: &mut F) -> Result<(), String>
    where
        F: FnMut(AgentSessionEvent),
    {
        self.runtime
            .run_prompt_events_cancellable(prompt, &CancelToken::new(), emit)
    }

    pub fn run_prompt_events_with_images<F>(
        &mut self,
        prompt: &str,
        images: &[ImageContent],
        emit: &mut F,
    ) -> Result<(), String>
    where
        F: FnMut(AgentSessionEvent),
    {
        self.runtime.run_prompt_events_with_images_cancellable(
            prompt,
            images,
            &CancelToken::new(),
            emit,
        )
    }

    pub fn set_hooks(&mut self, hooks: SharedAgentHooks) {
        self.runtime.set_hooks(hooks);
    }

    pub fn run_prompt_events_cancellable<F>(
        &mut self,
        prompt: &str,
        cancel: &CancelToken,
        emit: &mut F,
    ) -> Result<(), String>
    where
        F: FnMut(AgentSessionEvent),
    {
        self.runtime
            .run_prompt_events_cancellable(prompt, cancel, emit)
    }

    pub fn run_prompt_events_with_images_cancellable<F>(
        &mut self,
        prompt: &str,
        images: &[ImageContent],
        cancel: &CancelToken,
        emit: &mut F,
    ) -> Result<(), String>
    where
        F: FnMut(AgentSessionEvent),
    {
        self.runtime
            .run_prompt_events_with_images_cancellable(prompt, images, cancel, emit)
    }

    pub fn subscribe_to_agent_session(
        &mut self,
        callback: impl FnMut(&AgentSessionEvent) + Send + 'static,
    ) -> usize {
        let id = self.next_listener_id;
        self.next_listener_id = self.next_listener_id.saturating_add(1);
        let callback: RuntimeHostCallback = Arc::new(Mutex::new(Box::new(callback)));
        let runtime_listener_id = subscribe_runtime_listener(&mut self.runtime, &callback);
        self.listeners.push(RuntimeHostListener {
            id,
            runtime_listener_id,
            callback,
        });
        id
    }

    pub fn unsubscribe_from_agent_session(&mut self, id: usize) -> bool {
        let Some(index) = self.listeners.iter().position(|listener| listener.id == id) else {
            return false;
        };
        let listener = self.listeners.remove(index);
        self.runtime
            .unsubscribe_from_agent_session(listener.runtime_listener_id)
    }

    pub fn project_dir(&self) -> &Path {
        self.runtime.project_dir()
    }

    fn rebind_listeners(&mut self) {
        for listener in &mut self.listeners {
            listener.runtime_listener_id =
                subscribe_runtime_listener(&mut self.runtime, &listener.callback);
        }
    }
}

fn subscribe_runtime_listener(runtime: &mut AppRuntime, callback: &RuntimeHostCallback) -> usize {
    let callback = Arc::clone(callback);
    runtime.subscribe_to_agent_session(move |event| {
        if let Ok(mut callback) = callback.lock() {
            (callback)(event);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{agent::AgentEvent, session::Session};
    use std::sync::{Arc, Mutex};

    #[test]
    fn host_reload_preserves_current_session() {
        let dir = test_dir("host_reload_preserves_current_session");
        let _ = std::fs::remove_dir_all(&dir);

        let options = RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        };
        let mut host = AppRuntimeHost::new(options.clone()).unwrap();
        let first_session = host.session_id().to_string();

        host.reload().unwrap();

        assert_eq!(host.options(), &options);
        assert_eq!(host.session_id(), first_session);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_owns_session_replacement_actions() {
        let dir = test_dir("host_owns_session_replacement_actions");
        let _ = std::fs::remove_dir_all(&dir);

        let options = RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        };
        let mut host = AppRuntimeHost::new(options).unwrap();
        let first_session = host.session_id().to_string();

        host.start_new_session().unwrap();

        assert_ne!(host.session_id(), first_session);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_opens_session_by_replacing_runtime() {
        let dir = test_dir("host_opens_session_by_replacing_runtime");
        let _ = std::fs::remove_dir_all(&dir);

        let options = RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        };
        let mut host = AppRuntimeHost::new(options).unwrap();
        let first_session_path = host.session_path();
        host.start_new_session().unwrap();
        let second_session = host.session_id().to_string();

        host.open_session(Path::new(&first_session_path)).unwrap();

        assert_ne!(host.session_id(), second_session);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_open_session_restores_session_project_dir() {
        let dir = test_dir("host_open_session_restores_session_project_dir");
        let _ = std::fs::remove_dir_all(&dir);
        let project_dir = dir.join("project");
        std::fs::create_dir_all(&project_dir).unwrap();

        let options = RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        };
        let session =
            Session::create_default_with_cwd(options.config_path.as_deref(), &project_dir).unwrap();
        let session_path = session.path().to_path_buf();
        drop(session);

        let mut host = AppRuntimeHost::new(options).unwrap();
        host.open_session(&session_path).unwrap();

        assert_eq!(host.project_dir(), project_dir.as_path());
        assert!(host
            .system_prompt()
            .contains(&project_dir.display().to_string().replace('\\', "/")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_new_session_after_open_keeps_session_project_dir() {
        let dir = test_dir("host_new_session_after_open_keeps_session_project_dir");
        let _ = std::fs::remove_dir_all(&dir);
        let project_dir = dir.join("project");
        std::fs::create_dir_all(&project_dir).unwrap();

        let options = RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        };
        let session =
            Session::create_default_with_cwd(options.config_path.as_deref(), &project_dir).unwrap();
        let session_path = session.path().to_path_buf();
        drop(session);

        let mut host = AppRuntimeHost::new(options).unwrap();
        host.open_session(&session_path).unwrap();
        host.start_new_session().unwrap();

        assert_eq!(host.project_dir(), project_dir.as_path());
        assert!(host
            .system_prompt()
            .contains(&project_dir.display().to_string().replace('\\', "/")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_rejects_session_with_missing_project_dir() {
        let dir = test_dir("host_rejects_session_with_missing_project_dir");
        let _ = std::fs::remove_dir_all(&dir);
        let project_dir = dir.join("missing-project");
        std::fs::create_dir_all(&project_dir).unwrap();

        let options = RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        };
        let session =
            Session::create_default_with_cwd(options.config_path.as_deref(), &project_dir).unwrap();
        let session_path = session.path().to_path_buf();
        drop(session);
        std::fs::remove_dir_all(&project_dir).unwrap();

        let mut host = AppRuntimeHost::new(options).unwrap();
        let error = host.open_session(&session_path).unwrap_err();

        assert!(error.contains("session project directory does not exist"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_rebinds_listeners_after_session_replacement() {
        let dir = test_dir("host_rebinds_listeners_after_session_replacement");
        let _ = std::fs::remove_dir_all(&dir);

        let options = RuntimeOptions {
            config_path: Some(dir.display().to_string()),
            ..RuntimeOptions::default()
        };
        let mut host = AppRuntimeHost::new(options).unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let listener_observed = Arc::clone(&observed);
        host.subscribe_to_agent_session(move |event| {
            listener_observed.lock().unwrap().push(event.clone());
        });

        host.run_prompt_events("before", &mut |_| {}).unwrap();
        host.start_new_session().unwrap();
        host.run_prompt_events("after", &mut |_| {}).unwrap();

        let observed = observed.lock().unwrap();
        let starts = observed
            .iter()
            .filter(|event| matches!(event, AgentSessionEvent::Agent(AgentEvent::AgentStart)))
            .count();
        assert_eq!(starts, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("exgent_runtime_host_{name}_{stamp}"))
    }
}
