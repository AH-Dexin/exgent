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
        "ls" => ToolDefinition::new(
            "ls",
            "List entries in a directory, returning relative paths grouped by file vs. directory.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory to list, relative to the project directory unless absolute. Defaults to the project root."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum number of entries to return. Defaults to 200."
                    }
                },
                "additionalProperties": false
            }),
        )
        .with_label("List directory")
        .with_prompt_snippet("List directory entries (cheaper than bash ls)")
        .with_execution_mode(ToolExecutionMode::Parallel),
        "grep" => ToolDefinition::new(
            "grep",
            "Recursive substring search across files in the project. Skips common binary and VCS directories.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Literal substring to match. Matching is case-sensitive."
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory or file to search. Defaults to the project root."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum number of matches to return. Defaults to 200."
                    },
                    "case_insensitive": {
                        "type": "boolean",
                        "description": "Match case-insensitively."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        )
        .with_label("Grep")
        .with_prompt_snippet("Substring search across files (cheaper than bash grep)")
        .with_execution_mode(ToolExecutionMode::Parallel),
        "find" => ToolDefinition::new(
            "find",
            "Recursively list files whose name contains the given substring. Skips common binary and VCS directories.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Substring matched against each file's name (not the full path)."
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to walk. Defaults to the project root."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum number of paths to return. Defaults to 200."
                    },
                    "case_insensitive": {
                        "type": "boolean",
                        "description": "Match case-insensitively."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        )
        .with_label("Find files")
        .with_prompt_snippet("Find files by name substring")
        .with_execution_mode(ToolExecutionMode::Parallel),
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
