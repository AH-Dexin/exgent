mod catalog;

use catalog::{en, zh_hans};

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
#[allow(dead_code)]
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
    DialogCustomModel,
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
    CustomModel,
    CompatibleOpenAi,
    CompatibleAnthropic,
    CompatibleGoogle,
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
    FieldApiKeyHint,
    FieldUrl,
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
    CancelRequested,
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
