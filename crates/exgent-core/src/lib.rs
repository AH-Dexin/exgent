//! Core agent loop primitives for exgent.

use std::collections::BTreeMap;

use exgent_ai::{
    ChatMessage, Model, ProviderAdapter, ProviderEvent, ProviderRequest, TokenUsage, ToolCall,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentEvent {
    AgentStart,
    MessageStart {
        role: String,
    },
    MessageDelta {
        delta: String,
    },
    ReasoningDelta {
        delta: String,
    },
    Usage {
        usage: TokenUsage,
    },
    MessageEnd {
        content: String,
    },
    ToolCallStart {
        id: String,
        name: String,
        arguments: BTreeMap<String, String>,
    },
    ToolCallEnd {
        id: String,
        name: String,
        content: String,
        is_error: bool,
    },
    AgentEnd,
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolExecutionResult {
    pub content: String,
    pub is_error: bool,
}

impl ToolExecutionResult {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

pub trait ToolExecutor {
    fn execute_tool(&self, call: &ToolCall) -> ToolExecutionResult;
}

#[derive(Clone, Debug, Default)]
pub struct NoTools;

impl ToolExecutor for NoTools {
    fn execute_tool(&self, call: &ToolCall) -> ToolExecutionResult {
        ToolExecutionResult::error(format!("tool is not available: {}", call.name))
    }
}

#[derive(Clone, Debug)]
pub struct Agent<P> {
    model: Model,
    provider: P,
}

impl<P> Agent<P>
where
    P: ProviderAdapter,
{
    pub fn new(model: Model, provider: P) -> Self {
        Self { model, provider }
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn set_model(&mut self, model: Model) {
        self.model = model;
    }

    pub fn set_provider(&mut self, provider: P) {
        self.provider = provider;
    }

    pub fn run_prompt(&self, prompt: impl Into<String>) -> Vec<AgentEvent> {
        self.run_prompt_with_tools(prompt, &NoTools)
    }

    pub fn run_prompt_with_tools<T>(&self, prompt: impl Into<String>, tools: &T) -> Vec<AgentEvent>
    where
        T: ToolExecutor,
    {
        let mut events = Vec::new();
        self.run_prompt_with_tools_streaming(prompt, tools, &mut |event| events.push(event));
        events
    }

    pub fn run_prompt_with_tools_streaming<T, F>(
        &self,
        prompt: impl Into<String>,
        tools: &T,
        emit: &mut F,
    ) where
        T: ToolExecutor,
        F: FnMut(AgentEvent),
    {
        self.run_messages_with_tools_streaming(vec![ChatMessage::user(prompt)], tools, emit);
    }

    pub fn run_messages_with_tools<T>(
        &self,
        messages: Vec<ChatMessage>,
        tools: &T,
    ) -> Vec<AgentEvent>
    where
        T: ToolExecutor,
    {
        let mut events = Vec::new();
        self.run_messages_with_tools_streaming(messages, tools, &mut |event| events.push(event));
        events
    }

    pub fn run_messages_with_tools_streaming<T, F>(
        &self,
        mut messages: Vec<ChatMessage>,
        tools: &T,
        emit: &mut F,
    ) where
        T: ToolExecutor,
        F: FnMut(AgentEvent),
    {
        emit(AgentEvent::AgentStart);
        const MAX_TOOL_ROUNDS: usize = 8;

        for _ in 0..MAX_TOOL_ROUNDS {
            let request = ProviderRequest {
                model: self.model.clone(),
                messages: messages.clone(),
            };
            let mut content = String::new();
            let mut used_tool = false;

            self.provider
                .stream_events(request, &mut |event| match event {
                    ProviderEvent::Start => {
                        emit(AgentEvent::MessageStart {
                            role: "assistant".to_string(),
                        });
                    }
                    ProviderEvent::TextDelta(delta) => {
                        content.push_str(&delta);
                        emit(AgentEvent::MessageDelta { delta });
                    }
                    ProviderEvent::ReasoningDelta(delta) => {
                        emit(AgentEvent::ReasoningDelta { delta });
                    }
                    ProviderEvent::Usage(usage) => {
                        emit(AgentEvent::Usage { usage });
                    }
                    ProviderEvent::ToolCall(call) => {
                        used_tool = true;
                        emit(AgentEvent::ToolCallStart {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        });
                        let result = tools.execute_tool(&call);
                        emit(AgentEvent::ToolCallEnd {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            content: result.content.clone(),
                            is_error: result.is_error,
                        });
                        messages.push(ChatMessage::tool(result.content));
                    }
                    ProviderEvent::Done(message) => {
                        if content.is_empty() {
                            content = message.content;
                        }
                        if !content.is_empty() {
                            messages.push(ChatMessage::assistant(content.clone()));
                            emit(AgentEvent::MessageEnd {
                                content: content.clone(),
                            });
                        }
                    }
                    ProviderEvent::Error(message) => {
                        emit(AgentEvent::Error { message });
                    }
                });

            if !used_tool {
                emit(AgentEvent::AgentEnd);
                return;
            }
        }

        emit(AgentEvent::Error {
            message: "maximum tool rounds reached".to_string(),
        });
        emit(AgentEvent::AgentEnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exgent_ai::{fake_model, FakeProvider};

    #[derive(Clone, Debug)]
    struct EchoTools;

    impl ToolExecutor for EchoTools {
        fn execute_tool(&self, call: &ToolCall) -> ToolExecutionResult {
            ToolExecutionResult::ok(format!("{} {:?}", call.name, call.arguments))
        }
    }

    #[test]
    fn executes_tool_call_and_continues_provider() {
        let agent = Agent::new(fake_model(), FakeProvider);
        let events = agent.run_prompt_with_tools("tool read Cargo.toml", &EchoTools);

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCallStart { name, .. } if name == "read"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCallEnd {
                name,
                is_error: false,
                ..
            } if name == "read"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::MessageEnd { content } if content.contains("fake tool result:")
        )));
    }
}
