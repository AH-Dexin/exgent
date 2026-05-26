#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibleModelKind {
    OpenAi,
    Anthropic,
    Google,
}

impl CompatibleModelKind {
    pub const ALL: [Self; 3] = [Self::OpenAi, Self::Anthropic, Self::Google];

    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI-compatible",
            Self::Anthropic => "Anthropic-compatible",
            Self::Google => "Google-compatible",
        }
    }

    pub fn adapter(self) -> &'static str {
        match self {
            Self::OpenAi => "openai-completions",
            Self::Anthropic => "anthropic-messages",
            Self::Google => "google-generative-ai",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.example.com/v1",
            Self::Anthropic => "https://api.anthropic.com",
            Self::Google => "https://generativelanguage.googleapis.com/v1beta",
        }
    }

    pub fn provider_hint(self) -> &'static str {
        match self {
            Self::OpenAi => "provider id, e.g. deepseek",
            Self::Anthropic => "provider id, e.g. anthropic-proxy",
            Self::Google => "provider id, e.g. google-proxy",
        }
    }

    pub fn model_hint(self) -> &'static str {
        match self {
            Self::OpenAi => "model id, e.g. deepseek-chat",
            Self::Anthropic => "model id, e.g. claude-sonnet-4-5",
            Self::Google => "model id, e.g. gemini-2.5-flash",
        }
    }

    pub fn base_url_hint(self) -> &'static str {
        match self {
            Self::OpenAi => "base url, e.g. https://api.deepseek.com/v1",
            Self::Anthropic => "base url, e.g. https://api.anthropic.com",
            Self::Google => "base url, e.g. https://generativelanguage.googleapis.com/v1beta",
        }
    }
}
