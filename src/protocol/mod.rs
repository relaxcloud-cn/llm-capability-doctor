use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Protocol {
    OpenAiChat,
    OpenAiResponses,
    AnthropicMessages,
    GeminiGenerateContent,
    OllamaChat,
    Unknown,
}

impl fmt::Display for Protocol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OpenAiChat => "openai_chat",
            Self::OpenAiResponses => "openai_responses",
            Self::AnthropicMessages => "anthropic_messages",
            Self::GeminiGenerateContent => "gemini_generate_content",
            Self::OllamaChat => "ollama_chat",
            Self::Unknown => "unknown",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AuthMode {
    Bearer,
    ApiKey,
    XApiKey,
    XGoogApiKey,
    None,
}

impl fmt::Display for AuthMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Bearer => "bearer",
            Self::ApiKey => "api_key",
            Self::XApiKey => "x_api_key",
            Self::XGoogApiKey => "x_goog_api_key",
            Self::None => "none",
        })
    }
}
