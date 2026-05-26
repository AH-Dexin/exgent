use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
        Arc,
    },
    time::Duration,
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use reqwest::Url;
use serde::Deserialize;

use crate::auth::OAuthCredential;

use super::{
    callback::{oauth_callback_host, start_callback_server},
    current_time_millis,
    pkce::{create_oauth_state, create_pkce_challenge, create_pkce_verifier},
    AuthorizationCode,
};

const OPENAI_CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const OPENAI_CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_CODEX_CALLBACK_PORT: u16 = 1455;
const OPENAI_CODEX_CALLBACK_PATH: &str = "/auth/callback";
const OPENAI_CODEX_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const OPENAI_CODEX_SCOPES: &str = "openid profile email offline_access";
const OPENAI_CODEX_JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";

#[derive(Debug)]
pub struct OpenAiCodexOAuthFlow {
    pub url: String,
    verifier: String,
    state: String,
    redirect_uri: String,
    callback: Receiver<Result<AuthorizationCode, String>>,
    cancel_callback: Arc<AtomicBool>,
}

impl OpenAiCodexOAuthFlow {
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

impl Drop for OpenAiCodexOAuthFlow {
    fn drop(&mut self) {
        self.cancel_callback.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiCodexTokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

pub fn start_openai_codex_oauth_flow() -> Result<OpenAiCodexOAuthFlow, String> {
    let verifier = create_pkce_verifier();
    let challenge = create_pkce_challenge(&verifier);
    let state = create_oauth_state();
    let callback_host = oauth_callback_host();
    let callback = start_callback_server(
        &callback_host,
        OPENAI_CODEX_CALLBACK_PORT,
        &state,
        OPENAI_CODEX_CALLBACK_PATH,
        "OpenAI Codex",
    )?;

    let mut url = Url::parse(OPENAI_CODEX_AUTHORIZE_URL)
        .map_err(|error| format!("invalid OpenAI Codex authorize URL: {error}"))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", OPENAI_CODEX_CLIENT_ID)
        .append_pair("redirect_uri", OPENAI_CODEX_REDIRECT_URI)
        .append_pair("scope", OPENAI_CODEX_SCOPES)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("originator", "exgent");

    Ok(OpenAiCodexOAuthFlow {
        url: url.to_string(),
        verifier,
        state,
        redirect_uri: OPENAI_CODEX_REDIRECT_URI.to_string(),
        callback: callback.receiver,
        cancel_callback: callback.cancel,
    })
}

pub fn finish_openai_codex_oauth_flow(
    flow: &OpenAiCodexOAuthFlow,
    authorization: AuthorizationCode,
) -> Result<OAuthCredential, String> {
    if !authorization.state.is_empty() && authorization.state != flow.state {
        return Err("OAuth state mismatch".to_string());
    }

    exchange_openai_codex_authorization_code(
        &authorization.code,
        &flow.verifier,
        &flow.redirect_uri,
    )
}

fn exchange_openai_codex_authorization_code(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthCredential, String> {
    let client = exgent_ai::shared_blocking_client();
    let response = client
        .post(OPENAI_CODEX_TOKEN_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", OPENAI_CODEX_CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .map_err(|error| format!("OpenAI Codex token request failed: {error}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .unwrap_or_else(|error| format!("failed to read error body: {error}"));
        return Err(format!(
            "OpenAI Codex token exchange failed with {status}: {body}"
        ));
    }

    let token: OpenAiCodexTokenResponse = response
        .json()
        .map_err(|error| format!("invalid OpenAI Codex token response: {error}"))?;
    let account_id = openai_codex_account_id(&token.access_token)?;
    let mut extra = BTreeMap::new();
    extra.insert("account_id".to_string(), account_id);

    Ok(OAuthCredential {
        access: token.access_token,
        refresh: token.refresh_token,
        expires: current_time_millis() + token.expires_in * 1000 - 5 * 60 * 1000,
        extra,
    })
}

fn openai_codex_account_id(access_token: &str) -> Result<String, String> {
    let payload = access_token
        .split('.')
        .nth(1)
        .ok_or_else(|| "OpenAI Codex access token must be a JWT".to_string())?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|error| format!("invalid OpenAI Codex access token: {error}"))?;
    let value: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|error| format!("invalid OpenAI Codex access token payload: {error}"))?;
    value
        .get(OPENAI_CODEX_JWT_CLAIM_PATH)
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|account_id| !account_id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "OpenAI Codex access token missing chatgpt_account_id".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_codex_client_id_is_configured() {
        assert_eq!(OPENAI_CODEX_CLIENT_ID, "app_EMoamEEZ73f0CkXaXp7hrann");
        assert_eq!(
            OPENAI_CODEX_REDIRECT_URI,
            "http://localhost:1455/auth/callback"
        );
    }

    #[test]
    fn extracts_openai_codex_account_id_from_jwt() {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                OPENAI_CODEX_JWT_CLAIM_PATH: {
                    "chatgpt_account_id": "acct_123"
                }
            })
            .to_string(),
        );
        let token = format!("{header}.{payload}.sig");

        assert_eq!(openai_codex_account_id(&token).unwrap(), "acct_123");
    }
}
