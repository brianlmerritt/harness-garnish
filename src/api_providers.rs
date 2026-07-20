use crate::{
    domain::ApiBudget,
    secrets::{SecretReference, SecretValue},
};
use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt};

const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
const MAX_EVENTS: usize = 100_000;
const MAX_ITEMS: usize = 4_096;
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_INSTRUCTIONS_BYTES: usize = 256 * 1024;
const MAX_INPUT_BYTES: usize = 1024 * 1024;
const MAX_REQUEST_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOOLS: usize = 64;

#[derive(Clone, PartialEq)]
pub struct ApiToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Clone, PartialEq)]
pub struct ApiRequestSpec {
    pub provider: String,
    pub model: String,
    pub instructions: String,
    pub input: String,
    pub max_output_tokens: u64,
    pub tools: Vec<ApiToolDefinition>,
    pub stream: bool,
}

impl fmt::Debug for ApiRequestSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiRequestSpec")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("instructions", &"[REDACTED]")
            .field("input", &"[REDACTED]")
            .field("max_output_tokens", &self.max_output_tokens)
            .field(
                "tools",
                &self
                    .tools
                    .iter()
                    .map(|tool| tool.name.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("stream", &self.stream)
            .finish()
    }
}

pub struct PreparedApiRequest {
    provider: String,
    endpoint: &'static str,
    public_headers: BTreeMap<String, String>,
    secret_header_name: &'static str,
    secret_header_prefix: &'static [u8],
    secret: SecretValue,
    body: Vec<u8>,
}

impl PreparedApiRequest {
    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn endpoint(&self) -> &str {
        self.endpoint
    }

    pub fn body_sha256(&self) -> String {
        hex::encode(Sha256::digest(&self.body))
    }

    pub fn with_sensitive_parts<T>(
        &self,
        operation: impl FnOnce(&str, &BTreeMap<String, String>, &str, &[u8], &[u8], &[u8]) -> T,
    ) -> T {
        self.secret.expose(|secret| {
            operation(
                self.endpoint,
                &self.public_headers,
                self.secret_header_name,
                self.secret_header_prefix,
                secret,
                &self.body,
            )
        })
    }
}

impl fmt::Debug for PreparedApiRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedApiRequest")
            .field("provider", &self.provider)
            .field("endpoint", &self.endpoint)
            .field("public_headers", &self.public_headers)
            .field("secret_header_name", &self.secret_header_name)
            .field("secret_header", &"[REDACTED]")
            .field("body", &"[REDACTED]")
            .field("body_bytes", &self.body.len())
            .field("body_sha256", &self.body_sha256())
            .finish()
    }
}

pub trait ApiTransport {
    fn send(&mut self, request: &PreparedApiRequest) -> Result<ApiTransportResponse>;
}

pub struct ApiTransportResponse {
    status_code: u16,
    request_id: String,
    body: Vec<u8>,
    streamed: bool,
}

impl ApiTransportResponse {
    pub fn new(
        status_code: u16,
        request_id: String,
        body: Vec<u8>,
        streamed: bool,
    ) -> Result<Self> {
        if !(100..=599).contains(&status_code) {
            bail!("api.transport_status_invalid: HTTP status is out of range");
        }
        validate_opaque_id(&request_id, "provider request ID")?;
        validate_payload(&body)?;
        Ok(Self {
            status_code,
            request_id,
            body,
            streamed,
        })
    }

    pub fn status_code(&self) -> u16 {
        self.status_code
    }
}

impl fmt::Debug for ApiTransportResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiTransportResponse")
            .field("status_code", &self.status_code)
            .field("request_id", &"[REDACTED]")
            .field("body", &"[REDACTED]")
            .field("body_bytes", &self.body.len())
            .field("streamed", &self.streamed)
            .finish()
    }
}

pub fn parse_api_transport_response(
    provider: &str,
    response: &ApiTransportResponse,
) -> Result<ApiProviderResponse> {
    if !(200..=299).contains(&response.status_code) {
        let classification = classify_api_failure(provider, response.status_code, &response.body);
        bail!(
            "api.provider_failure: kind={:?} retryable={}",
            classification.kind,
            classification.retryable
        );
    }
    match (provider, response.streamed) {
        ("openai", false) => parse_openai_response(&response.request_id, &response.body),
        ("openai", true) => parse_openai_response_stream(&response.request_id, &response.body),
        ("anthropic", false) => parse_anthropic_response(&response.request_id, &response.body),
        ("anthropic", true) => {
            parse_anthropic_response_stream(&response.request_id, &response.body)
        }
        _ => bail!("api.provider_denied: unsupported API provider"),
    }
}

pub(crate) fn prepare_api_request(
    budget: &ApiBudget,
    spec: &ApiRequestSpec,
    now: DateTime<Utc>,
    expected_request_digest: &str,
) -> Result<PreparedApiRequest> {
    validate_request_against_budget(budget, spec, now)?;
    validate_sha256_text(expected_request_digest, "expected API request digest")?;
    let body = build_api_request_body(spec)?;
    if body.len() > MAX_REQUEST_BODY_BYTES {
        bail!("api.request_bounds: serialized request body exceeds byte limit");
    }
    let body_digest = hex::encode(Sha256::digest(&body));
    if !body_digest.eq_ignore_ascii_case(expected_request_digest) {
        bail!("api.request_digest_mismatch: prepared request differs from its reservation");
    }
    let (endpoint, headers, secret_header_name, secret_header_prefix) = match spec.provider.as_str()
    {
        "openai" => (
            "https://api.openai.com/v1/responses",
            BTreeMap::from([("content-type".into(), "application/json".into())]),
            "authorization",
            b"Bearer ".as_slice(),
        ),
        "anthropic" => (
            "https://api.anthropic.com/v1/messages",
            BTreeMap::from([
                ("content-type".into(), "application/json".into()),
                ("anthropic-version".into(), "2023-06-01".into()),
            ]),
            "x-api-key",
            b"".as_slice(),
        ),
        _ => bail!("api.provider_denied: unsupported API provider"),
    };
    // Resolution is deliberately last: every policy-shaped budget/model/tool/output/period
    // check and all local serialization bounds fail before a secret provider is touched.
    let secret = SecretReference::parse(&budget.secret_reference)?.resolve()?;
    Ok(PreparedApiRequest {
        provider: spec.provider.clone(),
        endpoint,
        public_headers: headers,
        secret_header_name,
        secret_header_prefix,
        secret,
        body,
    })
}

pub fn api_request_digest(
    budget: &ApiBudget,
    spec: &ApiRequestSpec,
    now: DateTime<Utc>,
) -> Result<String> {
    validate_request_against_budget(budget, spec, now)?;
    let body = build_api_request_body(spec)?;
    if body.len() > MAX_REQUEST_BODY_BYTES {
        bail!("api.request_bounds: serialized request body exceeds byte limit");
    }
    Ok(hex::encode(Sha256::digest(body)))
}

