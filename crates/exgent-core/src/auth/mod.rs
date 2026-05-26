use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{config::config_file, persistence::write_json_pretty};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthStore {
    path: PathBuf,
    credentials: BTreeMap<String, AuthCredential>,
}

impl AuthStore {
    pub fn load(config_path: Option<&str>) -> io::Result<Self> {
        let path = auth_path(config_path);
        if !path.exists() {
            return Ok(Self {
                path,
                credentials: BTreeMap::new(),
            });
        }

        let content = fs::read_to_string(&path)?;
        let file: AuthFile = serde_json::from_str(&content).map_err(invalid_data)?;
        Ok(Self {
            path,
            credentials: file.into_credentials(),
        })
    }

    pub fn token(&self, provider: &str) -> Option<&str> {
        match self.credentials.get(provider) {
            Some(AuthCredential::ApiKey { key }) => Some(key.as_str()),
            Some(AuthCredential::OAuth(credentials)) => Some(credentials.access.as_str()),
            None => None,
        }
    }

    pub fn has_api_key(&self, provider: &str) -> bool {
        matches!(
            self.credentials.get(provider),
            Some(AuthCredential::ApiKey { key }) if !key.is_empty()
        )
    }

    pub fn has_oauth(&self, provider: &str) -> bool {
        matches!(
            self.credentials.get(provider),
            Some(AuthCredential::OAuth(credentials)) if !credentials.access.is_empty()
        )
    }

    pub fn set_token(&mut self, provider: impl Into<String>, token: impl Into<String>) {
        self.credentials.insert(
            provider.into(),
            AuthCredential::ApiKey { key: token.into() },
        );
    }

    pub fn set_oauth(&mut self, provider: impl Into<String>, credentials: OAuthCredential) {
        self.credentials
            .insert(provider.into(), AuthCredential::OAuth(credentials));
    }

    pub fn remove_token(&mut self, provider: &str) {
        self.credentials.remove(provider);
    }

    pub fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = AuthFile {
            credentials: self.credentials.clone(),
            tokens: BTreeMap::new(),
        };
        write_json_pretty(&self.path, &file)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct AuthFile {
    #[serde(default)]
    credentials: BTreeMap<String, AuthCredential>,
    #[serde(default)]
    tokens: BTreeMap<String, String>,
}

impl AuthFile {
    fn into_credentials(self) -> BTreeMap<String, AuthCredential> {
        if !self.credentials.is_empty() {
            return self.credentials;
        }

        self.tokens
            .into_iter()
            .map(|(provider, key)| (provider, AuthCredential::ApiKey { key }))
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthCredential {
    ApiKey { key: String },
    OAuth(OAuthCredential),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OAuthCredential {
    pub access: String,
    pub refresh: String,
    pub expires: i64,
    #[serde(default)]
    pub extra: BTreeMap<String, String>,
}

fn auth_path(config_path: Option<&str>) -> PathBuf {
    config_file(config_path, "auth.json")
}

fn invalid_data(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_provider_tokens() {
        let dir = test_dir("saves_and_loads_provider_tokens");
        let _ = fs::remove_dir_all(&dir);

        let mut auth = AuthStore::load(Some(dir.to_str().unwrap())).unwrap();
        auth.set_token("deepseek", "test-token");
        auth.save().unwrap();

        let loaded = AuthStore::load(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(loaded.token("deepseek"), Some("test-token"));
        assert!(loaded.has_api_key("deepseek"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn removes_provider_tokens() {
        let dir = test_dir("removes_provider_tokens");
        let _ = fs::remove_dir_all(&dir);

        let mut auth = AuthStore::load(Some(dir.to_str().unwrap())).unwrap();
        auth.set_token("deepseek", "test-token");
        auth.remove_token("deepseek");
        auth.save().unwrap();

        let loaded = AuthStore::load(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(loaded.token("deepseek"), None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saves_and_loads_oauth_credentials() {
        let dir = test_dir("saves_and_loads_oauth_credentials");
        let _ = fs::remove_dir_all(&dir);

        let mut auth = AuthStore::load(Some(dir.to_str().unwrap())).unwrap();
        auth.set_oauth(
            "github-copilot",
            OAuthCredential {
                access: "access-token".to_string(),
                refresh: "refresh-token".to_string(),
                expires: 123,
                extra: BTreeMap::new(),
            },
        );
        auth.save().unwrap();

        let loaded = AuthStore::load(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(loaded.token("github-copilot"), Some("access-token"));
        assert!(loaded.has_oauth("github-copilot"));
        assert!(!loaded.has_api_key("github-copilot"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_legacy_tokens_as_api_key_credentials() {
        let dir = test_dir("loads_legacy_tokens_as_api_key_credentials");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("auth.json"),
            r#"{"tokens":{"deepseek":"legacy-token"}}"#,
        )
        .unwrap();

        let loaded = AuthStore::load(Some(dir.to_str().unwrap())).unwrap();
        assert_eq!(loaded.token("deepseek"), Some("legacy-token"));
        assert!(loaded.has_api_key("deepseek"));

        let _ = fs::remove_dir_all(&dir);
    }

    fn test_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("exgent_auth_{name}_{stamp}"))
    }
}
