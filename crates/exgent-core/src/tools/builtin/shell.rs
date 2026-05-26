use std::{
    env, io,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use exgent_ai::ToolCall;

use super::{arguments::required_argument, ToolOutput};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BashArgs {
    pub command: String,
}

pub(super) fn execute_bash_call(call: &ToolCall, project_dir: &Path) -> io::Result<ToolOutput> {
    bash(
        BashArgs {
            command: required_argument(call, "command")?.to_string(),
        },
        project_dir,
    )
}

pub(super) fn bash(args: BashArgs, project_dir: &Path) -> io::Result<ToolOutput> {
    if args.command.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command must not be empty",
        ));
    }

    let shell = resolve_shell()?;
    let output = Command::new(&shell.program)
        .args(shell.args(&args.command))
        .current_dir(project_dir)
        .output()?;

    Ok(ToolOutput {
        tool_name: "bash".to_string(),
        content: format_command_output(output),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_rejects_empty_command() {
        let error = bash(
            BashArgs {
                command: "  ".to_string(),
            },
            &env::temp_dir(),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn bash_runs_when_shell_is_available() {
        if resolve_shell().is_err() {
            return;
        }

        let output = bash(
            BashArgs {
                command: "printf exgent".to_string(),
            },
            &env::current_dir().unwrap(),
        )
        .unwrap();

        assert_eq!(output.tool_name, "bash");
        assert!(output.content.contains("exgent"));
        assert!(output.content.contains("exit_code: 0"));
    }
}
