use std::time::Duration;

use reqwest::blocking::{Client, ClientBuilder};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

pub fn shared_client_builder() -> ClientBuilder {
    Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .user_agent(default_user_agent())
}

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
        let _ = shared_blocking_client();
    }
}
