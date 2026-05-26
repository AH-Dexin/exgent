use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
        Arc,
    },
    time::Duration,
};

use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};

use crate::auth::OAuthCredential;

use super::{
    callback::{oauth_callback_host, start_callback_server},
    current_time_millis,
    pkce::{create_pkce_challenge, create_pkce_verifier},
    AuthorizationCode,
};

const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const ANTHROPIC_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const ANTHROPIC_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const ANTHROPIC_CALLBACK_PORT: u16 = 53692;
const ANTHROPIC_CALLBACK_PATH: &str = "/callback";
const ANTHROPIC_REDIRECT_URI: &str = "http://localhost:53692/callback";
const ANTHROPIC_SCOPES: &str =
    "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

#[derive(Debug)]
pub struct AnthropicOAuthFlow {
    pub url: String,
    verifier: String,
    state: String,
    redirect_uri: String,
    callback: Receiver<Result<AuthorizationCode, String>>,
    cancel_callback: Arc<AtomicBool>,
}

impl AnthropicOAuthFlow {
    pub fn wait_for_callback(&self, timeout: Duration) -> Result<AuthorizationCode, String> {
        self.poll_callback(timeout)?
            .ok_or_else(|| "timed out waiting for OAuth callback".to_string())
    }

    pub fn poll_callback(&self, timeout: Duration) -> Result<Option<AuthorizationCode>, String> {
        match self.callback.recv_timeout(timeout) {
            Ok(result) => result.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                Err("OAuth callback server stopped unexpectedly".to_string())
            }
        }
    }
}

impl Drop for AnthropicOAuthFlow {
    fn drop(&mut self) {
        self.cancel_callback.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Deserialize)]
struct AnthropicTokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

#[derive(Debug, Serialize)]
struct AnthropicTokenRequest<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    code: &'a str,
    state: &'a str,
    redirect_uri: &'a str,
    code_verifier: &'a str,
}

pub fn start_anthropic_oauth_flow() -> Result<AnthropicOAuthFlow, String> {
    let verifier = create_pkce_verifier();
    let challenge = create_pkce_challenge(&verifier);
    let callback_host = oauth_callback_host();
    let callback = start_callback_server(
        &callback_host,
        ANTHROPIC_CALLBACK_PORT,
        &verifier,
        ANTHROPIC_CALLBACK_PATH,
        "Anthropic",
    )?;

    let mut url = Url::parse(ANTHROPIC_AUTHORIZE_URL)
        .map_err(|error| format!("invalid Anthropic authorize URL: {error}"))?;
    url.query_pairs_mut()
        .append_pair("code", "true")
        .append_pair("client_id", ANTHROPIC_CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", ANTHROPIC_REDIRECT_URI)
        .append_pair("scope", ANTHROPIC_SCOPES)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &verifier);

    Ok(AnthropicOAuthFlow {
        url: url.to_string(),
        verifier: verifier.clone(),
        state: verifier,
        redirect_uri: ANTHROPIC_REDIRECT_URI.to_string(),
        callback: callback.receiver,
        cancel_callback: callback.cancel,
    })
}

pub fn finish_anthropic_oauth_flow(
    flow: &AnthropicOAuthFlow,
    authorization: AuthorizationCode,
) -> Result<OAuthCredential, String> {
    if !authorization.state.is_empty() && authorization.state != flow.state {
        return Err("OAuth state mismatch".to_string());
    }

    exchange_anthropic_authorization_code(
        &authorization.code,
        if authorization.state.is_empty() {
            &flow.state
        } else {
            &authorization.state
        },
        &flow.verifier,
        &flow.redirect_uri,
    )
}

fn exchange_anthropic_authorization_code(
    code: &str,
    state: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthCredential, String> {
    let client = Client::new();
    let response = client
        .post(ANTHROPIC_TOKEN_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&AnthropicTokenRequest {
            grant_type: "authorization_code",
            client_id: ANTHROPIC_CLIENT_ID,
            code,
            state,
            redirect_uri,
            code_verifier: verifier,
        })
        .send()
        .map_err(|error| format!("Anthropic token request failed: {error}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .unwrap_or_else(|error| format!("failed to read error body: {error}"));
        return Err(format!(
            "Anthropic token exchange failed with {status}: {body}"
        ));
    }

    let token: AnthropicTokenResponse = response
        .json()
        .map_err(|error| format!("invalid Anthropic token response: {error}"))?;

    Ok(OAuthCredential {
        access: token.access_token,
        refresh: token.refresh_token,
        expires: current_time_millis() + token.expires_in * 1000 - 5 * 60 * 1000,
        extra: BTreeMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_client_id_is_valid_uuid_shape() {
        let groups = ANTHROPIC_CLIENT_ID.split('-').collect::<Vec<_>>();

        assert_eq!(groups.len(), 5);
        assert_eq!(
            groups.iter().map(|group| group.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        assert_eq!(ANTHROPIC_CLIENT_ID, "9d1c250a-e61b-44d9-88ed-5944d1962f5e");
    }
}
