use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use exgent_ai::Model;
use serde::{Deserialize, Serialize};

use crate::{
    config::config_file,
    localization::{normalize_configured_locale, resolve_locale, Locale},
    persistence::write_json_pretty,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsStore {
    path: PathBuf,
    settings: SettingsFile,
}

impl SettingsStore {
    pub fn load(config_path: Option<&str>) -> io::Result<Self> {
        let path = config_file(config_path, "settings.json");
        if !path.exists() {
            return Ok(Self {
                path,
                settings: SettingsFile::default(),
            });
        }

        let content = fs::read_to_string(&path)?;
        let settings = serde_json::from_str(&content).map_err(invalid_data)?;
        Ok(Self { path, settings })
    }

    pub fn default_model(&self) -> Option<&ModelSelection> {
        self.settings.default_model.as_ref()
    }

    pub fn set_default_model(&mut self, selection: ModelSelection) {
        self.settings.default_model = Some(selection);
    }

    pub fn is_model_enabled(&self, model: &Model) -> bool {
        let Some(selections) = self.settings.enabled_models.get(&model.provider) else {
            return true;
        };
        selections
            .iter()
            .any(|selection| selection.matches_model(model))
    }

    pub fn set_enabled_models(&mut self, provider: &str, selections: Vec<ModelSelection>) {
        self.settings
            .enabled_models
            .insert(provider.to_string(), selections);
    }

    pub fn prompt_display_enabled(&self) -> bool {
        self.settings.prompt_display_enabled
    }

    pub fn set_prompt_display_enabled(&mut self, enabled: bool) {
        self.settings.prompt_display_enabled = enabled;
    }

    pub fn locale(&self) -> Locale {
        resolve_locale(&self.settings.locale)
    }

    pub fn locale_setting(&self) -> &str {
        &self.settings.locale
    }

    pub fn set_locale(&mut self, locale: &str) -> Result<(), String> {
        let Some(locale) = normalize_configured_locale(locale) else {
            return Err(format!(
                "invalid locale '{locale}'. Expected: auto, en, zh-Hans."
            ));
        };
        self.settings.locale = locale.to_string();
        Ok(())
    }

    pub fn theme(&self) -> ThemeSettings {
        self.settings.theme.clone()
    }

    pub fn set_theme(&mut self, theme: ThemeSettings) {
        self.settings.theme = theme;
    }

    pub fn save(&self) -> io::Result<()> {
        write_json_pretty(&self.path, &self.settings)
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    default_model: Option<ModelSelection>,
    #[serde(default)]
    enabled_models: BTreeMap<String, Vec<ModelSelection>>,
    #[serde(default)]
    prompt_display_enabled: bool,
    #[serde(default = "default_locale")]
    locale: String,
    #[serde(default)]
    theme: ThemeSettings,
}

fn default_locale() -> String {
    "auto".to_string()
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            default_model: None,
            enabled_models: BTreeMap::new(),
            prompt_display_enabled: false,
            locale: default_locale(),
            theme: ThemeSettings::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThemeRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl ThemeRgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ThemeSettings {
    pub name: String,
    pub rgb: ThemeRgb,
}

impl ThemeSettings {
    pub fn preset(preset: ThemePreset) -> Self {
        Self {
            name: preset.name.to_string(),
            rgb: preset.rgb,
        }
    }

    pub fn custom(rgb: ThemeRgb) -> Self {
        Self {
            name: "custom".to_string(),
            rgb,
        }
    }
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self::preset(THEME_PRESETS[0])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThemePreset {
    pub name: &'static str,
    pub rgb: ThemeRgb,
}

pub const THEME_PRESETS: &[ThemePreset] = &[
    ThemePreset {
        name: "blue",
        rgb: ThemeRgb::new(59, 130, 246),
    },
    ThemePreset {
        name: "cyan",
        rgb: ThemeRgb::new(34, 211, 238),
    },
    ThemePreset {
        name: "violet",
        rgb: ThemeRgb::new(167, 139, 250),
    },
    ThemePreset {
        name: "emerald",
        rgb: ThemeRgb::new(52, 211, 153),
    },
    ThemePreset {
        name: "amber",
        rgb: ThemeRgb::new(245, 158, 11),
    },
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelSelection {
    pub provider: String,
    pub id: String,
    pub adapter: String,
}

impl ModelSelection {
    pub fn from_model(model: &Model) -> Self {
        Self {
            provider: model.provider.clone(),
            id: model.id.clone(),
            adapter: model.adapter.clone(),
        }
    }

    pub fn matches_model(&self, model: &Model) -> bool {
        self.provider == model.provider && self.id == model.id && self.adapter == model.adapter
    }
}

fn invalid_data(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn saves_and_loads_default_model() {
        let dir = test_dir("saves_and_loads_default_model");
        let _ = fs::remove_dir_all(&dir);

        let mut settings = SettingsStore::load(Some(dir.to_str().unwrap())).unwrap();
        settings.set_default_model(ModelSelection {
            provider: "deepseek".to_string(),
            id: "deepseek-chat".to_string(),
            adapter: "openai-completions".to_string(),
        });
        settings.save().unwrap();

        let loaded = SettingsStore::load(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(
            loaded.default_model(),
            Some(&ModelSelection {
                provider: "deepseek".to_string(),
                id: "deepseek-chat".to_string(),
                adapter: "openai-completions".to_string(),
            })
        );
        assert!(loaded.path().ends_with("settings.json"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saves_enabled_models_by_provider() {
        let dir = test_dir("saves_enabled_models_by_provider");
        let _ = fs::remove_dir_all(&dir);

        let mut settings = SettingsStore::load(Some(dir.to_str().unwrap())).unwrap();
        settings.set_enabled_models(
            "test-provider",
            vec![ModelSelection {
                provider: "test-provider".to_string(),
                id: "selected-model".to_string(),
                adapter: "openai-completions".to_string(),
            }],
        );
        settings.save().unwrap();

        let loaded = SettingsStore::load(Some(dir.to_str().unwrap())).unwrap();
        assert!(loaded.is_model_enabled(&Model::new(
            "test-provider",
            "selected-model",
            "openai-completions"
        )));
        assert!(!loaded.is_model_enabled(&Model::new(
            "test-provider",
            "hidden-model",
            "openai-completions"
        )));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompt_display_is_disabled_by_default_and_persisted() {
        let dir = test_dir("prompt_display_is_disabled_by_default_and_persisted");
        let _ = fs::remove_dir_all(&dir);

        let mut settings = SettingsStore::load(Some(dir.to_str().unwrap())).unwrap();
        assert!(!settings.prompt_display_enabled());

        settings.set_prompt_display_enabled(true);
        settings.save().unwrap();

        let loaded = SettingsStore::load(Some(dir.to_str().unwrap())).unwrap();
        assert!(loaded.prompt_display_enabled());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn theme_defaults_and_persists_custom_rgb() {
        let dir = test_dir("theme_defaults_and_persists_custom_rgb");
        let _ = fs::remove_dir_all(&dir);

        let mut settings = SettingsStore::load(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(settings.theme(), ThemeSettings::default());

        settings.set_theme(ThemeSettings::custom(ThemeRgb::new(12, 34, 56)));
        settings.save().unwrap();

        let loaded = SettingsStore::load(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(
            loaded.theme(),
            ThemeSettings {
                name: "custom".to_string(),
                rgb: ThemeRgb::new(12, 34, 56),
            }
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn locale_defaults_normalizes_and_persists() {
        let dir = test_dir("locale_defaults_normalizes_and_persists");
        let _ = fs::remove_dir_all(&dir);

        let mut settings = SettingsStore::load(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(settings.locale_setting(), "auto");

        settings.set_locale("zh_CN.UTF-8").unwrap();
        settings.save().unwrap();

        let loaded = SettingsStore::load(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(loaded.locale_setting(), "zh-Hans");
        assert_eq!(loaded.locale(), Locale::ZhHans);

        let _ = fs::remove_dir_all(&dir);
    }

    fn test_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("exgent_settings_{name}_{stamp}"))
    }
}
