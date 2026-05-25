use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
        Arc,
    },
    thread,
    time::Duration,
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::auth::OAuthCredential;

const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const ANTHROPIC_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const ANTHROPIC_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const ANTHROPIC_CALLBACK_PORT: u16 = 53692;
const ANTHROPIC_CALLBACK_PATH: &str = "/callback";
const ANTHROPIC_REDIRECT_URI: &str = "http://localhost:53692/callback";
const ANTHROPIC_SCOPES: &str =
    "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
const GITHUB_COPILOT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB_COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.35.0";

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationCode {
    code: String,
    state: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GithubDeviceFlow {
    pub domain: String,
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval_seconds: u64,
    pub expires_in_seconds: u64,
    pub enterprise_domain: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval: u64,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct DeviceTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    token: String,
    expires_at: i64,
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
    let callback = start_callback_server(&callback_host, ANTHROPIC_CALLBACK_PORT, &verifier)?;

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

pub fn parse_authorization_input(input: &str) -> Result<Option<AuthorizationCode>, String> {
    let value = input.trim();
    if value.is_empty() {
        return Ok(None);
    }

    if let Ok(url) = Url::parse(value) {
        let code = url
            .query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, value)| value.to_string());
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.to_string());
        return Ok(match (code, state) {
            (Some(code), Some(state)) => Some(AuthorizationCode { code, state }),
            (Some(code), None) => Some(AuthorizationCode {
                code,
                state: String::new(),
            }),
            _ => None,
        });
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
        let code = url
            .query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, value)| value.to_string());
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.to_string());
        return Ok(match (code, state) {
            (Some(code), Some(state)) => Some(AuthorizationCode { code, state }),
            (Some(code), None) => Some(AuthorizationCode {
                code,
                state: String::new(),
            }),
            _ => None,
        });
    }

    Ok(Some(AuthorizationCode {
        code: value.to_string(),
        state: String::new(),
    }))
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

pub fn normalize_github_domain(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let url = if trimmed.contains("://") {
        Url::parse(trimmed).ok()?
    } else {
        Url::parse(&format!("https://{trimmed}")).ok()?
    };

    url.host_str().map(str::to_string)
}