pub fn api_request_content_digest(spec: &ApiRequestSpec) -> Result<String> {
    validate_request_shape(spec)?;
    let body = build_api_request_body(spec)?;
    if body.len() > MAX_REQUEST_BODY_BYTES {
        bail!("api.request_bounds: serialized request body exceeds byte limit");
    }
    Ok(hex::encode(Sha256::digest(body)))
}

pub fn api_request_conservative_input_token_bound(
    budget: &ApiBudget,
    spec: &ApiRequestSpec,
    now: DateTime<Utc>,
) -> Result<u64> {
    validate_request_against_budget(budget, spec, now)?;
    let body = build_api_request_body(spec)?;
    if body.len() > MAX_REQUEST_BODY_BYTES {
        bail!("api.request_bounds: serialized request body exceeds byte limit");
    }
    // One UTF-8 byte per token is deliberately conservative for admission control.
    u64::try_from(body.len()).map_err(|_| anyhow!("api.request_bounds: request size overflow"))
}

pub fn api_request_conservative_content_token_bound(spec: &ApiRequestSpec) -> Result<u64> {
    validate_request_shape(spec)?;
    let body = build_api_request_body(spec)?;
    if body.len() > MAX_REQUEST_BODY_BYTES {
        bail!("api.request_bounds: serialized request body exceeds byte limit");
    }
    u64::try_from(body.len()).map_err(|_| anyhow!("api.request_bounds: request size overflow"))
}

fn build_api_request_body(spec: &ApiRequestSpec) -> Result<Vec<u8>> {
    match spec.provider.as_str() {
        "openai" => build_openai_body(spec),
        "anthropic" => build_anthropic_body(spec),
        _ => bail!("api.provider_denied: unsupported API provider"),
    }
}

fn validate_request_against_budget(
    budget: &ApiBudget,
    spec: &ApiRequestSpec,
    now: DateTime<Utc>,
) -> Result<()> {
    validate_request_shape(spec)?;
    if !budget.enabled {
        bail!("api.disabled: the latest project API budget is disabled");
    }
    if now < budget.period_start || now >= budget.period_end {
        bail!("api.period_inactive: the API budget period is not active");
    }
    if spec.provider != budget.provider {
        bail!("api.provider_mismatch: request provider differs from its budget");
    }
    if !budget.allowed_models.contains(&spec.model) {
        bail!("api.model_denied: model is not in the project allowlist");
    }
    if spec.max_output_tokens > budget.max_output_tokens {
        bail!("api.output_limit: request exceeds the project output-token ceiling");
    }
    for tool in &spec.tools {
        if !budget.allowed_tools.contains(&tool.name) {
            bail!("api.tool_denied: tool is not in the project allowlist");
        }
    }
    Ok(())
}

fn validate_request_shape(spec: &ApiRequestSpec) -> Result<()> {
    if !matches!(spec.provider.as_str(), "openai" | "anthropic") {
        bail!("api.provider_denied: unsupported API provider");
    }
    validate_name(&spec.model, "API model")?;
    if spec.max_output_tokens == 0 {
        bail!("api.output_limit: request output-token maximum must be greater than zero");
    }
    if spec.instructions.is_empty() || spec.instructions.len() > MAX_INSTRUCTIONS_BYTES {
        bail!("api.request_bounds: instructions must be nonempty and bounded");
    }
    if spec.input.is_empty() || spec.input.len() > MAX_INPUT_BYTES {
        bail!("api.request_bounds: input must be nonempty and bounded");
    }
    if spec.tools.len() > MAX_TOOLS {
        bail!("api.tool_limit: request contains too many tools");
    }
    let mut names = std::collections::BTreeSet::new();
    for tool in &spec.tools {
        validate_name(&tool.name, "API tool name")?;
        if !names.insert(tool.name.as_str()) {
            bail!("api.tool_duplicate: request tool names must be unique");
        }
        if tool.description.is_empty() || tool.description.len() > 10_000 {
            bail!("api.tool_invalid: tool description must be nonempty and bounded");
        }
        if !tool.input_schema.is_object() {
            bail!("api.tool_invalid: tool input schema must be an object");
        }
    }
    Ok(())
}

fn build_openai_body(spec: &ApiRequestSpec) -> Result<Vec<u8>> {
    let tools = spec
        .tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
                "strict": true,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&serde_json::json!({
        "model": spec.model,
        "instructions": spec.instructions,
        "input": [{
            "role": "user",
            "content": [{"type": "input_text", "text": spec.input}],
        }],
        "max_output_tokens": spec.max_output_tokens,
        "tools": tools,
        "stream": spec.stream,
    }))
    .map_err(Into::into)
}

fn build_anthropic_body(spec: &ApiRequestSpec) -> Result<Vec<u8>> {
    let tools = spec
        .tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&serde_json::json!({
        "model": spec.model,
        "max_tokens": spec.max_output_tokens,
        "system": spec.instructions,
        "messages": [{"role": "user", "content": spec.input}],
        "tools": tools,
        "stream": spec.stream,
    }))
    .map_err(Into::into)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiTerminalStatus {
    Completed,
    ToolUse,
    Refused,
    Truncated,
    Paused,
}

