use std::path::Path;

use exgent_ai::{ChatMessage, MessageRole as ChatMessageRole, TokenUsage};

use crate::{
    cli::CliOptions,
    session::{MessageRole, Session, SessionInfo, SessionMessagePreview},
};

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
    pub fn create(options: &CliOptions) -> Result<Self, String> {
        let session = Session::create_default(options.config_path.as_deref())
            .map_err(|error| format!("failed to create session: {error}"))?;
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

    pub fn compact_context(&mut self) -> Result<usize, String> {
        let messages = self.session.chat_messages();
        if messages.is_empty() {
            return Err("no messages to compact".to_string());
        }

        let summary = build_compaction_summary(&messages);
        self.session
            .append_compaction(summary)
            .map_err(|error| format!("failed to compact context: {error}"))
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, String> {
        Session::list_default(self.config_path.as_deref())
            .map_err(|error| format!("failed to list sessions: {error}"))
    }

    pub fn start_new_session(&mut self) -> Result<(), String> {
        self.session = Session::create_default(self.config_path.as_deref())
            .map_err(|error| format!("failed to create session: {error}"))?;
        Ok(())
    }

    pub fn open_session(&mut self, path: &Path) -> Result<(), String> {
        self.session =
            Session::open(path).map_err(|error| format!("failed to open session: {error}"))?;
        Ok(())
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

fn message_preview(preview: SessionMessagePreview) -> MessagePreview {
    MessagePreview {
        role: match preview.role {
            MessageRole::User => "user".to_string(),
            MessageRole::Assistant => "assistant".to_string(),
        },
        content: preview.content,
    }
}

fn build_compaction_summary(messages: &[ChatMessage]) -> String {
    let mut summary = format!("Context summary for {} message(s):", messages.len());
    for message in messages
        .iter()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        summary.push('\n');
        summary.push_str("- ");
        summary.push_str(role_label(&message.role));
        summary.push_str(": ");
        summary.push_str(&truncate_single_line(&message.content, 160));
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
