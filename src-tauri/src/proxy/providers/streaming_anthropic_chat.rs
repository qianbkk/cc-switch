//! Anthropic Messages SSE → OpenAI Chat Completions SSE conversion.
//!
//! 统一网关响应侧使用：OpenAI Chat 客户端接 Anthropic 格式上游时，
//! 请求已由 forwarder 转换，本模块负责把上游的 Anthropic SSE 事件流
//! 转成 Chat Completions 的 chunk 流。
//!
//! 反方向（Chat SSE → Anthropic SSE）已有现成实现：
//! `streaming::create_anthropic_sse_stream`（Claude 客户端 + OpenAI 兼容上游场景）。
//!
//! 事件映射：
//! - message_start            → 首个 chunk（delta.role = assistant）
//! - content_block_start(tool)→ delta.tool_calls[{index,id,function.name}]
//! - content_block_delta:
//!     text_delta             → delta.content 增量
//!     input_json_delta       → delta.tool_calls[{index,function.arguments}] 增量
//!     thinking_delta 等      → 丢弃（Chat 协议无对应表达）
//! - content_block_stop(tool) → 若 start 携带完整 input 且无增量，补发 arguments
//! - message_delta            → 记录 stop_reason / usage
//! - message_stop             → 终止 chunk（finish_reason）+ usage chunk + [DONE]

use crate::proxy::sse::{strip_sse_field, take_sse_block};
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Anthropic stop_reason → OpenAI Chat finish_reason
fn map_stop_reason_to_finish(stop_reason: Option<&str>) -> &'static str {
    match stop_reason {
        Some("tool_use") => "tool_calls",
        Some("max_tokens") => "length",
        // end_turn / stop_sequence / 其它 → stop
        _ => "stop",
    }
}

struct AnthropicToChatState {
    chat_id: String,
    model: String,
    created: i64,
    started: bool,
    finished: bool,
    /// anthropic content block index → chat tool_calls index
    tool_indices: HashMap<u64, u32>,
    next_tool_index: u32,
    /// content_block_start 自带完整 input 时的兜底参数（部分网关不发增量）
    tool_start_input: HashMap<u64, String>,
    tool_saw_delta: HashMap<u64, bool>,
    stop_reason: Option<String>,
    usage: Map<String, Value>,
}

impl Default for AnthropicToChatState {
    fn default() -> Self {
        Self {
            chat_id: "chatcmpl-ccswitch".to_string(),
            model: String::new(),
            created: chrono::Utc::now().timestamp(),
            started: false,
            finished: false,
            tool_indices: HashMap::new(),
            next_tool_index: 0,
            tool_start_input: HashMap::new(),
            tool_saw_delta: HashMap::new(),
            stop_reason: None,
            usage: Map::new(),
        }
    }
}

