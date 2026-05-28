use std::env;
#[cfg(not(target_os = "windows"))]
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use exgent_core::ImageContent;
#[cfg(target_os = "windows")]
use image::{ColorType, ImageEncoder};

#[cfg(not(target_os = "windows"))]
const SUPPORTED_IMAGE_MIME_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];

pub(super) fn read_clipboard_image() -> Result<Option<ImageContent>, String> {
    if env::var_os("TERMUX_VERSION").is_some() {
        return Ok(None);
    }

    let image = read_clipboard_image_raw();
    Ok(image.map(|image| ImageContent::new(STANDARD.encode(image.bytes), image.mime_type)))
}

#[cfg(not(target_os = "windows"))]
fn read_clipboard_image_raw() -> Option<ClipboardImage> {
    let is_wsl = is_wsl();
    let is_wayland = is_wayland_session();

    if is_wayland || is_wsl {
        read_via_wl_paste().or_else(read_via_xclip)
    } else {
        read_via_xclip()
    }
    .or_else(|| is_wsl.then(read_via_powershell_wsl).flatten())
}

#[cfg(target_os = "windows")]
fn read_clipboard_image_raw() -> Option<ClipboardImage> {
    read_via_arboard()
}

struct ClipboardImage {
    bytes: Vec<u8>,
    mime_type: String,
}

#[cfg(target_os = "windows")]
fn read_via_arboard() -> Option<ClipboardImage> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let image = clipboard.get_image().ok()?;
    let bytes = encode_rgba_as_png(image.width, image.height, image.bytes.as_ref())?;
    Some(ClipboardImage {
        bytes,
        mime_type: "image/png".to_string(),
    })
}

#[cfg(target_os = "windows")]
fn encode_rgba_as_png(width: usize, height: usize, bytes: &[u8]) -> Option<Vec<u8>> {
    let width = u32::try_from(width).ok()?;
    let height = u32::try_from(height).ok()?;
    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    let mut rgba = bytes.to_vec();
    if rgba.len() < expected {
        rgba.resize(expected, 0);
    } else if rgba.len() > expected {
        rgba.truncate(expected);
    }

    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png);
    encoder
        .write_image(&rgba, width, height, ColorType::Rgba8.into())
        .ok()?;
    Some(png)
}

#[cfg(not(target_os = "windows"))]
fn read_via_wl_paste() -> Option<ClipboardImage> {
    let list = command_output("wl-paste", &["--list-types"])?;
    let types = split_mime_types(&list);
    let selected = select_preferred_image_mime_type(&types)?;
    let bytes = command_output("wl-paste", &["--type", selected.as_str(), "--no-newline"])?;
    Some(ClipboardImage {
        bytes,
        mime_type: base_mime_type(&selected),
    })
}

#[cfg(not(target_os = "windows"))]
fn read_via_xclip() -> Option<ClipboardImage> {
    let target_types = command_output("xclip", &["-selection", "clipboard", "-t", "TARGETS", "-o"])
        .map(|output| split_mime_types(&output))
        .unwrap_or_default();
    let preferred = select_preferred_image_mime_type(&target_types);
    let mut candidates = preferred.into_iter().collect::<Vec<_>>();
    candidates.extend(
        SUPPORTED_IMAGE_MIME_TYPES
            .iter()
            .map(|mime| mime.to_string()),
    );
    candidates.dedup();

    for mime_type in candidates {
        let Some(bytes) = command_output(
            "xclip",
            &["-selection", "clipboard", "-t", mime_type.as_str(), "-o"],
        ) else {
            continue;
        };
        return Some(ClipboardImage {
            bytes,
            mime_type: base_mime_type(&mime_type),
        });
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn read_via_powershell_wsl() -> Option<ClipboardImage> {
    let path = temp_png_path();
    let win_path = command_output("wslpath", &["-w", path.to_str()?])
        .and_then(|output| String::from_utf8(output).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())?;
    let escaped_path = win_path.replace('\'', "''");
    let script = [
        "Add-Type -AssemblyName System.Windows.Forms",
        "Add-Type -AssemblyName System.Drawing",
        &format!("$path = '{escaped_path}'"),
        "$img = [System.Windows.Forms.Clipboard]::GetImage()",
        "if ($img) { $img.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); Write-Output 'ok' } else { Write-Output 'empty' }",
    ]
    .join("; ");
    let output = command_output(
        "powershell.exe",
        &["-NoProfile", "-Command", script.as_str()],
    )
    .and_then(|output| String::from_utf8(output).ok())?;
    if output.trim() != "ok" {
        let _ = fs::remove_file(path);
        return None;
    }

    let bytes = fs::read(&path).ok().filter(|bytes| !bytes.is_empty());
    let _ = fs::remove_file(path);
    bytes.map(|bytes| ClipboardImage {
        bytes,
        mime_type: "image/png".to_string(),
    })
}

#[cfg(not(target_os = "windows"))]
fn command_output(command: &str, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new(command).args(args).output().ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    Some(output.stdout)
}

#[cfg(not(target_os = "windows"))]
fn split_mime_types(output: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(output)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn select_preferred_image_mime_type(mime_types: &[String]) -> Option<String> {
    let normalized = mime_types
        .iter()
        .map(|mime| (mime.as_str(), base_mime_type(mime)))
        .collect::<Vec<_>>();

    for preferred in SUPPORTED_IMAGE_MIME_TYPES {
        if let Some((raw, _)) = normalized.iter().find(|(_, base)| base == preferred) {
            return Some((*raw).to_string());
        }
    }

    None
}

#[cfg(not(target_os = "windows"))]
fn base_mime_type(mime_type: &str) -> String {
    mime_type
        .split(';')
        .next()
        .unwrap_or(mime_type)
        .trim()
        .to_ascii_lowercase()
}

#[cfg(not(target_os = "windows"))]
fn is_wayland_session() -> bool {
    env::var_os("WAYLAND_DISPLAY").is_some()
        || env::var("XDG_SESSION_TYPE").is_ok_and(|value| value == "wayland")
}

#[cfg(not(target_os = "windows"))]
fn is_wsl() -> bool {
    if env::var_os("WSL_DISTRO_NAME").is_some() || env::var_os("WSLENV").is_some() {
        return true;
    }
    fs::read_to_string("/proc/version")
        .map(|version| version.to_ascii_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn temp_png_path() -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    env::temp_dir().join(format!(
        "exgent-clipboard-{}-{millis}.png",
        std::process::id()
    ))
}
