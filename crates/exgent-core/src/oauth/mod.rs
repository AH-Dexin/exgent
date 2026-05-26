use reqwest::Url;

mod anthropic;
mod callback;
mod github_copilot;
mod openai_codex;
mod pkce;

pub use anthropic::{finish_anthropic_oauth_flow, start_anthropic_oauth_flow, AnthropicOAuthFlow};
pub use github_copilot::{
    finish_github_copilot_device_flow, finish_github_copilot_device_flow_cancellable,
    normalize_github_domain, refresh_github_copilot_token, start_github_copilot_device_flow,
    GithubDeviceFlow,
};
pub use openai_codex::{
    finish_openai_codex_oauth_flow, start_openai_codex_oauth_flow, OpenAiCodexOAuthFlow,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationCode {
    code: String,
    state: String,
}

pub fn parse_authorization_input(input: &str) -> Result<Option<AuthorizationCode>, String> {
    let value = input.trim();
    if value.is_empty() {
        return Ok(None);
    }

    if let Ok(url) = Url::parse(value) {
        return Ok(authorization_from_query(url));
    }

    if let Some((code, state)) = value.split_once('#') {
        return Ok(Some(AuthorizationCode {
            code: code.to_string(),
            state: state.to_string(),
        }));
    }

    if value.contains("code=") {
        let url = Url::parse(&format!("http://localhost/callback?{value}"))
            .map_err(|error| format!("invalid authorization input: {error}"))?;
        return Ok(authorization_from_query(url));
    }

    Ok(Some(AuthorizationCode {
        code: value.to_string(),
        state: String::new(),
    }))
}

fn authorization_from_query(url: Url) -> Option<AuthorizationCode> {
    let code = url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.to_string());
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.to_string());

    match (code, state) {
        (Some(code), Some(state)) => Some(AuthorizationCode { code, state }),
        (Some(code), None) => Some(AuthorizationCode {
            code,
            state: String::new(),
        }),
        _ => None,
    }
}

fn current_time_millis() -> i64 {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    duration.as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_authorization_redirect_url() {
        let parsed =
            parse_authorization_input("http://localhost:53692/callback?code=abc&state=xyz")
                .unwrap()
                .unwrap();

        assert_eq!(
            parsed,
            AuthorizationCode {
                code: "abc".to_string(),
                state: "xyz".to_string()
            }
        );
    }

    #[test]
    fn parses_hash_authorization_input() {
        let parsed = parse_authorization_input("abc#xyz").unwrap().unwrap();

        assert_eq!(
            parsed,
            AuthorizationCode {
                code: "abc".to_string(),
                state: "xyz".to_string()
            }
        );
    }

    #[test]
    fn creates_pkce_challenge() {
        assert_eq!(
            pkce::create_pkce_challenge("test-verifier"),
            "JBbiqONGWPaAmwXk_8bT6UnlPfrn65D32eZlJS-zGG0"
        );
    }

    #[test]
    fn binds_ipv4_and_ipv6_loopback_for_localhost_callback() {
        assert_eq!(
            callback::callback_bind_hosts("127.0.0.1"),
            vec!["127.0.0.1".to_string(), "::1".to_string()]
        );
        assert_eq!(
            callback::callback_bind_hosts("localhost"),
            vec!["127.0.0.1".to_string(), "::1".to_string()]
        );
        assert_eq!(
            callback::callback_bind_hosts("0.0.0.0"),
            vec!["0.0.0.0".to_string()]
        );
    }
}