#[derive(Clone, PartialEq)]
pub enum ApiOutputItem {
    Text {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    Refusal {
        text: String,
    },
}

impl fmt::Debug for ApiOutputItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text { .. } => formatter.write_str("Text([REDACTED])"),
            Self::ToolCall { name, .. } => formatter
                .debug_struct("ToolCall")
                .field("id", &"[REDACTED]")
                .field("name", name)
                .field("arguments", &"[REDACTED]")
                .finish(),
            Self::Refusal { .. } => formatter.write_str("Refusal([REDACTED])"),
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct ApiProviderResponse {
    pub provider: String,
    pub provider_response_id: String,
    pub request_id: String,
    pub model: String,
    pub terminal_status: ApiTerminalStatus,
    pub output: Vec<ApiOutputItem>,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub output_tokens: u64,
}

impl fmt::Debug for ApiProviderResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiProviderResponse")
            .field("provider", &self.provider)
            .field("provider_response_id", &"[REDACTED]")
            .field("request_id", &"[REDACTED]")
            .field("model", &self.model)
            .field("terminal_status", &self.terminal_status)
            .field("output_items", &self.output.len())
            .field("input_tokens", &self.input_tokens)
            .field("cached_input_tokens", &self.cached_input_tokens)
            .field(
                "cache_creation_input_tokens",
                &self.cache_creation_input_tokens,
            )
            .field("output_tokens", &self.output_tokens)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiFailureKind {
    Authentication,
    Permission,
    RateLimited,
    UsageExhausted,
    InvalidRequest,
    Transient,
    Provider,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiFailureClassification {
    pub kind: ApiFailureKind,
    pub retryable: bool,
}

pub fn parse_openai_response(request_id: &str, body: &[u8]) -> Result<ApiProviderResponse> {
    validate_payload(body)?;
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| anyhow!("openai.response_malformed: response is not valid JSON"))?;
    parse_openai_value(request_id, &value)
}

pub fn parse_openai_response_stream(request_id: &str, body: &[u8]) -> Result<ApiProviderResponse> {
    validate_payload(body)?;
    let events = parse_sse(body)?;
    let mut completed = None;
    for (event_name, data) in events {
        if data == b"[DONE]" {
            continue;
        }
        if completed.is_some() {
            bail!("openai.stream_trailing_event: event followed response.completed");
        }
        let event: Value = serde_json::from_slice(&data)
            .map_err(|_| anyhow!("openai.stream_malformed: event data is not valid JSON"))?;
        let event_type = required_string(&event, "type", "OpenAI stream event type")?;
        if let Some(event_name) = event_name.as_deref()
            && event_name != event_type
        {
            bail!("openai.stream_type_mismatch: SSE and JSON event types differ");
        }
        match event_type {
            "response.completed" => {
                let response = event
                    .get("response")
                    .ok_or_else(|| anyhow!("openai.stream_missing_response: completed event"))?;
                completed = Some(parse_openai_value(request_id, response)?);
            }
            "response.failed" | "response.incomplete" | "error" => {
                bail!("openai.stream_terminal_failure: provider reported a failed response");
            }
            _ => {
                // OpenAI documents new event types as additive. Only the complete response is
                // trusted for output and usage, so unknown incremental events carry no authority.
            }
        }
    }
    completed.ok_or_else(|| anyhow!("openai.stream_partial: response.completed was not received"))
}

fn parse_openai_value(request_id: &str, value: &Value) -> Result<ApiProviderResponse> {
    validate_opaque_id(request_id, "OpenAI request ID")?;
    if required_string(value, "object", "OpenAI response object")? != "response" {
        bail!("openai.schema_drift: object must be response");
    }
    if required_string(value, "status", "OpenAI response status")? != "completed" {
        bail!("openai.response_partial: status is not completed");
    }
    let provider_response_id = required_string(value, "id", "OpenAI response ID")?;
    validate_opaque_id(provider_response_id, "OpenAI response ID")?;
    let model = required_string(value, "model", "OpenAI model")?;
    validate_model(model)?;
    let values = required_array(value, "output", "OpenAI output")?;
    if values.is_empty() || values.len() > MAX_ITEMS {
        bail!("openai.output_bounds: output must contain 1..={MAX_ITEMS} items");
    }
    let mut output = Vec::new();
    for item in values {
        match required_string(item, "type", "OpenAI output item type")? {
            "message" => parse_openai_message(item, &mut output)?,
            "function_call" => {
                if required_string(item, "status", "OpenAI function status")? != "completed" {
                    bail!("openai.function_partial: function call is not completed");
                }
                let id = required_string(item, "call_id", "OpenAI function call ID")?;
                let name = required_string(item, "name", "OpenAI function name")?;
                validate_opaque_id(id, "OpenAI function call ID")?;
                validate_name(name, "OpenAI function name")?;
                let arguments = required_string(item, "arguments", "OpenAI function arguments")?;
                let arguments = parse_arguments(arguments, "OpenAI function arguments")?;
                output.push(ApiOutputItem::ToolCall {
                    id: id.into(),
                    name: name.into(),
                    arguments,
                });
            }
            _ => bail!("openai.output_type_denied: unsupported authoritative output item"),
        }
    }
    if output.is_empty() || output.len() > MAX_ITEMS {
        bail!("openai.output_bounds: parsed output must contain 1..={MAX_ITEMS} items");
    }
    let usage = value
        .get("usage")
        .ok_or_else(|| anyhow!("openai.usage_missing: completed response requires usage"))?;
    let input_tokens = required_u64(usage, "input_tokens", "OpenAI input tokens")?;
    let details = usage.get("input_tokens_details");
    if details.is_some_and(|value| !value.is_object()) {
        bail!("openai.usage_details_invalid: input token details must be an object");
    }
    let cached_input_tokens = optional_u64(details, "cached_tokens", "OpenAI cached input tokens")?;
    let cache_creation_input_tokens = optional_u64(
        details,
        "cache_write_tokens",
        "OpenAI cache-write input tokens",
    )?;
    if cached_input_tokens
        .checked_add(cache_creation_input_tokens)
        .is_none_or(|categorized| categorized > input_tokens)
    {
        bail!("openai.usage_inconsistent: categorized input tokens exceed input tokens");
    }
    let output_tokens = required_u64(usage, "output_tokens", "OpenAI output tokens")?;
    let total_tokens = required_u64(usage, "total_tokens", "OpenAI total tokens")?;
    if input_tokens.checked_add(output_tokens) != Some(total_tokens) {
        bail!("openai.usage_inconsistent: total tokens do not equal input plus output");
    }
    Ok(ApiProviderResponse {
        provider: "openai".into(),
        provider_response_id: provider_response_id.into(),
        request_id: request_id.into(),
        model: model.into(),
        terminal_status: if output
            .iter()
            .any(|item| matches!(item, ApiOutputItem::ToolCall { .. }))
        {
            ApiTerminalStatus::ToolUse
        } else if output
            .iter()
            .any(|item| matches!(item, ApiOutputItem::Refusal { .. }))
        {
            ApiTerminalStatus::Refused
        } else {
            ApiTerminalStatus::Completed
        },
        output,
        input_tokens,
        cached_input_tokens,
        cache_creation_input_tokens,
        output_tokens,
    })
}

fn parse_openai_message(item: &Value, output: &mut Vec<ApiOutputItem>) -> Result<()> {
    if required_string(item, "status", "OpenAI message status")? != "completed"
        || required_string(item, "role", "OpenAI message role")? != "assistant"
    {
        bail!("openai.message_invalid: message must be a completed assistant message");
    }
    for content in required_array(item, "content", "OpenAI message content")? {
        match required_string(content, "type", "OpenAI content type")? {
            "output_text" => output.push(ApiOutputItem::Text {
                text: bounded_text(
                    required_string(content, "text", "OpenAI output text")?,
                    "OpenAI output text",
                )?,
            }),
            "refusal" => output.push(ApiOutputItem::Refusal {
                text: bounded_text(
                    required_string(content, "refusal", "OpenAI refusal")?,
                    "OpenAI refusal",
                )?,
            }),
            _ => bail!("openai.content_type_denied: unsupported authoritative content"),
        }
    }
    Ok(())
}

pub fn parse_anthropic_response(request_id: &str, body: &[u8]) -> Result<ApiProviderResponse> {
    validate_payload(body)?;
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| anyhow!("anthropic.response_malformed: response is not valid JSON"))?;
    parse_anthropic_value(request_id, &value)
}

