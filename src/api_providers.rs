use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
const MAX_EVENTS: usize = 100_000;
const MAX_ITEMS: usize = 4_096;
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiTerminalStatus {
    Completed,
    ToolUse,
    Refused,
    Truncated,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiProviderResponse {
    pub provider: String,
    pub provider_response_id: String,
    pub request_id: String,
    pub model: String,
    pub terminal_status: ApiTerminalStatus,
    pub output: Vec<ApiOutputItem>,
    pub input_tokens: u64,
    pub output_tokens: u64,
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
    let input_tokens = anthropic_input_tokens(usage)?;
    let output_tokens = required_u64(usage, "output_tokens", "Anthropic output tokens")?;
    Ok(ApiProviderResponse {
        provider: "anthropic".into(),
        provider_response_id: provider_response_id.into(),
        request_id: request_id.into(),
        model: model.into(),
        terminal_status,
        output,
        input_tokens,
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
                input_tokens = Some(anthropic_input_tokens(usage)?);
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

fn anthropic_input_tokens(usage: &Value) -> Result<u64> {
    let mut total = required_u64(usage, "input_tokens", "Anthropic input tokens")?;
    for key in ["cache_creation_input_tokens", "cache_read_input_tokens"] {
        if let Some(value) = usage.get(key) {
            total = total
                .checked_add(value.as_u64().ok_or_else(|| {
                    anyhow!("api.schema_drift: Anthropic cache tokens are invalid")
                })?)
                .ok_or_else(|| anyhow!("anthropic.usage_overflow: input token total overflow"))?;
        }
    }
    Ok(total)
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
        "usage":{"input_tokens":12,"output_tokens":5,"total_tokens":17,"future":1},
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

    #[test]
    fn openai_response_accepts_additive_fields_and_requires_exact_usage() {
        let response =
            parse_openai_response("req_fixture_001", OPENAI_RESPONSE.as_bytes()).unwrap();
        assert_eq!(response.provider, "openai");
        assert_eq!(response.input_tokens, 12);
        assert_eq!(response.output_tokens, 5);
        assert_eq!(response.terminal_status, ApiTerminalStatus::Completed);
        assert_eq!(
            response.output,
            vec![ApiOutputItem::Text {
                text: "fixture result".into()
            }]
        );

        let missing_usage = OPENAI_RESPONSE.replace(
            r#""usage":{"input_tokens":12,"output_tokens":5,"total_tokens":17,"future":1},"#,
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
