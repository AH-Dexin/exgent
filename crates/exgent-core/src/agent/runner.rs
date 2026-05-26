use exgent_ai::{ChatMessage, Model, ProviderAdapter};

use super::{agent_loop, AgentEvent, ToolExecutor};

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

    #[cfg(test)]
    pub fn run_prompt_with_tools<T>(&self, prompt: impl Into<String>, tools: &T) -> Vec<AgentEvent>
    where
        T: ToolExecutor,
    {
        let mut events = Vec::new();
        self.run_prompt_with_tools_streaming(prompt, tools, &mut |event| events.push(event));
        events
    }

    #[cfg(test)]
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
        messages: Vec<ChatMessage>,
        tools: &T,
        emit: &mut F,
    ) where
        T: ToolExecutor,
        F: FnMut(AgentEvent),
    {
        agent_loop::run_messages_with_tools_streaming(
            &self.model,
            &self.provider,
            messages,
            tools,
            emit,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exgent_ai::{
        fake_model, AssistantMessage, FakeProvider, MessageRole, ProviderEvent, ProviderRequest,
        ToolCall, ToolDefinition, ToolExecutionMode,
    };

    use crate::agent::ToolExecutionResult;

    #[derive(Clone, Debug)]
    struct EchoTools;

    impl ToolExecutor for EchoTools {
        fn execute_tool(&self, call: &ToolCall) -> ToolExecutionResult {
            ToolExecutionResult::ok(format!("{} {:?}", call.name, call.arguments))
        }
    }

    #[derive(Clone, Debug)]
    struct HookedTools;

    impl ToolExecutor for HookedTools {
        fn before_tool_call(&self, call: &ToolCall) -> Option<ToolExecutionResult> {
            (call.name == "read").then(|| ToolExecutionResult::error("blocked before execution"))
        }

        fn execute_tool(&self, call: &ToolCall) -> ToolExecutionResult {
            ToolExecutionResult::ok(format!("executed {}", call.name))
        }

        fn after_tool_call(
            &self,
            _call: &ToolCall,
            mut result: ToolExecutionResult,
        ) -> ToolExecutionResult {
            result.content.push_str(" + after hook");
            result
        }
    }

    #[derive(Clone, Debug)]
    struct TwoToolProvider;

    impl ProviderAdapter for TwoToolProvider {
        fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
            if request
                .messages
                .iter()
                .any(|message| message.role == MessageRole::Tool)
            {
                emit(ProviderEvent::Start);
                emit(ProviderEvent::Done(Box::new(AssistantMessage {
                    model: request.model,
                    content: "done".to_string(),
                })));
                return;
            }

            emit(ProviderEvent::ToolCall(ToolCall::new("call_1", "panic")));
            emit(ProviderEvent::ToolCall(ToolCall::new("call_2", "ok")));
        }
    }

    #[derive(Clone, Debug)]
    struct ParallelPanicTools;

    impl ToolExecutor for ParallelPanicTools {
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![
                ToolDefinition::new("panic", "panic", serde_json::json!({}))
                    .with_execution_mode(ToolExecutionMode::Parallel),
                ToolDefinition::new("ok", "ok", serde_json::json!({}))
                    .with_execution_mode(ToolExecutionMode::Parallel),
            ]
        }

        fn execute_tool(&self, call: &ToolCall) -> ToolExecutionResult {
            if call.name == "panic" {
                panic!("intentional tool panic");
            }
            ToolExecutionResult::ok("ok")
        }
    }

    #[derive(Clone, Debug)]
    struct ToolThenErrorProvider;

    impl ProviderAdapter for ToolThenErrorProvider {
        fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
            emit(ProviderEvent::Start);
            emit(ProviderEvent::ToolCall(ToolCall::new("call_1", "danger")));
            emit(ProviderEvent::Error("provider failed".to_string()));
            emit(ProviderEvent::Done(Box::new(AssistantMessage {
                model: request.model,
                content: "should be ignored".to_string(),
            })));
        }
    }

    #[derive(Clone, Debug)]
    struct TextAndToolProvider;

    impl ProviderAdapter for TextAndToolProvider {
        fn stream_events(&self, request: ProviderRequest, emit: &mut dyn FnMut(ProviderEvent)) {
            if request
                .messages
                .iter()
                .any(|message| message.role == MessageRole::Tool)
            {
                emit(ProviderEvent::Start);
                emit(ProviderEvent::Done(Box::new(AssistantMessage {
                    model: request.model,
                    content: "done".to_string(),
                })));
                return;
            }

            emit(ProviderEvent::Start);
            emit(ProviderEvent::TextDelta("checking".to_string()));
            emit(ProviderEvent::ToolCall(ToolCall::new("call_1", "read")));
            emit(ProviderEvent::Done(Box::new(AssistantMessage {
                model: request.model,
                content: String::new(),
            })));
        }
    }

    #[derive(Clone, Debug)]
    struct PanicIfExecutedTools;

    impl ToolExecutor for PanicIfExecutedTools {
        fn execute_tool(&self, call: &ToolCall) -> ToolExecutionResult {
            panic!(
                "tool should not execute after provider error: {}",
                call.name
            );
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

    #[test]
    fn tool_hooks_can_block_and_finalize_execution() {
        let agent = Agent::new(fake_model(), FakeProvider);
        let events = agent.run_prompt_with_tools("tool read Cargo.toml", &HookedTools);

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCallEnd {
                content,
                is_error: true,
                ..
            } if content == "blocked before execution + after hook"
        )));
    }

    #[test]
    fn parallel_tool_panic_preserves_original_call_id() {
        let agent = Agent::new(fake_model(), TwoToolProvider);
        let events = agent.run_prompt_with_tools("run tools", &ParallelPanicTools);

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCallStart { id, name, .. } if id == "call_1" && name == "panic"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCallEnd {
                id,
                name,
                is_error: true,
                ..
            } if id == "call_1" && name == "panic"
        )));
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolCallEnd { id, .. } if id == "panic")));
    }

    #[test]
    fn tool_only_turns_emit_balanced_message_events() {
        let agent = Agent::new(fake_model(), FakeProvider);
        let events = agent.run_prompt_with_tools("tool read Cargo.toml", &EchoTools);

        let starts = events
            .iter()
            .filter(|event| matches!(event, AgentEvent::MessageStart { .. }))
            .count();
        let ends = events
            .iter()
            .filter(|event| matches!(event, AgentEvent::MessageEnd { .. }))
            .count();

        assert_eq!(starts, ends);
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::MessageEnd { content } if content.is_empty()
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::AssistantToolCalls { calls } if calls.len() == 1 && calls[0].name == "read"
        )));
    }

    #[test]
    fn provider_errors_prevent_tool_execution() {
        let agent = Agent::new(fake_model(), ToolThenErrorProvider);
        let events = agent.run_prompt_with_tools("run tools", &PanicIfExecutedTools);

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::Error { message } if message == "provider failed"
        )));
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::AssistantToolCalls { .. })));
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::MessageEnd { .. })));
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolCallStart { .. })));
        assert_eq!(events.last(), Some(&AgentEvent::AgentEnd));
    }

    #[test]
    fn mixed_text_and_tool_turns_emit_tool_metadata_without_duplicate_content() {
        let agent = Agent::new(fake_model(), TextAndToolProvider);
        let events = agent.run_prompt_with_tools("inspect", &EchoTools);

        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentEvent::MessageEnd { content } if content == "checking"
                ))
                .count(),
            1
        );
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::AssistantToolCalls { calls } if calls.len() == 1 && calls[0].id == "call_1"
        )));
    }
}
