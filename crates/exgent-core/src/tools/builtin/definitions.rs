use exgent_ai::{ToolDefinition, ToolExecutionMode};
use serde_json::json;

pub(super) fn builtin_definition(name: &str) -> ToolDefinition {
    match name {
        "read" => ToolDefinition::new(
            "read",
            "Read text from a file in the current project.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path, relative to the project directory unless absolute."
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Zero-based line offset."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum number of lines to read."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        )
        .with_label("Read file")
        .with_prompt_snippet("Read file contents")
        .with_execution_mode(ToolExecutionMode::Parallel),
        "write" => ToolDefinition::new(
            "write",
            "Create or overwrite a text file in the current project.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path, relative to the project directory unless absolute."
                    },
                    "content": {
                        "type": "string",
                        "description": "Complete file content to write."
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        )
        .with_label("Write file")
        .with_prompt_snippet("Create or overwrite files")
        .with_execution_mode(ToolExecutionMode::Sequential),
        "edit" => ToolDefinition::new(
            "edit",
            "Replace exact text inside an existing file.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path, relative to the project directory unless absolute."
                    },
                    "old_text": {
                        "type": "string",
                        "description": "Exact text to replace."
                    },
                    "new_text": {
                        "type": "string",
                        "description": "Replacement text."
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace every match instead of requiring exactly one match."
                    }
                },
                "required": ["path", "old_text", "new_text"],
                "additionalProperties": false
            }),
        )
        .with_label("Edit file")
        .with_prompt_snippet("Make precise file edits with exact text replacement")
        .with_execution_mode(ToolExecutionMode::Sequential),
        "bash" => ToolDefinition::new(
            "bash",
            "Run a shell command in the current project and return stdout, stderr, and exit code.",
            json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Command to run."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        )
        .with_label("Run command")
        .with_prompt_snippet("Execute shell commands")
        .with_execution_mode(ToolExecutionMode::Sequential),
        _ => ToolDefinition::new(
            name,
            "Execute a registered tool.",
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            }),
        ),
    }
}