pub fn start_github_copilot_device_flow(
    enterprise_domain: Option<&str>,
) -> Result<GithubDeviceFlow, String> {
    let domain = enterprise_domain.unwrap_or("github.com").to_string();
    let urls = github_urls(&domain);
    let client = Client::new();
    let response = client
        .post(urls.device_code_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(reqwest::header::USER_AGENT, GITHUB_COPILOT_USER_AGENT)
        .form(&[
            ("client_id", GITHUB_COPILOT_CLIENT_ID),
            ("scope", "read:user"),
        ])
        .send()
        .map_err(|error| format!("device flow request failed: {error}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .unwrap_or_else(|error| format!("failed to read error body: {error}"));
        return Err(format!("device flow failed with {status}: {body}"));
    }

    let device: DeviceCodeResponse = response
        .json()
        .map_err(|error| format!("invalid device flow response: {error}"))?;

    Ok(GithubDeviceFlow {
        domain,
        device_code: device.device_code,
        user_code: device.user_code,
        verification_uri: device.verification_uri,
        interval_seconds: device.interval,
        expires_in_seconds: device.expires_in,
        enterprise_domain: enterprise_domain.map(str::to_string),
    })
}

pub fn finish_github_copilot_device_flow<F>(
    flow: &GithubDeviceFlow,
    mut progress: F,
) -> Result<OAuthCredential, String>
where
    F: FnMut(&str),
{
    let github_access_token = poll_for_github_access_token(flow, &mut |message| {
        progress(message);
        true
    })?;
    refresh_github_copilot_token(&github_access_token, flow.enterprise_domain.as_deref())
}

pub fn finish_github_copilot_device_flow_cancellable<F>(
    flow: &GithubDeviceFlow,
    progress: &mut F,
) -> Result<OAuthCredential, String>
where
    F: FnMut(&str) -> bool,
{
    let github_access_token = poll_for_github_access_token(flow, progress)?;
    refresh_github_copilot_token(&github_access_token, flow.enterprise_domain.as_deref())
}

fn poll_for_github_access_token<F>(
    flow: &GithubDeviceFlow,
    progress: &mut F,
) -> Result<String, String>
where
    F: FnMut(&str) -> bool,
{
    let urls = github_urls(&flow.domain);
    let client = Client::new();
    let mut interval_seconds = flow.interval_seconds.max(1);
    let deadline = std::time::Instant::now() + Duration::from_secs(flow.expires_in_seconds);

    loop {
        if std::time::Instant::now() >= deadline {
            return Err("device flow timed out".to_string());
        }

        let wait_until = std::time::Instant::now() + Duration::from_secs(interval_seconds);
        while std::time::Instant::now() < wait_until {
            if !progress("Waiting for browser authentication... Esc cancels.") {
                return Err("device flow cancelled".to_string());
            }
            thread::sleep(Duration::from_millis(100));
        }

        let response = client
            .post(&urls.access_token_url)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(reqwest::header::USER_AGENT, GITHUB_COPILOT_USER_AGENT)
            .form(&[
                ("client_id", GITHUB_COPILOT_CLIENT_ID),
                ("device_code", flow.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .map_err(|error| format!("device token request failed: {error}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .unwrap_or_else(|error| format!("failed to read error body: {error}"));
            return Err(format!("device token request failed with {status}: {body}"));
        }

        let token: DeviceTokenResponse = response
            .json()
            .map_err(|error| format!("invalid device token response: {error}"))?;
        if let Some(access_token) = token.access_token {
            return Ok(access_token);
        }

        match token.error.as_deref() {
            Some("authorization_pending") => {}
            Some("slow_down") => {
                interval_seconds = token.interval.unwrap_or(interval_seconds + 5).max(1);
            }
            Some(error) => {
                let description = token
                    .error_description
                    .map(|value| format!(": {value}"))
                    .unwrap_or_default();
                return Err(format!("device flow failed: {error}{description}"));
            }
            None => return Err("device token response missing access_token".to_string()),
        }
    }
}

pub fn refresh_github_copilot_token(
    github_access_token: &str,
    enterprise_domain: Option<&str>,
) -> Result<OAuthCredential, String> {
    let domain = enterprise_domain.unwrap_or("github.com");
    let urls = github_urls(domain);
    let client = Client::new();
    let response = client
        .get(urls.copilot_token_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, GITHUB_COPILOT_USER_AGENT)
        .header("Editor-Version", "vscode/1.107.0")
        .header("Editor-Plugin-Version", "copilot-chat/0.35.0")
        .header("Copilot-Integration-Id", "vscode-chat")
        .bearer_auth(github_access_token)
        .send()
        .map_err(|error| format!("copilot token request failed: {error}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .unwrap_or_else(|error| format!("failed to read error body: {error}"));
        return Err(format!(
            "copilot token request failed with {status}: {body}"
        ));
    }

    let token: CopilotTokenResponse = response
        .json()
        .map_err(|error| format!("invalid copilot token response: {error}"))?;
    let mut extra = BTreeMap::new();
    if let Some(domain) = enterprise_domain {
        extra.insert("enterprise_url".to_string(), domain.to_string());
    }

    Ok(OAuthCredential {
        access: token.token,
        refresh: github_access_token.to_string(),
        expires: token.expires_at * 1000 - 5 * 60 * 1000,
        extra,
    })
}

struct GithubUrls {
    device_code_url: String,
    access_token_url: String,
    copilot_token_url: String,
}

fn github_urls(domain: &str) -> GithubUrls {
    GithubUrls {
        device_code_url: format!("https://{domain}/login/device/code"),
        access_token_url: format!("https://{domain}/login/oauth/access_token"),
        copilot_token_url: format!("https://api.{domain}/copilot_internal/v2/token"),
    }
}

struct CallbackServer {
    receiver: Receiver<Result<AuthorizationCode, String>>,
    cancel: Arc<AtomicBool>,
}

fn start_callback_server(
    host: &str,
    port: u16,
    expected_state: &str,
) -> Result<CallbackServer, String> {
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut bind_errors = Vec::new();
    let mut listener_count = 0usize;

    for bind_host in callback_bind_hosts(host) {
        match TcpListener::bind((bind_host.as_str(), port)) {
            Ok(listener) => {
                listener.set_nonblocking(true).map_err(|error| {
                    format!("failed to configure callback server on {bind_host}:{port}: {error}")
                })?;
                listener_count += 1;
                spawn_callback_listener(
                    listener,
                    expected_state.to_string(),
                    Arc::clone(&cancel),
                    sender.clone(),
                );
            }
            Err(error) => {
                bind_errors.push(format!("{bind_host}:{port}: {error}"));
            }
        }
    }

    if listener_count == 0 {
        return Err(format!(
            "failed to listen for OAuth callback: {}",
            bind_errors.join("; ")
        ));
    }

    Ok(CallbackServer { receiver, cancel })
}

fn callback_bind_hosts(host: &str) -> Vec<String> {
    match host {
        "127.0.0.1" | "localhost" => vec!["127.0.0.1".to_string(), "::1".to_string()],
        value => vec![value.to_string()],
    }
}

fn spawn_callback_listener(
    listener: TcpListener,
    expected_state: String,
    cancel: Arc<AtomicBool>,
    sender: mpsc::Sender<Result<AuthorizationCode, String>>,
) {
    thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10 * 60);
        loop {
            if cancel.load(Ordering::Relaxed) || std::time::Instant::now() >= deadline {
                return;
            }

            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let result = handle_callback_stream(&mut stream, &expected_state);
                    let _ = sender.send(result);
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                }
                Err(error) => {
                    let _ = sender.send(Err(format!("callback server failed: {error}")));
                    return;
                }
            }
        }
    });
}

fn handle_callback_stream(
    stream: &mut TcpStream,
    expected_state: &str,
) -> Result<AuthorizationCode, String> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| format!("failed to read callback request: {error}"))?,
    );
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|error| format!("failed to read callback request: {error}"))?;
    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "invalid callback request".to_string())?;
    let url = Url::parse(&format!("http://localhost{path}"))
        .map_err(|error| format!("invalid callback URL: {error}"))?;

    if url.path() != ANTHROPIC_CALLBACK_PATH {
        write_callback_response(stream, 404, "Callback route not found.");
        return Err("callback route not found".to_string());
    }

    if let Some(error) = url
        .query_pairs()
        .find(|(key, _)| key == "error")
        .map(|(_, value)| value.to_string())
    {
        write_callback_response(stream, 400, "Anthropic authentication did not complete.");
        return Err(format!("Anthropic authentication error: {error}"));
    }

    let code = url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| "missing authorization code".to_string())?;
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.to_string())
        .ok_or_else(|| "missing OAuth state".to_string())?;

    if state != expected_state {
        write_callback_response(stream, 400, "OAuth state mismatch.");
        return Err("OAuth state mismatch".to_string());
    }

    write_callback_response(
        stream,
        200,
        "Anthropic authentication completed. You can close this window.",
    );
    Ok(AuthorizationCode { code, state })
}

