use crate::ai::{TokenUsage, ToolArguments, ToolCall};

#[derive(Clone, Debug, PartialEq)]
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
    AssistantToolCalls {
        calls: Vec<ToolCall>,
    },
    ToolCallStart {
        id: String,
        name: String,
        arguments: ToolArguments,
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
