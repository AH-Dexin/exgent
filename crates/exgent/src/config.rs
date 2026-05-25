use std::{env, path::PathBuf};

pub fn config_root(config_path: Option<&str>) -> PathBuf {
    if let Some(path) = config_path {
        return PathBuf::from(path);
    }

    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".exgent")
}

pub fn config_file(config_path: Option<&str>, file_name: &str) -> PathBuf {
    config_root(config_path).join(file_name)
}

pub fn config_dir(config_path: Option<&str>, dir_name: &str) -> PathBuf {
    config_root(config_path).join(dir_name)
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(user_profile) = env::var_os("USERPROFILE") {
        return Some(PathBuf::from(user_profile));
    }

    env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_config_path_is_used_as_root() {
        let root = PathBuf::from("custom-root");

        assert_eq!(config_root(Some("custom-root")), root);
        assert_eq!(
            config_file(Some("custom-root"), "settings.json"),
            PathBuf::from("custom-root").join("settings.json")
        );
    }
}
