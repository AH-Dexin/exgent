use crate::{
    localization::{resolve_locale, Locale},
    settings::SettingsStore,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CliOptions {
    pub config_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Help(CliOptions),
    Version,
    Run(CliOptions),
}

pub fn parse<I>(args: I) -> Result<Command, String>
where
    I: IntoIterator,
    I::Item: Into<String>,
{
    let mut options = CliOptions::default();
    let mut args = args.into_iter().map(Into::into).peekable();
    let mut help = false;
    let mut version = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => help = true,
            "--version" | "-V" => version = true,
            "--config" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--config requires a path".to_string())?;
                options.config_path = Some(value);
            }
            _ if arg.starts_with("--config=") => {
                options.config_path = Some(arg["--config=".len()..].to_string());
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }

    if help {
        Ok(Command::Help(options))
    } else if version {
        Ok(Command::Version)
    } else {
        Ok(Command::Run(options))
    }
}

pub fn localized_help_text(options: &CliOptions) -> String {
    let locale = SettingsStore::load(options.config_path.as_deref())
        .map(|settings| settings.locale())
        .unwrap_or_else(|_| resolve_locale("auto"));
    help_text(locale)
}

pub fn help_text(locale: Locale) -> String {
    match locale {
        Locale::En => english_help_text(),
        Locale::ZhHans => chinese_help_text(),
    }
}

fn english_help_text() -> String {
    format!(
        "exgent {}\n\nUsage:\n  exgent\n  exgent --config <path>\n  exgent --help\n  exgent --version\n\nInteractive commands:\n  /auth               Configure provider authentication\n  /model              Select an enabled model\n  /settings           Open settings\n  /settings auth      Enable, disable, or remove configured providers\n  /settings model     Enable or disable models for enabled providers\n  /settings theme     Change TUI theme\n  /settings language  Change interface language\n  /debug              Open debug settings\n  /debug show         Print the current system prompt\n  /session            Manage sessions\n  /compact            Compact current session context\n  /quit               Exit\n\nDebug menu:\n  /debug -> prompt -> enable   Show reasoning/thinking deltas in gray italic text\n  /debug -> prompt -> disable  Hide reasoning/thinking deltas\n",
        env!("CARGO_PKG_VERSION")
    )
}

fn chinese_help_text() -> String {
    format!(
        "exgent {}\n\n用法:\n  exgent\n  exgent --config <路径>\n  exgent --help\n  exgent --version\n\n交互命令:\n  /auth               配置服务商认证\n  /model              选择已启用模型\n  /settings           打开设置\n  /settings auth      启用、禁用或移除已配置服务商\n  /settings model     为已启用服务商启用或禁用模型\n  /settings theme     更改 TUI 主题\n  /settings language  更改界面语言\n  /debug              打开调试设置\n  /debug show         打印当前系统提示词\n  /session            管理会话\n  /compact            压缩当前会话上下文\n  /quit               退出\n\n调试菜单:\n  /debug -> prompt -> enable   显示灰色斜体 reasoning/thinking 增量\n  /debug -> prompt -> disable  隐藏 reasoning/thinking 增量\n",
        env!("CARGO_PKG_VERSION")
    )
}

pub fn version_text() -> String {
    format!("exgent {}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_keeps_config_path() {
        assert_eq!(
            parse(["--config", "/tmp/exgent-test", "--help"]),
            Ok(Command::Help(CliOptions {
                config_path: Some("/tmp/exgent-test".to_string()),
            }))
        );
    }

    #[test]
    fn help_text_is_localized() {
        assert!(help_text(Locale::En).contains("Usage:"));
        assert!(help_text(Locale::ZhHans).contains("用法:"));
        assert!(help_text(Locale::ZhHans).contains("更改界面语言"));
    }
}