fn parse_anthropic_value(request_id: &str, value: &Value) -> Result<ApiProviderResponse> {
    validate_opaque_id(request_id, "Anthropic request ID")?;
    if required_string(value, "type", "Anthropic response type")? != "message"
        || required_string(value, "role", "Anthropic response role")? != "assistant"
    {
        bail!("anthropic.schema_drift: response must be an assistant message");
    }
    let provider_response_id = required_string(value, "id", "Anthropic message ID")?;
    validate_opaque_id(provider_response_id, "Anthropic message ID")?;
    let model = required_string(value, "model", "Anthropic model")?;
    validate_model(model)?;
    let terminal_status = anthropic_terminal(required_string(
        value,
        "stop_reason",
        "Anthropic stop reason",
    )?)?;
    let values = required_array(value, "content", "Anthropic content")?;
    if values.is_empty() || values.len() > MAX_ITEMS {
        bail!("anthropic.output_bounds: content must contain 1..={MAX_ITEMS} items");
    }
    let mut output = Vec::with_capacity(values.len());
    for item in values {
        match required_string(item, "type", "Anthropic content type")? {
            "text" => output.push(ApiOutputItem::Text {
                text: bounded_text(
                    required_string(item, "text", "Anthropic output text")?,
                    "Anthropic output text",
                )?,
            }),
            "tool_use" => {
                let id = required_string(item, "id", "Anthropic tool-use ID")?;
                let name = required_string(item, "name", "Anthropic tool name")?;
                validate_opaque_id(id, "Anthropic tool-use ID")?;
                validate_name(name, "Anthropic tool name")?;
                let arguments = item
                    .get("input")
                    .filter(|value| value.is_object())
                    .cloned()
                    .ok_or_else(|| {
                        anyhow!("anthropic.tool_input_invalid: input must be an object")
                    })?;
                output.push(ApiOutputItem::ToolCall {
                    id: id.into(),
                    name: name.into(),
                    arguments,
                });
            }
            _ => bail!("anthropic.content_type_denied: unsupported authoritative content"),
        }
    }
    let usage = value
        .get("usage")
        .ok_or_else(|| anyhow!("anthropic.usage_missing: message requires usage"))?;
    let input_usage = anthropic_input_usage(usage)?;
    let output_tokens = required_u64(usage, "output_tokens", "Anthropic output tokens")?;
    Ok(ApiProviderResponse {
        provider: "anthropic".into(),
        provider_response_id: provider_response_id.into(),
        request_id: request_id.into(),
        model: model.into(),
        terminal_status,
        output,
        input_tokens: input_usage.total,
        cached_input_tokens: input_usage.cached,
        cache_creation_input_tokens: input_usage.cache_creation,
        output_tokens,
    })
}

#[derive(Debug)]
enum AnthropicStreamBlock {
    Text {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        initial: Value,
        partial_json: String,
    },
}

#[derive(Debug)]
struct StreamBlock {
    value: AnthropicStreamBlock,
    stopped: bool,
}

