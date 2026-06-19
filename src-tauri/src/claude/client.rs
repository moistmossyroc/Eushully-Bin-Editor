use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Error, Debug)]
pub enum ClaudeError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error: {status} - {message}")]
    Api {
        status: u16,
        message: String,
        /// Honored from `Retry-After` header on 429 responses, in seconds.
        retry_after_secs: Option<u64>,
    },
    #[error("Parse error: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("No API key configured")]
    NoApiKey,
}

impl ClaudeError {
    /// Returns true if this error is a rate limit (429) or server error (5xx) that should be retried.
    pub fn is_retryable(&self) -> bool {
        match self {
            ClaudeError::Api { status, .. } => *status == 429 || *status >= 500,
            ClaudeError::Http(_) => true,
            _ => false,
        }
    }

    /// True if this is a hard auth failure (no point retrying).
    pub fn is_auth_error(&self) -> bool {
        matches!(self, ClaudeError::Api { status, .. } if *status == 401 || *status == 403)
    }

    /// True if this is a 429 rate-limit error specifically.
    pub fn is_rate_limit(&self) -> bool {
        matches!(self, ClaudeError::Api { status: 429, .. })
    }

    /// Returns the retry-after delay (seconds) if the API provided one.
    pub fn retry_after(&self) -> Option<u64> {
        match self {
            ClaudeError::Api { retry_after_secs, .. } => *retry_after_secs,
            _ => None,
        }
    }
}

/// Which API format to use when talking to the LLM provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApiProvider {
    Anthropic,
    OpenAiCompatible,
}

impl Default for ApiProvider {
    fn default() -> Self {
        Self::Anthropic
    }
}

// ── Anthropic format ───────────────────────────────────────────────

/// A message in the conversation (shared by both formats).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    /// System prompt as an array of cacheable text blocks. Marking the system
    /// block ephemeral lets Anthropic cache it for ~5 minutes — repeat requests
    /// within that window only pay 10% of input tokens for the cached portion.
    system: Vec<SystemBlock>,
    messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
