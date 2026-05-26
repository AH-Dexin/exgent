use std::{io, path::Path};

use exgent_ai::{ToolCall, ToolDefinition};

use super::registry::{Tool, ToolRegistry};
use definitions::builtin_definition;
use files::{execute_edit_call, execute_read_call, execute_write_call};
use shell::execute_bash_call;

#[cfg(test)]
use crate::agent::ToolExecutor;

mod arguments;
mod definitions;
mod files;
mod shell;

pub(super) fn register_builtin_tools(registry: &mut ToolRegistry) -> io::Result<()> {
    registry.register(BuiltinTool::new("read", execute_read_call))?;
    registry.register(BuiltinTool::new("write", execute_write_call))?;
    registry.register(BuiltinTool::new("edit", execute_edit_call))?;
    registry.register(BuiltinTool::new("bash", execute_bash_call))?;
    Ok(())
}

struct BuiltinTool {
    definition: ToolDefinition,
    execute: fn(&ToolCall, &Path) -> io::Result<ToolOutput>,
}

impl BuiltinTool {
    fn new(name: &'static str, execute: fn(&ToolCall, &Path) -> io::Result<ToolOutput>) -> Self {
        Self {
            definition: builtin_definition(name),
            execute,
        }
    }
}

impl Tool for BuiltinTool {
    fn name(&self) -> &str {
        &self.definition.name
    }

    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    fn execute_call(&self, call: &ToolCall, project_dir: &Path) -> io::Result<ToolOutput> {
        (self.execute)(call, project_dir)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolOutput {
    pub tool_name: String,
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exposes_builtin_tool_definitions() {
        let definitions = ToolRegistry::default_builtin().definitions();
        let read = definitions
            .iter()
            .find(|definition| definition.name == "read")
            .unwrap();

        assert_eq!(definitions.len(), 4);
        assert_eq!(read.parameters["required"], json!(["path"]));
        assert_eq!(read.parameters["properties"]["path"]["type"], "string");
    }

    #[test]
    fn loads_builtin_tools() {
        let registry = ToolRegistry::default_builtin_in(".");

        assert_eq!(registry.names(), vec!["read", "write", "edit", "bash"]);
    }

    #[test]
    fn tool_executor_returns_error_results() {
        let result =
            ToolRegistry::default_builtin().execute_tool(&ToolCall::new("call_1", "unknown"));

        assert!(result.is_error);
        assert!(result.content.contains("unknown tool"));
    }

    #[test]
    fn rejects_duplicate_tool_registration() {
        let mut registry = ToolRegistry::new();
        registry
            .register(BuiltinTool::new("read", execute_read_call))
            .unwrap();

        let error = registry
            .register(BuiltinTool::new("read", execute_read_call))
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    }
}
