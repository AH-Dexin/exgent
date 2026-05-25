use std::{
    env, fs, io,
    path::PathBuf,
    process::{Command, Output},
    sync::Arc,
};

use exgent_ai::ToolCall;
use exgent_core::{ToolExecutionResult, ToolExecutor};

#[derive(Clone)]
pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn default_builtin() -> Self {
        let mut registry = Self::new();
        registry
            .register(BuiltinTool::new("read", execute_read_call))
            .expect("built-in tool names must be unique");
        registry
            .register(BuiltinTool::new("write", execute_write_call))
            .expect("built-in tool names must be unique");
        registry
            .register(BuiltinTool::new("edit", execute_edit_call))
            .expect("built-in tool names must be unique");
        registry
            .register(BuiltinTool::new("bash", execute_bash_call))
            .expect("built-in tool names must be unique");
        registry
    }

    pub fn new() -> Self {
        Self { tools: Vec::new() }
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

    pub fn execute_builtin(&self, request: ToolRequest) -> io::Result<ToolOutput> {
        match request {
            ToolRequest::Read(args) => read(args),
            ToolRequest::Write(args) => write(args),
            ToolRequest::Edit(args) => edit(args),
            ToolRequest::Bash(args) => bash(args),
        }
    }

    pub fn execute_call(&self, call: &ToolCall) -> io::Result<ToolOutput> {
        let Some(tool) = self.tools.iter().find(|tool| tool.name() == call.name) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown tool: {}", call.name),
            ));
        };

        tool.execute_call(call)
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
    fn execute_call(&self, call: &ToolCall) -> io::Result<ToolOutput>;
}

struct BuiltinTool {
    name: &'static str,
    execute: fn(&ToolCall) -> io::Result<ToolOutput>,
}

impl BuiltinTool {
    fn new(name: &'static str, execute: fn(&ToolCall) -> io::Result<ToolOutput>) -> Self {
        Self { name, execute }
    }
}

impl Tool for BuiltinTool {
    fn name(&self) -> &str {
        self.name
    }

    fn execute_call(&self, call: &ToolCall) -> io::Result<ToolOutput> {
        (self.execute)(call)
    }
}

impl ToolExecutor for ToolRegistry {
    fn execute_tool(&self, call: &ToolCall) -> ToolExecutionResult {
        match self.execute_call(call) {
            Ok(output) => ToolExecutionResult::ok(output.content),
            Err(error) => ToolExecutionResult::error(error.to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolRequest {
    Read(ReadArgs),
    Write(WriteArgs),
    Edit(EditArgs),
    Bash(BashArgs),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadArgs {
    pub path: String,
    pub offset: usize,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteArgs {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditArgs {
    pub path: String,
    pub old_text: String,
    pub new_text: String,
    pub replace_all: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BashArgs {
    pub command: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolOutput {
    pub tool_name: String,
    pub content: String,
}

fn execute_read_call(call: &ToolCall) -> io::Result<ToolOutput> {
    read(ReadArgs {
        path: required_argument(call, "path")?.to_string(),
        offset: optional_usize_argument(call, "offset")?.unwrap_or(0),
        limit: optional_usize_argument(call, "limit")?,
    })
}

fn execute_write_call(call: &ToolCall) -> io::Result<ToolOutput> {
    write(WriteArgs {
        path: required_argument(call, "path")?.to_string(),
        content: required_argument(call, "content")?.to_string(),
    })
}

fn execute_edit_call(call: &ToolCall) -> io::Result<ToolOutput> {
    edit(EditArgs {
        path: required_argument(call, "path")?.to_string(),
        old_text: required_argument(call, "old_text")?.to_string(),
        new_text: required_argument(call, "new_text")?.to_string(),
        replace_all: optional_bool_argument(call, "replace_all")?.unwrap_or(false),
    })
}

fn execute_bash_call(call: &ToolCall) -> io::Result<ToolOutput> {
    bash(BashArgs {
        command: required_argument(call, "command")?.to_string(),
    })
}

fn read(args: ReadArgs) -> io::Result<ToolOutput> {
    let path = resolve_path(&args.path)?;
    let content = fs::read_to_string(&path)?;
    let selected = select_lines(&content, args.offset, args.limit);

    Ok(ToolOutput {
        tool_name: "read".to_string(),
        content: selected,
    })
}

fn write(args: WriteArgs) -> io::Result<ToolOutput> {
    let path = resolve_path(&args.path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, args.content)?;

    Ok(ToolOutput {
        tool_name: "write".to_string(),
        content: format!("wrote {}", path.display()),
    })
}

fn edit(args: EditArgs) -> io::Result<ToolOutput> {
    if args.old_text.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "old_text must not be empty",
        ));
    }

    let path = resolve_path(&args.path)?;
    let original = fs::read_to_string(&path)?;
    let replacement_count = original.matches(&args.old_text).count();

    if replacement_count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "old_text was not found",
        ));
    }

    if !args.replace_all && replacement_count > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "old_text matched more than once; set replace_all to true",
        ));
    }

