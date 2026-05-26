use std::{
    env, io,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::Duration,
};

use exgent_ai::ToolCall;

use super::{arguments::required_argument, ToolOutput};
use crate::cancel::CancelToken;

const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BashArgs {
    pub command: String,
}

pub(super) fn execute_bash_call(
    call: &ToolCall,
    project_dir: &Path,
    cancel: &CancelToken,
) -> io::Result<ToolOutput> {
    bash(
        BashArgs {
            command: required_argument(call, "command")?.to_string(),
        },
        project_dir,
        cancel,
    )
}

pub(super) fn bash(
    args: BashArgs,
    project_dir: &Path,
    cancel: &CancelToken,
) -> io::Result<ToolOutput> {
    if args.command.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command must not be empty",
        ));
    }

    let shell = resolve_shell()?;
    let mut command = Command::new(&shell.program);
    command
        .args(shell.args(&args.command))
        .current_dir(project_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    spawn_in_new_process_group(&mut command);
    let mut child = command.spawn()?;
    let child_pid = child.id();

    let cancelled = loop {
        if cancel.is_cancelled() {
            break true;
        }
        match child.try_wait()? {
            Some(_status) => break false,
            None => thread::sleep(POLL_INTERVAL),
        }
    };

    if cancelled {
        kill_process_group(child_pid);
        let _ = child.kill();
    }

    let output = child.wait_with_output()?;

    if cancelled {
        return Ok(ToolOutput {
            tool_name: "bash".to_string(),
            content: format!(
                "{}\n[cancelled]",
                format_command_output(output).trim_end_matches('\n')
            ),
        });
    }

    Ok(ToolOutput {
        tool_name: "bash".to_string(),
        content: format_command_output(output),
    })
}

#[cfg(unix)]
fn spawn_in_new_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // Put the child in its own process group so we can SIGKILL the whole
    // tree (e.g. bash → sleep) on cancellation. `setsid` returns -1 on
    // error; we don't propagate it because failing to detach is not fatal.
    unsafe {
        command.pre_exec(|| {
            libc_setsid();
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn spawn_in_new_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn libc_setsid() {
    extern "C" {
        fn setsid() -> i32;
    }
    unsafe {
        setsid();
    }
}

#[cfg(unix)]
fn kill_process_group(pid: u32) {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    const SIGKILL: i32 = 9;
    // Negative pid targets the process group whose leader has that pid.
    unsafe {
        kill(-(pid as i32), SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_process_group(_pid: u32) {}

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
            &CancelToken::new(),
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
            &CancelToken::new(),
        )
        .unwrap();

        assert_eq!(output.tool_name, "bash");
        assert!(output.content.contains("exgent"));
        assert!(output.content.contains("exit_code: 0"));
    }

    #[test]
    fn bash_cancel_kills_long_running_command() {
        if cfg!(windows) || resolve_shell().is_err() {
            return;
        }

        let cancel = CancelToken::new();
        let signal = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            signal.cancel();
        });

        let output = bash(
            BashArgs {
                command: "sleep 30; echo finished".to_string(),
            },
            &env::current_dir().unwrap(),
            &cancel,
        )
        .unwrap();

        assert!(output.content.contains("[cancelled]"));
        assert!(!output.content.contains("finished"));
    }
}
