use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_SUBMITTED_INPUT_CHARS: usize = 16_000;

pub(super) fn normalize_paste_text(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

pub(super) fn prepare_paste_text(project_dir: &Path, value: &str) -> io::Result<String> {
    prepare_paste_text_with_limit(project_dir, value, MAX_SUBMITTED_INPUT_CHARS)
}

fn prepare_paste_text_with_limit(
    project_dir: &Path,
    value: &str,
    max_chars: usize,
) -> io::Result<String> {
    let normalized = normalize_paste_text(value);
    if normalized.chars().count() <= max_chars {
        return Ok(normalized);
    }

    let rel_path = write_large_paste(project_dir, &normalized)?;
    let display_path = rel_path.display().to_string().replace('\\', "/");
    Ok(format!(
        "Large pasted content was saved to `{display_path}`.\nPlease read that file and treat it as the pasted content."
    ))
}

pub(super) fn truncate_large_paste_fallback(value: &str) -> String {
    let normalized = normalize_paste_text(value);
    let mut truncated = normalized
        .chars()
        .take(MAX_SUBMITTED_INPUT_CHARS)
        .collect::<String>();
    truncated.push_str("\n\n[Large paste truncated because exgent could not write a paste file.]");
    truncated
}

fn write_large_paste(project_dir: &Path, content: &str) -> io::Result<PathBuf> {
    let rel_dir = PathBuf::from(".exgent").join("pastes");
    let abs_dir = project_dir.join(&rel_dir);
    fs::create_dir_all(&abs_dir)?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let filename = format!("paste-{stamp}-{}.md", std::process::id());
    let rel_path = rel_dir.join(filename);
    fs::write(project_dir.join(&rel_path), content)?;
    Ok(rel_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_crlf_and_cr_paste_text() {
        assert_eq!(normalize_paste_text("a\r\nb\rc"), "a\nb\nc");
    }

    #[test]
    fn short_paste_stays_inline() {
        let prepared = prepare_paste_text_with_limit(Path::new("."), "hello", 16).unwrap();
        assert_eq!(prepared, "hello");
    }

    #[test]
    fn large_paste_spills_to_project_file() {
        let dir = std::env::temp_dir().join(format!(
            "exgent-paste-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();

        let prepared = prepare_paste_text_with_limit(&dir, "abcdef", 3).unwrap();
        assert!(prepared.contains(".exgent/pastes/paste-"));

        let rel_path = prepared
            .split('`')
            .nth(1)
            .expect("prepared text should include a quoted path");
        assert_eq!(fs::read_to_string(dir.join(rel_path)).unwrap(), "abcdef");

        let _ = fs::remove_dir_all(dir);
    }
}
