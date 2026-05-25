use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::config::config_dir;
use exgent_ai::{ChatMessage, MessageRole as ChatMessageRole, TokenUsage};

const CURRENT_SESSION_VERSION: u32 = 1;

static ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionHeader {
    #[serde(rename = "type")]
    record_type: String,
    version: u32,
    id: String,
    timestamp_ms: u128,
    cwd: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MessageEntry {
    #[serde(rename = "type")]
    record_type: String,
    id: String,
    parent_id: Option<String>,
    timestamp_ms: u128,
    role: MessageRole,
    content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage: Option<TokenUsage>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompactionEntry {
    #[serde(rename = "type")]
    record_type: String,
    id: String,
    parent_id: Option<String>,
    timestamp_ms: u128,
    summary: String,
    compacted_message_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionInfo {
    pub path: PathBuf,
    pub id: String,
    pub cwd: String,
    pub message_count: usize,
    pub preview: Option<String>,
    pub modified_ms: u128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionMessagePreview {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    header: SessionHeader,
    entries: Vec<MessageEntry>,
    compactions: Vec<CompactionEntry>,
    leaf_id: Option<String>,
    file_path: PathBuf,
}

impl Session {
    pub fn create_default(config_path: Option<&str>) -> io::Result<Self> {
        Self::create_in_dir(session_dir(config_path))
    }

    pub fn create_in_dir(session_dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&session_dir)?;
        let id = new_id("sess");
        let file_path = session_dir.join(format!("{id}.jsonl"));
        let cwd = env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .display()
            .to_string();
        let header = SessionHeader {
            record_type: "session".to_string(),
            version: CURRENT_SESSION_VERSION,
            id,
            timestamp_ms: now_ms(),
            cwd,
        };

        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&file_path)?;
        write_json_line(&mut file, &header)?;

        Ok(Self {
            header,
            entries: Vec::new(),
            compactions: Vec::new(),
            leaf_id: None,
            file_path,
        })
    }

    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut header = None;
        let mut entries = Vec::new();
        let mut compactions = Vec::new();
        let mut leaf_id = None;

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let value: serde_json::Value = serde_json::from_str(&line).map_err(invalid_data)?;
            match value.get("type").and_then(serde_json::Value::as_str) {
                Some("session") => {
                    let parsed: SessionHeader =
                        serde_json::from_value(value).map_err(invalid_data)?;
                    header = Some(parsed);
                }
                Some("message") => {
                    let entry: MessageEntry =
                        serde_json::from_value(value).map_err(invalid_data)?;
                    leaf_id = Some(entry.id.clone());
                    entries.push(entry);
                }
                Some("compaction") => {
                    let entry: CompactionEntry =
                        serde_json::from_value(value).map_err(invalid_data)?;
                    leaf_id = Some(entry.id.clone());
                    compactions.push(entry);
                }
                Some(other) => {
                    return Err(invalid_data(format!(
                        "unknown session record type: {other}"
                    )));
                }
                None => return Err(invalid_data("session record missing type")),
            }
        }

        let header = header.ok_or_else(|| invalid_data("session header missing"))?;

        Ok(Self {
            header,
            entries,
            compactions,
            leaf_id,
            file_path: path.to_path_buf(),
        })
    }

    pub fn list_default(config_path: Option<&str>) -> io::Result<Vec<SessionInfo>> {
        let dir = session_dir(config_path);
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                continue;
            }

            let session = match Self::open(&path) {
                Ok(session) => session,
                Err(_) => continue,
            };
            let modified_ms = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(system_time_ms)
                .unwrap_or(session.header.timestamp_ms);

            sessions.push(SessionInfo {
                path,
                id: session.header.id,
                cwd: session.header.cwd,
                message_count: session.entries.len(),
                preview: session
                    .entries
                    .last()
                    .map(|entry| truncate_single_line(&entry.content, 80)),
                modified_ms,
            });
        }

        sessions.sort_by(|left, right| {
            right
                .modified_ms
                .cmp(&left.modified_ms)
                .then_with(|| right.id.cmp(&left.id))
        });
        Ok(sessions)
    }

    pub fn append_user(&mut self, content: impl Into<String>) -> io::Result<()> {
        self.append_message(MessageRole::User, content.into(), None)
    }

    pub fn append_assistant(&mut self, content: impl Into<String>) -> io::Result<()> {
        self.append_message(MessageRole::Assistant, content.into(), None)
    }

    pub fn append_assistant_with_usage(
        &mut self,
        content: impl Into<String>,
        usage: TokenUsage,
    ) -> io::Result<()> {
        self.append_message(MessageRole::Assistant, content.into(), Some(usage))
    }

    pub fn append_compaction(&mut self, summary: impl Into<String>) -> io::Result<usize> {
        let compacted_message_count = self.entries.len();
        if compacted_message_count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no messages to compact",
            ));
        }

        let entry = CompactionEntry {
            record_type: "compaction".to_string(),
            id: new_id("compact"),
            parent_id: self.leaf_id.clone(),
            timestamp_ms: now_ms(),
            summary: summary.into(),
            compacted_message_count,
        };
        self.append_compaction_entry(entry)?;
        Ok(compacted_message_count)
    }

    pub fn message_count(&self) -> usize {
        self.entries.len()
    }

    pub fn usage_totals(&self) -> TokenUsage {
        self.entries
            .iter()
            .filter(|entry| entry.role == MessageRole::Assistant)
            .filter_map(|entry| entry.usage.as_ref())
            .fold(TokenUsage::default(), |mut totals, usage| {
                totals.input = totals.input.saturating_add(usage.input);
                totals.output = totals.output.saturating_add(usage.output);
                totals.cache_read = totals.cache_read.saturating_add(usage.cache_read);
                totals.cache_write = totals.cache_write.saturating_add(usage.cache_write);
                totals
            })
    }

    pub fn recent_message_previews(&self, limit: usize) -> Vec<SessionMessagePreview> {
        let start = self.entries.len().saturating_sub(limit);
        self.entries[start..]
            .iter()
            .map(|entry| SessionMessagePreview {
                role: entry.role.clone(),
                content: entry.content.clone(),
            })
            .collect()
    }

    pub fn chat_messages(&self) -> Vec<ChatMessage> {
        let (summary, entries) = match self.compactions.last() {
            Some(compaction) => (
                Some(compaction.summary.as_str()),
                &self.entries[compaction.compacted_message_count.min(self.entries.len())..],
            ),
            None => (None, self.entries.as_slice()),
        };

        summary
            .into_iter()
            .map(|summary| ChatMessage {
                role: ChatMessageRole::System,
                content: summary.to_string(),
            })
            .chain(entries.iter().map(|entry| ChatMessage {
                role: match entry.role {
                    MessageRole::User => ChatMessageRole::User,
                    MessageRole::Assistant => ChatMessageRole::Assistant,
                },
                content: entry.content.clone(),
            }))
            .collect()
    }

    pub fn id(&self) -> &str {
        &self.header.id
    }

    pub fn path(&self) -> &Path {
        &self.file_path
    }

    fn append_message(
        &mut self,
        role: MessageRole,
        content: String,
        usage: Option<TokenUsage>,
    ) -> io::Result<()> {
        let entry = MessageEntry {
            record_type: "message".to_string(),
            id: new_id("msg"),
            parent_id: self.leaf_id.clone(),
            timestamp_ms: now_ms(),
            role,
            content,
            usage,
        };
        self.append_entry(entry)
    }

    fn append_entry(&mut self, entry: MessageEntry) -> io::Result<()> {
        let mut file = OpenOptions::new().append(true).open(&self.file_path)?;
        write_json_line(&mut file, &entry)?;
        self.leaf_id = Some(entry.id.clone());
        self.entries.push(entry);
        Ok(())
    }

    fn append_compaction_entry(&mut self, entry: CompactionEntry) -> io::Result<()> {
        let mut file = OpenOptions::new().append(true).open(&self.file_path)?;
        write_json_line(&mut file, &entry)?;
        self.leaf_id = Some(entry.id.clone());
        self.compactions.push(entry);
        Ok(())
    }
}

