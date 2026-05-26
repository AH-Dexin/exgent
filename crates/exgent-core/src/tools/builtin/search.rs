use std::{
    fs::{self, DirEntry},
    io,
    path::{Component, Path, PathBuf},
};

use exgent_ai::ToolCall;

use super::{
    arguments::{optional_bool_argument, optional_string_argument, optional_usize_argument},
    ToolOutput,
};
use crate::cancel::CancelToken;

const DEFAULT_LIMIT: usize = 200;
const MAX_FILE_BYTES_FOR_GREP: u64 = 2 * 1024 * 1024;
const SKIP_DIRECTORIES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
];

pub(super) fn execute_ls_call(
    call: &ToolCall,
    project_dir: &Path,
    _cancel: &CancelToken,
) -> io::Result<ToolOutput> {
    let raw = optional_string_argument(call, "path")?.unwrap_or(".");
    let limit = optional_usize_argument(call, "limit")?.unwrap_or(DEFAULT_LIMIT);
    let target = resolve_path(project_dir, raw);

    if !target.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "not a directory: {}",
                display_relative(project_dir, &target)
            ),
        ));
    }

    let mut entries = fs::read_dir(&target)?
        .collect::<io::Result<Vec<DirEntry>>>()?
        .into_iter()
        .filter_map(|entry| classify_entry(&entry, project_dir).ok())
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.is_dir
            .cmp(&right.is_dir)
            .reverse()
            .then_with(|| left.display.cmp(&right.display))
    });

    let truncated = entries.len() > limit;
    entries.truncate(limit);

    let mut content = String::new();
    let header = display_relative(project_dir, &target);
    content.push_str(&format!("dir: {header}\n"));
    for entry in &entries {
        if entry.is_dir {
            content.push_str(&format!("  {}/\n", entry.display));
        } else {
            content.push_str(&format!("  {}\n", entry.display));
        }
    }
    if truncated {
        content.push_str(&format!("(truncated to first {limit} entries)\n"));
    } else if entries.is_empty() {
        content.push_str("(empty)\n");
    }

    Ok(ToolOutput {
        tool_name: "ls".to_string(),
        content,
    })
}

pub(super) fn execute_find_call(
    call: &ToolCall,
    project_dir: &Path,
    cancel: &CancelToken,
) -> io::Result<ToolOutput> {
    let query = required_query(call)?;
    let limit = optional_usize_argument(call, "limit")?.unwrap_or(DEFAULT_LIMIT);
    let case_insensitive = optional_bool_argument(call, "case_insensitive")?.unwrap_or(false);
    let root = resolve_path(
        project_dir,
        optional_string_argument(call, "path")?.unwrap_or("."),
    );

    if !root.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("not found: {}", display_relative(project_dir, &root)),
        ));
    }

    let needle = if case_insensitive {
        query.to_lowercase()
    } else {
        query.to_string()
    };
    let matches = needle_match_factory(&needle, case_insensitive);

    let mut results = Vec::new();
    let mut truncated = false;
    walk_dir(&root, cancel, &mut |path, file_name| {
        if matches(file_name) {
            results.push(display_relative(project_dir, path));
            if results.len() >= limit {
                truncated = true;
                return WalkControl::Stop;
            }
        }
        WalkControl::Continue
    })?;

    let mut content = String::new();
    if results.is_empty() {
        content.push_str("(no matches)\n");
    } else {
        for path in &results {
            content.push_str(path);
            content.push('\n');
        }
    }
    if truncated {
        content.push_str(&format!("(truncated to first {limit} matches)\n"));
    }

    Ok(ToolOutput {
        tool_name: "find".to_string(),
        content,
    })
}

pub(super) fn execute_grep_call(
    call: &ToolCall,
    project_dir: &Path,
    cancel: &CancelToken,
) -> io::Result<ToolOutput> {
    let query = required_query(call)?;
    let limit = optional_usize_argument(call, "limit")?.unwrap_or(DEFAULT_LIMIT);
    let case_insensitive = optional_bool_argument(call, "case_insensitive")?.unwrap_or(false);
    let root = resolve_path(
        project_dir,
        optional_string_argument(call, "path")?.unwrap_or("."),
    );

    if !root.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("not found: {}", display_relative(project_dir, &root)),
        ));
    }

    let needle = if case_insensitive {
        query.to_lowercase()
    } else {
        query.to_string()
    };

    let mut matches_found = 0usize;
    let mut truncated = false;
    let mut output = String::new();

    let mut search_file = |path: &Path| -> io::Result<bool> {
        if cancel.is_cancelled() {
            return Ok(false);
        }
        let metadata = match fs::metadata(path) {
            Ok(value) => value,
            Err(_) => return Ok(true),
        };
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES_FOR_GREP {
            return Ok(true);
        }
        let bytes = match fs::read(path) {
            Ok(value) => value,
            Err(_) => return Ok(true),
        };
        if bytes.contains(&0) {
            return Ok(true);
        }
        let text = match std::str::from_utf8(&bytes) {
            Ok(value) => value,
            Err(_) => return Ok(true),
        };

        let label = display_relative(project_dir, path);
        for (line_no, line) in text.lines().enumerate() {
            let haystack = if case_insensitive {
                line.to_lowercase()
            } else {
                line.to_string()
            };
            if haystack.contains(&needle) {
                output.push_str(&format!("{label}:{}: {}\n", line_no + 1, line));
                matches_found += 1;
                if matches_found >= limit {
                    truncated = true;
                    return Ok(false);
                }
            }
        }
        Ok(true)
    };

    if root.is_file() {
        search_file(&root)?;
    } else {
        walk_dir(
            &root,
            cancel,
            &mut |path, _file_name| match search_file(path) {
                Ok(true) => WalkControl::Continue,
                Ok(false) => WalkControl::Stop,
                Err(_) => WalkControl::Continue,
            },
        )?;
    }

    if matches_found == 0 {
        output.push_str("(no matches)\n");
    }
    if truncated {
        output.push_str(&format!("(truncated to first {limit} matches)\n"));
    }

    Ok(ToolOutput {
        tool_name: "grep".to_string(),
        content: output,
    })
}