impl AnthropicToChatState {
    fn chunk(&self, delta: Value, finish_reason: Option<&str>) -> Bytes {
        let payload = json!({
            "id": self.chat_id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": delta,
                "finish_reason": finish_reason
            }]
        });
        Bytes::from(format!(
            "data: {}\n\n",
            serde_json::to_string(&payload).unwrap_or_default()
        ))
    }

    fn usage_chunk(&self) -> Option<Bytes> {
        if self.usage.is_empty() {
            return None;
        }
        let get = |key: &str| -> u64 {
            self.usage
                .get(key)
                .and_then(Value::as_u64)
                .unwrap_or_default()
        };
        // OpenAI 惯例：prompt_tokens 含缓存读写部分
        let prompt = get("input_tokens")
            + get("cache_read_input_tokens")
            + get("cache_creation_input_tokens");
        let completion = get("output_tokens");
        let payload = json!({
            "id": self.chat_id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [],
            "usage": {
                "prompt_tokens": prompt,
                "completion_tokens": completion,
                "total_tokens": prompt + completion
            }
        });
        Some(Bytes::from(format!(
            "data: {}\n\n",
            serde_json::to_string(&payload).unwrap_or_default()
        )))
    }

    fn done() -> Bytes {
        Bytes::from_static(b"data: [DONE]\n\n")
    }

    fn merge_usage(&mut self, usage: &Value) {
        if let Some(obj) = usage.as_object() {
            for (key, value) in obj {
                if !value.is_null() {
                    self.usage.insert(key.clone(), value.clone());
                }
            }
        }
    }

    fn ensure_started(&mut self) -> Vec<Bytes> {
        if self.started {
            return Vec::new();
        }
        self.started = true;
        vec![self.chunk(json!({"role": "assistant", "content": ""}), None)]
    }

    fn handle_message_start(&mut self, data: &Value) -> Vec<Bytes> {
        if let Some(message) = data.get("message") {
            if let Some(id) = message.get("id").and_then(Value::as_str) {
                self.chat_id = if id.starts_with("chatcmpl-") {
                    id.to_string()
                } else {
                    format!("chatcmpl-{id}")
                };
            }
            if let Some(model) = message.get("model").and_then(Value::as_str) {
                if !model.is_empty() {
                    self.model = model.to_string();
                }
            }
            if let Some(usage) = message.get("usage") {
                self.merge_usage(usage);
            }
        }
        self.ensure_started()
    }

    fn handle_content_block_start(&mut self, data: &Value) -> Vec<Bytes> {
        let mut events = self.ensure_started();
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        let Some(block) = data.get("content_block") else {
            return events;
        };
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            // text / thinking 等在 start 时无输出
            return events;
        }
        let tool_index = self.next_tool_index;
        self.next_tool_index += 1;
        self.tool_indices.insert(index, tool_index);
        self.tool_saw_delta.insert(index, false);

        let id = block
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("call_ccswitch")
            .to_string();
        let name = block
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        // 兜底：start 事件直接携带完整 input（部分网关无 input_json_delta）
        if let Some(input) = block.get("input") {
            if input.is_object() && input.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
                if let Ok(serialized) = serde_json::to_string(input) {
                    self.tool_start_input.insert(index, serialized);
                }
            }
        }
        events.push(self.chunk(
            json!({
                "tool_calls": [{
                    "index": tool_index,
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": ""}
                }]
            }),
            None,
        ));
        events
    }

    fn handle_content_block_delta(&mut self, data: &Value) -> Vec<Bytes> {
        let mut events = self.ensure_started();
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        let Some(delta) = data.get("delta") else {
            return events;
        };
        match delta.get("type").and_then(Value::as_str) {
            Some("text_delta") => {
                if let Some(text) = delta.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        events.push(self.chunk(json!({"content": text}), None));
                    }
                }
            }
            Some("input_json_delta") => {
                if let Some(tool_index) = self.tool_indices.get(&index).copied() {
                    self.tool_saw_delta.insert(index, true);
                    if let Some(partial) = delta.get("partial_json").and_then(Value::as_str) {
                        if !partial.is_empty() {
                            events.push(self.chunk(
                                json!({
                                    "tool_calls": [{
                                        "index": tool_index,
                                        "function": {"arguments": partial}
                                    }]
                                }),
                                None,
                            ));
                        }
                    }
                }
            }
            // thinking_delta / signature_delta 等：Chat 协议无对应表达，丢弃
            _ => {}
        }
        events
    }

    fn handle_content_block_stop(&mut self, data: &Value) -> Vec<Bytes> {
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        let mut events = Vec::new();
        // tool block 无增量但 start 带了完整 input → 补发 arguments
        if let Some(tool_index) = self.tool_indices.get(&index).copied() {
            let saw_delta = self.tool_saw_delta.get(&index).copied().unwrap_or(false);
            if !saw_delta {
                if let Some(start_input) = self.tool_start_input.remove(&index) {
                    events.push(self.chunk(
                        json!({
                            "tool_calls": [{
                                "index": tool_index,
                                "function": {"arguments": start_input}
                            }]
                        }),
                        None,
                    ));
                }
            }
        }
        events
    }

    fn handle_message_delta(&mut self, data: &Value) -> Vec<Bytes> {
        if let Some(delta) = data.get("delta") {
            if let Some(reason) = delta.get("stop_reason").and_then(Value::as_str) {
                self.stop_reason = Some(reason.to_string());
            }
        }
        if let Some(usage) = data.get("usage") {
            self.merge_usage(usage);
        }
        Vec::new()
    }

    fn finalize(&mut self) -> Vec<Bytes> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        let mut events = self.ensure_started();
        let finish = map_stop_reason_to_finish(self.stop_reason.as_deref());
        events.push(self.chunk(json!({}), Some(finish)));
        if let Some(usage) = self.usage_chunk() {
            events.push(usage);
        }
        events.push(Self::done());
        events
    }

    fn error_event(&mut self, message: String, error_type: Option<String>) -> Vec<Bytes> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        let payload = json!({
            "error": {
                "message": message,
                "type": error_type.unwrap_or_else(|| "upstream_error".to_string())
            }
        });
        vec![
            Bytes::from(format!(
                "data: {}\n\n",
                serde_json::to_string(&payload).unwrap_or_default()
            )),
            Self::done(),
        ]
    }
}

