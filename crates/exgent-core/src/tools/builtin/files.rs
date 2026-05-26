use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use exgent_ai::ToolCall;

use super::{arguments::*, ToolOutput};
use crate::cancel::CancelToken;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReadArgs {
    pub path: String,
    pub offset: usize,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WriteArgs {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EditArgs {
    pub path: String,
    pub old_text: String,
    pub new_text: String,
    pub replace_all: bool,
}

pub(super) fn execute_read_call(
    call: &ToolCall,
    project_dir: &Path,
    _cancel: &CancelToken,
) -> io::Result<ToolOutput> {
    read(
        ReadArgs {
            path: required_argument(call, "path")?.to_string(),
            offset: optional_usize_argument(call, "offset")?.unwrap_or(0),
            limit: optional_usize_argument(call, "limit")?,
        },
        project_dir,
    )
}

pub(super) fn execute_write_call(
    call: &ToolCall,
    project_dir: &Path,
    _cancel: &CancelToken,
) -> io::Result<ToolOutput> {
    write(
        WriteArgs {
            path: required_argument(call, "path")?.to_string(),
            content: required_argument(call, "content")?.to_string(),
        },
        project_dir,
    )
}

pub(super) fn execute_edit_call(
    call: &ToolCall,
    project_dir: &Path,
    _cancel: &CancelToken,
) -> io::Result<ToolOutput> {
    edit(
        EditArgs {
            path: required_argument(call, "path")?.to_string(),
            old_text: required_argument(call, "old_text")?.to_string(),
            new_text: required_argument(call, "new_text")?.to_string(),
            replace_all: optional_bool_argument(call, "replace_all")?.unwrap_or(false),
        },
        project_dir,
    )
}

fn read(args: ReadArgs, project_dir: &Path) -> io::Result<ToolOutput> {
    let path = resolve_path(&args.path, project_dir)?;
    let content = fs::read_to_string(&path)?;
    let selected = select_lines(&content, args.offset, args.limit);

    Ok(ToolOutput {
        tool_name: "read".to_string(),
        content: selected,
    })
}

fn write(args: WriteArgs, project_dir: &Path) -> io::Result<ToolOutput> {
    let path = resolve_path(&args.path, project_dir)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, args.content)?;

    Ok(ToolOutput {
        tool_name: "write".to_string(),
        content: format!("wrote {}", path.display()),
    })
}

fn edit(args: EditArgs, project_dir: &Path) -> io::Result<ToolOutput> {
    if args.old_text.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "old_text must not be empty",
        ));
    }

    let path = resolve_path(&args.path, project_dir)?;
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

fn resolve_path(path: &str, project_dir: &Path) -> io::Result<PathBuf> {
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
        Ok(project_dir.join(path))
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

        let output = read(
            ReadArgs {
                path: path.display().to_string(),
                offset: 1,
                limit: Some(2),
            },
            &dir,
        )
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

        write(
            WriteArgs {
                path: path.display().to_string(),
                content: "hello".to_string(),
            },
            &dir,
        )
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

        edit(
            EditArgs {
                path: path.display().to_string(),
                old_text: "beta".to_string(),
                new_text: "delta".to_string(),
                replace_all: false,
            },
            &dir,
        )
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

        let error = edit(
            EditArgs {
                path: path.display().to_string(),
                old_text: "same".to_string(),
                new_text: "new".to_string(),
                replace_all: false,
            },
            &dir,
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(fs::read_to_string(&path).unwrap(), "same same");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn executes_read_tool_call() {
        let dir = test_dir("executes_read_tool_call");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.txt");
        fs::write(&path, "hello\nworld\n").unwrap();

        let output = execute_read_call(
            &ToolCall::new("call_1", "read")
                .with_argument("path", path.display().to_string())
                .with_argument("offset", 1),
            &dir,
            &CancelToken::new(),
        )
        .unwrap();

        assert_eq!(output.content, "world\n");

        let _ = fs::remove_dir_all(&dir);
    }

    fn test_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("exgent_tools_{name}_{stamp}"))
    }
}
