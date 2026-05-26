use std::{collections::BTreeMap, thread, time::Duration};

use reqwest::{blocking::Client, Url};
use serde::Deserialize;

use crate::auth::OAuthCredential;

const GITHUB_COPILOT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB_COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.35.0";

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

#[cfg(test)]
mod tests {
    use super::*;

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