fn required_query(call: &ToolCall) -> io::Result<String> {
    super::arguments::required_argument(call, "query").map(|value| value.to_string())
}

#[derive(Debug)]
struct ListedEntry {
    display: String,
    is_dir: bool,
}

fn classify_entry(entry: &DirEntry, project_dir: &Path) -> io::Result<ListedEntry> {
    let metadata = entry.metadata()?;
    let path = entry.path();
    let display = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| display_relative(project_dir, &path));
    Ok(ListedEntry {
        display,
        is_dir: metadata.is_dir(),
    })
}

enum WalkControl {
    Continue,
    Stop,
}

fn walk_dir<F>(root: &Path, cancel: &CancelToken, visit: &mut F) -> io::Result<()>
where
    F: FnMut(&Path, &str) -> WalkControl,
{
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let entries = match fs::read_dir(&dir) {
            Ok(value) => value,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let path = entry.path();
            let file_name = path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();

            if file_name.is_empty() || file_name.starts_with('.') && file_name != "." {
                // hidden file/dir
                if SKIP_DIRECTORIES.contains(&file_name.as_str()) {
                    continue;
                }
            }

            let metadata = match entry.metadata() {
                Ok(value) => value,
                Err(_) => continue,
            };

            if metadata.is_dir() {
                if SKIP_DIRECTORIES.contains(&file_name.as_str()) {
                    continue;
                }
                if matches!(visit(&path, &file_name), WalkControl::Stop) {
                    return Ok(());
                }
                stack.push(path);
            } else if matches!(visit(&path, &file_name), WalkControl::Stop) {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn needle_match_factory(needle: &str, case_insensitive: bool) -> impl Fn(&str) -> bool + '_ {
    move |candidate: &str| {
        if case_insensitive {
            candidate.to_lowercase().contains(needle)
        } else {
            candidate.contains(needle)
        }
    }
}

fn resolve_path(project_dir: &Path, raw: &str) -> PathBuf {
    let candidate = PathBuf::from(raw);
    if candidate.is_absolute() {
        candidate
    } else {
        normalize(&project_dir.join(candidate))
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn display_relative(project_dir: &Path, path: &Path) -> String {
    path.strip_prefix(project_dir)
        .map(|relative| {
            let display = relative.display().to_string().replace('\\', "/");
            if display.is_empty() {
                ".".to_string()
            } else {
                display
            }
        })
        .unwrap_or_else(|_| path.display().to_string().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("exgent_search_{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("target")).unwrap();
        fs::write(dir.join("src/lib.rs"), "fn main() { println!(\"hi\"); }").unwrap();
        fs::write(dir.join("src/util.rs"), "pub fn helper() { hi(); }").unwrap();
        fs::write(dir.join("README.md"), "# project\nhello world\n").unwrap();
        fs::write(dir.join("target/should-skip.txt"), "indexed").unwrap();
        dir
    }

    #[test]
    fn ls_returns_relative_entries() {
        let dir = fixture();
        let output = execute_ls_call(&ToolCall::new("1", "ls"), &dir, &CancelToken::new()).unwrap();

        assert!(output.content.contains("src/"));
        assert!(output.content.contains("README.md"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn grep_matches_substrings_and_skips_target() {
        let dir = fixture();
        let output = execute_grep_call(
            &ToolCall::new("1", "grep").with_argument("query", "hi"),
            &dir,
            &CancelToken::new(),
        )
        .unwrap();

        assert!(output.content.contains("src/lib.rs"));
        assert!(output.content.contains("src/util.rs"));
        assert!(!output.content.contains("should-skip.txt"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_matches_filename_substrings() {
        let dir = fixture();
        let output = execute_find_call(
            &ToolCall::new("1", "find").with_argument("query", "lib"),
            &dir,
            &CancelToken::new(),
        )
        .unwrap();

        assert!(output.content.contains("src/lib.rs"));
        assert!(!output.content.contains("util.rs"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn grep_case_insensitive_finds_capitalized() {
        let dir = fixture();
        let output = execute_grep_call(
            &ToolCall::new("1", "grep")
                .with_argument("query", "HELLO")
                .with_argument("case_insensitive", true),
            &dir,
            &CancelToken::new(),
        )
        .unwrap();

        assert!(output.content.contains("README.md"));

        let _ = fs::remove_dir_all(&dir);
    }
}
