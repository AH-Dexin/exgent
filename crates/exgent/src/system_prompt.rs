use std::{
    env,
    path::{Path, PathBuf},
};

use chrono::{Datelike, Local};

pub fn build_system_prompt(
    tool_names: &[&str],
    selected_model: Option<&str>,
    interface: &str,
) -> String {
    build_system_prompt_with_date_and_cwd(
        tool_names,
        selected_model,
        interface,
        current_date(),
        current_working_directory(),
    )
}

fn build_system_prompt_with_date_and_cwd(
    tool_names: &[&str],
    selected_model: Option<&str>,
    interface: &str,
    date: String,
    cwd: PathBuf,
) -> String {
    let tools_list = tool_names
        .iter()
        .filter_map(|name| tool_prompt_snippet(name).map(|snippet| format!("- {name}: {snippet}")))
        .collect::<Vec<_>>()
        .join("\n");
    let tools_list = if tools_list.is_empty() {
        "(none)".to_string()
    } else {
        tools_list
    };

    let selected_model = selected_model.unwrap_or("not configured");

    format!(
        "Exgent {interface}; model: {selected_model}; date: {date}.\nTools:\n{tools_list}\nUse the user's language. Be concise.\nProject dir for relative tool paths: {}. Do not mention it unless asked or needed for a file operation.",
        prompt_path(&cwd)
    )
}

fn current_date() -> String {
    let now = Local::now();
    format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day())
}

fn current_working_directory() -> PathBuf {
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn tool_prompt_snippet(name: &str) -> Option<&'static str> {
    match name {
        "read" => Some("Read file contents"),
        "bash" => Some("Execute bash commands (ls, grep, find, etc.)"),
        "edit" => Some("Make precise file edits with exact text replacement"),
        "write" => Some("Create or overwrite files"),
        _ => None,
    }
}

fn prompt_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_prompt_with_tools_date_and_cwd() {
        let prompt = build_system_prompt_with_date_and_cwd(
            &["read", "bash", "edit", "write"],
            Some("test-provider/test-model"),
            "TUI",
            "2026-05-24".to_string(),
            PathBuf::from(r"E:\Work\Project\GitRepo\exgent"),
        );

        assert!(prompt.contains("- read: Read file contents"));
        assert!(prompt.contains("Exgent TUI; model: test-provider/test-model; date: 2026-05-24."));
        assert!(prompt.contains("Use the user's language. Be concise."));
        assert!(
            prompt.contains("Project dir for relative tool paths: E:/Work/Project/GitRepo/exgent.")
        );
        assert!(prompt.contains("Do not mention it unless asked"));
    }
}
