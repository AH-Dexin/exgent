use std::{
    env, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use exgent_ai::{ToolCall, ToolDefinition};

use crate::{
    agent::{ToolExecutionResult, ToolExecutor},
    cancel::CancelToken,
};

use super::builtin::{register_builtin_tools, ToolOutput};

#[derive(Clone)]
pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
    pub(super) project_dir: Arc<PathBuf>,
    cancel: CancelToken,
}

impl ToolRegistry {
    pub fn default_builtin() -> Self {
        Self::default_builtin_in(current_working_directory())
    }

    pub fn default_builtin_in(project_dir: impl Into<PathBuf>) -> Self {
        let mut registry = Self::new_in(project_dir);
        register_builtin_tools(&mut registry).expect("built-in tool names must be unique");
        registry
    }

    #[cfg(test)]
    pub fn new() -> Self {
        Self::new_in(current_working_directory())
    }

    pub fn new_in(project_dir: impl Into<PathBuf>) -> Self {
        Self {
            tools: Vec::new(),
            project_dir: Arc::new(project_dir.into()),
            cancel: CancelToken::new(),
        }
    }

    pub fn with_cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }

    pub fn register(&mut self, tool: impl Tool + 'static) -> io::Result<()> {
        let name = tool.name().to_string();
        if self.tools.iter().any(|existing| existing.name() == name) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("tool already registered: {name}"),
            ));
        }
        self.tools.push(Arc::new(tool));
        Ok(())
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.iter().map(|tool| tool.name()).collect()
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|tool| tool.definition()).collect()
    }

    pub fn execute_call(&self, call: &ToolCall) -> io::Result<ToolOutput> {
        let Some(tool) = self.tools.iter().find(|tool| tool.name() == call.name) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown tool: {}", call.name),
            ));
        };

        tool.execute_call(call, &self.project_dir, &self.cancel)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::default_builtin()
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolRegistry")
            .field("tools", &self.names())
            .finish()
    }
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn definition(&self) -> ToolDefinition;
    fn execute_call(
        &self,
        call: &ToolCall,
        project_dir: &Path,
        cancel: &CancelToken,
    ) -> io::Result<ToolOutput>;
}

impl ToolExecutor for ToolRegistry {
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.definitions()
    }

    fn execute_tool(&self, call: &ToolCall) -> ToolExecutionResult {
        match self.execute_call(call) {
            Ok(output) => ToolExecutionResult::ok(output.content),
            Err(error) => ToolExecutionResult::error(error.to_string()),
        }
    }
}

fn current_working_directory() -> PathBuf {
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