pub fn parse_anthropic_response_stream(
    request_id: &str,
    body: &[u8],
) -> Result<ApiProviderResponse> {
    validate_payload(body)?;
    validate_opaque_id(request_id, "Anthropic request ID")?;
    let events = parse_sse(body)?;
    let mut response_id = None;
    let mut model = None;
    let mut input_tokens = None;
    let mut cached_input_tokens = None;
    let mut cache_creation_input_tokens = None;
    let mut output_tokens = None;
    let mut terminal = None;
    let mut blocks = BTreeMap::<usize, StreamBlock>::new();
    let mut stopped = false;
    for (event_name, data) in events {
        if data == b"[DONE]" {
            continue;
        }
        if stopped {
            bail!("anthropic.stream_trailing_event: event followed message_stop");
        }
        let event: Value = serde_json::from_slice(&data)
            .map_err(|_| anyhow!("anthropic.stream_malformed: event data is not valid JSON"))?;
        let event_type = required_string(&event, "type", "Anthropic stream event type")?;
        if let Some(event_name) = event_name.as_deref()
            && event_name != event_type
        {
            bail!("anthropic.stream_type_mismatch: SSE and JSON event types differ");
        }
        match event_type {
            "message_start" => {
                if response_id.is_some() {
                    bail!("anthropic.stream_duplicate_start: message_start repeated");
                }
                let message = event
                    .get("message")
                    .ok_or_else(|| anyhow!("anthropic.stream_missing_message: message_start"))?;
                if required_string(message, "type", "Anthropic message type")? != "message"
                    || required_string(message, "role", "Anthropic message role")? != "assistant"
                {
                    bail!("anthropic.stream_schema_drift: invalid starting message");
                }
                let id = required_string(message, "id", "Anthropic message ID")?;
                validate_opaque_id(id, "Anthropic message ID")?;
                let model_value = required_string(message, "model", "Anthropic model")?;
                validate_model(model_value)?;
                if message
                    .get("stop_reason")
                    .is_none_or(|value| !value.is_null())
                {
                    bail!("anthropic.stream_schema_drift: starting stop reason must be null");
                }
                let usage = message.get("usage").ok_or_else(|| {
                    anyhow!("anthropic.stream_usage_missing: message_start requires usage")
                })?;
                response_id = Some(id.to_owned());
                model = Some(model_value.to_owned());
                let input_usage = anthropic_input_usage(usage)?;
                input_tokens = Some(input_usage.total);
                cached_input_tokens = Some(input_usage.cached);
                cache_creation_input_tokens = Some(input_usage.cache_creation);
            }
            "content_block_start" => {
                ensure_anthropic_started(&response_id)?;
                let index = required_index(&event)?;
                if blocks.len() >= MAX_ITEMS || blocks.contains_key(&index) {
                    bail!("anthropic.stream_block_invalid: duplicate or excessive block");
                }
                let content = event.get("content_block").ok_or_else(|| {
                    anyhow!("anthropic.stream_block_missing: content_block_start")
                })?;
                let value = match required_string(
                    content,
                    "type",
                    "Anthropic streaming content type",
                )? {
                    "text" => AnthropicStreamBlock::Text {
                        text: bounded_text(
                            required_string_allow_empty(content, "text", "Anthropic initial text")?,
                            "Anthropic initial text",
                        )?,
                    },
                    "tool_use" => {
                        let id = required_string(content, "id", "Anthropic tool-use ID")?;
                        let name = required_string(content, "name", "Anthropic tool name")?;
                        validate_opaque_id(id, "Anthropic tool-use ID")?;
                        validate_name(name, "Anthropic tool name")?;
                        let initial = content
                            .get("input")
                            .filter(|value| value.is_object())
                            .cloned()
                            .ok_or_else(|| {
                                anyhow!("anthropic.tool_input_invalid: input must be an object")
                            })?;
                        AnthropicStreamBlock::Tool {
                            id: id.into(),
                            name: name.into(),
                            initial,
                            partial_json: String::new(),
                        }
                    }
                    _ => bail!(
                        "anthropic.content_type_denied: unsupported authoritative stream content"
                    ),
                };
                blocks.insert(
                    index,
                    StreamBlock {
                        value,
                        stopped: false,
                    },
                );
            }
            "content_block_delta" => {
                let index = required_index(&event)?;
                let block = blocks.get_mut(&index).ok_or_else(|| {
                    anyhow!("anthropic.stream_block_order: delta before block start")
                })?;
                if block.stopped {
                    bail!("anthropic.stream_block_order: delta after block stop");
                }
                let delta = event
                    .get("delta")
                    .ok_or_else(|| anyhow!("anthropic.stream_delta_missing: content delta"))?;
                match (
                    &mut block.value,
                    required_string(delta, "type", "Anthropic delta type")?,
                ) {
                    (AnthropicStreamBlock::Text { text }, "text_delta") => {
                        text.push_str(required_string(delta, "text", "Anthropic text delta")?);
                        if text.len() > MAX_TEXT_BYTES {
                            bail!("anthropic.output_bounds: text exceeds byte limit");
                        }
                    }
                    (AnthropicStreamBlock::Tool { partial_json, .. }, "input_json_delta") => {
                        partial_json.push_str(required_string(
                            delta,
                            "partial_json",
                            "Anthropic tool input delta",
                        )?);
                        if partial_json.len() > MAX_TEXT_BYTES {
                            bail!("anthropic.output_bounds: tool input exceeds byte limit");
                        }
                    }
                    _ => bail!("anthropic.stream_delta_mismatch: delta does not match its block"),
                }
            }
            "content_block_stop" => {
                let block = blocks.get_mut(&required_index(&event)?).ok_or_else(|| {
                    anyhow!("anthropic.stream_block_order: stop before block start")
                })?;
                if std::mem::replace(&mut block.stopped, true) {
                    bail!("anthropic.stream_block_order: block stop repeated");
                }
            }
            "message_delta" => {
                ensure_anthropic_started(&response_id)?;
                if terminal.is_some() {
                    bail!("anthropic.stream_duplicate_delta: terminal message delta repeated");
                }
                let delta = event
                    .get("delta")
                    .ok_or_else(|| anyhow!("anthropic.stream_delta_missing: message delta"))?;
                terminal = Some(anthropic_terminal(required_string(
                    delta,
                    "stop_reason",
                    "Anthropic stop reason",
                )?)?);
                let usage = event.get("usage").ok_or_else(|| {
                    anyhow!("anthropic.stream_usage_missing: message_delta requires usage")
                })?;
                output_tokens = Some(required_u64(
                    usage,
                    "output_tokens",
                    "Anthropic output tokens",
                )?);
            }
            "message_stop" => {
                ensure_anthropic_started(&response_id)?;
                stopped = true;
            }
            "ping" => {}
            "error" => bail!("anthropic.stream_terminal_failure: provider reported an error"),
            _ => {
                // Anthropic may add event types. Unknown events cannot contribute output,
                // usage, completion, or tool authority and are therefore safely ignored.
            }
        }
    }
    if !stopped || terminal.is_none() || blocks.is_empty() || blocks.values().any(|b| !b.stopped) {
        bail!("anthropic.stream_partial: complete terminal stream was not received");
    }
    let mut output = Vec::with_capacity(blocks.len());
    for block in blocks.into_values() {
        match block.value {
            AnthropicStreamBlock::Text { text } => output.push(ApiOutputItem::Text { text }),
            AnthropicStreamBlock::Tool {
                id,
                name,
                initial,
                partial_json,
            } => {
                let arguments = if partial_json.is_empty() {
                    initial
                } else {
                    parse_arguments(&partial_json, "Anthropic tool arguments")?
                };
                output.push(ApiOutputItem::ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
        }
    }
    Ok(ApiProviderResponse {
        provider: "anthropic".into(),
        provider_response_id: response_id.unwrap_or_default(),
        request_id: request_id.into(),
        model: model.unwrap_or_default(),
        terminal_status: terminal.unwrap_or(ApiTerminalStatus::Paused),
        output,
        input_tokens: input_tokens.unwrap_or_default(),
        cached_input_tokens: cached_input_tokens.unwrap_or_default(),
        cache_creation_input_tokens: cache_creation_input_tokens.unwrap_or_default(),
        output_tokens: output_tokens.unwrap_or_default(),
    })
}

pub fn classify_api_failure(
    provider: &str,
    status_code: u16,
    body: &[u8],
) -> ApiFailureClassification {
    let code = if body.len() <= MAX_PAYLOAD_BYTES {
        serde_json::from_slice::<Value>(body)
            .ok()
            .as_ref()
            .and_then(|value| provider_error_code(provider, value))
            .map(str::to_owned)
    } else {
        None
    };
    let kind = match (provider, status_code, code.as_deref()) {
        (_, 401, _) => ApiFailureKind::Authentication,
        (_, 403, _) => ApiFailureKind::Permission,
        ("openai", 429, Some("insufficient_quota" | "billing_hard_limit_reached")) => {
            ApiFailureKind::UsageExhausted
        }
        ("anthropic", 429, Some("billing_error")) => ApiFailureKind::UsageExhausted,
        (_, 429, _) => ApiFailureKind::RateLimited,
        (_, 400 | 404 | 405 | 409 | 422, _) => ApiFailureKind::InvalidRequest,
        (_, 408 | 500 | 502 | 503 | 504 | 529, _) => ApiFailureKind::Transient,
        (_, 400..=499, _) => ApiFailureKind::Provider,
        (_, 500..=599, _) => ApiFailureKind::Transient,
        _ => ApiFailureKind::Unknown,
    };
    ApiFailureClassification {
        kind,
        retryable: matches!(
            kind,
            ApiFailureKind::RateLimited | ApiFailureKind::Transient
        ),
    }
}

fn provider_error_code<'a>(provider: &str, value: &'a Value) -> Option<&'a str> {
    let error = value.get("error")?;
    match provider {
        "openai" => error
            .get("code")
            .and_then(Value::as_str)
            .or_else(|| error.get("type").and_then(Value::as_str)),
        "anthropic" => error.get("type").and_then(Value::as_str),
        _ => None,
    }
}

