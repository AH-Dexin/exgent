use std::{
    io::Write,
    process::{Command, Stdio},
};

use base64::{engine::general_purpose, Engine as _};

pub(super) fn write_clipboard_text(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }

    if cfg!(target_os = "macos") && pipe_to("pbcopy", &[], text).is_ok() {
        return Ok(());
    }
    if cfg!(target_os = "windows") && pipe_to("clip", &[], text).is_ok() {
        return Ok(());
    }
    if pipe_to("wl-copy", &[], text).is_ok() {
        return Ok(());
    }
    if pipe_to("xclip", &["-selection", "clipboard"], text).is_ok() {
        return Ok(());
    }
    if pipe_to("xsel", &["--clipboard", "--input"], text).is_ok() {
        return Ok(());
    }

    let encoded = general_purpose::STANDARD.encode(text.as_bytes());
    print!("\x1b]52;c;{encoded}\x07");
    std::io::stdout()
        .flush()
        .map_err(|error| format!("failed to write clipboard escape: {error}"))
}

fn pipe_to(command: &str, args: &[&str], text: &str) -> Result<(), ()> {
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ())?;

    let Some(stdin) = child.stdin.as_mut() else {
        return Err(());
    };
    stdin.write_all(text.as_bytes()).map_err(|_| ())?;
    let status = child.wait().map_err(|_| ())?;
    status.success().then_some(()).ok_or(())
}