    let updated = if args.replace_all {
        original.replace(&args.old_text, &args.new_text)
    } else {
        original.replacen(&args.old_text, &args.new_text, 1)
    };

    fs::write(&path, updated)?;

    Ok(ToolOutput {
        tool_name: "edit".to_string(),
        content: format!(
            "edited {} replacement(s) in {}",
            replacement_count,
            path.display()
        ),
    })
}

fn bash(args: BashArgs) -> io::Result<ToolOutput> {
    if args.command.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command must not be empty",
        ));
    }

    let shell = resolve_shell()?;
    let output = Command::new(&shell.program)
        .args(shell.args(&args.command))
        .output()?;

    Ok(ToolOutput {
        tool_name: "bash".to_string(),
        content: format_command_output(output),
    })
}

fn select_lines(content: &str, offset: usize, limit: Option<usize>) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if offset >= lines.len() {
        return String::new();
    }

    let end = match limit {
        Some(limit) => offset.saturating_add(limit).min(lines.len()),
        None => lines.len(),
    };

    let mut selected = lines[offset..end].join("\n");
    if content.ends_with('\n') && end == lines.len() {
        selected.push('\n');
    }
    selected
}

fn resolve_path(path: &str) -> io::Result<PathBuf> {
    if path.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path must not be empty",
        ));
    }

    let expanded = expand_file_url(path);
    let expanded = expand_home(&expanded)?;
    let path = PathBuf::from(expanded);

    if path.is_absolute() {
        Ok(path)
    } else {
        env::current_dir().map(|cwd| cwd.join(path))
    }
}

fn expand_file_url(path: &str) -> String {
    let Some(rest) = path.strip_prefix("file://") else {
        return path.to_string();
    };

    if cfg!(windows) && rest.len() > 3 && rest.starts_with('/') && rest.as_bytes()[2] == b':' {
        rest[1..].to_string()
    } else {
        rest.to_string()
    }
}

fn expand_home(path: &str) -> io::Result<String> {
    if path == "~" {
        return home_dir()
            .map(|home| home.display().to_string())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "home directory was not found")
            });
    }

    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        return home_dir()
            .map(|home| home.join(rest).display().to_string())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "home directory was not found")
            });
    }

    Ok(path.to_string())
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedShell {
    program: PathBuf,
    mode: ShellMode,
}

impl ResolvedShell {
    fn args<'a>(&self, command: &'a str) -> Vec<&'a str> {
        match self.mode {
            ShellMode::Bash => vec!["-lc", command],
            ShellMode::Sh => vec!["-c", command],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ShellMode {
    Bash,
    Sh,
}

fn resolve_shell() -> io::Result<ResolvedShell> {
    if let Some(path) = env::var_os("EXGENT_SHELL").map(PathBuf::from) {
        return Ok(ResolvedShell {
            program: path,
            mode: ShellMode::Bash,
        });
    }

    if cfg!(windows) {
        return resolve_windows_shell();
    }

    if PathBuf::from("/bin/bash").is_file() {
        return Ok(ResolvedShell {
            program: PathBuf::from("/bin/bash"),
            mode: ShellMode::Bash,
        });
    }

    if let Some(path) = find_on_path("bash") {
        return Ok(ResolvedShell {
            program: path,
            mode: ShellMode::Bash,
        });
    }

    if let Some(path) = find_on_path("sh") {
        return Ok(ResolvedShell {
            program: path,
            mode: ShellMode::Sh,
        });
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "bash-compatible shell was not found",
    ))
}

