use std::path::{Path, PathBuf};

use exgent_ai::{ChatMessage, DynamicProvider, Model, TokenUsage, ToolCall};

use crate::{
    config::RuntimeOptions,
    model_service::no_model_configured_message,
    session::{MessagePreview, SessionInfo, SessionService},
    system_prompt::build_system_prompt_for_cwd,
    tools::ToolRegistry,
};

use super::{Agent, AgentEvent, NoTools};

pub struct AgentSession {
    agent: Option<Agent<DynamicProvider>>,
    session_service: SessionService,
    tools: ToolRegistry,
    project_dir: PathBuf,
    usage_totals: UsageTotals,
    listeners: Vec<AgentSessionEventListener>,
    next_listener_id: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum TurnSessionRecord {
    Assistant(String),
    AssistantToolCalls {
        content: String,
        calls: Vec<ToolCall>,
    },
    ToolResult {
        id: String,
        name: String,
        content: String,
        is_error: bool,
    },
    Error(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageTotals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentSessionEvent {
    Agent(AgentEvent),
    CompactionStarted { message_count: usize },
    CompactionFinished { compacted_count: usize },
    UsageUpdated(UsageTotals),
    TurnCommitted { message_count: usize },
}

struct AgentSessionEventListener {
    id: usize,
    callback: Box<dyn FnMut(&AgentSessionEvent) + Send>,
}

impl AgentSession {
    pub fn create(
        options: &RuntimeOptions,
        agent: Option<Agent<DynamicProvider>>,
        current_model: Option<&Model>,
        project_dir: PathBuf,
    ) -> Result<Self, String> {
        let session_service = SessionService::create(options, &project_dir)?;
        Ok(Self::from_session_service(
            session_service,
            agent,
            current_model,
            project_dir,
        ))
    }

    pub fn open_existing(
        options: &RuntimeOptions,
        agent: Option<Agent<DynamicProvider>>,
        current_model: Option<&Model>,
        path: &Path,
    ) -> Result<Self, String> {
        let session_service = SessionService::open(options, path)?;
        let project_dir = PathBuf::from(session_service.cwd());
        Ok(Self::from_session_service(
            session_service,
            agent,
            current_model,
            project_dir,
        ))
    }

    fn from_session_service(
        session_service: SessionService,
        agent: Option<Agent<DynamicProvider>>,
        current_model: Option<&Model>,
        project_dir: PathBuf,
    ) -> Self {
        let usage_totals = UsageTotals::from_usage(&session_service.usage_totals(), current_model);

        Self {
            agent,
            session_service,
            tools: ToolRegistry::default_builtin_in(project_dir.clone()),
            project_dir,
            usage_totals,
            listeners: Vec::new(),
            next_listener_id: 0,
        }
    }

    pub fn set_agent(&mut self, agent: Option<Agent<DynamicProvider>>) {
        self.agent = agent;
    }

    pub fn usage_totals(&self) -> &UsageTotals {
        &self.usage_totals
    }

    pub fn subscribe(
        &mut self,
        callback: impl FnMut(&AgentSessionEvent) + Send + 'static,
    ) -> usize {
        let id = self.next_listener_id;
        self.next_listener_id = self.next_listener_id.saturating_add(1);
        self.listeners.push(AgentSessionEventListener {
            id,
            callback: Box::new(callback),
        });
        id
    }

    pub fn unsubscribe(&mut self, id: usize) -> bool {
        let before = self.listeners.len();
        self.listeners.retain(|listener| listener.id != id);
        before != self.listeners.len()
    }

    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.names()
    }

    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    pub fn system_prompt(&self) -> String {
        build_system_prompt_for_cwd(
            &self.tools.definitions(),
            self.selected_model_label().as_deref(),
            "TUI",
            self.project_dir.clone(),
        )
    }

    fn selected_model_label(&self) -> Option<String> {
        self.agent
            .as_ref()
            .map(|agent| format!("{}/{}", agent.model().provider, agent.model().id))
    }

    pub fn message_count(&self) -> usize {
        self.session_service.message_count()
    }

    pub fn id(&self) -> &str {
        self.session_service.id()
    }

    pub fn path(&self) -> String {
        self.session_service.path()
    }

    pub fn recent_messages(&self, limit: usize) -> Vec<MessagePreview> {
        self.session_service.recent_messages(limit)
    }

    pub fn compact_context(&mut self) -> Result<usize, String> {
        let messages = self.session_service.compaction_messages();
        if messages.is_empty() {
            return Err("no messages to compact".to_string());
        }

        self.emit_to_listeners(&AgentSessionEvent::CompactionStarted {
            message_count: messages.len(),
        });
        let summary = self
            .agent
            .as_ref()
            .and_then(|agent| model_compaction_summary(agent, &messages))
            .unwrap_or_else(|| {
                self.session_service
                    .deterministic_compaction_summary()
                    .unwrap_or_else(|_| "Context summary\n- Summary unavailable".to_string())
            });
        let compacted_count = self.session_service.compact_context_with_summary(summary)?;
        self.emit_to_listeners(&AgentSessionEvent::CompactionFinished { compacted_count });
        Ok(compacted_count)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, String> {
        self.session_service.list_sessions()
    }

    pub fn run_prompt_events<F>(&mut self, prompt: &str, emit: &mut F) -> Result<(), String>
    where
        F: FnMut(AgentSessionEvent),
    {
        let Some(agent) = self.agent.as_ref() else {
            let message = no_model_configured_message().to_string();
            self.emit_event(AgentSessionEvent::Agent(AgentEvent::AgentStart), emit);
            self.emit_event(
                AgentSessionEvent::Agent(AgentEvent::MessageStart {
                    role: "assistant".to_string(),
                }),
                emit,
            );
            self.emit_event(
                AgentSessionEvent::Agent(AgentEvent::MessageDelta {
                    delta: message.clone(),
                }),
                emit,
            );
            self.emit_event(
                AgentSessionEvent::Agent(AgentEvent::MessageEnd { content: message }),
                emit,
            );
            self.emit_event(AgentSessionEvent::Agent(AgentEvent::AgentEnd), emit);
            return Ok(());
        };
        let agent = agent.clone();
        let tools = self.tools.clone();

        let mut messages = vec![ChatMessage::system(self.system_prompt())];
        messages.extend(self.session_service.chat_messages());
        messages.push(ChatMessage::user(prompt));

        let mut session_records = Vec::new();
        let mut errors = Vec::new();
        let mut turn_usage = UsageTotals::default();
        agent.run_messages_with_tools_streaming(messages, &tools, &mut |event| {
            match &event {
                AgentEvent::MessageEnd { content } => {
                    session_records.push(TurnSessionRecord::Assistant(content.clone()));
                }
                AgentEvent::AssistantToolCalls { calls } => {
                    push_tool_call_turn_record(&mut session_records, calls.clone());
                }
                AgentEvent::ToolCallStart { .. } => {}
                AgentEvent::ToolCallEnd {
                    id,
                    name,
                    content,
                    is_error,
                } => {
                    session_records.push(TurnSessionRecord::ToolResult {
                        id: id.clone(),
                        name: name.clone(),
                        content: content.clone(),
                        is_error: *is_error,
                    });
                }
                AgentEvent::Usage { usage } => {
                    turn_usage.add_usage(usage, agent.model());
                }
                AgentEvent::Error { message } => {
                    errors.push(message.clone());
                }
                _ => {}
            }
            self.emit_event(AgentSessionEvent::Agent(event), emit);
        });

        let error = (!errors.is_empty()).then(|| errors.join("\n"));
        if let Some(message) = &error {
            if !records_have_tool_results(&session_records) {
                return Err(message.clone());
            }
            session_records.push(TurnSessionRecord::Error(message.clone()));
        }

        self.session_service.append_user(prompt)?;
        self.commit_session_records(session_records, turn_usage.to_usage())?;
        self.usage_totals.add_totals(&turn_usage);
        self.emit_event(
            AgentSessionEvent::UsageUpdated(self.usage_totals.clone()),
            emit,
        );
        self.emit_event(
            AgentSessionEvent::TurnCommitted {
                message_count: self.session_service.message_count(),
            },
            emit,
        );

        match error {
            Some(message) => Err(message),
            None => Ok(()),
        }
    }

    fn commit_session_records(
        &mut self,
        session_records: Vec<TurnSessionRecord>,
        usage: TokenUsage,
    ) -> Result<(), String> {
        let last_assistant_index = session_records.iter().rposition(|record| {
            matches!(
                record,
                TurnSessionRecord::Assistant(_) | TurnSessionRecord::AssistantToolCalls { .. }
            )
        });
        for (index, record) in session_records.into_iter().enumerate() {
            match record {
                TurnSessionRecord::Assistant(content) => {
                    if Some(index) == last_assistant_index {
                        self.session_service
                            .append_assistant_with_usage(content, usage.clone())?;
                    } else {
                        self.session_service.append_assistant(content)?;
                    }
                }
                TurnSessionRecord::AssistantToolCalls { content, calls } => {
                    let usage = (Some(index) == last_assistant_index).then(|| usage.clone());
                    self.session_service
                        .append_assistant_tool_calls(content, calls, usage)?;
                }
                TurnSessionRecord::ToolResult {
                    id,
                    name,
                    content,
                    is_error,
                } => {
                    self.session_service
                        .append_tool_result(id, name, content, is_error)?;
                }
                TurnSessionRecord::Error(content) => {
                    self.session_service.append_error(content)?;
                }
            }
        }
        Ok(())
    }

    pub fn refresh_usage_totals(&mut self, current_model: Option<&Model>) {
        self.usage_totals =
            UsageTotals::from_usage(&self.session_service.usage_totals(), current_model);
    }

    fn emit_event<F>(&mut self, event: AgentSessionEvent, emit: &mut F)
    where
        F: FnMut(AgentSessionEvent),
    {
        for listener in &mut self.listeners {
            (listener.callback)(&event);
        }
        emit(event);
    }

    fn emit_to_listeners(&mut self, event: &AgentSessionEvent) {
        for listener in &mut self.listeners {
            (listener.callback)(event);
        }
    }
}

fn model_compaction_summary(
    agent: &Agent<DynamicProvider>,
    messages: &[ChatMessage],
) -> Option<String> {
    let prompt = build_model_compaction_prompt(messages);
    let events = agent.run_messages_with_tools(
        vec![
            ChatMessage::system(
                "Summarize conversation context for future model turns. Preserve concrete user requests, decisions, file paths, tool results, and unresolved errors. Be concise.",
            ),
            ChatMessage::user(prompt),
        ],
        &NoTools,
    );
    if events
        .iter()
        .any(|event| matches!(event, AgentEvent::Error { .. }))
    {
        return None;
    }

    events.iter().rev().find_map(|event| match event {
        AgentEvent::MessageEnd { content } if !content.trim().is_empty() => {
            Some(content.trim().to_string())
        }
        _ => None,
    })
}

fn build_model_compaction_prompt(messages: &[ChatMessage]) -> String {
    let mut prompt = String::from("Create a replay-safe summary of this conversation:\n");
    for message in messages {
        prompt.push_str("- ");
        prompt.push_str(match message.role {
            exgent_ai::MessageRole::System => "system",
            exgent_ai::MessageRole::User => "user",
            exgent_ai::MessageRole::Assistant => "assistant",
            exgent_ai::MessageRole::Tool => "tool",
        });
        if !message.tool_calls.is_empty() {
            let calls = message
                .tool_calls
                .iter()
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            prompt.push_str(" requested tools: ");
            prompt.push_str(&calls);
        }
        if !message.content.trim().is_empty() {
            prompt.push_str(": ");
            prompt.push_str(&message.content.replace('\n', " "));
        }
        prompt.push('\n');
    }
    prompt
}

fn push_tool_call_turn_record(records: &mut Vec<TurnSessionRecord>, calls: Vec<ToolCall>) {
    match records.last_mut() {
        Some(TurnSessionRecord::AssistantToolCalls {
            calls: existing_calls,
            ..
        }) => {
            existing_calls.extend(calls);
        }
        Some(TurnSessionRecord::Assistant(existing_content)) => {
            let existing_content = std::mem::take(existing_content);
            records.pop();
            records.push(TurnSessionRecord::AssistantToolCalls {
                content: existing_content,
                calls,
            });
        }
        _ => records.push(TurnSessionRecord::AssistantToolCalls {
            content: String::new(),
            calls,
        }),
    }
}

fn records_have_tool_results(records: &[TurnSessionRecord]) -> bool {
    records
        .iter()
        .any(|record| matches!(record, TurnSessionRecord::ToolResult { .. }))
}

impl UsageTotals {
    fn from_usage(usage: &TokenUsage, model: Option<&Model>) -> Self {
        let mut totals = Self::default();
        if let Some(model) = model {
            totals.add_usage(usage, model);
        } else {
            totals.input = usage.input;
            totals.output = usage.output;
            totals.cache_read = usage.cache_read;
            totals.cache_write = usage.cache_write;
        }
        totals
    }

    fn add_usage(&mut self, usage: &TokenUsage, model: &Model) {
        self.input = self.input.saturating_add(usage.input);
        self.output = self.output.saturating_add(usage.output);
        self.cache_read = self.cache_read.saturating_add(usage.cache_read);
        self.cache_write = self.cache_write.saturating_add(usage.cache_write);

        if let Some(cost) = &model.cost {
            self.cost += (usage.input as f64 * cost.input
                + usage.output as f64 * cost.output
                + usage.cache_read as f64 * cost.cache_read
                + usage.cache_write as f64 * cost.cache_write)
                / 1_000_000.0;
        }
    }

    fn add_totals(&mut self, totals: &UsageTotals) {
        self.input = self.input.saturating_add(totals.input);
        self.output = self.output.saturating_add(totals.output);
        self.cache_read = self.cache_read.saturating_add(totals.cache_read);
        self.cache_write = self.cache_write.saturating_add(totals.cache_write);
        self.cost += totals.cost;
    }

    pub fn context_tokens(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    fn to_usage(&self) -> TokenUsage {
        TokenUsage {
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
        }
    }
}