fn write_callback_response(stream: &mut TcpStream, status: u16, message: &str) {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>exgent OAuth</title></head><body><h1>{message}</h1></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn oauth_callback_host() -> String {
    std::env::var("EXGENT_OAUTH_CALLBACK_HOST")
        .or_else(|_| std::env::var("PI_OAUTH_CALLBACK_HOST"))
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

fn create_pkce_verifier() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn create_pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
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
            create_pkce_challenge("test-verifier"),
            "JBbiqONGWPaAmwXk_8bT6UnlPfrn65D32eZlJS-zGG0"
        );
    }

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

    #[test]
    fn binds_ipv4_and_ipv6_loopback_for_localhost_callback() {
        assert_eq!(
            callback_bind_hosts("127.0.0.1"),
            vec!["127.0.0.1".to_string(), "::1".to_string()]
        );
        assert_eq!(
            callback_bind_hosts("localhost"),
            vec!["127.0.0.1".to_string(), "::1".to_string()]
        );
        assert_eq!(callback_bind_hosts("0.0.0.0"), vec!["0.0.0.0".to_string()]);
    }

    #[test]
    fn normalizes_github_enterprise_domain() {
        assert_eq!(
            normalize_github_domain("https://company.ghe.com/path"),
            Some("company.ghe.com".to_string())
        );
        assert_eq!(
            normalize_github_domain("company.ghe.com"),
            Some("company.ghe.com".to_string())
        );
        assert_eq!(normalize_github_domain(""), None);
    }
}