fn extract_anthropic_error(data: &Value) -> (String, Option<String>) {
    let error = data.get("error").unwrap_or(data);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .filter(|m| !m.trim().is_empty())
        .unwrap_or("Upstream error")
        .to_string();
    let error_type = error
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string);
    (message, error_type)
}

/// 处理一个完整的 Anthropic SSE 块，返回 (输出事件, 是否终止)
fn process_block(state: &mut AnthropicToChatState, block: &str) -> (Vec<Bytes>, bool) {
    let mut event_name: Option<String> = None;
    let mut data_parts: Vec<&str> = Vec::new();
    for line in block.lines() {
        if let Some(value) = strip_sse_field(line, "event") {
            event_name = Some(value.to_string());
        } else if let Some(value) = strip_sse_field(line, "data") {
            data_parts.push(value);
        }
    }
    if data_parts.is_empty() {
        return (Vec::new(), false);
    }
    let Ok(data) = serde_json::from_str::<Value>(&data_parts.join("\n")) else {
        return (Vec::new(), false);
    };
    let msg_type = data
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(event_name)
        .unwrap_or_default();

    let events = match msg_type.as_str() {
        "message_start" => state.handle_message_start(&data),
        "content_block_start" => state.handle_content_block_start(&data),
        "content_block_delta" => state.handle_content_block_delta(&data),
        "content_block_stop" => state.handle_content_block_stop(&data),
        "message_delta" => state.handle_message_delta(&data),
        "message_stop" => state.finalize(),
        "error" => {
            let (message, error_type) = extract_anthropic_error(&data);
            return (state.error_event(message, error_type), true);
        }
        _ => Vec::new(),
    };
    let finished = state.finished;
    (events, finished)
}

fn json_document_candidate(input: &str) -> Option<&str> {
    let trimmed = input.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '\u{feff}');
    matches!(trimmed.as_bytes().first(), Some(b'{')).then_some(trimmed)
}

