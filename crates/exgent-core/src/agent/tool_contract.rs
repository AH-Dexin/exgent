use exgent_ai::{ToolCall, ToolDefinition};

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
