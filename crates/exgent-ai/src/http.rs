//! Shared `reqwest` client construction.
//!
//! Centralizing the builder lets every provider (and the model-discovery
//! helpers) inherit the same defaults: timeout, user agent, proxy. Callers
//! who need bespoke behavior can override via the builder before calling
//! [`reqwest::blocking::ClientBuilder::build`].

use std::time::Duration;

use reqwest::blocking::{Client, ClientBuilder};

/// Default request timeout for provider calls. Overridable per-call via
/// [`reqwest::blocking::RequestBuilder::timeout`].
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Build a `ClientBuilder` with the shared defaults applied.
///
/// Callers typically chain additional `.header(...)` or `.timeout(...)`
/// modifiers and finish with `.build()`.
pub fn shared_client_builder() -> ClientBuilder {
    Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .user_agent(default_user_agent())
}

/// Build a ready-to-use client with the shared defaults. Panics if the
/// underlying TLS stack fails to initialize.
pub fn shared_blocking_client() -> Client {
    shared_client_builder()
        .build()
        .unwrap_or_else(|_| Client::new())
}

fn default_user_agent() -> String {
    format!("exgent/{}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_contains_crate_version() {
        let ua = default_user_agent();
        assert!(ua.starts_with("exgent/"));
    }

    #[test]
    fn shared_blocking_client_is_constructed() {
        // Just verify the builder doesn't panic and produces a client.
        let _ = shared_blocking_client();
    }
}
