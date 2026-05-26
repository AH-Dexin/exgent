use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

use chrono::{SecondsFormat, Utc};
use exgent_ai::{DiscoveredModel, Model};
use serde::{Deserialize, Serialize};

use crate::{config::config_file, persistence::write_json_pretty};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelAvailabilityCache {
    path: PathBuf,
    providers: BTreeMap<String, CachedProviderModels>,
}

impl ModelAvailabilityCache {
    pub fn load(config_path: Option<&str>) -> io::Result<Self> {
        let path = config_file(config_path, "model_cache.json");
        if !path.exists() {
            return Ok(Self {
                path,
                providers: BTreeMap::new(),
            });
        }

        let content = fs::read_to_string(&path)?;
        let file: ModelCacheFile = serde_json::from_str(&content).map_err(invalid_data)?;
        Ok(Self {
            path,
            providers: file.providers,
        })
    }

    pub fn set_provider_models(&mut self, provider: &str, models: Vec<DiscoveredModel>) {
        let mut seen = BTreeSet::new();
        let models = models
            .into_iter()
            .filter(|model| seen.insert(model.id.clone()))
            .collect();
        self.providers.insert(
            provider.to_string(),
            CachedProviderModels {
                fetched_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                models,
            },
        );
    }

    pub fn remove_provider(&mut self, provider: &str) {
        self.providers.remove(provider);
    }

    pub fn provider_models(&self, provider: &str) -> Option<&[DiscoveredModel]> {
        self.providers
            .get(provider)
            .map(|provider| provider.models.as_slice())
    }

    pub fn providers(&self) -> impl Iterator<Item = (&str, &[DiscoveredModel])> {
        self.providers
            .iter()
            .map(|(provider, models)| (provider.as_str(), models.models.as_slice()))
    }

    pub fn allows_model(&self, model: &Model) -> bool {
        let Some(models) = self.provider_models(&model.provider) else {
            return true;
        };
        models.iter().any(|cached| cached.id == model.id)
    }

    pub fn save(&self) -> io::Result<()> {
        write_json_pretty(
            &self.path,
            &ModelCacheFile {
                providers: self.providers.clone(),
            },
        )
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct ModelCacheFile {
    #[serde(default)]
    providers: BTreeMap<String, CachedProviderModels>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct CachedProviderModels {
    fetched_at: String,
    #[serde(default)]
    models: Vec<DiscoveredModel>,
}

fn invalid_data(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_provider_model_cache() {
        let dir = test_dir("saves_and_loads_provider_model_cache");
        let _ = fs::remove_dir_all(&dir);

        let mut cache = ModelAvailabilityCache::load(Some(dir.to_str().unwrap())).unwrap();
        cache.set_provider_models(
            "anthropic",
            vec![DiscoveredModel {
                id: "claude-sonnet-4-5".to_string(),
                name: Some("Claude Sonnet 4.5".to_string()),
            }],
        );
        cache.save().unwrap();

        let loaded = ModelAvailabilityCache::load(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(
            loaded.provider_models("anthropic").unwrap(),
            &[DiscoveredModel {
                id: "claude-sonnet-4-5".to_string(),
                name: Some("Claude Sonnet 4.5".to_string()),
            }]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    fn test_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("exgent_model_cache_{name}_{stamp}"))
    }
}
