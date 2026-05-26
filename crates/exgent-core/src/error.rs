//! Structured errors for crate-level operations.
//!
//! Many existing call sites still return `Result<T, String>`. New code should
//! prefer the typed errors in this module; the string-based shapes are kept
//! to avoid churn but should be migrated as ergonomics permit.

use std::io;

use thiserror::Error;

/// Errors raised while loading or saving persisted state.
#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

/// Errors raised while resolving or mutating the model catalog and auth state.
#[derive(Debug, Error)]
pub enum ModelServiceError {
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    #[error("model selection out of range: {0}")]
    SelectionOutOfRange(usize),
    #[error("model is not configured: {provider}/{id}. Please enter /auth to configure a model.")]
    NotConfigured { provider: String, id: String },
    #[error(
        "model is not available for this account: {provider}/{id}. Refresh /auth to update available models."
    )]
    NotAvailable { provider: String, id: String },
    #[error(
        "model is hidden in settings: {provider}/{id}. Please enter /settings model to enable it."
    )]
    Disabled { provider: String, id: String },
    #[error("no model is configured")]
    NoModelConfigured,
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error("provider returned no usable models for {0}")]
    EmptyProviderModels(String),
    #[error("failed to verify API key and fetch provider models: {0}")]
    Discovery(String),
}

/// Errors raised by an agent session lifecycle (compaction, run loop, etc.).
#[derive(Debug, Error)]
pub enum AgentSessionError {
    #[error("no messages to compact")]
    NothingToCompact,
    #[error("agent run was cancelled")]
    Cancelled,
    #[error("provider error: {0}")]
    Provider(String),
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_io_round_trips() {
        let err: PersistenceError =
            io::Error::new(io::ErrorKind::PermissionDenied, "no access").into();
        assert!(err.to_string().contains("no access"));
    }

    #[test]
    fn model_service_messages_match_existing_text() {
        let unknown = ModelServiceError::UnknownProvider("missing-provider".to_string());
        assert_eq!(unknown.to_string(), "unknown provider: missing-provider");

        let disabled = ModelServiceError::Disabled {
            provider: "anthropic".to_string(),
            id: "claude".to_string(),
        };
        assert!(disabled.to_string().contains("/settings model"));
    }
}
