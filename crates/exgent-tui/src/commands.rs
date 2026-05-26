use exgent_core::MessageId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppCommand<'a> {
    Quit,
    Model,
    Auth,
    Settings,
    SettingsAuth,
    SettingsModel,
    SettingsTheme,
    SettingsLanguage,
    Debug,
    DebugEnable,
    DebugDisable,
    DebugShow,
    Session,
    Compact,
    Unknown(&'a str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandHelp {
    pub command: &'static str,
    pub description_id: MessageId,
}

pub const COMMAND_HELP: &[CommandHelp] = &[
    CommandHelp {
        command: "/auth",
        description_id: MessageId::CmdAuthDescription,
    },
    CommandHelp {
        command: "/model",
        description_id: MessageId::CmdModelDescription,
    },
    CommandHelp {
        command: "/settings",
        description_id: MessageId::CmdSettingsDescription,
    },
    CommandHelp {
        command: "/settings auth",
        description_id: MessageId::CmdSettingsAuthDescription,
    },
    CommandHelp {
        command: "/settings model",
        description_id: MessageId::CmdSettingsModelDescription,
    },
    CommandHelp {
        command: "/settings theme",
        description_id: MessageId::CmdSettingsThemeDescription,
    },
    CommandHelp {
        command: "/settings language",
        description_id: MessageId::CmdSettingsLanguageDescription,
    },
    CommandHelp {
        command: "/debug",
        description_id: MessageId::CmdDebugDescription,
    },
    CommandHelp {
        command: "/debug show",
        description_id: MessageId::CmdDebugShowDescription,
    },
    CommandHelp {
        command: "/debug prompt enable",
        description_id: MessageId::CmdDebugPromptEnableDescription,
    },
    CommandHelp {
        command: "/debug prompt disable",
        description_id: MessageId::CmdDebugPromptDisableDescription,
    },
    CommandHelp {
        command: "/session",
        description_id: MessageId::CmdSessionDescription,
    },
    CommandHelp {
        command: "/compact",
        description_id: MessageId::CmdCompactDescription,
    },
    CommandHelp {
        command: "/quit",
        description_id: MessageId::CmdQuitDescription,
    },
];

pub fn parse_command(input: &str) -> Option<AppCommand<'_>> {
    match input {
        "/quit" => Some(AppCommand::Quit),
        "/model" => Some(AppCommand::Model),
        "/auth" => Some(AppCommand::Auth),
        "/setting" | "/settings" => Some(AppCommand::Settings),
        "/setting auth" | "/settings auth" => Some(AppCommand::SettingsAuth),
        "/setting model" | "/settings model" => Some(AppCommand::SettingsModel),
        "/setting theme" | "/settings theme" => Some(AppCommand::SettingsTheme),
        "/setting language" | "/settings language" => Some(AppCommand::SettingsLanguage),
        "/debug" => Some(AppCommand::Debug),
        "/debug enable" | "/debug prompt enable" => Some(AppCommand::DebugEnable),
        "/debug disable" | "/debug prompt disable" => Some(AppCommand::DebugDisable),
        "/debug show" => Some(AppCommand::DebugShow),
        "/session" => Some(AppCommand::Session),
        "/compact" => Some(AppCommand::Compact),
        _ if input.starts_with('/') => Some(AppCommand::Unknown(input)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_commands() {
        assert_eq!(parse_command("/model"), Some(AppCommand::Model));
        assert_eq!(parse_command("/settings"), Some(AppCommand::Settings));
        assert_eq!(
            parse_command("/settings model"),
            Some(AppCommand::SettingsModel)
        );
        assert_eq!(
            parse_command("/settings auth"),
            Some(AppCommand::SettingsAuth)
        );
        assert_eq!(
            parse_command("/settings theme"),
            Some(AppCommand::SettingsTheme)
        );
        assert_eq!(
            parse_command("/setting theme"),
            Some(AppCommand::SettingsTheme)
        );
        assert_eq!(
            parse_command("/settings language"),
            Some(AppCommand::SettingsLanguage)
        );
        assert_eq!(
            parse_command("/setting language"),
            Some(AppCommand::SettingsLanguage)
        );
        assert_eq!(parse_command("/compact"), Some(AppCommand::Compact));
        assert_eq!(parse_command("/debug"), Some(AppCommand::Debug));
        assert_eq!(
            parse_command("/debug enable"),
            Some(AppCommand::DebugEnable)
        );
        assert_eq!(
            parse_command("/debug prompt enable"),
            Some(AppCommand::DebugEnable)
        );
        assert_eq!(
            parse_command("/debug disable"),
            Some(AppCommand::DebugDisable)
        );
        assert_eq!(
            parse_command("/debug prompt disable"),
            Some(AppCommand::DebugDisable)
        );
        assert_eq!(parse_command("/debug show"), Some(AppCommand::DebugShow));
        assert_eq!(parse_command("hello"), None);
    }
}
