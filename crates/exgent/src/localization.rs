#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Locale {
    En,
    ZhHans,
}

impl Locale {
    pub fn tag(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhHans => "zh-Hans",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::ZhHans => "简体中文",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LanguageOption {
    pub locale: Locale,
    pub setting: &'static str,
    pub label: &'static str,
}

pub const LANGUAGE_OPTIONS: &[LanguageOption] = &[
    LanguageOption {
        locale: Locale::En,
        setting: "en",
        label: "English",
    },
    LanguageOption {
        locale: Locale::ZhHans,
        setting: "zh-Hans",
        label: "简体中文",
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum MessageId {
    WelcomeNote,
    EmptyTranscriptHint,
    StatusReady,
    StatusThinking,
    RuntimeRunningTool,
    ComposerTitle,
    SidebarStatus,
    SidebarSession,
    SidebarModel,
    SidebarReasoning,
    SidebarUsage,
    SidebarInput,
    SidebarOutput,
    SidebarCacheRead,
    SidebarContext,
    SidebarCommands,
    CmdAuthDescription,
    CmdModelDescription,
    CmdSettingsDescription,
    CmdSettingsAuthDescription,
    CmdSettingsModelDescription,
    CmdSettingsThemeDescription,
    CmdSettingsLanguageDescription,
    CmdDebugDescription,
    CmdDebugShowDescription,
    CmdSessionDescription,
    CmdCompactDescription,
    CmdQuitDescription,
    DialogModel,
    DialogSettings,
    DialogSettingsAuth,
    DialogSettingsModel,
    DialogTheme,
    DialogLanguage,
    DialogProviderAction,
    DialogModelAction,
    DialogSession,
    DialogDebug,
    DialogDebugPrompt,
    DialogAuthentication,
    DialogApiKey,
    DialogProviderToken,
    DialogAddOpenAiModel,
    DialogSubscription,
    SettingsAuth,
    SettingsModel,
    SettingsTheme,
    SettingsLanguage,
    ActionEnable,
    ActionDisable,
    ActionRemove,
    AuthSubscription,
    AuthApiKey,
    AddOpenAiCompatibleModel,
    NewSession,
    DebugPrompt,
    FieldProvider,
    FieldModelId,
    FieldBaseUrl,
    FieldApiKey,
    FieldRed,
    FieldGreen,
    FieldBlue,
    EmptyValue,
    CustomRgb,
    CustomRgbHint,
    BlankRemovesStoredToken,
    EnterSavesEscapeCancels,
    AuthConfigured,
    AuthMissing,
    ReasoningHigh,
    ReasoningOff,
    LabelCurrent,
    LabelMessages,
    NoModelsAvailable,
    NoEnabledModelsAvailable,
    NoConfiguredProviders,
    NoConfiguredModelsAuthFirst,
    NoConfiguredModelsEnableProvider,
    NoSubscriptionProviders,
    SettingsCancelled,
    ThemeCancelled,
    UnknownCommand,
    CompactSuccess,
    DebugDisplayEnabled,
    DebugDisplayDisabled,
    DebugPromptEnabled,
    DebugPromptDisabled,
    NewSessionCreated,
    SessionLoaded,
    ThemeSaved,
    LanguageSaved,
}

pub fn tr(locale: Locale, id: MessageId) -> &'static str {
    match locale {
        Locale::En => en(id),
        Locale::ZhHans => zh_hans(id).unwrap_or_else(|| en(id)),
    }
}

pub fn normalize_configured_locale(input: &str) -> Option<&'static str> {
    let trimmed = input.trim();
    if trimmed == "简体中文" || trimmed == "中文" {
        return Some("zh-Hans");
    }

    let normalized = normalize_locale_input(trimmed);
    if matches!(normalized.as_str(), "" | "auto" | "system") {
        return Some("auto");
    }
    parse_locale(&normalized).map(Locale::tag)
}

pub fn resolve_locale(setting: &str) -> Locale {
    resolve_locale_with_env(setting, |key| std::env::var(key).ok())
}

fn resolve_locale_with_env<F>(setting: &str, env: F) -> Locale
where
    F: Fn(&str) -> Option<String>,
{
    let normalized = normalize_locale_input(setting);
    if !matches!(normalized.as_str(), "" | "auto" | "system") {
        return parse_locale(&normalized).unwrap_or(Locale::En);
    }

    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = env(key) {
            if let Some(locale) = parse_locale(&normalize_locale_input(&value)) {
                return locale;
            }
        }
    }

    Locale::En
}

fn normalize_locale_input(input: &str) -> String {
    input
        .split('.')
        .next()
        .unwrap_or(input)
        .split('@')
        .next()
        .unwrap_or(input)
        .trim()
        .replace('_', "-")
        .to_lowercase()
}

fn parse_locale(value: &str) -> Option<Locale> {
    if value == "c" || value == "posix" || value.starts_with("en") {
        return Some(Locale::En);
    }
    if value.starts_with("zh") || value == "cn" || value.contains("hans") {
        return Some(Locale::ZhHans);
    }
    None
}

fn en(id: MessageId) -> &'static str {
    match id {
        MessageId::WelcomeNote => "Welcome to Exgent. Type / for commands.",
        MessageId::EmptyTranscriptHint => "Type / for commands or write a task.",
        MessageId::StatusReady => "ready",
        MessageId::StatusThinking => "thinking",
        MessageId::RuntimeRunningTool => "running {tool}",
        MessageId::ComposerTitle => "Composer",
        MessageId::SidebarStatus => "Status",
        MessageId::SidebarSession => "session",
        MessageId::SidebarModel => "model",
        MessageId::SidebarReasoning => "reasoning",
        MessageId::SidebarUsage => "Usage",
        MessageId::SidebarInput => "input",
        MessageId::SidebarOutput => "output",
        MessageId::SidebarCacheRead => "cache read",
        MessageId::SidebarContext => "context",
        MessageId::SidebarCommands => "Commands",
        MessageId::CmdAuthDescription => "Configure provider authentication",
        MessageId::CmdModelDescription => "Select an enabled model",
        MessageId::CmdSettingsDescription => "Open settings",
        MessageId::CmdSettingsAuthDescription => "Enable, disable, or remove providers",
        MessageId::CmdSettingsModelDescription => "Enable or disable models",
        MessageId::CmdSettingsThemeDescription => "Change TUI theme",
        MessageId::CmdSettingsLanguageDescription => "Change interface language",
        MessageId::CmdDebugDescription => "Open debug settings",
        MessageId::CmdDebugShowDescription => "Print the current system prompt",
        MessageId::CmdSessionDescription => "Manage sessions",
        MessageId::CmdCompactDescription => "Compact current session context",
        MessageId::CmdQuitDescription => "Exit",
        MessageId::DialogModel => "Model",
        MessageId::DialogSettings => "Settings",
        MessageId::DialogSettingsAuth => "Settings Auth",
        MessageId::DialogSettingsModel => "Settings Model",
        MessageId::DialogTheme => "Theme",
        MessageId::DialogLanguage => "Language",
        MessageId::DialogProviderAction => "Provider action",
        MessageId::DialogModelAction => "Model action",
        MessageId::DialogSession => "Session",
        MessageId::DialogDebug => "Debug",
        MessageId::DialogDebugPrompt => "Debug Prompt",
        MessageId::DialogAuthentication => "Authentication",
        MessageId::DialogApiKey => "API Key",
        MessageId::DialogProviderToken => "Provider Token",
        MessageId::DialogAddOpenAiModel => "Add OpenAI Model",
        MessageId::DialogSubscription => "Subscription",
        MessageId::SettingsAuth => "auth",
        MessageId::SettingsModel => "model",
        MessageId::SettingsTheme => "theme",
        MessageId::SettingsLanguage => "language",
        MessageId::ActionEnable => "enable",
        MessageId::ActionDisable => "disable",
        MessageId::ActionRemove => "remove",
        MessageId::AuthSubscription => "subscription",
        MessageId::AuthApiKey => "api key",
        MessageId::AddOpenAiCompatibleModel => "add OpenAI-compatible model",
        MessageId::NewSession => "new session",
        MessageId::DebugPrompt => "prompt",
        MessageId::FieldProvider => "provider",
        MessageId::FieldModelId => "model id",
        MessageId::FieldBaseUrl => "base url",
        MessageId::FieldApiKey => "api key",
        MessageId::FieldRed => "red",
        MessageId::FieldGreen => "green",
        MessageId::FieldBlue => "blue",
        MessageId::EmptyValue => "(empty)",
        MessageId::CustomRgb => "custom rgb",
        MessageId::CustomRgbHint => "enter saves   tab moves",
        MessageId::BlankRemovesStoredToken => "(blank removes stored token)",
        MessageId::EnterSavesEscapeCancels => "enter saves   escape cancels",
        MessageId::AuthConfigured => "configured",
        MessageId::AuthMissing => "missing",
        MessageId::ReasoningHigh => "high",
        MessageId::ReasoningOff => "off",
        MessageId::LabelCurrent => "current",
        MessageId::LabelMessages => "messages",
        MessageId::NoModelsAvailable => "No models available.",
        MessageId::NoEnabledModelsAvailable => "No enabled models available. Use /auth first.",
        MessageId::NoConfiguredProviders => "No configured providers.",
        MessageId::NoConfiguredModelsAuthFirst => {
            "No configured models available. Use /auth to configure a provider first."
        }
        MessageId::NoConfiguredModelsEnableProvider => {
            "No configured models available. Enable a provider first."
        }
        MessageId::NoSubscriptionProviders => "No subscription providers available.",
        MessageId::SettingsCancelled => "settings cancelled",
        MessageId::ThemeCancelled => "theme cancelled",
        MessageId::UnknownCommand => "unknown command",
        MessageId::CompactSuccess => "compacted {count} message(s)",
        MessageId::DebugDisplayEnabled => "debug display: enabled",
        MessageId::DebugDisplayDisabled => "debug display: disabled",
        MessageId::DebugPromptEnabled => "debug prompt: enabled",
        MessageId::DebugPromptDisabled => "debug prompt: disabled",
        MessageId::NewSessionCreated => "new session: {id}",
        MessageId::SessionLoaded => "loaded session: {id}",
        MessageId::ThemeSaved => "theme: {name}",
        MessageId::LanguageSaved => "language: {name}",
    }
}

