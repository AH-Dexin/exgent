use std::path::{Path, PathBuf};

use exgent_ai::{ChatMessage, MessageRole as ChatMessageRole, TokenUsage, ToolCall};

use crate::config::RuntimeOptions;

use super::{MessageRole, Session, SessionInfo, SessionMessagePreview};

pub struct SessionService {
    config_path: Option<String>,
    session: Session,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessagePreview {
    pub role: String,
    pub content: String,
}

impl SessionService {
    pub fn create(options: &RuntimeOptions, project_dir: &Path) -> Result<Self, String> {
        let session = Session::create_default_with_cwd(options.config_path.as_deref(), project_dir)
            .map_err(|error| format!("failed to create session: {error}"))?;
        Ok(Self {
            config_path: options.config_path.clone(),
            session,
        })
    }

    pub fn open(options: &RuntimeOptions, path: &Path) -> Result<Self, String> {
        let mut session =
            Session::open(path).map_err(|error| format!("failed to open session: {error}"))?;
        let cwd = normalize_session_cwd(session.cwd())?;
        session.set_runtime_cwd(cwd);
        Ok(Self {
            config_path: options.config_path.clone(),
            session,
        })
    }

    pub fn append_user(&mut self, content: &str) -> Result<(), String> {
        self.session
            .append_user(content)
            .map_err(|error| format!("failed to write user message: {error}"))
    }

    pub fn append_assistant(&mut self, content: String) -> Result<(), String> {
        self.session
            .append_assistant(content)
            .map_err(|error| format!("failed to write assistant message: {error}"))
    }

    pub fn append_assistant_tool_calls(
        &mut self,
        content: String,
        calls: Vec<ToolCall>,
        usage: Option<TokenUsage>,
    ) -> Result<(), String> {
        self.session
            .append_assistant_tool_calls(content, calls, usage)
            .map_err(|error| format!("failed to write assistant tool calls: {error}"))
    }

    #[allow(dead_code)]
    pub fn append_tool(&mut self, content: String) -> Result<(), String> {
        self.session
            .append_tool(content)
            .map_err(|error| format!("failed to write tool message: {error}"))
    }

    pub fn append_tool_result(
        &mut self,
        tool_call_id: String,
        tool_name: String,
        content: String,
        is_error: bool,
    ) -> Result<(), String> {
        self.session
            .append_tool_result(tool_call_id, tool_name, content, is_error)
            .map_err(|error| format!("failed to write tool result: {error}"))
    }

    pub fn append_error(&mut self, content: String) -> Result<(), String> {
        self.session
            .append_error(content)
            .map_err(|error| format!("failed to write error message: {error}"))
    }

    pub fn append_assistant_with_usage(
        &mut self,
        content: String,
        usage: TokenUsage,
    ) -> Result<(), String> {
        self.session
            .append_assistant_with_usage(content, usage)
            .map_err(|error| format!("failed to write assistant message: {error}"))
    }

    pub fn chat_messages(&self) -> Vec<ChatMessage> {
        self.session.chat_messages()
    }

    pub fn compaction_messages(&self) -> Vec<ChatMessage> {
        self.session.chat_messages()
    }

    pub fn deterministic_compaction_summary(&self) -> Result<String, String> {
        let messages = self.compaction_messages();
        if messages.is_empty() {
            return Err("no messages to compact".to_string());
        }
        Ok(build_compaction_summary(&messages))
    }

    pub fn compact_context_with_summary(&mut self, summary: String) -> Result<usize, String> {
        self.session
            .append_compaction(summary)
            .map_err(|error| format!("failed to compact context: {error}"))
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, String> {
        Session::list_default(self.config_path.as_deref())
            .map_err(|error| format!("failed to list sessions: {error}"))
    }

    pub fn message_count(&self) -> usize {
        self.session.message_count()
    }

    pub fn usage_totals(&self) -> TokenUsage {
        self.session.usage_totals()
    }

    pub fn id(&self) -> &str {
        self.session.id()
    }

    pub fn cwd(&self) -> &str {
        self.session.cwd()
    }

    pub fn path(&self) -> String {
        self.session.path().display().to_string()
    }

    pub fn recent_messages(&self, limit: usize) -> Vec<MessagePreview> {
        self.session
            .recent_message_previews(limit)
            .into_iter()
            .map(message_preview)
            .collect()
    }
}

fn normalize_session_cwd(cwd: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(cwd);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|error| format!("failed to resolve current directory: {error}"))?
            .join(path)
    };
    if absolute.is_dir() {
        Ok(absolute.components().collect())
    } else {
        Err(format!(
            "session project directory does not exist: {}",
            absolute.display()
        ))
    }
}

fn message_preview(preview: SessionMessagePreview) -> MessagePreview {
    MessagePreview {
        role: match preview.role {
            MessageRole::User => "user".to_string(),
            MessageRole::Assistant => "assistant".to_string(),
            MessageRole::Tool => "tool".to_string(),
            MessageRole::Error => "error".to_string(),
        },
        content: preview.content,
    }
}