fn parse_sse(body: &[u8]) -> Result<Vec<(Option<String>, Vec<u8>)>> {
    let text =
        std::str::from_utf8(body).map_err(|_| anyhow!("api.stream_encoding: SSE must be UTF-8"))?;
    let normalized = text.replace("\r\n", "\n");
    let mut events = Vec::new();
    for frame in normalized.split("\n\n") {
        if frame.trim().is_empty() {
            continue;
        }
        if events.len() >= MAX_EVENTS {
            bail!("api.stream_bounds: too many SSE events");
        }
        let mut event_name = None;
        let mut data = Vec::new();
        for line in frame.lines() {
            if line.starts_with(':') {
                continue;
            }
            if let Some(value) = line.strip_prefix("event:") {
                if event_name.replace(value.trim().to_owned()).is_some() {
                    bail!("api.stream_malformed: duplicate SSE event field");
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push(b'\n');
                }
                data.extend_from_slice(value.trim_start().as_bytes());
            } else if !line.trim().is_empty() {
                bail!("api.stream_malformed: unsupported SSE field");
            }
        }
        if data.is_empty() {
            bail!("api.stream_malformed: SSE event has no data");
        }
        events.push((event_name, data));
    }
    Ok(events)
}

fn validate_payload(body: &[u8]) -> Result<()> {
    if body.is_empty() || body.len() > MAX_PAYLOAD_BYTES {
        bail!("api.payload_bounds: payload must contain 1..={MAX_PAYLOAD_BYTES} bytes");
    }
    Ok(())
}

fn required_string<'a>(value: &'a Value, key: &str, label: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("api.schema_drift: {label} is missing or invalid"))
}

fn required_string_allow_empty<'a>(value: &'a Value, key: &str, label: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("api.schema_drift: {label} is missing or invalid"))
}

fn required_array<'a>(value: &'a Value, key: &str, label: &str) -> Result<&'a Vec<Value>> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("api.schema_drift: {label} is missing or invalid"))
}

fn required_u64(value: &Value, key: &str, label: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("api.schema_drift: {label} is missing or invalid"))
}

fn required_index(value: &Value) -> Result<usize> {
    let index = required_u64(value, "index", "Anthropic content index")?;
    usize::try_from(index)
        .ok()
        .filter(|index| *index < MAX_ITEMS)
        .ok_or_else(|| anyhow!("anthropic.stream_block_invalid: index is out of bounds"))
}

fn validate_opaque_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 512
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\'' | b'\\'))
    {
        bail!("api.identifier_invalid: {label} is invalid");
    }
    Ok(())
}

fn validate_sha256_text(value: &str, label: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("api.digest_invalid: {label} must be a hexadecimal SHA-256");
    }
    Ok(())
}

fn validate_model(value: &str) -> Result<()> {
    if value.len() > 200 || value.chars().any(char::is_whitespace) {
        bail!("api.model_invalid: provider model identifier is invalid");
    }
    Ok(())
}

fn validate_name(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 200
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        bail!("api.name_invalid: {label} is invalid");
    }
    Ok(())
}

fn bounded_text(value: &str, label: &str) -> Result<String> {
    if value.len() > MAX_TEXT_BYTES {
        bail!("api.output_bounds: {label} exceeds byte limit");
    }
    Ok(value.to_owned())
}

fn parse_arguments(value: &str, label: &str) -> Result<Value> {
    if value.len() > MAX_TEXT_BYTES {
        bail!("api.output_bounds: {label} exceeds byte limit");
    }
    serde_json::from_str::<Value>(value)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| anyhow!("api.tool_arguments_invalid: {label} must be a JSON object"))
}

fn anthropic_terminal(value: &str) -> Result<ApiTerminalStatus> {
    match value {
        "end_turn" | "stop_sequence" => Ok(ApiTerminalStatus::Completed),
        "tool_use" => Ok(ApiTerminalStatus::ToolUse),
        "refusal" => Ok(ApiTerminalStatus::Refused),
        "max_tokens" => Ok(ApiTerminalStatus::Truncated),
        "pause_turn" => Ok(ApiTerminalStatus::Paused),
        _ => bail!("anthropic.stop_reason_unknown: unsupported stop reason"),
    }
}

struct ApiInputUsage {
    total: u64,
    cached: u64,
    cache_creation: u64,
}

fn anthropic_input_usage(usage: &Value) -> Result<ApiInputUsage> {
    let base = required_u64(usage, "input_tokens", "Anthropic input tokens")?;
    let cache_creation = optional_u64(
        Some(usage),
        "cache_creation_input_tokens",
        "Anthropic cache-creation input tokens",
    )?;
    let cached = optional_u64(
        Some(usage),
        "cache_read_input_tokens",
        "Anthropic cache-read input tokens",
    )?;
    let total = base
        .checked_add(cache_creation)
        .and_then(|value| value.checked_add(cached))
        .ok_or_else(|| anyhow!("anthropic.usage_overflow: input token total overflow"))?;
    Ok(ApiInputUsage {
        total,
        cached,
        cache_creation,
    })
}

fn optional_u64(object: Option<&Value>, key: &str, label: &str) -> Result<u64> {
    match object.and_then(|value| value.get(key)) {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .ok_or_else(|| anyhow!("api.schema_drift: {label} are invalid")),
    }
}