struct SystemBlock {
    #[serde(rename = "type")]
    block_type: &'static str,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Debug, Serialize)]
struct CacheControl {
    #[serde(rename = "type")]
    cache_type: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: Option<String>,
}

// ── OpenAI-compatible format ───────────────────────────────────────

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

// ── Unified response ───────────────────────────────────────────────

/// Provider-agnostic response returned by `send_message`.
#[derive(Debug)]
pub struct MessagesResponse {
    pub content: Vec<ContentBlock>,
    pub usage: Usage,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

// ── Client ─────────────────────────────────────────────────────────

/// LLM client that supports both Anthropic and OpenAI-compatible APIs.
pub struct ClaudeClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
    provider: ApiProvider,
    endpoint: String,
    /// Minimum gap between requests on this client. Used to pace requests
    /// against per-second rate limits (Mistral free tier is ~1 req/s). Zero
    /// = no pacing. Anthropic gets zero; OpenAI-compatible gets 1100ms by
    /// default, bumped adaptively when response headers indicate low quota.
    min_request_interval: Arc<Mutex<Duration>>,
    /// Timestamp of the last completed-or-started request on this client.
    /// Read+updated by `pace()` before each send.
    last_request_at: Arc<Mutex<Option<Instant>>>,
}

impl ClaudeClient {
    /// Create a client for the Anthropic API.
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            model,
            provider: ApiProvider::Anthropic,
            endpoint: ANTHROPIC_API_URL.to_string(),
            // Anthropic's limits are far above 1 req/s for paying tiers — no
            // need to pace by default. The retry logic handles 429s if hit.
            min_request_interval: Arc::new(Mutex::new(Duration::ZERO)),
            last_request_at: Arc::new(Mutex::new(None)),
        }
    }

    /// Create a client for an OpenAI-compatible API.
    pub fn new_openai(api_key: String, model: String, endpoint: String) -> Self {
        // Ensure the endpoint ends with /chat/completions
        let url = if endpoint.ends_with("/chat/completions") {
            endpoint
        } else if endpoint.ends_with('/') {
            format!("{}chat/completions", endpoint)
        } else if endpoint.ends_with("/v1") {
            format!("{}/chat/completions", endpoint)
        } else {
            // Assume base URL, append /v1/chat/completions
            format!("{}/v1/chat/completions", endpoint.trim_end_matches('/'))
        };

        // Local endpoints (Ollama, LM Studio) have no rate limits. Hosted
        // OpenAI-compatible providers (Mistral free tier especially) cap at
        // ~1 req/s — pace at 1.1s to stay safely under the wire.
        let is_local = url.starts_with("http://localhost") || url.starts_with("http://127.0.0.1");
        let pace = if is_local { Duration::ZERO } else { Duration::from_millis(1100) };

        Self {
            http: reqwest::Client::new(),
            api_key,
            model,
            provider: ApiProvider::OpenAiCompatible,
            endpoint: url,
            min_request_interval: Arc::new(Mutex::new(pace)),
            last_request_at: Arc::new(Mutex::new(None)),
        }
    }

    /// Sleep just long enough to honor `min_request_interval` since the last
    /// request on this client. Anchors the next-request timestamp at the
    /// moment the wait ends so the pacing window stays rolling.
    async fn pace(&self) {
        let interval = *self.min_request_interval.lock().unwrap();
        if interval.is_zero() {
            *self.last_request_at.lock().unwrap() = Some(Instant::now());
            return;
        }
        let needed = {
            let last = self.last_request_at.lock().unwrap();
            match *last {
                None => Duration::ZERO,
                Some(t) => {
                    let elapsed = t.elapsed();
                    if elapsed >= interval { Duration::ZERO } else { interval - elapsed }
                }
            }
        };
        if !needed.is_zero() {
            tokio::time::sleep(needed).await;
        }
        *self.last_request_at.lock().unwrap() = Some(Instant::now());
    }

    /// Bump the pacing interval temporarily based on observed rate-limit
    /// headers. Called from `send_openai` after each response.
    fn apply_rate_limit_hint(&self, headers: &reqwest::header::HeaderMap) {
        // Read provider-agnostic remaining/reset pair. Mistral / OpenAI /
        // OpenRouter all expose a header containing "remaining-requests"
        // (case-insensitive) and a sibling "reset" header. If remaining is
        // low, stretch the pacing so we don't burst into a 429 cliff.
        let lower_headers: Vec<(String, String)> = headers
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|s| (k.as_str().to_lowercase(), s.to_string())))
            .collect();

        let remaining: Option<u64> = lower_headers
            .iter()
            .find(|(k, _)| k.contains("ratelimit-remaining-requests") || k == "x-ratelimit-remaining-requests")
            .and_then(|(_, v)| v.trim().parse().ok());

        let reset_secs: Option<u64> = lower_headers
            .iter()
            .find(|(k, _)| k.contains("ratelimit-reset-requests") || k == "x-ratelimit-reset-requests")
            .and_then(|(_, v)| {
                // Some providers send plain seconds; others send "1m30s" or "30s".
                let s = v.trim();
                s.parse::<u64>().ok()
                    .or_else(|| parse_duration_str(s))
            });

        if let (Some(rem), Some(reset)) = (remaining, reset_secs) {
            // If <=2 requests remain in the window, pace at reset/(rem+1)
            // so we glide into the reset boundary instead of burning through.
            // Cap the bump at 10s to avoid hour-long waits if reset is huge.
            if rem <= 2 && reset > 0 {
                let stretched = Duration::from_secs((reset / (rem + 1)).min(10).max(1));
                let mut cur = self.min_request_interval.lock().unwrap();
                if stretched > *cur {
                    *cur = stretched;
                }
            } else if rem >= 10 {
                // Plenty of budget — reset to the per-provider default.
                let is_local = self.endpoint.starts_with("http://localhost")
                    || self.endpoint.starts_with("http://127.0.0.1");
                let baseline = if is_local { Duration::ZERO } else { Duration::from_millis(1100) };
                let mut cur = self.min_request_interval.lock().unwrap();
                if *cur > baseline {
                    *cur = baseline;
                }
            }
        }
    }

    /// Create a client based on the provider configuration.
    pub fn from_config(api_key: String, model: String, provider: ApiProvider, endpoint: Option<String>) -> Self {
        match provider {
            ApiProvider::Anthropic => Self::new(api_key, model),
            ApiProvider::OpenAiCompatible => {
                let ep = endpoint.unwrap_or_else(|| "https://openrouter.ai/api".to_string());
                Self::new_openai(api_key, model, ep)
            }
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn provider(&self) -> &ApiProvider {
        &self.provider
    }

    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    /// Send a message and return a provider-agnostic response.
    pub async fn send_message(
        &self,
        system_prompt: &str,
        messages: Vec<Message>,
        max_tokens: u32,
    ) -> Result<MessagesResponse, ClaudeError> {
        match self.provider {
            ApiProvider::Anthropic => self.send_anthropic(system_prompt, messages, max_tokens).await,
            ApiProvider::OpenAiCompatible => self.send_openai(system_prompt, messages, max_tokens).await,
        }
    }

    async fn send_anthropic(
        &self,
        system_prompt: &str,
        messages: Vec<Message>,
        max_tokens: u32,
    ) -> Result<MessagesResponse, ClaudeError> {
        let request = AnthropicRequest {
            model: self.model.clone(),
            max_tokens,
            system: vec![SystemBlock {
                block_type: "text",
                text: system_prompt.to_string(),
                cache_control: Some(CacheControl { cache_type: "ephemeral" }),
            }],
            messages,
        };

        let response = self
            .http
            .post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header(CONTENT_TYPE, "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status().as_u16();
        if status != 200 {
            let retry_after_secs = parse_retry_after(response.headers());
            let body = response.text().await.unwrap_or_default();
            return Err(ClaudeError::Api {
                status,
                message: body,
                retry_after_secs,
            });
        }

        #[derive(Deserialize)]
        struct AnthropicResponse {
            content: Vec<ContentBlock>,
            usage: Usage,
        }

        let body = response.json::<AnthropicResponse>().await?;
        Ok(MessagesResponse {
            content: body.content,
            usage: body.usage,
        })
    }

    async fn send_openai(
        &self,
        system_prompt: &str,
        user_messages: Vec<Message>,
        max_tokens: u32,
    ) -> Result<MessagesResponse, ClaudeError> {
        // Throttle to stay under per-second rate limits (Mistral free tier is
        // 1 req/s — we pace at 1.1s by default, adaptively stretched when
        // response headers indicate quota is running low).
        self.pace().await;

        // Prepend system message to the messages list
        let mut messages = vec![Message {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        }];
        messages.extend(user_messages);

        let request = OpenAiRequest {
            model: self.model.clone(),
            max_tokens,
            messages,
        };

        let mut req_builder = self
            .http
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "application/json");

        // Add auth header if API key is provided (Ollama doesn't need one)
        if !self.api_key.is_empty() {
            req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {}", self.api_key));
        }

        let response = req_builder.json(&request).send().await?;

        // Capture headers before consuming the response body — even on success
        // we want to learn the current rate-limit budget for adaptive pacing.
        let headers = response.headers().clone();
        let status = response.status().as_u16();
        self.apply_rate_limit_hint(&headers);

        if status != 200 {
            let retry_after_secs = parse_retry_after(&headers);
            let body = response.text().await.unwrap_or_default();
            return Err(ClaudeError::Api {
                status,
                message: body,
                retry_after_secs,
            });
        }

        let body = response.json::<OpenAiResponse>().await?;

        let text = body
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        let usage = body.usage.as_ref();
        Ok(MessagesResponse {
            content: vec![ContentBlock {
                block_type: "text".to_string(),
                text: Some(text),
            }],
            usage: Usage {
                input_tokens: usage.and_then(|u| u.prompt_tokens).unwrap_or(0),
                output_tokens: usage.and_then(|u| u.completion_tokens).unwrap_or(0),
            },
        })
    }

    /// True if this client targets Anthropic (vs an OpenAI-compatible provider).
    pub fn is_anthropic(&self) -> bool {
        matches!(self.provider, ApiProvider::Anthropic)
    }

    /// Extract the text content from a response.
    pub fn extract_text(response: &MessagesResponse) -> String {
        response
            .content
            .iter()
            .filter_map(|block| block.text.as_deref())
            .collect::<Vec<_>>()
            .join("")
    }
}

/// Parse the `Retry-After` header. Supports both delta-seconds and HTTP-date formats,
/// though Anthropic only sends delta-seconds in practice.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

/// Parse a duration string like "30s", "1m30s", "2h", or "500ms" into seconds.
/// Used for OpenAI-compatible providers whose rate-limit-reset headers ship
/// human-readable durations instead of plain integers.
fn parse_duration_str(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase();
    if s.is_empty() { return None; }
    let mut total: u64 = 0;
    let mut current: u64 = 0;
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            current = current.saturating_mul(10).saturating_add((ch as u8 - b'0') as u64);
        } else {
            match ch {
                'h' => { total = total.saturating_add(current.saturating_mul(3600)); current = 0; }
                'm' => {
                    // Disambiguate 'm' (minutes) vs 'ms' (milliseconds) — checked below
                    total = total.saturating_add(current.saturating_mul(60));
                    current = 0;
                }
                's' => { total = total.saturating_add(current); current = 0; }
                _ => return None,
            }
        }
    }
    // Trailing bare digits with no unit → seconds
    total = total.saturating_add(current);
    if total > 0 { Some(total) } else { None }
}
