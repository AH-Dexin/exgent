use std::path::{Path, PathBuf};

use chrono::{Datelike, Local};
use exgent_ai::ToolDefinition;

pub fn build_system_prompt_for_cwd(
    tools: &[ToolDefinition],
    selected_model: Option<&str>,
    interface: &str,
    cwd: impl Into<PathBuf>,
) -> String {
    build_system_prompt_with_date_and_cwd(
        tools,
        selected_model,
        interface,
        current_date(),
        cwd.into(),
    )
}

fn build_system_prompt_with_date_and_cwd(
    tools: &[ToolDefinition],
    selected_model: Option<&str>,
    interface: &str,
    date: String,
    cwd: PathBuf,
) -> String {
    let tools_list = tools
        .iter()
        .filter_map(|tool| {
            tool.prompt_snippet
                .as_ref()
                .map(|snippet| format!("- {}: {}", tool.name, snippet))
        })
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

fn prompt_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_prompt_with_tools_date_and_cwd() {
        let prompt = build_system_prompt_with_date_and_cwd(
            &[
                ToolDefinition::new("read", "Read text from a file.", serde_json::json!({}))
                    .with_prompt_snippet("Read file contents"),
                ToolDefinition::new("bash", "Run a shell command.", serde_json::json!({}))
                    .with_prompt_snippet("Execute bash commands (ls, grep, find, etc.)"),
                ToolDefinition::new("edit", "Edit a file.", serde_json::json!({}))
                    .with_prompt_snippet("Make precise file edits with exact text replacement"),
                ToolDefinition::new("write", "Write a file.", serde_json::json!({}))
                    .with_prompt_snippet("Create or overwrite files"),
            ],
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
