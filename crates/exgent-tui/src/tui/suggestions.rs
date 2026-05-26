use crate::commands::{CommandHelp, COMMAND_HELP};

pub(super) fn slash_suggestions(input: &str) -> Vec<&'static CommandHelp> {
    if !input.starts_with('/') {
        return Vec::new();
    }
    COMMAND_HELP
        .iter()
        .filter(|help| help.command.starts_with(input))
        .filter(|help| !help.command.trim_start_matches('/').contains(' '))
        .collect()
}