fn ensure_anthropic_started(response_id: &Option<String>) -> Result<()> {
    if response_id.is_none() {
        bail!("anthropic.stream_order: event arrived before message_start");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    const OPENAI_RESPONSE: &str = r#"{
        "id":"resp_fixture_001",
        "object":"response",
        "status":"completed",
        "model":"gpt-fixture-2026-01-01",
        "output":[{
            "id":"msg_fixture_001",
            "type":"message",
            "status":"completed",
            "role":"assistant",
            "content":[{"type":"output_text","text":"fixture result","annotations":[]}],
            "additive_future_field":true
        }],
        "usage":{"input_tokens":12,"input_tokens_details":{"cached_tokens":3,"cache_write_tokens":2},"output_tokens":5,"total_tokens":17,"future":1},
        "additive_future_field":{"safe":"ignored"}
    }"#;

    const ANTHROPIC_RESPONSE: &str = r#"{
        "id":"msg_fixture_001",
        "type":"message",
        "role":"assistant",
        "model":"claude-fixture-20260101",
        "content":[{
            "type":"tool_use",
            "id":"toolu_fixture_001",
            "name":"read_fixture",
            "input":{"path":"fixture.txt"},
            "future":true
        }],
        "stop_reason":"tool_use",
        "stop_sequence":null,
        "usage":{
            "input_tokens":10,
            "cache_creation_input_tokens":2,
            "cache_read_input_tokens":3,
            "output_tokens":4
        },
        "future":{"safe":"ignored"}
    }"#;

    fn request_budget(provider: &str, secret_reference: String, now: DateTime<Utc>) -> ApiBudget {
        ApiBudget {
            id: "budget-fixture".into(),
            project_id: "project-fixture".into(),
            provider: provider.into(),
            account: "default".into(),
            enabled: true,
            secret_reference,
            currency: Some("USD".into()),
            currency_limit_micros: Some(1_000_000),
            token_limit: Some(100_000),
            request_limit: Some(10),
            period_start: now - Duration::minutes(1),
            period_end: now + Duration::days(1),
            allowed_models: vec![format!("{provider}-fixture-model")],
            allowed_tools: vec!["read_fixture".into()],
            allowed_roles: vec!["planner".into()],
            max_output_tokens: 1_000,
            max_retries: 0,
            max_concurrent_requests: 1,
            reason: "request fixture".into(),
            created_at: now,
            supersedes_id: None,
        }
    }

    fn request_spec(provider: &str) -> ApiRequestSpec {
        ApiRequestSpec {
            provider: provider.into(),
            model: format!("{provider}-fixture-model"),
            instructions: "system-canary-never-debug-01".into(),
            input: "prompt-canary-never-debug-02".into(),
            max_output_tokens: 100,
            tools: vec![ApiToolDefinition {
                name: "read_fixture".into(),
                description: "Read one fixture".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                    "additionalProperties": false,
                }),
            }],
            stream: true,
        }
    }

    #[cfg(unix)]
    #[test]
    fn prepared_requests_use_fixed_endpoints_and_redact_secret_and_prompt_canaries() {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        use tempfile::tempdir;

        const SECRET: &str = "provider-secret-canary-never-debug-03";
        let directory = tempdir().unwrap();
        let secret_path = directory.path().join("api-key");
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&secret_path)
            .unwrap();
        writeln!(file, "{SECRET}").unwrap();
        drop(file);
        let now = Utc::now();
        for provider in ["openai", "anthropic"] {
            let budget = request_budget(provider, format!("file:{}", secret_path.display()), now);
            let spec = request_spec(provider);
            let digest = api_request_digest(&budget, &spec, now).unwrap();
            let request = prepare_api_request(&budget, &spec, now, &digest).unwrap();
            let debug = format!("{request:?}");
            for canary in [
                SECRET,
                "system-canary-never-debug-01",
                "prompt-canary-never-debug-02",
            ] {
                assert!(!debug.contains(canary));
            }
            assert_eq!(request.provider(), provider);
            assert_eq!(request.body_sha256().len(), 64);
            request.with_sensitive_parts(|endpoint, headers, secret_name, prefix, secret, body| {
                assert_eq!(secret, SECRET.as_bytes());
                assert_eq!(headers.get("content-type").unwrap(), "application/json");
                let body: Value = serde_json::from_slice(body).unwrap();
                assert_eq!(body["model"], format!("{provider}-fixture-model"));
                assert_eq!(body["stream"], true);
                match provider {
                    "openai" => {
                        assert_eq!(endpoint, "https://api.openai.com/v1/responses");
                        assert_eq!(secret_name, "authorization");
                        assert_eq!(prefix, b"Bearer ");
                        assert_eq!(body["max_output_tokens"], 100);
                        assert_eq!(body["tools"][0]["strict"], true);
                    }
                    "anthropic" => {
                        assert_eq!(endpoint, "https://api.anthropic.com/v1/messages");
                        assert_eq!(secret_name, "x-api-key");
                        assert!(prefix.is_empty());
                        assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
                        assert_eq!(body["max_tokens"], 100);
                        assert_eq!(body["tools"][0]["name"], "read_fixture");
                    }
                    _ => unreachable!(),
                }
            });
        }
    }

    #[test]
    fn request_gates_fail_before_an_unavailable_secret_is_resolved() {
        let now = Utc::now();
        let mut budget = request_budget(
            "openai",
            "file:/definitely/not/a/real/garnish-secret".into(),
            now,
        );
        let mut spec = request_spec("openai");

        budget.enabled = false;
        let error = prepare_api_request(&budget, &spec, now, &"0".repeat(64)).unwrap_err();
        assert!(error.to_string().contains("api.disabled"));
        budget.enabled = true;

        spec.model = "denied-model".into();
        let error = prepare_api_request(&budget, &spec, now, &"0".repeat(64)).unwrap_err();
        assert!(error.to_string().contains("api.model_denied"));
        spec.model = "openai-fixture-model".into();

        spec.tools[0].name = "denied_tool".into();
        let error = prepare_api_request(&budget, &spec, now, &"0".repeat(64)).unwrap_err();
        assert!(error.to_string().contains("api.tool_denied"));
        spec.tools[0].name = "read_fixture".into();

        spec.max_output_tokens = 1_001;
        let error = prepare_api_request(&budget, &spec, now, &"0".repeat(64)).unwrap_err();
        assert!(error.to_string().contains("api.output_limit"));
    }

    #[cfg(unix)]
    #[test]
    fn fake_transport_receives_sensitive_parts_without_debug_or_error_disclosure() {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        use tempfile::tempdir;

        const SECRET: &str = "fake-transport-secret-canary-04";
        struct FakeTransport {
            sent: bool,
        }
        impl ApiTransport for FakeTransport {
            fn send(&mut self, request: &PreparedApiRequest) -> Result<ApiTransportResponse> {
                request.with_sensitive_parts(|endpoint, _, secret_name, prefix, secret, body| {
                    assert_eq!(endpoint, "https://api.openai.com/v1/responses");
                    assert_eq!(secret_name, "authorization");
                    assert_eq!(prefix, b"Bearer ");
                    assert_eq!(secret, SECRET.as_bytes());
                    let body: Value = serde_json::from_slice(body).unwrap();
                    assert_eq!(body["model"], "openai-fixture-model");
                });
                self.sent = true;
                ApiTransportResponse::new(
                    200,
                    "req_fake_transport_001".into(),
                    OPENAI_RESPONSE.as_bytes().to_vec(),
                    false,
                )
            }
        }

        let directory = tempdir().unwrap();
        let secret_path = directory.path().join("api-key");
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&secret_path)
            .unwrap();
        writeln!(file, "{SECRET}").unwrap();
        drop(file);
        let now = Utc::now();
        let budget = request_budget("openai", format!("file:{}", secret_path.display()), now);
        let mut spec = request_spec("openai");
        spec.stream = false;
        let digest = api_request_digest(&budget, &spec, now).unwrap();
        let request = prepare_api_request(&budget, &spec, now, &digest).unwrap();
        let mut transport = FakeTransport { sent: false };
        let response = transport.send(&request).unwrap();
        assert!(transport.sent);
        let debug = format!("{response:?}");
        assert!(!debug.contains(SECRET));
        assert!(!debug.contains("req_fake_transport_001"));
        assert!(!debug.contains("fixture result"));
        let parsed = parse_api_transport_response("openai", &response).unwrap();
        assert_eq!(parsed.provider_response_id, "resp_fixture_001");

        let failure = ApiTransportResponse::new(
            429,
            "req_fake_transport_002".into(),
            br#"{"error":{"code":"insufficient_quota","message":"canary body"}}"#.to_vec(),
            false,
        )
        .unwrap();
        let error = parse_api_transport_response("openai", &failure)
            .unwrap_err()
            .to_string();
        assert!(error.contains("UsageExhausted"));
        assert!(!error.contains("canary body"));
        assert!(!error.contains("req_fake_transport_002"));
    }

    #[test]
    fn openai_response_accepts_additive_fields_and_requires_exact_usage() {
        let response =
            parse_openai_response("req_fixture_001", OPENAI_RESPONSE.as_bytes()).unwrap();
        assert_eq!(response.provider, "openai");
        assert_eq!(response.input_tokens, 12);
        assert_eq!(response.cached_input_tokens, 3);
        assert_eq!(response.cache_creation_input_tokens, 2);
        assert_eq!(response.output_tokens, 5);
        assert_eq!(response.terminal_status, ApiTerminalStatus::Completed);
        let debug = format!("{response:?}");
        assert!(!debug.contains("req_fixture_001"));
        assert!(!debug.contains("resp_fixture_001"));
        assert!(!debug.contains("fixture result"));
        assert_eq!(
            response.output,
            vec![ApiOutputItem::Text {
                text: "fixture result".into()
            }]
        );

        let missing_usage = OPENAI_RESPONSE.replace(
            r#""usage":{"input_tokens":12,"input_tokens_details":{"cached_tokens":3,"cache_write_tokens":2},"output_tokens":5,"total_tokens":17,"future":1},"#,
            "",
        );
        assert!(parse_openai_response("req_fixture_001", missing_usage.as_bytes()).is_err());
        let inconsistent = OPENAI_RESPONSE.replace("\"total_tokens\":17", "\"total_tokens\":18");
        assert!(parse_openai_response("req_fixture_001", inconsistent.as_bytes()).is_err());
        let unknown_output =
            OPENAI_RESPONSE.replace("\"type\":\"message\"", "\"type\":\"computer_call\"");
        assert!(parse_openai_response("req_fixture_001", unknown_output.as_bytes()).is_err());
    }

    #[test]
    fn openai_stream_trusts_only_one_completed_response() {
        let openai = serde_json::to_string(
            &serde_json::from_str::<Value>(OPENAI_RESPONSE).expect("OpenAI fixture JSON"),
        )
        .unwrap();
        let stream = format!(
            "event: response.created\ndata: {{\"type\":\"response.created\",\"response\":{{\"status\":\"in_progress\"}}}}\n\nevent: future.additive\ndata: {{\"type\":\"future.additive\",\"value\":1}}\n\nevent: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{openai}}}\n\n"
        );
        let response =
            parse_openai_response_stream("req_fixture_stream", stream.as_bytes()).unwrap();
        assert_eq!(response.provider_response_id, "resp_fixture_001");
        assert!(
            parse_openai_response_stream(
                "req_fixture_stream",
                b"event: response.created\ndata: {\"type\":\"response.created\"}\n\n"
            )
            .is_err()
        );
        let duplicate = format!("{stream}{stream}");
        assert!(parse_openai_response_stream("req_fixture_stream", duplicate.as_bytes()).is_err());
    }

    #[test]
    fn anthropic_response_accepts_additive_fields_and_counts_cache_tokens() {
        let response =
            parse_anthropic_response("req_fixture_002", ANTHROPIC_RESPONSE.as_bytes()).unwrap();
        assert_eq!(response.provider, "anthropic");
        assert_eq!(response.input_tokens, 15);
        assert_eq!(response.cached_input_tokens, 3);
        assert_eq!(response.cache_creation_input_tokens, 2);
        assert_eq!(response.output_tokens, 4);
        assert_eq!(response.terminal_status, ApiTerminalStatus::ToolUse);
        assert!(matches!(
            &response.output[0],
            ApiOutputItem::ToolCall { name, .. } if name == "read_fixture"
        ));

        let unknown =
            ANTHROPIC_RESPONSE.replace("\"type\":\"tool_use\"", "\"type\":\"future_action\"");
        assert!(parse_anthropic_response("req_fixture_002", unknown.as_bytes()).is_err());
        let no_usage = ANTHROPIC_RESPONSE.replace(
            r#",
        "usage":{
            "input_tokens":10,
            "cache_creation_input_tokens":2,
            "cache_read_input_tokens":3,
            "output_tokens":4
        }"#,
            "",
        );
        assert!(parse_anthropic_response("req_fixture_002", no_usage.as_bytes()).is_err());
    }

    #[test]
    fn anthropic_stream_requires_ordered_stopped_blocks_and_terminal_usage() {
        let stream = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_fixture_stream","type":"message","role":"assistant","model":"claude-fixture-20260101","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":7,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"fixture "}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"stream"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":3}}