fn zh_hans(id: MessageId) -> Option<&'static str> {
    Some(match id {
        MessageId::WelcomeNote => "欢迎使用 Exgent。输入 / 查看命令。",
        MessageId::EmptyTranscriptHint => "输入 / 查看命令，或直接输入任务。",
        MessageId::StatusReady => "就绪",
        MessageId::StatusThinking => "思考中",
        MessageId::RuntimeRunningTool => "正在执行 {tool}",
        MessageId::ComposerTitle => "输入框",
        MessageId::SidebarStatus => "状态",
        MessageId::SidebarSession => "会话",
        MessageId::SidebarModel => "模型",
        MessageId::SidebarReasoning => "推理",
        MessageId::SidebarUsage => "用量",
        MessageId::SidebarInput => "输入",
        MessageId::SidebarOutput => "输出",
        MessageId::SidebarCacheRead => "缓存读取",
        MessageId::SidebarContext => "上下文",
        MessageId::SidebarCommands => "命令",
        MessageId::CmdAuthDescription => "配置服务商认证",
        MessageId::CmdModelDescription => "选择已启用模型",
        MessageId::CmdSettingsDescription => "打开设置",
        MessageId::CmdSettingsAuthDescription => "启用、禁用或移除服务商",
        MessageId::CmdSettingsModelDescription => "启用或禁用模型",
        MessageId::CmdSettingsThemeDescription => "更改 TUI 主题",
        MessageId::CmdSettingsLanguageDescription => "更改界面语言",
        MessageId::CmdDebugDescription => "打开调试设置",
        MessageId::CmdDebugShowDescription => "打印当前系统提示词",
        MessageId::CmdSessionDescription => "管理会话",
        MessageId::CmdCompactDescription => "压缩当前会话上下文",
        MessageId::CmdQuitDescription => "退出",
        MessageId::DialogModel => "模型",
        MessageId::DialogSettings => "设置",
        MessageId::DialogSettingsAuth => "认证设置",
        MessageId::DialogSettingsModel => "模型设置",
        MessageId::DialogTheme => "主题",
        MessageId::DialogLanguage => "语言",
        MessageId::DialogProviderAction => "服务商操作",
        MessageId::DialogModelAction => "模型操作",
        MessageId::DialogSession => "会话",
        MessageId::DialogDebug => "调试",
        MessageId::DialogDebugPrompt => "调试提示词",
        MessageId::DialogAuthentication => "认证",
        MessageId::DialogApiKey => "API Key",
        MessageId::DialogProviderToken => "服务商 Token",
        MessageId::DialogAddOpenAiModel => "添加 OpenAI 兼容模型",
        MessageId::DialogSubscription => "订阅",
        MessageId::SettingsAuth => "认证",
        MessageId::SettingsModel => "模型",
        MessageId::SettingsTheme => "主题",
        MessageId::SettingsLanguage => "语言",
        MessageId::ActionEnable => "启用",
        MessageId::ActionDisable => "禁用",
        MessageId::ActionRemove => "移除",
        MessageId::AuthSubscription => "订阅",
        MessageId::AuthApiKey => "API Key",
        MessageId::AddOpenAiCompatibleModel => "添加 OpenAI 兼容模型",
        MessageId::NewSession => "新建会话",
        MessageId::DebugPrompt => "提示词",
        MessageId::FieldProvider => "服务商",
        MessageId::FieldModelId => "模型 ID",
        MessageId::FieldBaseUrl => "基础 URL",
        MessageId::FieldApiKey => "API Key",
        MessageId::FieldRed => "红",
        MessageId::FieldGreen => "绿",
        MessageId::FieldBlue => "蓝",
        MessageId::EmptyValue => "（空）",
        MessageId::CustomRgb => "自定义 RGB",
        MessageId::CustomRgbHint => "回车保存   Tab 切换",
        MessageId::BlankRemovesStoredToken => "（留空会移除已保存 Token）",
        MessageId::EnterSavesEscapeCancels => "回车保存   Esc 取消",
        MessageId::AuthConfigured => "已配置",
        MessageId::AuthMissing => "缺失",
        MessageId::ReasoningHigh => "高",
        MessageId::ReasoningOff => "关闭",
        MessageId::LabelCurrent => "当前",
        MessageId::LabelMessages => "消息",
        MessageId::NoModelsAvailable => "没有可用模型。",
        MessageId::NoEnabledModelsAvailable => "没有已启用模型。请先使用 /auth。",
        MessageId::NoConfiguredProviders => "没有已配置的服务商。",
        MessageId::NoConfiguredModelsAuthFirst => "没有已配置模型。请先使用 /auth 配置服务商。",
        MessageId::NoConfiguredModelsEnableProvider => "没有已配置模型。请先启用一个服务商。",
        MessageId::NoSubscriptionProviders => "没有可用订阅服务商。",
        MessageId::SettingsCancelled => "已取消设置",
        MessageId::ThemeCancelled => "已取消主题设置",
        MessageId::UnknownCommand => "未知命令",
        MessageId::CompactSuccess => "已压缩 {count} 条消息",
        MessageId::DebugDisplayEnabled => "调试显示：已启用",
        MessageId::DebugDisplayDisabled => "调试显示：已禁用",
        MessageId::DebugPromptEnabled => "调试提示词：已启用",
        MessageId::DebugPromptDisabled => "调试提示词：已禁用",
        MessageId::NewSessionCreated => "新建会话：{id}",
        MessageId::SessionLoaded => "已加载会话：{id}",
        MessageId::ThemeSaved => "主题：{name}",
        MessageId::LanguageSaved => "语言：{name}",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_locale_values() {
        assert_eq!(normalize_configured_locale("en-US"), Some("en"));
        assert_eq!(normalize_configured_locale("zh_CN.UTF-8"), Some("zh-Hans"));
        assert_eq!(normalize_configured_locale("简体中文"), Some("zh-Hans"));
        assert_eq!(normalize_configured_locale("auto"), Some("auto"));
        assert_eq!(normalize_configured_locale("ja"), None);
    }

    #[test]
    fn translates_known_messages() {
        assert_eq!(tr(Locale::En, MessageId::CmdQuitDescription), "Exit");
        assert_eq!(tr(Locale::ZhHans, MessageId::CmdQuitDescription), "退出");
    }
}