/// 把完整的非流式 Anthropic message JSON 合成 Chat SSE 事件序列。
/// 兼容"上游忽略 stream:true 直接返回 JSON"的网关。
fn chat_sse_events_from_anthropic_message(body: &Value) -> Vec<Bytes> {
    let mut state = AnthropicToChatState::default();
    if body.get("type").and_then(Value::as_str) == Some("error") || body.get("error").is_some() {
        let (message, error_type) = extract_anthropic_error(body);
        return state.error_event(message, error_type);
    }

    let mut message_start = body.clone();
    message_start["content"] = json!([]);
    let mut events = state.handle_message_start(&json!({
        "type": "message_start",
        "message": message_start
    }));

    if let Some(content) = body.get("content").and_then(Value::as_array) {
        for (index, block) in content.iter().enumerate() {
            let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
            events.extend(state.handle_content_block_start(&json!({
                "type": "content_block_start",
                "index": index,
                "content_block": block
            })));
            if block_type == "text" {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    events.extend(state.handle_content_block_delta(&json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {"type": "text_delta", "text": text}
                    })));
                }
            }
            events.extend(state.handle_content_block_stop(&json!({
                "type": "content_block_stop",
                "index": index
            })));
        }
    }

    events.extend(state.handle_message_delta(&json!({
        "type": "message_delta",
        "delta": {"stop_reason": body.get("stop_reason").cloned().unwrap_or(Value::Null)},
        "usage": body.get("usage").cloned().unwrap_or(Value::Null)
    })));
    events.extend(state.finalize());
    events
}

