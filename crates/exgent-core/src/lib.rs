//! Runtime, agent, and tool primitives shared between the `exgent` CLI binary
//! and any embedder that wants to drive the agent programmatically.
//!
//! The public surface is intentionally narrow: external code should reach for
//! [`AppRuntimeHost`] and the types re-exported below. Anything not listed here
//! is internal and may change between releases.

mod agent;
mod auth;
mod cancel;
mod config;
mod error;
mod localization;
mod model_service;
mod models;
mod oauth;
mod persistence;
mod runtime;
mod session;
mod settings;
mod storage;
mod system_prompt;
mod tools;

pub use agent::{
    AgentEvent, AgentHooks, AgentSessionEvent, NoHooks, SharedAgentHooks, ToolExecutionResult,
    TurnTelemetry, UsageTotals,
};
pub use auth::OAuthCredential;
pub use cancel::CancelToken;
pub use config::{AgentLoopConfig, RuntimeOptions};
pub use error::{AgentSessionError, ModelServiceError, PersistenceError};
pub use exgent_ai::ImageContent;
pub use localization::{resolve_locale, tr, LanguageOption, Locale, MessageId, LANGUAGE_OPTIONS};
pub use models::CompatibleModelKind;
pub use oauth::{
    finish_anthropic_oauth_flow, finish_github_copilot_device_flow,
    finish_github_copilot_device_flow_cancellable, finish_openai_codex_oauth_flow,
    normalize_github_domain, parse_authorization_input, refresh_github_copilot_token,
    start_anthropic_oauth_flow, start_github_copilot_device_flow, start_openai_codex_oauth_flow,
    AnthropicOAuthFlow, AuthorizationCode, GithubDeviceFlow, OpenAiCodexOAuthFlow,
};
pub use runtime::{
    AddedModelInfo, AppRuntimeHost, AuthProviderInfo, MessagePreview, ModelMenuItem,
    ModelSettingsItem, ModelStatus, SessionInfo, SubscriptionProviderInfo,
};
pub use settings::{
    KeyAction, KeyBindings, SettingsStore, ThemePreset, ThemeRgb, ThemeSettings, THEME_PRESETS,
};
pub use storage::{FsStorage, InMemoryStorage, Storage};
pub use tools::{Tool, ToolOutput, ToolRegistry};

/// Curated façade for embedders.
///
/// `sdk` re-exports the minimum surface needed to drive an agent without
/// pulling in optional knobs. Prefer this module when integrating from
/// another binary (an IDE extension, a daemon, an RPC bridge) — it gives
/// you a stable contract while the underlying modules evolve.
pub mod sdk {
    pub use crate::agent::{
        AgentEvent, AgentHooks, AgentSessionEvent, NoHooks, SharedAgentHooks, ToolExecutionResult,
        TurnTelemetry, UsageTotals,
    };
    pub use crate::cancel::CancelToken;
    pub use crate::config::{AgentLoopConfig, RuntimeOptions};
    pub use crate::error::{AgentSessionError, ModelServiceError, PersistenceError};
    pub use crate::runtime::AppRuntimeHost;
    pub use crate::tools::{Tool, ToolOutput, ToolRegistry};
}
