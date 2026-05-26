use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredModel {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl DiscoveredModel {
    fn new(id: impl Into<String>, name: Option<String>) -> Self {
        Self {
            id: id.into(),
            name,
        }
    }
}

pub fn supports_model_discovery(provider: &str) -> bool {
    matches!(provider, "openai" | "anthropic" | "google")
}

pub fn discover_available_models(
    provider: &str,
    api_key: &str,
    base_url: Option<&str>,
) -> Result<Vec<DiscoveredModel>, String> {
    match provider {
        "openai" => discover_openai_models(api_key, base_url),
        "anthropic" => discover_anthropic_models(api_key, base_url),
        "google" => discover_google_models(api_key, base_url),
        _ => Ok(Vec::new()),
    }
}

fn discover_openai_models(
    api_key: &str,
    base_url: Option<&str>,
) -> Result<Vec<DiscoveredModel>, String> {
    let url = format!(
        "{}/models",
        base_url
            .unwrap_or("https://api.openai.com/v1")
            .trim_end_matches('/')
    );
    let response = crate::shared_blocking_client()
        .get(url)
        .bearer_auth(api_key)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;

    let body = checked_body(response)?;
    parse_openai_models(&body)
}

fn discover_anthropic_models(
    api_key: &str,
    base_url: Option<&str>,
) -> Result<Vec<DiscoveredModel>, String> {
    let url = format!(
        "{}/v1/models",
        base_url
            .unwrap_or("https://api.anthropic.com")
            .trim_end_matches('/')
    );
    let response = crate::shared_blocking_client()
        .get(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .map_err(|error| format!("request failed: {error}"))?;

    let body = checked_body(response)?;
    parse_anthropic_models(&body)
}

fn discover_google_models(
    api_key: &str,
    base_url: Option<&str>,
) -> Result<Vec<DiscoveredModel>, String> {
    let url = format!(
        "{}/models",
        base_url
            .unwrap_or("https://generativelanguage.googleapis.com/v1beta")
            .trim_end_matches('/')
    );
    let response = crate::shared_blocking_client()
        .get(url)
        .header("x-goog-api-key", api_key)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;

    let body = checked_body(response)?;
    parse_google_models(&body)
}

fn checked_body(response: reqwest::blocking::Response) -> Result<String, String> {
    if response.status().is_success() {
        return response
            .text()
            .map_err(|error| format!("failed to read response body: {error}"));
    }

    let status = response.status();
    let body = response
        .text()
        .unwrap_or_else(|error| format!("failed to read error body: {error}"));
    Err(format!("provider returned {status}: {body}"))
}

#[derive(Debug, Deserialize)]
struct OpenAiModelsResponse {
    #[serde(default)]
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
}

fn parse_openai_models(body: &str) -> Result<Vec<DiscoveredModel>, String> {
    let response: OpenAiModelsResponse =
        serde_json::from_str(body).map_err(|error| format!("invalid models response: {error}"))?;
    Ok(response
        .data
        .into_iter()
        .map(|model| DiscoveredModel::new(model.id, None))
        .collect())
}

#[derive(Debug, Deserialize)]
struct AnthropicModelsResponse {
    #[serde(default)]
    data: Vec<AnthropicModel>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModel {
    id: String,
    display_name: Option<String>,
}

fn parse_anthropic_models(body: &str) -> Result<Vec<DiscoveredModel>, String> {
    let response: AnthropicModelsResponse =
        serde_json::from_str(body).map_err(|error| format!("invalid models response: {error}"))?;
    Ok(response
        .data
        .into_iter()
        .map(|model| DiscoveredModel::new(model.id, model.display_name))
        .collect())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleModelsResponse {
    #[serde(default)]
    models: Vec<GoogleModel>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleModel {
    name: String,
    display_name: Option<String>,
    #[serde(default)]
    supported_generation_methods: Vec<String>,
}

fn parse_google_models(body: &str) -> Result<Vec<DiscoveredModel>, String> {
    let response: GoogleModelsResponse =
        serde_json::from_str(body).map_err(|error| format!("invalid models response: {error}"))?;
    Ok(response
        .models
        .into_iter()
        .filter(|model| {
            model
                .supported_generation_methods
                .iter()
                .any(|method| method == "generateContent" || method == "streamGenerateContent")
        })
        .filter_map(|model| {
            let id = model
                .name
                .strip_prefix("models/")
                .unwrap_or(&model.name)
                .to_string();
            (!id.trim().is_empty()).then(|| DiscoveredModel::new(id, model.display_name))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_models() {
        let models = parse_openai_models(
            r#"{
              "object": "list",
              "data": [
                { "id": "gpt-4.1", "object": "model" },
                { "id": "gpt-5", "object": "model" }
              ]
            }"#,
        )
        .unwrap();

        assert_eq!(
            models,
            vec![
                DiscoveredModel::new("gpt-4.1", None),
                DiscoveredModel::new("gpt-5", None)
            ]
        );
    }

    #[test]
    fn parses_anthropic_models() {
        let models = parse_anthropic_models(
            r#"{
              "data": [
                { "id": "claude-sonnet-4-5", "display_name": "Claude Sonnet 4.5" }
              ]
            }"#,
        )
        .unwrap();

        assert_eq!(
            models,
            vec![DiscoveredModel::new(
                "claude-sonnet-4-5",
                Some("Claude Sonnet 4.5".to_string())
            )]
        );
    }

    #[test]
    fn parses_google_models_and_filters_non_generate_models() {
        let models = parse_google_models(
            r#"{
              "models": [
                {
                  "name": "models/gemini-2.5-flash",
                  "displayName": "Gemini 2.5 Flash",
                  "supportedGenerationMethods": ["generateContent", "countTokens"]
                },
                {
                  "name": "models/embedding-001",
                  "displayName": "Embedding",
                  "supportedGenerationMethods": ["embedContent"]
                }
              ]
            }"#,
        )
        .unwrap();

        assert_eq!(
            models,
            vec![DiscoveredModel::new(
                "gemini-2.5-flash",
                Some("Gemini 2.5 Flash".to_string())
            )]
        );
    }
}