event: message_stop
data: {"type":"message_stop"}

"#;
        let response =
            parse_anthropic_response_stream("req_fixture_stream_2", stream.as_bytes()).unwrap();
        assert_eq!(response.input_tokens, 7);
        assert_eq!(response.output_tokens, 3);
        assert_eq!(
            response.output,
            vec![ApiOutputItem::Text {
                text: "fixture stream".into()
            }]
        );
        let partial = stream.replace(
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            "",
        );
        assert!(
            parse_anthropic_response_stream("req_fixture_stream_2", partial.as_bytes()).is_err()
        );
        let wrong_delta =
            stream.replace("\"type\":\"text_delta\"", "\"type\":\"input_json_delta\"");
        assert!(
            parse_anthropic_response_stream("req_fixture_stream_2", wrong_delta.as_bytes())
                .is_err()
        );
    }

    #[test]
    fn provider_failures_keep_rate_usage_auth_and_transient_distinct() {
        assert_eq!(
            classify_api_failure(
                "openai",
                429,
                br#"{"error":{"type":"insufficient_quota","code":"insufficient_quota"}}"#
            ),
            ApiFailureClassification {
                kind: ApiFailureKind::UsageExhausted,
                retryable: false
            }
        );
        assert_eq!(
            classify_api_failure(
                "openai",
                429,
                br#"{"error":{"type":"rate_limit_error","code":"rate_limit_exceeded"}}"#
            )
            .kind,
            ApiFailureKind::RateLimited
        );
        assert_eq!(
            classify_api_failure(
                "anthropic",
                529,
                br#"{"type":"error","error":{"type":"overloaded_error"}}"#
            ),
            ApiFailureClassification {
                kind: ApiFailureKind::Transient,
                retryable: true
            }
        );
        assert_eq!(
            classify_api_failure("anthropic", 401, b"not-json").kind,
            ApiFailureKind::Authentication
        );
    }
}
