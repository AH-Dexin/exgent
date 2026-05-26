use std::sync::Arc;

use exgent_ai::{ChatMessage, ToolCall, ToolDefinition};

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

pub trait ToolExecutor: Sync {
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }

    fn before_tool_call(&self, _call: &ToolCall) -> Option<ToolExecutionResult> {
        None
    }

    fn execute_tool(&self, call: &ToolCall) -> ToolExecutionResult;

    fn after_tool_call(
        &self,
        _call: &ToolCall,
        result: ToolExecutionResult,
    ) -> ToolExecutionResult {
        result
    }
}

#[derive(Clone, Debug, Default)]
pub struct NoTools;

impl ToolExecutor for NoTools {
    fn execute_tool(&self, call: &ToolCall) -> ToolExecutionResult {
        ToolExecutionResult::error(format!("tool is not available: {}", call.name))
    }
}

/// User-facing hooks invoked by [`AgentSession::run_prompt_events_cancellable`]
/// at well-defined boundaries.
///
/// Hooks are best-effort: an implementation can mutate the outgoing message
/// list (`transform_messages`), short-circuit a tool call before it runs
/// (`before_tool_call`), or override a tool's result after it runs
/// (`after_tool_call`). The default implementation is a no-op, so consumers
/// can override only the hooks they care about.
pub trait AgentHooks: Send + Sync {
    /// Inspect or rewrite the chat messages just before they are sent to the
    /// provider. Returning `None` keeps the existing list unchanged.
    fn transform_messages(&self, _messages: &[ChatMessage]) -> Option<Vec<ChatMessage>> {
        None
    }

    /// Decide whether to allow a tool call. Returning `Some(result)` blocks the
    /// tool from running and uses the supplied result as the tool's output.
    fn before_tool_call(&self, _call: &ToolCall) -> Option<ToolExecutionResult> {
        None
    }

    /// Inspect or rewrite a finished tool result. Returning `None` keeps the
    /// original result unchanged.
    fn after_tool_call(
        &self,
        _call: &ToolCall,
        _result: &ToolExecutionResult,
    ) -> Option<ToolExecutionResult> {
        None
    }
}

#[derive(Clone, Debug, Default)]
pub struct NoHooks;

impl AgentHooks for NoHooks {}

/// Shared, type-erased agent hooks suitable for passing into the session.
pub type SharedAgentHooks = Arc<dyn AgentHooks>;