fn resolve_windows_shell() -> io::Result<ResolvedShell> {
    for candidate in [
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files\Git\usr\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Ok(ResolvedShell {
                program: path,
                mode: ShellMode::Bash,
            });
        }
    }

    for name in ["bash.exe", "bash"] {
        if let Some(path) = find_on_path(name) {
            return Ok(ResolvedShell {
                program: path,
                mode: ShellMode::Bash,
            });
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "Git Bash or bash.exe was not found",
    ))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn format_command_output(output: Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut content = String::new();

    if !stdout.is_empty() {
        content.push_str("stdout:\n");
        content.push_str(stdout.trim_end_matches(['\r', '\n']));
        content.push('\n');
    }

    if !stderr.is_empty() {
        content.push_str("stderr:\n");
        content.push_str(stderr.trim_end_matches(['\r', '\n']));
        content.push('\n');
    }

    let status = output.status.code().map_or_else(
        || "terminated by signal".to_string(),
        |code| code.to_string(),
    );
    content.push_str("exit_code: ");
    content.push_str(&status);

    content
}

fn required_argument<'a>(call: &'a ToolCall, name: &str) -> io::Result<&'a str> {
    call.arguments
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("missing required argument: {name}"),
            )
        })
}

fn optional_usize_argument(call: &ToolCall, name: &str) -> io::Result<Option<usize>> {
    call.arguments
        .get(name)
        .map(|value| {
            value.parse::<usize>().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid numeric argument {name}: {error}"),
                )
            })
        })
        .transpose()
}

fn optional_bool_argument(call: &ToolCall, name: &str) -> io::Result<Option<bool>> {
    call.arguments
        .get(name)
        .map(|value| match value.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid boolean argument {name}: {value}"),
            )),
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_selected_lines() {
        let dir = test_dir("reads_selected_lines");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.txt");
        fs::write(&path, "one\ntwo\nthree\nfour\n").unwrap();

        let registry = ToolRegistry::default_builtin();
        let output = registry
            .execute_builtin(ToolRequest::Read(ReadArgs {
                path: path.display().to_string(),
                offset: 1,
                limit: Some(2),
            }))
            .unwrap();

        assert_eq!(output.tool_name, "read");
        assert_eq!(output.content, "two\nthree");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_parent_directories() {
        let dir = test_dir("writes_parent_directories");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("sample.txt");

        ToolRegistry::default_builtin()
            .execute_builtin(ToolRequest::Write(WriteArgs {
                path: path.display().to_string(),
                content: "hello".to_string(),
            }))
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn edits_single_match() {
        let dir = test_dir("edits_single_match");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.txt");
        fs::write(&path, "alpha beta gamma").unwrap();

        ToolRegistry::default_builtin()
            .execute_builtin(ToolRequest::Edit(EditArgs {
                path: path.display().to_string(),
                old_text: "beta".to_string(),
                new_text: "delta".to_string(),
                replace_all: false,
            }))
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "alpha delta gamma");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn edit_requires_replace_all_for_multiple_matches() {
        let dir = test_dir("edit_requires_replace_all_for_multiple_matches");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.txt");
        fs::write(&path, "same same").unwrap();

        let error = ToolRegistry::default_builtin()
            .execute_builtin(ToolRequest::Edit(EditArgs {
                path: path.display().to_string(),
                old_text: "same".to_string(),
                new_text: "new".to_string(),
                replace_all: false,
            }))
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(fs::read_to_string(&path).unwrap(), "same same");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bash_rejects_empty_command() {
        let error = ToolRegistry::default_builtin()
            .execute_builtin(ToolRequest::Bash(BashArgs {
                command: "  ".to_string(),
            }))
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn bash_runs_when_shell_is_available() {
        if resolve_shell().is_err() {
            return;
        }

        let output = ToolRegistry::default_builtin()
            .execute_builtin(ToolRequest::Bash(BashArgs {
                command: "printf exgent".to_string(),
            }))
            .unwrap();

        assert_eq!(output.tool_name, "bash");
        assert!(output.content.contains("exgent"));
        assert!(output.content.contains("exit_code: 0"));
    }

    #[test]
    fn executes_read_tool_call() {
        let dir = test_dir("executes_read_tool_call");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.txt");
        fs::write(&path, "hello\nworld\n").unwrap();

        let output = ToolRegistry::default_builtin()
            .execute_call(
                &ToolCall::new("call_1", "read")
                    .with_argument("path", path.display().to_string())
                    .with_argument("offset", "1"),
            )
            .unwrap();

        assert_eq!(output.content, "world\n");

        let _ = fs::remove_dir_all(&dir);
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

    fn test_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("exgent_tools_{name}_{stamp}"))
    }
}
