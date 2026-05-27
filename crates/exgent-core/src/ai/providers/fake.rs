#[cfg(test)]
use super::super::Model;
use super::super::{
    AssistantMessage, MessageRole, ProviderAdapter, ProviderEvent, ProviderRequest, ToolCall,
};

#[derive(Clone, Debug, Default)]
pub struct FakeProvider;

impl ProviderAdapter for FakeProvider {
    fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
        if let Some(tool_result) = request
            .messages
            .last()
            .filter(|message| message.role == MessageRole::Tool)
        {
            let text = format!("fake tool result:\n{}", tool_result.content);
            emit(ProviderEvent::Start);
            emit(ProviderEvent::TextDelta(text.clone()));
            emit(ProviderEvent::Done(Box::new(AssistantMessage {
                model: request.model,
                content: text,
            })));
            return;
        }

        let prompt = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .map(|message| message.content.as_str())
            .unwrap_or("");

        let tool_calls = parse_fake_tool_calls(prompt);
        if !tool_calls.is_empty() {
            for tool_call in tool_calls {
                emit(ProviderEvent::ToolCall(tool_call));
            }
            return;
        }

        let text = if prompt == "history count" {
            format!("fake history count: {}", request.messages.len())
        } else if prompt == "system prompt" {
            request
                .messages
                .iter()
                .find(|message| message.role == MessageRole::System)
                .map(|message| format!("fake system prompt:\n{}", message.content))
                .unwrap_or_else(|| "fake system prompt: missing".to_string())
        } else if prompt.trim().is_empty() {
            "fake response".to_string()
        } else {
            format!("fake response: {prompt}")
        };

        emit(ProviderEvent::Start);
        emit(ProviderEvent::TextDelta(text.clone()));
        emit(ProviderEvent::Done(Box::new(AssistantMessage {
            model: request.model,
            content: text,
        })));
    }
}

#[cfg(test)]
pub fn fake_model() -> Model {
    Model::new("fake", "fake-chat", "fake")
}

fn parse_fake_tool_calls(prompt: &str) -> Vec<ToolCall> {
    let prompt = prompt.trim();
    if let Some(rest) = prompt.strip_prefix("tool read pair ") {
        let paths = rest.split_whitespace().take(2).collect::<Vec<_>>();
        if paths.len() == 2 {
            return paths
                .into_iter()
                .enumerate()
                .map(|(index, path)| {
                    ToolCall::new(format!("fake_tool_read_{}", index + 1), "read")
                        .with_argument("path", path)
                        .with_argument("offset", 0)
                })
                .collect();
        }
    }

    if let Some(path) = prompt.strip_prefix("tool read ") {
        return vec![ToolCall::new("fake_tool_read_1", "read")
            .with_argument("path", path.trim())
            .with_argument("offset", 0)];
    }

    if let Some(rest) = prompt.strip_prefix("tool write ") {
        if let Some((path, content)) = rest.trim().split_once(' ') {
            return vec![ToolCall::new("fake_tool_write_1", "write")
                .with_argument("path", path)
                .with_argument("content", content)];
        }
    }

    if let Some(rest) = prompt.strip_prefix("tool edit ") {
        if let Some((path, rest)) = rest.trim().split_once(' ') {
            if let Some((old_text, new_text)) = rest.split_once(" => ") {
                return vec![ToolCall::new("fake_tool_edit_1", "edit")
                    .with_argument("path", path)
                    .with_argument("old_text", old_text)
                    .with_argument("new_text", new_text)];
            }
        }
    }

    if let Some(command) = prompt.strip_prefix("tool bash ") {
        return vec![
            ToolCall::new("fake_tool_bash_1", "bash").with_argument("command", command.trim())
        ];
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::ChatMessage;

    #[test]
    fn fake_provider_emits_tool_call() {
        let provider = FakeProvider;
        let events = provider.stream(ProviderRequest {
            model: fake_model(),
            messages: vec![ChatMessage::user("tool read Cargo.toml")],
            tools: Vec::new(),
        });

        assert_eq!(
            events,
            vec![ProviderEvent::ToolCall(
                ToolCall::new("fake_tool_read_1", "read")
                    .with_argument("path", "Cargo.toml")
                    .with_argument("offset", 0)
            )]
        );
    }

    #[test]
    fn fake_provider_emits_multiple_tool_calls() {
        let provider = FakeProvider;
        let events = provider.stream(ProviderRequest {
            model: fake_model(),
            messages: vec![ChatMessage::user("tool read pair Cargo.toml Cargo.lock")],
            tools: Vec::new(),
        });

        assert_eq!(
            events,
            vec![
                ProviderEvent::ToolCall(
                    ToolCall::new("fake_tool_read_1", "read")
                        .with_argument("path", "Cargo.toml")
                        .with_argument("offset", 0)
                ),
                ProviderEvent::ToolCall(
                    ToolCall::new("fake_tool_read_2", "read")
                        .with_argument("path", "Cargo.lock")
                        .with_argument("offset", 0)
                ),
            ]
        );
    }

    #[test]
    fn fake_provider_responds_after_tool_result() {
        let provider = FakeProvider;
        let events = provider.stream(ProviderRequest {
            model: fake_model(),
            messages: vec![
                ChatMessage::user("tool read Cargo.toml"),
                ChatMessage::tool("read output"),
            ],
            tools: Vec::new(),
        });

        assert!(events.contains(&ProviderEvent::TextDelta(
            "fake tool result:\nread output".to_string()
        )));
    }

    #[test]
    fn fake_provider_can_observe_history() {
        let provider = FakeProvider;
        let events = provider.stream(ProviderRequest {
            model: fake_model(),
            messages: vec![
                ChatMessage::user("hello"),
                ChatMessage::assistant("hi"),
                ChatMessage::user("history count"),
            ],
            tools: Vec::new(),
        });

        assert!(events.contains(&ProviderEvent::TextDelta(
            "fake history count: 3".to_string()
        )));
    }

    #[test]
    fn fake_provider_ignores_non_terminal_tool_history() {
        let provider = FakeProvider;
        let events = provider.stream(ProviderRequest {
            model: fake_model(),
            messages: vec![
                ChatMessage::user("tool read Cargo.toml"),
                ChatMessage::tool("old tool output"),
                ChatMessage::assistant("done"),
                ChatMessage::user("history count"),
            ],
            tools: Vec::new(),
        });

        assert!(events.contains(&ProviderEvent::TextDelta(
            "fake history count: 4".to_string()
        )));
    }
}