/// 上游 Anthropic Messages SSE → OpenAI Chat Completions SSE。
pub fn create_chat_sse_stream_from_anthropic<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut state = AnthropicToChatState::default();
        let mut stream_failed = false;

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    crate::proxy::sse::append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

                    // 兼容"上游忽略 stream:true 返回单个 JSON 文档"
                    if json_document_candidate(&buffer).is_none() {
                        while let Some(block) = take_sse_block(&mut buffer) {
                            let (events, finished) = process_block(&mut state, &block);
                            for event in events {
                                yield Ok(event);
                            }
                            if finished {
                                break;
                            }
                        }
                    }

                    if state.finished {
                        break;
                    }
                }
                Err(e) => {
                    for event in state.error_event(
                        format!("Stream error: {e}"),
                        Some("stream_error".to_string()),
                    ) {
                        yield Ok(event);
                    }
                    stream_failed = true;
                    break;
                }
            }
        }

        // EOF 收尾：完整 JSON 文档 或 缺失终止事件的补齐
        if !stream_failed && !state.finished {
            if !state.started {
                if let Some(candidate) = json_document_candidate(&buffer) {
                    if let Ok(body) = serde_json::from_str::<Value>(candidate) {
                        for event in chat_sse_events_from_anthropic_message(&body) {
                            yield Ok(event);
                        }
                        return;
                    }
                }
            }
            // 已开始但上游断流：处理残余块 + 强制 finalize
            while let Some(block) = take_sse_block(&mut buffer) {
                let (events, _) = process_block(&mut state, &block);
                for event in events {
                    yield Ok(event);
                }
            }
            if !buffer.trim().is_empty() {
                let (events, _) = process_block(&mut state, &buffer.clone());
                for event in events {
                    yield Ok(event);
                }
            }
            for event in state.finalize() {
                yield Ok(event);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    async fn collect_chat_chunks(input: &str) -> Vec<String> {
        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(
            input.as_bytes().to_vec(),
        ))]);
        let converted = create_chat_sse_stream_from_anthropic(upstream);
        let chunks: Vec<_> = converted.collect().await;
        let merged = chunks
            .into_iter()
            .map(|c| String::from_utf8_lossy(c.unwrap().as_ref()).to_string())
            .collect::<String>();
        merged
            .split("\n\n")
            .filter(|b| !b.trim().is_empty())
            .map(|b| b.trim().to_string())
            .collect()
    }

    fn parse_data(block: &str) -> Option<Value> {
        let data = block
            .lines()
            .find_map(|line| strip_sse_field(line, "data"))?;
        serde_json::from_str::<Value>(data).ok()
    }

    #[tokio::test]
    async fn text_conversation_maps_to_chat_chunks() {
        let input = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-x\",\"usage\":{\"input_tokens\":10}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let blocks = collect_chat_chunks(input).await;

        // 首块 role
        let first = parse_data(&blocks[0]).unwrap();
        assert_eq!(first["choices"][0]["delta"]["role"], "assistant");
        assert!(first["id"].as_str().unwrap().starts_with("chatcmpl-msg_1"));
        assert_eq!(first["model"], "claude-x");

        // 文本增量
        let c1 = parse_data(&blocks[1]).unwrap();
        assert_eq!(c1["choices"][0]["delta"]["content"], "Hello");
        let c2 = parse_data(&blocks[2]).unwrap();
        assert_eq!(c2["choices"][0]["delta"]["content"], " world");

        // finish_reason
        let fin = parse_data(&blocks[3]).unwrap();
        assert_eq!(fin["choices"][0]["finish_reason"], "stop");

        // usage chunk：input 10 + output 5
        let usage = parse_data(&blocks[4]).unwrap();
        assert_eq!(usage["usage"]["prompt_tokens"], 10);
        assert_eq!(usage["usage"]["completion_tokens"], 5);
        assert_eq!(usage["usage"]["total_tokens"], 15);

        // [DONE]
        assert_eq!(blocks.last().unwrap(), "data: [DONE]");
    }

    #[tokio::test]
    async fn tool_call_maps_to_tool_calls_chunks() {
        let input = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_2\",\"model\":\"claude-x\"}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"get_weather\",\"input\":{}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"SF\\\"}\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let blocks = collect_chat_chunks(input).await;

        // tool_call 开始块
        let start = parse_data(&blocks[1]).unwrap();
        let tc = &start["choices"][0]["delta"]["tool_calls"][0];
        assert_eq!(tc["index"], 0);
        assert_eq!(tc["id"], "toolu_1");
        assert_eq!(tc["function"]["name"], "get_weather");
        assert_eq!(tc["function"]["arguments"], "");

        // arguments 增量
        let d1 = parse_data(&blocks[2]).unwrap();
        assert_eq!(
            d1["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
            "{\"city\":"
        );
        let d2 = parse_data(&blocks[3]).unwrap();
        assert_eq!(
            d2["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
            "\"SF\"}"
        );

        // finish_reason = tool_calls
        let fin = parse_data(&blocks[4]).unwrap();
        assert_eq!(fin["choices"][0]["finish_reason"], "tool_calls");
    }

    #[tokio::test]
    async fn tool_call_with_start_input_and_no_delta_emits_fallback_arguments() {
        let input = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_3\",\"model\":\"m\"}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"f\",\"input\":{\"a\":1}}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let blocks = collect_chat_chunks(input).await;
        // start 块之后应有 arguments 兜底块
        let fallback = parse_data(&blocks[2]).unwrap();
        assert_eq!(
            fallback["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
            "{\"a\":1}"
        );
    }

    #[tokio::test]
    async fn non_streaming_json_document_synthesizes_chat_stream() {
        let input = "{\"id\":\"msg_4\",\"type\":\"message\",\"model\":\"claude-x\",\"content\":[{\"type\":\"text\",\"text\":\"Hi\"}],\"stop_reason\":\"end_turn\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}";
        let blocks = collect_chat_chunks(input).await;
        let first = parse_data(&blocks[0]).unwrap();
        assert_eq!(first["choices"][0]["delta"]["role"], "assistant");
        let content = parse_data(&blocks[1]).unwrap();
        assert_eq!(content["choices"][0]["delta"]["content"], "Hi");
        let fin = parse_data(&blocks[2]).unwrap();
        assert_eq!(fin["choices"][0]["finish_reason"], "stop");
        assert_eq!(blocks.last().unwrap(), "data: [DONE]");
    }

    #[tokio::test]
    async fn upstream_error_event_maps_to_error_and_done() {
        let input = concat!(
            "event: error\n",
            "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n",
        );
        let blocks = collect_chat_chunks(input).await;
        let err = parse_data(&blocks[0]).unwrap();
        assert_eq!(err["error"]["message"], "Overloaded");
        assert_eq!(err["error"]["type"], "overloaded_error");
        assert_eq!(blocks.last().unwrap(), "data: [DONE]");
    }
}