fn build_compaction_summary(messages: &[ChatMessage]) -> String {
    let counts = MessageRoleCounts::from_messages(messages);
    let mut summary = String::new();
    summary.push_str("Context summary\n");
    summary.push_str(&format!("- Compacted messages: {}\n", messages.len()));
    summary.push_str(&format!(
        "- Roles: user={}, assistant={}, tool={}, system={}\n",
        counts.user, counts.assistant, counts.tool, counts.system
    ));

    if let Some(message) = latest_content_for_role(messages, ChatMessageRole::User) {
        summary.push_str("- Latest user request: ");
        summary.push_str(&truncate_single_line(message, 220));
        summary.push('\n');
    }
    if let Some(message) = latest_content_for_role(messages, ChatMessageRole::Assistant) {
        summary.push_str("- Latest assistant result: ");
        summary.push_str(&truncate_single_line(message, 220));
        summary.push('\n');
    }
    if let Some(message) = latest_tool_result(messages) {
        summary.push_str("- Latest tool result: ");
        summary.push_str(&truncate_single_line(&message, 220));
        summary.push('\n');
    }

    let tool_call_count = messages
        .iter()
        .map(|message| message.tool_calls.len())
        .sum::<usize>();
    if tool_call_count > 0 {
        summary.push_str(&format!("- Tool calls requested: {tool_call_count}\n"));
    }

    summary.push_str("Recent timeline:");
    for message in messages
        .iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        summary.push('\n');
        summary.push_str("- ");
        summary.push_str(&summarize_message(message));
    }
    summary
}

#[derive(Default)]
struct MessageRoleCounts {
    user: usize,
    assistant: usize,
    tool: usize,
    system: usize,
}

impl MessageRoleCounts {
    fn from_messages(messages: &[ChatMessage]) -> Self {
        let mut counts = Self::default();
        for message in messages {
            match message.role {
                ChatMessageRole::User => counts.user += 1,
                ChatMessageRole::Assistant => counts.assistant += 1,
                ChatMessageRole::Tool => counts.tool += 1,
                ChatMessageRole::System => counts.system += 1,
            }
        }
        counts
    }
}

fn latest_content_for_role(messages: &[ChatMessage], role: ChatMessageRole) -> Option<&str> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == role && !message.content.trim().is_empty())
        .map(|message| message.content.as_str())
}

fn latest_tool_result(messages: &[ChatMessage]) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        if message.role != ChatMessageRole::Tool {
            return None;
        }
        let name = message.tool_name.as_deref().unwrap_or("tool");
        let status = if message.tool_is_error.unwrap_or(false) {
            "error"
        } else {
            "ok"
        };
        Some(format!("{name} {status}: {}", message.content))
    })
}

fn summarize_message(message: &ChatMessage) -> String {
    let mut summary = String::new();
    summary.push_str(role_label(&message.role));

    if !message.tool_calls.is_empty() {
        let calls = message
            .tool_calls
            .iter()
            .map(|call| {
                let args = if call.arguments.is_empty() {
                    String::new()
                } else {
                    let arguments =
                        serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".to_string());
                    format!(" {arguments}")
                };
                format!("{}{} [{}]", call.name, args, call.id)
            })
            .collect::<Vec<_>>()
            .join(", ");
        summary.push_str(" requested tools: ");
        summary.push_str(&truncate_single_line(&calls, 220));
    }

    if message.role == ChatMessageRole::Tool {
        let name = message.tool_name.as_deref().unwrap_or("tool");
        let status = if message.tool_is_error.unwrap_or(false) {
            "error"
        } else {
            "ok"
        };
        summary.push_str(&format!(" {name} {status}"));
        if let Some(id) = message.tool_call_id.as_deref() {
            summary.push_str(&format!(" [{id}]"));
        }
    }

    if !message.content.trim().is_empty() {
        summary.push_str(": ");
        summary.push_str(&truncate_single_line(&message.content, 180));
    }

    summary
}

fn role_label(role: &ChatMessageRole) -> &'static str {
    match role {
        ChatMessageRole::User => "user",
        ChatMessageRole::Assistant => "assistant",
        ChatMessageRole::Tool => "tool",
        ChatMessageRole::System => "system",
    }
}

fn truncate_single_line(content: &str, max_chars: usize) -> String {
    let mut text = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max_chars {
        return text;
    }

    text = text.chars().take(max_chars.saturating_sub(3)).collect();
    text.push_str("...");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_summary_preserves_structure_and_tool_context() {
        let messages = vec![
            ChatMessage::user("please inspect the manifest"),
            ChatMessage::assistant_tool_calls(
                "checking files",
                vec![ToolCall::new("call_1", "read").with_argument("path", "Cargo.toml")],
            ),
            ChatMessage::tool_result("call_1", "read", "workspace = true", false),
            ChatMessage::assistant("the manifest is a workspace"),
        ];

        let summary = build_compaction_summary(&messages);

        assert!(summary.contains("Context summary"));
        assert!(summary.contains("- Compacted messages: 4"));
        assert!(summary.contains("- Roles: user=1, assistant=2, tool=1, system=0"));
        assert!(summary.contains("- Latest user request: please inspect the manifest"));
        assert!(summary.contains("- Latest assistant result: the manifest is a workspace"));
        assert!(summary.contains("- Latest tool result: read ok: workspace = true"));
        assert!(summary.contains("- Tool calls requested: 1"));
        assert!(summary.contains("assistant requested tools: read"));
        assert!(summary.contains("tool read ok [call_1]: workspace = true"));
    }
}