fn session_dir(config_path: Option<&str>) -> PathBuf {
    config_dir(config_path, "sessions")
}

fn write_json_line<T: Serialize>(file: &mut File, value: &T) -> io::Result<()> {
    serde_json::to_writer(&mut *file, value).map_err(invalid_data)?;
    file.write_all(b"\n")
}

fn new_id(prefix: &str) -> String {
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{}", now_ms(), counter)
}

fn now_ms() -> u128 {
    system_time_ms(SystemTime::now()).unwrap_or(0)
}

fn system_time_ms(time: SystemTime) -> Option<u128> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis())
}

fn invalid_data(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
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
    fn writes_and_loads_session_jsonl() {
        let dir = test_dir("writes_and_loads_session_jsonl");
        let _ = fs::remove_dir_all(&dir);

        let mut session = Session::create_in_dir(dir.join("sessions")).unwrap();
        session.append_user("hello").unwrap();
        session
            .append_assistant_with_usage(
                "hi",
                TokenUsage {
                    input: 10,
                    output: 2,
                    cache_read: 4,
                    cache_write: 1,
                },
            )
            .unwrap();

        let loaded = Session::open(session.path()).unwrap();
        assert_eq!(loaded.id(), session.id());
        assert_eq!(loaded.message_count(), 2);
        assert_eq!(
            loaded.entries[1].parent_id,
            Some(loaded.entries[0].id.clone())
        );
        assert_eq!(loaded.compactions.len(), 0);
        assert_eq!(
            loaded.usage_totals(),
            TokenUsage {
                input: 10,
                output: 2,
                cache_read: 4,
                cache_write: 1,
            }
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lists_sessions_newest_first() {
        let dir = test_dir("lists_sessions_newest_first");
        let _ = fs::remove_dir_all(&dir);

        let mut first = Session::create_default(Some(dir.to_str().unwrap())).unwrap();
        first.append_user("first").unwrap();
        let mut second = Session::create_default(Some(dir.to_str().unwrap())).unwrap();
        second.append_user("second").unwrap();

        let sessions = Session::list_default(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id, second.id());
        assert_eq!(sessions[0].preview.as_deref(), Some("second"));
        assert_eq!(sessions[1].id, first.id());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compaction_replaces_prior_context_with_summary() {
        let dir = test_dir("compaction_replaces_prior_context_with_summary");
        let _ = fs::remove_dir_all(&dir);

        let mut session = Session::create_in_dir(dir.join("sessions")).unwrap();
        session.append_user("hello").unwrap();
        session.append_assistant("hi").unwrap();
        let compacted_count = session.append_compaction("summary").unwrap();
        session.append_user("after").unwrap();

        assert_eq!(compacted_count, 2);
        let messages = session.chat_messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, ChatMessageRole::System);
        assert_eq!(messages[0].content, "summary");
        assert_eq!(messages[1].role, ChatMessageRole::User);
        assert_eq!(messages[1].content, "after");

        let loaded = Session::open(session.path()).unwrap();
        assert_eq!(loaded.chat_messages(), messages);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exports_chat_messages_in_order() {
        let dir = test_dir("exports_chat_messages_in_order");
        let _ = fs::remove_dir_all(&dir);

        let mut session = Session::create_in_dir(dir.join("sessions")).unwrap();
        session.append_user("hello").unwrap();
        session.append_assistant("hi").unwrap();

        let messages = session.chat_messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, ChatMessageRole::User);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].role, ChatMessageRole::Assistant);
        assert_eq!(messages[1].content, "hi");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn returns_recent_message_previews() {
        let dir = test_dir("returns_recent_message_previews");
        let _ = fs::remove_dir_all(&dir);

        let mut session = Session::create_in_dir(dir.join("sessions")).unwrap();
        session.append_user("one").unwrap();
        session.append_assistant("two").unwrap();
        session.append_user("three").unwrap();

        let previews = session.recent_message_previews(2);
        assert_eq!(previews.len(), 2);
        assert_eq!(previews[0].role, MessageRole::Assistant);
        assert_eq!(previews[0].content, "two");
        assert_eq!(previews[1].role, MessageRole::User);
        assert_eq!(previews[1].content, "three");

        let _ = fs::remove_dir_all(&dir);
    }

    fn test_dir(name: &str) -> PathBuf {
        env::temp_dir().join(format!("exgent_{name}_{}", new_id("test")))
    }
}
