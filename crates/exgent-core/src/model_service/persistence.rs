use super::ModelService;
use crate::{
    auth::AuthStore,
    models::{ModelAvailabilityCache, ModelRegistry},
    settings::SettingsStore,
};

pub(super) struct ModelServiceSnapshot {
    models: ModelRegistry,
    model_cache: ModelAvailabilityCache,
    auth: AuthStore,
    settings: SettingsStore,
    current_model_index: Option<usize>,
}

impl ModelService {
    pub(super) fn snapshot(&self) -> ModelServiceSnapshot {
        ModelServiceSnapshot {
            models: self.models.clone(),
            model_cache: self.model_cache.clone(),
            auth: self.auth.clone(),
            settings: self.settings.clone(),
            current_model_index: self.current_model_index,
        }
    }

    pub(super) fn restore(&mut self, snapshot: ModelServiceSnapshot) {
        self.models = snapshot.models;
        self.model_cache = snapshot.model_cache;
        self.auth = snapshot.auth;
        self.settings = snapshot.settings;
        self.current_model_index = snapshot.current_model_index;
    }

    pub(super) fn save_settings(&mut self, previous: ModelServiceSnapshot) -> Result<(), String> {
        if let Err(error) = self.settings.save() {
            self.restore(previous);
            return Err(format!("failed to save settings: {error}"));
        }
        Ok(())
    }

    pub(super) fn save_auth_and_settings(
        &mut self,
        previous: ModelServiceSnapshot,
    ) -> Result<(), String> {
        if let Err(error) = self.auth.save() {
            self.restore(previous);
            return Err(format!("failed to save auth: {error}"));
        }

        if let Err(error) = self.settings.save() {
            self.restore(previous);
            let _ = self.auth.save();
            return Err(format!("failed to save settings: {error}"));
        }
        Ok(())
    }

    pub(super) fn save_auth_cache_and_settings(
        &mut self,
        previous: ModelServiceSnapshot,
    ) -> Result<(), String> {
        if let Err(error) = self.auth.save() {
            self.restore(previous);
            return Err(format!("failed to save auth: {error}"));
        }

        if let Err(error) = self.model_cache.save() {
            self.restore(previous);
            let _ = self.auth.save();
            return Err(format!("failed to save model cache: {error}"));
        }

        if let Err(error) = self.settings.save() {
            self.restore(previous);
            let _ = self.auth.save();
            let _ = self.model_cache.save();
            return Err(format!("failed to save settings: {error}"));
        }
        Ok(())
    }

    pub(super) fn save_models_and_settings(
        &mut self,
        previous: ModelServiceSnapshot,
    ) -> Result<(), String> {
        if let Err(error) = self.models.save() {
            self.restore(previous);
            return Err(format!("failed to save model: {error}"));
        }

        if let Err(error) = self.settings.save() {
            self.restore(previous);
            let _ = self.models.save();
            return Err(format!("failed to save settings: {error}"));
        }
        Ok(())
    }

    pub(super) fn save_models_auth_and_settings(
        &mut self,
        previous: ModelServiceSnapshot,
    ) -> Result<(), String> {
        if let Err(error) = self.models.save() {
            self.restore(previous);
            return Err(format!("failed to save model: {error}"));
        }

        if let Err(error) = self.auth.save() {
            self.restore(previous);
            let _ = self.models.save();
            return Err(format!("failed to save auth: {error}"));
        }

        if let Err(error) = self.settings.save() {
            self.restore(previous);
            let _ = self.models.save();
            let _ = self.auth.save();
            return Err(format!("failed to save settings: {error}"));
        }
        Ok(())
    }
}
