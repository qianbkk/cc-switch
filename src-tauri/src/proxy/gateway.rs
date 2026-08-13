//! 统一网关 HTTP 处理器
//!
//! 入口路径全部以 `/gateway/*` 为前缀（避免与现有 `/v1/*`、`/claude-desktop/*`
//! 等冲突）：
//!
//! | 路径                              | 入站协议                    |
//! |-----------------------------------|----------------------------|
//! | `POST /gateway/v1/messages`         | Anthropic Messages           |
//! | `POST /gateway/v1/chat/completions` | OpenAI Chat Completions      |
//! | `POST /gateway/v1/responses`        | OpenAI Responses             |
//! | `GET  /gateway/v1/models`           | OpenAI 风格模型列表          |
//!
//! 处理流程：
//! 1. 读 `gateway_config`（settings KV 键 `gateway_config`）
//! 2. `Authorization: Bearer <key>` 或 `x-api-key: <key>` 鉴权
//!    - 缺/错 → 401 JSON `{error:{message,type:"invalid_request_error"}}`
//!    - `enabled=false` → 403（同 JSON 形态）
//! 3. 解析 body 取 `model` 字段，与 `models[].alias` 精确匹配
//!    - 未命中 → 400 JSON，message 含可用 alias 列表
//! 4. 命中后改写 `body.model` 为真实模型名；从 DB 读 provider 记录，
//!    走单 provider 候选列表调用现有 `RequestForwarder`（保留
//!    熔断器 key `app_type:provider_id`，但跳过 failover 队列）
//! 5. 上游响应按入站协议反向转换后返回：
//!
//!    - 协议一致时直接透传，避免无谓解析；
//!    - 非流式响应先归一为 Anthropic JSON，再转换为 Anthropic、OpenAI Chat
//!      或 OpenAI Responses 目标格式；
//!    - SSE 响应先归一为 Anthropic 事件流，再通过 `streaming_responses` 或
//!      `streaming_anthropic_chat` 转成对应的入站协议；
//!    - Gemini 上游复用 `streaming_gemini` / JSON 转换路径。
//!
//!    因而当前网关同时覆盖三种入站协议与各 provider `api_format` 的请求、响应
//!    双向转换；新增协议时必须同时补齐非流式和流式矩阵测试。

use super::{
    forwarder::RequestForwarder,
    hyper_client::{ProxyResponse, MAX_RESPONSE_BODY_BYTES},
    limits::{MAX_REQUEST_BODY_BYTES, MAX_SSE_EVENT_BYTES},
    providers::{
        streaming, streaming_anthropic_chat, streaming_codex_anthropic, streaming_gemini,
        streaming_responses, transform, transform_responses,
    },
    server::ProxyState,
    types::{CopilotOptimizerConfig, OptimizerConfig, RectifierConfig},
    ProxyError,
};
use crate::app_config::AppType;
use crate::commands::gateway::{load_gateway_config, GatewayConfig, GatewayModelEntry};
use crate::provider::Provider;
use crate::proxy::error_mapper::map_proxy_error_to_status;
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use bytes::Bytes;
use futures::StreamExt;
use http::Extensions;
use serde_json::{json, Value};
use std::str::FromStr;

/// 入站协议（用于决定 body.model 字段位置）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InboundProtocol {
    Anthropic,
    OpenAiChat,
    OpenAiResponses,
}

impl InboundProtocol {
    #[allow(dead_code)]
    fn from_path(path: &str) -> Option<Self> {
        let normalized = path.split('?').next().unwrap_or(path);
        if normalized.ends_with("/v1/messages") {
            Some(Self::Anthropic)
        } else if normalized.ends_with("/v1/chat/completions") {
            Some(Self::OpenAiChat)
        } else if normalized.ends_with("/v1/responses") {
            Some(Self::OpenAiResponses)
        } else {
            None
        }
    }

    fn default_endpoint(&self) -> &'static str {
        match self {
            Self::Anthropic => "/v1/messages",
            Self::OpenAiChat => "/v1/chat/completions",
            Self::OpenAiResponses => "/v1/responses",
        }
    }
}

// =====================================================================
// 鉴权辅助
// =====================================================================

/// 从 header 提取客户端 key，支持：
/// - `Authorization: Bearer <k>`（大小写不敏感前缀）
/// - `x-api-key: <k>`（裸 header 值）
fn extract_client_key(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get(http::header::AUTHORIZATION) {
        if let Ok(s) = v.to_str() {
            if let Some(stripped) = s
                .strip_prefix("Bearer ")
                .or_else(|| s.strip_prefix("bearer "))
                .or_else(|| s.strip_prefix("BEARER "))
            {
                return Some(stripped.trim().to_string());
            }
        }
    }
    if let Some(v) = headers.get("x-api-key") {
        if let Ok(s) = v.to_str() {
            return Some(s.trim().to_string());
        }
    }
    None
}

/// 构造统一错误响应（稳定契约，路线图第 17 项）：
///
/// ```json
/// {
///   "error": {
///     "message": "人类可读信息",
///     "type": "invalid_request_error",
///     "code": "稳定机器可读错误码",
///     "request_id": "本次请求唯一 id"
///   }
/// }
/// ```
///
/// 同时写入 `x-request-id` 响应头，方便客户端与服务端日志关联。
/// 客户端应依赖稳定的 `status` + `code` 处理异常，`message` 仅供展示。
fn gateway_error_response(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string();
    let body = Json(json!({
        "error": {
            "message": message.into(),
            "type": "invalid_request_error",
            "code": code,
            "request_id": request_id,
        }
    }));
    let mut resp = (status, body).into_response();
    if let Ok(v) = request_id.parse() {
        resp.headers_mut().insert("x-request-id", v);
    }
    resp
}

/// 鉴权/路由层错误响应（code 为稳定契约错误码）。
fn auth_error_response(status: StatusCode, code: &str, msg: &str) -> Response {
    gateway_error_response(status, code, msg)
}

/// 将 `ProxyError` 映射为稳定契约错误码（供客户端机器处理）。
fn proxy_error_code(error: &ProxyError) -> &'static str {
    match error {
        ProxyError::RequestBodyTooLarge(_) => "payload_too_large",
        ProxyError::InvalidRequest(_) => "invalid_request",
        ProxyError::ConfigError(_) => "config_error",
        ProxyError::AuthError(_) => "unauthorized",
        ProxyError::Timeout(_) => "upstream_timeout",
        ProxyError::StreamIdleTimeout(_) => "upstream_idle_timeout",
        ProxyError::ForwardFailed(_) => "upstream_unreachable",
        ProxyError::NoAvailableProvider
        | ProxyError::AllProvidersCircuitOpen
        | ProxyError::NoProvidersConfigured
        | ProxyError::MaxRetriesExceeded
        | ProxyError::ProviderUnhealthy(_) => "upstream_unavailable",
        ProxyError::TransformError(_) => "transform_error",
        ProxyError::DatabaseError(_) => "database_error",
        ProxyError::ResponseBodyTooLarge(_) => "response_too_large",
        _ => "internal_error",
    }
}

/// 鉴权：返回 `GatewayConfig`（用于 alias 查找）或 401/403 响应
fn authorize(
    db: &crate::database::Database,
    headers: &HeaderMap,
) -> Result<GatewayConfig, Box<Response>> {
    let cfg = load_gateway_config(db).map_err(|e| {
        log::error!("[Gateway] 加载配置失败: {e}");
        Box::new(auth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "网关配置加载失败",
        ))
    })?;

    // 魔改总开关关闭时，统一网关整体停用（配置保留，重新打开即恢复）。
    if !crate::settings::fork_features_enabled() {
        return Err(Box::new(auth_error_response(
            StatusCode::FORBIDDEN,
            "feature_disabled",
            "魔改功能已在设置中整体关闭，统一网关不可用",
        )));
    }

    if !cfg.enabled {
        return Err(Box::new(auth_error_response(
            StatusCode::FORBIDDEN,
            "gateway_disabled",
            "网关未启用，请在设置中打开「统一网关」开关",
        )));
    }

    let client_key = match extract_client_key(headers) {
        Some(k) if !k.is_empty() => k,
        _ => {
            return Err(Box::new(auth_error_response(
                StatusCode::UNAUTHORIZED,
                "missing_api_key",
                "缺少鉴权头（Authorization: Bearer <key> 或 x-api-key: <key>）",
            )));
        }
    };

    if client_key != cfg.api_key {
        return Err(Box::new(auth_error_response(
            StatusCode::UNAUTHORIZED,
            "invalid_api_key",
            "网关 key 无效",
        )));
    }

    Ok(cfg)
}

// =====================================================================
// Body 解析
// =====================================================================

fn extract_model(body: &Value, _proto: InboundProtocol) -> Option<&str> {
    // 三种协议都使用顶层 `model` 字段
    body.get("model").and_then(|v| v.as_str())
}

fn resolve_alias<'a>(
    cfg: &'a GatewayConfig,
    model: &str,
) -> Result<&'a GatewayModelEntry, Vec<String>> {
    let aliases: Vec<String> = cfg.models.iter().map(|m| m.alias.clone()).collect();
    cfg.models.iter().find(|m| m.alias == model).ok_or(aliases)
}

fn alias_not_found_response(aliases: Vec<String>) -> Response {
    let joined = if aliases.is_empty() {
        "当前没有任何模型被勾选暴露".to_string()
    } else {
        format!("可用 alias: [{}]", aliases.join(", "))
    };
    auth_error_response(
        StatusCode::BAD_REQUEST,
        "alias_not_found",
        &format!("alias 未命中；{joined}"),
    )
}

// =====================================================================
// Provider 加载
// =====================================================================

fn load_provider(
    db: &crate::database::Database,
    entry: &GatewayModelEntry,
) -> Result<Provider, Box<Response>> {
    let app_type = AppType::from_str(&entry.app_type).map_err(|_| {
        Box::new(auth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_app_type",
            &format!(
                "非法 appType: {}（期望 claude | codex | gemini）",
                entry.app_type
            ),
        ))
    })?;

    let provider = db
        .get_provider_by_id(&entry.provider_id, app_type.as_str())
        .map_err(|e| {
            log::error!("[Gateway] 数据库读取 provider 失败: {e}");
            Box::new(auth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "读取供应商失败",
            ))
        })?
        .ok_or_else(|| {
            Box::new(auth_error_response(
                StatusCode::NOT_FOUND,
                "provider_not_found",
                &format!(
                    "供应商不存在：provider_id={}, app_type={}",
                    entry.provider_id, entry.app_type
                ),
            ))
        })?;

    Ok(provider)
}

// =====================================================================
// 转发构造
// =====================================================================

/// 构建单 provider 候选的 `RequestForwarder`。
///
/// 语义（路线图第 13 项）：
/// - 保持「单别名只路由一个供应商」：`max_retries=0`，不悄悄故障转移。
/// - 超时不再全部禁用：非流式 / 首字节 / 流空闲超时读取网关配置
///   （`GatewayConfig::*_timeout_secs`，默认 600/60/120），0 仍表示禁用。
fn build_forwarder(
    state: &ProxyState,
    provider: &Provider,
    cfg: &GatewayConfig,
) -> RequestForwarder {
    RequestForwarder::new(
        state.provider_router.clone(),
        cfg.non_streaming_timeout_secs,
        state.status.clone(),
        state.current_providers.clone(),
        state.gemini_shadow.clone(),
        state.codex_chat_history.clone(),
        state.failover_manager.clone(),
        state.app_handle.clone(),
        provider.id.clone(),
        String::new(), // session_id (unused for gateway)
        false,         // session_client_provided
        cfg.streaming_first_byte_timeout_secs,
        cfg.streaming_idle_timeout_secs,
        RectifierConfig::default(),
        OptimizerConfig::default(),
        CopilotOptimizerConfig::default(),
        0, // max_retries => single attempt
    )
}

/// 实际发起单 provider 转发
#[allow(clippy::too_many_arguments)] // 与 forwarder::forward_with_retry_inner 一致：参数为转发所需上下文
async fn forward_with_single_provider(
    state: &ProxyState,
    provider: Provider,
    app_type: AppType,
    endpoint: &str,
    body: Value,
    headers: HeaderMap,
    extensions: Extensions,
    cfg: &GatewayConfig,
) -> Result<
    (
        ProxyResponse,
        Option<String>, // claude_api_format（上游协议）
        bool,           // is_streaming
    ),
    ProxyError,
> {
    let is_streaming = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let forwarder = build_forwarder(state, &provider, cfg);
    let result = forwarder
        .forward_with_retry(
            &app_type,
            http::Method::POST,
            endpoint,
            body,
            headers,
            extensions,
            vec![provider], // 强制单 provider 列表
        )
        .await
        .map_err(|fe| fe.error)?;
    Ok((result.response, result.claude_api_format, is_streaming))
}

/// 把 `ProxyResponse` 转成 axum `Response`：按上游/入站协议做 JSON 或 SSE 反向转换。
///
/// 设计：non-streaming 完整实现（链式转 Anthropic 中间表示，再转出）；
/// streaming 走"上游 SSE 协议 → Anthropic SSE → 入站 SSE"两步链式。
/// OpenAI Chat、OpenAI Responses 与 Gemini 上游均先转换为 Anthropic SSE，
/// 再按入站协议输出 Anthropic、OpenAI Chat 或 OpenAI Responses SSE。
/// 协议分支已接通；完整互操作性仍由转换器单测和后续网关 E2E 矩阵持续验证。
async fn proxy_response_to_axum(
    resp: ProxyResponse,
    inbound: InboundProtocol,
    upstream_format: Option<String>,
    is_streaming: bool,
    streaming_idle_timeout_secs: u64,
) -> Response {
    let status = resp.status();
    let headers = resp.headers().clone();

    if is_streaming {
        // 流式路径：直接拿 stream，构造链式 SSE 转换。
        // 先加单事件大小守卫（在原始上游字节流上逐事件检查），
        // 防止超大单事件撑爆下游转换器内存。
        let guarded_stream = limit_sse_event_size(
            Box::pin(resp.bytes_stream().map(|item| match item {
                Ok(b) => Ok::<Bytes, std::io::Error>(b),
                Err(e) => Err(e),
            })),
            MAX_SSE_EVENT_BYTES,
        );
        // 再加流空闲超时守卫：两个数据块之间的最大间隔超过配置值即终止流，
        // 防止上游卡死导致连接无限挂起（0 = 禁用）。
        let guarded_stream = if streaming_idle_timeout_secs > 0 {
            limit_sse_idle_timeout(guarded_stream, streaming_idle_timeout_secs)
        } else {
            guarded_stream
        };
        let body_stream =
            match build_inbound_sse_stream(guarded_stream, upstream_format.as_deref(), inbound) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("[Gateway] SSE 转换失败，透传上游: {e}");
                    // 重新拿 stream（已经在上面 move 走；只能返回错误）
                    return error_to_response(ProxyError::Internal(format!("SSE transform: {e}")));
                }
            };
        let mut response_builder = Response::builder().status(status);
        for (name, value) in headers.iter() {
            if matches!(
                name.as_str(),
                "content-length"
                    | "transfer-encoding"
                    | "connection"
                    | "keep-alive"
                    | "proxy-authenticate"
                    | "proxy-authorization"
                    | "te"
                    | "trailers"
                    | "upgrade"
            ) {
                continue;
            }
            response_builder = response_builder.header(name.clone(), value.clone());
        }
        response_builder = response_builder.header("content-type", "text/event-stream");
        return response_builder
            .body(Body::from_stream(body_stream))
            .unwrap_or_else(|e| {
                error_to_response(ProxyError::Internal(format!("build response: {e}")))
            });
    }

    // 非流式路径：读完整 body
    let bytes = match resp.bytes_with_limit(MAX_RESPONSE_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            return error_to_response(ProxyError::Internal(format!("read upstream body: {e}")));
        }
    };

    let body_bytes: Bytes = if !status.is_success() {
        bytes
    } else {
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(body_json) => {
                match convert_response_body(body_json, upstream_format.as_deref(), inbound) {
                    Ok(v) => match serde_json::to_vec(&v) {
                        Ok(buf) => Bytes::from(buf),
                        Err(e) => {
                            log::warn!("[Gateway] 响应序列化失败，透传原始: {e}");
                            bytes
                        }
                    },
                    Err(e) => {
                        log::warn!(
                            "[Gateway] 响应转换失败 ({inbound:?}<-{upstream_format:?}): {e}，透传原始"
                        );
                        bytes
                    }
                }
            }
            Err(_) => bytes,
        }
    };

    let mut response_builder = Response::builder().status(status);
    for (name, value) in headers.iter() {
        if matches!(
            name.as_str(),
            "content-length"
                | "transfer-encoding"
                | "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailers"
                | "upgrade"
        ) {
            continue;
        }
        response_builder = response_builder.header(name.clone(), value.clone());
    }
    response_builder
        .body(Body::from(body_bytes))
        .unwrap_or_else(|e| error_to_response(ProxyError::Internal(format!("build response: {e}"))))
}

/// 上游协议 → Anthropic JSON（中间表示）
fn upstream_to_anthropic_json(
    body: Value,
    upstream_format: Option<&str>,
) -> Result<Value, ProxyError> {
    match upstream_format {
        None => Ok(body),
        Some("openai_chat") => transform::openai_to_anthropic(body),
        Some("openai_responses") => transform_responses::responses_to_anthropic(body),
        Some("gemini_native") => {
            crate::proxy::providers::transform_gemini::gemini_to_anthropic(body)
        }
        Some(other) => {
            log::warn!("[Gateway] 未知上游协议 {other}，按 Anthropic 透传");
            Ok(body)
        }
    }
}

/// Anthropic JSON → 入站协议
fn anthropic_to_inbound_json(body: Value, inbound: InboundProtocol) -> Result<Value, ProxyError> {
    match inbound {
        InboundProtocol::Anthropic => Ok(body),
        InboundProtocol::OpenAiChat => transform::anthropic_to_openai(body),
        InboundProtocol::OpenAiResponses => {
            // cache_key/is_codex_oauth/codex_fast_mode 网关场景下保持默认。
            transform_responses::anthropic_to_responses(body, None, false, false)
        }
    }
}

/// 反向转换入口。
fn convert_response_body(
    upstream_body: Value,
    claude_api_format: Option<&str>,
    inbound: InboundProtocol,
) -> Result<Value, ProxyError> {
    // 同协议零转换
    if inbound == InboundProtocol::Anthropic && claude_api_format.is_none() {
        return Ok(upstream_body);
    }
    if inbound == InboundProtocol::OpenAiResponses && claude_api_format == Some("openai_responses")
    {
        return Ok(upstream_body);
    }
    if inbound == InboundProtocol::OpenAiChat && claude_api_format == Some("openai_chat") {
        return Ok(upstream_body);
    }
    let canonical = upstream_to_anthropic_json(upstream_body, claude_api_format)?;
    anthropic_to_inbound_json(canonical, inbound)
}

/// SSE 流类型：上游/入站方按字节读取的 stream。
type SseStream =
    std::pin::Pin<Box<dyn futures::Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

/// 受限读取请求体：逐块累积，超过 `max_bytes` 立即返回 413（而非无限收集）。
async fn collect_body_with_limit(
    body: axum::body::Body,
    max_bytes: usize,
) -> Result<Bytes, ProxyError> {
    use bytes::BytesMut;
    let mut stream = body.into_data_stream();
    let mut buf = BytesMut::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ProxyError::Internal(format!("read body: {e}")))?;
        if buf.len() + chunk.len() > max_bytes {
            return Err(ProxyError::RequestBodyTooLarge(buf.len() + chunk.len()));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf.freeze())
}

/// 流式单事件大小守卫：在原始上游 SSE 字节流上逐事件检查。
///
/// 事件边界为 `\n\n` 或 `\r\n\r\n`（SSE 通用分隔符）。单个事件（不含分隔符）
/// 超过 `max_event_bytes` 立即终止流并报错，防止异常/恶意上游发送超大单事件
/// 撑爆下游转换器内存。事件按原样透传（chunk 边界可能被重组，SSE 语义不变）。
fn limit_sse_event_size(upstream: SseStream, max_event_bytes: usize) -> SseStream {
    Box::pin(async_stream::stream! {
        let mut pending: Vec<u8> = Vec::new();
        let mut stream = upstream;
        while let Some(item) = stream.next().await {
            match item {
                Ok(bytes) => {
                    pending.extend_from_slice(&bytes);
                    loop {
                        match find_sse_event_delimiter(&pending) {
                            Some((event_len, delim_len)) => {
                                if event_len > max_event_bytes {
                                    log::warn!(
                                        "[Gateway] SSE 单事件超限（{event_len} > {max_event_bytes} 字节），终止流"
                                    );
                                    yield Err(std::io::Error::other(format!(
                                        "SSE event exceeds {max_event_bytes} bytes"
                                    )));
                                    return;
                                }
                                let complete_len = event_len + delim_len;
                                let complete: Vec<u8> = pending.drain(..complete_len).collect();
                                yield Ok(Bytes::from(complete));
                            }
                            None => {
                                // 无完整事件；若累积已超限（单个无界事件），提前报错
                                if pending.len() > max_event_bytes {
                                    log::warn!(
                                        "[Gateway] SSE 单事件无边界且累积超限（{} > {max_event_bytes} 字节），终止流",
                                        pending.len()
                                    );
                                    yield Err(std::io::Error::other(format!(
                                        "SSE event exceeds {max_event_bytes} bytes"
                                    )));
                                    return;
                                }
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    yield Err(e);
                    return;
                }
            }
        }
        // 流正常结束：尾部未完成事件（无分隔符）直接丢弃，不转发。
    })
}

/// 在字节 buffer 中找最早的 SSE 事件分隔符（`\n\n` 或 `\r\n\r\n`）。
/// 返回 `(事件起始处到分隔符的距离 = 事件体长度, 分隔符字节数)`。
fn find_sse_event_delimiter(buf: &[u8]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for (needle, len) in [
        (b"\r\n\r\n".as_slice(), 4usize),
        (b"\n\n".as_slice(), 2usize),
    ] {
        if let Some(pos) = buf.windows(needle.len()).position(|w| w == needle) {
            let cand = (pos, len);
            if best.is_none_or(|(best_pos, _)| pos < best_pos) {
                best = Some(cand);
            }
        }
    }
    best
}

/// 流空闲超时守卫：包装 SSE 字节流，两个数据块之间的间隔超过 `idle_secs`
/// 立即终止流并报错（504 语义），防止上游卡死导致连接无限挂起。
///
/// 注意：只对「块间间隔」计时，不限制总时长——长流式补全（如长文档生成）
/// 只要持续有数据就正常通过。`idle_secs = 0` 应在外层过滤，不调用本函数。
fn limit_sse_idle_timeout(upstream: SseStream, idle_secs: u64) -> SseStream {
    Box::pin(async_stream::stream! {
        let mut stream = upstream;
        loop {
            let next = tokio::time::timeout(
                std::time::Duration::from_secs(idle_secs),
                stream.next(),
            )
            .await;
            match next {
                Ok(Some(item)) => match item {
                    Ok(bytes) => yield Ok(bytes),
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                },
                Ok(None) => {
                    // 上游流正常结束
                    return;
                }
                Err(_elapsed) => {
                    log::warn!(
                        "[Gateway] 流式响应空闲超时（{idle_secs}s 无数据），终止流"
                    );
                    yield Err(std::io::Error::other(format!(
                        "streaming idle timeout after {idle_secs}s without data"
                    )));
                    return;
                }
            }
        }
    })
}

/// SSE 链式构造：上游 → Anthropic SSE → 入站 SSE
fn build_inbound_sse_stream(
    upstream: SseStream,
    upstream_format: Option<&str>,
    inbound: InboundProtocol,
) -> Result<SseStream, ProxyError> {
    let anthropic_stream: SseStream = match upstream_format {
        None => upstream,
        Some("openai_responses") => {
            Box::pin(streaming_responses::create_anthropic_sse_stream_from_responses(upstream))
        }
        Some("gemini_native") => {
            Box::pin(streaming_gemini::create_anthropic_sse_stream_from_gemini(
                upstream, None, None, None, None,
            ))
        }
        Some("openai_chat") => Box::pin(
            // Chat SSE → Anthropic SSE（复用 Claude 客户端 + OpenAI 兼容上游的现成转换器）
            streaming::create_anthropic_sse_stream(upstream),
        ),
        Some(other) => {
            log::warn!("[Gateway] 未知上游协议 {other}，SSE 按 Anthropic 透传");
            upstream
        }
    };

    let final_stream: SseStream = match inbound {
        InboundProtocol::Anthropic => anthropic_stream,
        InboundProtocol::OpenAiResponses => Box::pin(
            streaming_codex_anthropic::create_responses_sse_stream_from_anthropic(anthropic_stream),
        ),
        InboundProtocol::OpenAiChat => Box::pin(
            // Anthropic SSE → Chat SSE（网关新增转换器）
            streaming_anthropic_chat::create_chat_sse_stream_from_anthropic(anthropic_stream),
        ),
    };

    Ok(final_stream)
}

fn error_to_response(error: ProxyError) -> Response {
    let status_code = map_proxy_error_to_status(&error);
    let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    gateway_error_response(status, proxy_error_code(&error), error.to_string())
}

// =====================================================================
// 端点
// =====================================================================

pub async fn handle_gateway_messages(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Response {
    process_gateway_request(state, request, InboundProtocol::Anthropic).await
}

pub async fn handle_gateway_chat_completions(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Response {
    process_gateway_request(state, request, InboundProtocol::OpenAiChat).await
}

pub async fn handle_gateway_responses(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Response {
    process_gateway_request(state, request, InboundProtocol::OpenAiResponses).await
}

/// `GET /gateway/v1/models`：OpenAI 风格模型列表
/// `{"object":"list","data":[{"id":alias,"object":"model","owned_by":"供应商名"}, ...]}`
pub async fn handle_gateway_models(
    State(state): State<ProxyState>,
    headers: HeaderMap,
) -> Response {
    let cfg = match authorize(&state.db, &headers) {
        Ok(c) => c,
        Err(resp) => return *resp,
    };

    let mut data: Vec<Value> = Vec::with_capacity(cfg.models.len());
    for m in &cfg.models {
        let owned_by = load_provider(&state.db, m)
            .map(|p| p.name)
            .unwrap_or_else(|_| "<unknown>".to_string());
        data.push(json!({
            "id": m.alias,
            "object": "model",
            "owned_by": owned_by,
        }));
    }
    let body = Json(json!({
        "object": "list",
        "data": data,
    }));
    (StatusCode::OK, body).into_response()
}

// =====================================================================
// 统一入口处理逻辑
// =====================================================================

async fn process_gateway_request(
    state: ProxyState,
    request: axum::extract::Request,
    proto: InboundProtocol,
) -> Response {
    let (parts, req_body) = request.into_parts();
    let headers = parts.headers;
    let extensions = parts.extensions;

    let body_bytes = match collect_body_with_limit(req_body, MAX_REQUEST_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            return error_to_response(e);
        }
    };
    let mut body: Value = match serde_json::from_slice(&body_bytes) {
        Ok(b) => b,
        Err(e) => {
            // 无效 JSON 属于客户端错误：400，而非 500。
            return error_to_response(ProxyError::InvalidRequest(format!(
                "请求体不是合法 JSON: {e}"
            )));
        }
    };

    // 鉴权
    let cfg = match authorize(&state.db, &headers) {
        Ok(c) => c,
        Err(resp) => return *resp,
    };

    // alias 匹配
    let model_in_body = match extract_model(&body, proto) {
        Some(m) => m.to_string(),
        None => {
            return auth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "请求体缺少 model 字段",
            );
        }
    };

    let entry = match resolve_alias(&cfg, &model_in_body) {
        Ok(e) => e,
        Err(list) => return alias_not_found_response(list),
    };

    // 防御：重复 alias（配置保存层已校验，路由层兜底拒绝，避免歧义路由）
    if cfg.models.iter().filter(|m| m.alias == entry.alias).count() > 1 {
        return auth_error_response(
            StatusCode::BAD_REQUEST,
            "duplicate_alias",
            &format!("alias 重复定义: {}", entry.alias),
        );
    }

    // 改写 model
    if let Some(obj) = body.as_object_mut() {
        obj.insert("model".into(), Value::String(entry.model.clone()));
    }

    // appType 解析
    let app_type = match AppType::from_str(&entry.app_type) {
        Ok(a) => a,
        Err(_) => {
            return auth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_app_type",
                &format!("非法 appType: {}", entry.app_type),
            );
        }
    };

    // 加载 provider
    let provider = match load_provider(&state.db, entry) {
        Ok(p) => p,
        Err(resp) => return *resp,
    };

    let endpoint = proto.default_endpoint();
    log::info!(
        "[Gateway] 入站={:?}, alias={}, 上游={} (app_type={}, real_model={})",
        proto,
        entry.alias,
        provider.name,
        entry.app_type,
        entry.model
    );

    let inbound = proto;
    let is_streaming = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match forward_with_single_provider(
        &state, provider, app_type, endpoint, body, headers, extensions, &cfg,
    )
    .await
    {
        Ok((resp, upstream_format, _is_streaming_request)) => {
            proxy_response_to_axum(
                resp,
                inbound,
                upstream_format,
                is_streaming,
                cfg.streaming_idle_timeout_secs,
            )
            .await
        }
        Err(e) => error_to_response(e),
    }
}

// =====================================================================
// 冒烟测试：路由级端到端（鉴权 / alias 路由 / 模型列表）
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::proxy::server::ProxyServer;
    use crate::proxy::types::ProxyConfig;
    use axum::body::Body as AxumBody;
    use http_body_util::BodyExt as _;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use serial_test::serial;
    use tower::ServiceExt;

    const KEY: &str = "ccs-testkey";

    fn setup_router(enabled: bool) -> axum::Router {
        let cfg = json!({
            "enabled": enabled,
            "apiKey": KEY,
            "models": [{
                "alias": "test-provider/claude-test-1",
                "providerId": "prov-1",
                "appType": "claude",
                "model": "claude-test-1"
            }]
        });
        setup_router_with_cfg(cfg)
    }

    /// 用自定义配置构建路由，同时返回 db 句柄（测试可更新配置验证 Key 轮换等场景）。
    fn setup_router_with_db(cfg: Value) -> (axum::Router, Arc<crate::database::Database>) {
        let db = Arc::new(Database::memory().expect("memory db"));
        db.set_setting("gateway_config", &cfg.to_string())
            .expect("save gateway config");
        let server = ProxyServer::new(ProxyConfig::default(), db.clone(), None);
        (server.build_router(), db)
    }

    fn setup_router_with_cfg(cfg: Value) -> axum::Router {
        setup_router_with_db(cfg).0
    }

    /// 构造带已保存 provider 的路由（models 端点不泄露 Key / 供应商存在场景）。
    fn setup_router_with_provider(cfg: Value, provider_id: &str, app_type: &str) -> axum::Router {
        let db = Arc::new(Database::memory().expect("memory db"));
        db.set_setting("gateway_config", &cfg.to_string())
            .expect("save gateway config");
        let provider = crate::Provider::with_id(
            provider_id.to_string(),
            "Test Provider".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "sk-ant-secret-key-must-not-leak",
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:1"
                }
            }),
            None,
        );
        db.save_provider(app_type, &provider)
            .expect("save provider");
        let server = ProxyServer::new(ProxyConfig::default(), db, None);
        server.build_router()
    }

    async fn body_json(resp: axum::response::Response) -> Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    fn post_messages(auth: Option<&str>, model: &str) -> http::Request<AxumBody> {
        let mut builder = http::Request::builder()
            .method("POST")
            .uri("/gateway/v1/messages")
            .header("content-type", "application/json");
        if let Some(key) = auth {
            builder = builder.header("authorization", format!("Bearer {key}"));
        }
        builder
            .body(AxumBody::from(
                json!({"model": model, "max_tokens": 8, "messages": []}).to_string(),
            ))
            .unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn missing_auth_returns_401() {
        let router = setup_router(true);
        let resp = router
            .oneshot(post_messages(None, "test-provider/claude-test-1"))
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[serial]
    async fn wrong_key_returns_401() {
        let router = setup_router(true);
        let resp = router
            .oneshot(post_messages(
                Some("wrong-key"),
                "test-provider/claude-test-1",
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[serial]
    async fn disabled_gateway_returns_403() {
        let router = setup_router(false);
        let resp = router
            .oneshot(post_messages(Some(KEY), "test-provider/claude-test-1"))
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    #[serial]
    async fn unknown_alias_returns_400_with_available_list() {
        let router = setup_router(true);
        let resp = router
            .oneshot(post_messages(Some(KEY), "nonexistent/model"))
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        let body = body_json(resp).await;
        let message = body["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("test-provider/claude-test-1"),
            "error should list available aliases, got: {message}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn models_endpoint_lists_aliases() {
        let router = setup_router(true);
        let req = http::Request::builder()
            .method("GET")
            .uri("/gateway/v1/models")
            .header("authorization", format!("Bearer {KEY}"))
            .body(AxumBody::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["object"], "list");
        assert_eq!(body["data"][0]["id"], "test-provider/claude-test-1");
    }

    #[tokio::test]
    #[serial]
    async fn x_api_key_header_is_accepted() {
        let router = setup_router(true);
        let req = http::Request::builder()
            .method("GET")
            .uri("/gateway/v1/models")
            .header("x-api-key", KEY)
            .body(AxumBody::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);
    }

    // ------------------------------------------------------------------
    // 大小限制（路线图 12）
    // ------------------------------------------------------------------

    #[test]
    fn sse_event_delimiter_finds_lf_and_crlf() {
        // \n\n 分隔
        assert_eq!(
            find_sse_event_delimiter(b"data: {\"a\":1}\n\nrest"),
            Some((13, 2)) // "data: {\"a\":1}" 是 13 字节
        );
        // \r\n\r\n 分隔："data: x" 是 7 字节，\r 从第 7 位开始（分隔符 4 字节）
        assert_eq!(
            find_sse_event_delimiter(b"data: x\r\n\r\nrest"),
            Some((7, 4))
        );
        // 无分隔符
        assert_eq!(find_sse_event_delimiter(b"data: incomplete"), None);
        // 空 buffer
        assert_eq!(find_sse_event_delimiter(b""), None);
    }

    #[test]
    fn sse_event_delimiter_prefers_earliest_position() {
        // \r\n\r\n 在 \n\n 之前出现时应取更早的
        let buf = b"a\r\n\r\nb\n\nc";
        assert_eq!(find_sse_event_delimiter(buf), Some((1, 4)));
    }

    #[tokio::test]
    #[serial]
    async fn limit_sse_event_size_passes_normal_events_through() {
        use bytes::Bytes;
        use futures::stream;
        use futures::StreamExt as _;

        let input: SseStream = Box::pin(stream::iter(vec![
            Ok::<Bytes, std::io::Error>(Bytes::from("event: message_start\ndata: {\"t\":1}\n\n")),
            Ok(Bytes::from("event: message_delta\ndata: {\"t\":2}\n\n")),
            Ok(Bytes::from("event: message_stop\ndata: {}\n\n")),
        ]));
        let mut out = limit_sse_event_size(input, 1024);
        let mut collected = String::new();
        while let Some(item) = out.next().await {
            collected.push_str(&String::from_utf8_lossy(&item.unwrap()));
        }
        assert_eq!(collected.matches("\n\n").count(), 3);
        assert!(collected.contains("\"t\":1"));
        assert!(collected.contains("\"t\":2"));
    }

    #[tokio::test]
    #[serial]
    async fn limit_sse_event_size_rejects_oversized_single_event() {
        use bytes::Bytes;
        use futures::stream;
        use futures::StreamExt as _;

        // 单个事件 2KB，上限 1KB → 流报错终止
        let big_data = "x".repeat(2048);
        let input: SseStream = Box::pin(stream::iter(vec![Ok::<Bytes, std::io::Error>(
            Bytes::from(format!("data: {big_data}\n\n")),
        )]));
        let mut out = limit_sse_event_size(input, 1024);
        let first = out.next().await;
        assert!(first.is_some(), "应产生一个错误 item");
        assert!(first.unwrap().is_err(), "超限事件应报错");
        assert!(
            out.next().await.is_none(),
            "报错后流应终止（不产生更多 item）"
        );
    }

    #[tokio::test]
    #[serial]
    async fn limit_sse_event_size_rejects_boundless_accumulation() {
        use bytes::Bytes;
        use futures::stream;
        use futures::StreamExt as _;

        // 事件无分隔符且持续累积超过上限 → 提前终止
        let chunks: Vec<Bytes> = vec![
            Bytes::from("data: chunk1\n"),
            Bytes::from("data: chunk2\n"),
            Bytes::from("data: chunk3\n"),
        ];
        let input: SseStream = Box::pin(stream::iter(
            chunks.into_iter().map(Ok::<Bytes, std::io::Error>),
        ));
        let mut out = limit_sse_event_size(input, 30);
        let mut err_seen = false;
        while let Some(item) = out.next().await {
            if item.is_err() {
                err_seen = true;
                break;
            }
        }
        assert!(err_seen, "无边界累积超过上限应报错");
    }

    #[tokio::test]
    #[serial]
    async fn limit_sse_event_size_allows_event_at_exact_boundary() {
        use bytes::Bytes;
        use futures::stream;
        use futures::StreamExt as _;

        // 事件体恰好 10 字节（含分隔符 12），上限 10 → 允许
        let input: SseStream = Box::pin(stream::iter(vec![Ok::<Bytes, std::io::Error>(
            Bytes::from("0123456789\n\n"),
        )]));
        let mut out = limit_sse_event_size(input, 10);
        let item = out.next().await.unwrap();
        assert!(item.is_ok(), "等于上限的事件应放行");
        assert_eq!(item.unwrap(), Bytes::from("0123456789\n\n"));
    }

    #[tokio::test]
    #[serial]
    async fn collect_body_with_limit_rejects_oversized_body() {
        use axum::body::Body;
        let big = vec![0u8; 4096];
        let body = Body::from(big);
        let result = collect_body_with_limit(body, 1024).await;
        assert!(
            matches!(result, Err(ProxyError::RequestBodyTooLarge(_))),
            "超过上限应返回 RequestBodyTooLarge, got: {result:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn collect_body_with_limit_accepts_body_under_limit() {
        use axum::body::Body;
        let small = b"{\"model\":\"m\"}".to_vec();
        let body = Body::from(small.clone());
        let result = collect_body_with_limit(body, 1024).await;
        assert!(result.is_ok(), "低于上限应成功, got: {result:?}");
        assert_eq!(result.unwrap(), Bytes::from(small));
    }

    #[test]
    fn limits_constants_are_reasonable() {
        // 常量值 sanity check：防止意外改小/改大导致行为漂移
        assert!(crate::proxy::limits::MAX_REQUEST_BODY_BYTES >= 100 * 1024 * 1024);
        assert!(crate::proxy::limits::MAX_RESPONSE_BODY_BYTES >= 100 * 1024 * 1024);
        assert_eq!(
            crate::proxy::limits::MAX_DECOMPRESSED_BODY_BYTES,
            crate::proxy::limits::MAX_RESPONSE_BODY_BYTES
        );
        assert!(crate::proxy::limits::MAX_SSE_EVENT_BYTES >= 4 * 1024 * 1024);
        // 流式单事件上限应远小于响应体上限（事件是细粒度单元）
        assert!(
            crate::proxy::limits::MAX_SSE_EVENT_BYTES
                < crate::proxy::limits::MAX_RESPONSE_BODY_BYTES
        );
    }

    // ------------------------------------------------------------------
    // 超时 / 重试语义（路线图 13）
    // ------------------------------------------------------------------

    #[tokio::test]
    #[serial]
    async fn idle_timeout_guard_terminates_silent_stream() {
        use futures::stream;
        use futures::StreamExt as _;

        // 用一个「永不产出数据」的 pending stream 模拟上游卡死
        let input: SseStream = Box::pin(stream::pending::<Result<Bytes, std::io::Error>>());
        let mut out = limit_sse_idle_timeout(input, 1); // 1s 空闲超时
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), out.next()).await;
        assert!(first.is_ok(), "超时后应产生终止 item");
        assert!(
            first.unwrap().unwrap().is_err(),
            "空闲超时应报错终止（而非无限挂起）"
        );
    }

    #[tokio::test]
    #[serial]
    async fn idle_timeout_guard_passes_active_stream() {
        use bytes::Bytes;
        use futures::stream;
        use futures::StreamExt as _;

        // 持续有数据的流（间隔远小于超时）应全部通过
        let input: SseStream = Box::pin(stream::iter(vec![
            Ok::<Bytes, std::io::Error>(Bytes::from("data: a\n\n")),
            Ok(Bytes::from("data: b\n\n")),
        ]));
        let mut out = limit_sse_idle_timeout(input, 60);
        let mut collected = Vec::new();
        while let Some(item) = out.next().await {
            collected.push(item.unwrap());
        }
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0], Bytes::from("data: a\n\n"));
        assert_eq!(collected[1], Bytes::from("data: b\n\n"));
    }

    #[tokio::test]
    #[serial]
    async fn idle_timeout_guard_ends_normally_on_upstream_close() {
        use futures::stream;
        use futures::StreamExt as _;

        let input: SseStream = Box::pin(stream::iter(Vec::<Result<Bytes, std::io::Error>>::new()));
        let mut out = limit_sse_idle_timeout(input, 60);
        assert!(out.next().await.is_none(), "空流应立即正常结束");
    }

    #[test]
    fn gateway_config_timeout_defaults_are_sane() {
        // 旧配置（无超时字段）反序列化时应获得默认值，而非报错（向后兼容契约）
        let old_json = r#"{"enabled":true,"apiKey":"ccs-old","models":[]}"#;
        let cfg: GatewayConfig = serde_json::from_str(old_json).expect("旧配置应可解析");
        assert_eq!(cfg.non_streaming_timeout_secs, 600);
        assert_eq!(cfg.streaming_first_byte_timeout_secs, 60);
        assert_eq!(cfg.streaming_idle_timeout_secs, 120);
        // 显式填写时应原样保留
        let full_json = r#"{"enabled":true,"apiKey":"ccs-x","models":[],
            "nonStreamingTimeoutSecs":300,"streamingFirstByteTimeoutSecs":30,
            "streamingIdleTimeoutSecs":90}"#;
        let cfg: GatewayConfig = serde_json::from_str(full_json).expect("完整配置应可解析");
        assert_eq!(cfg.non_streaming_timeout_secs, 300);
        assert_eq!(cfg.streaming_first_byte_timeout_secs, 30);
        assert_eq!(cfg.streaming_idle_timeout_secs, 90);
    }

    #[test]
    fn gateway_config_timeout_zero_disables() {
        // 用户显式填 0 表示禁用对应超时，反序列化应保留 0
        let json = r#"{"enabled":true,"apiKey":"ccs-0","models":[],
            "nonStreamingTimeoutSecs":0,"streamingFirstByteTimeoutSecs":0,
            "streamingIdleTimeoutSecs":0}"#;
        let cfg: GatewayConfig = serde_json::from_str(json).expect("应可解析");
        assert_eq!(cfg.non_streaming_timeout_secs, 0);
        assert_eq!(cfg.streaming_first_byte_timeout_secs, 0);
        assert_eq!(cfg.streaming_idle_timeout_secs, 0);
    }

    // ------------------------------------------------------------------
    // 契约测试（路线图第 17 项）：鉴权 / 路由 / 错误契约
    // ------------------------------------------------------------------

    #[tokio::test]
    #[serial]
    async fn key_rotation_invalidates_old_key() {
        let cfg = json!({
            "enabled": true,
            "apiKey": KEY,
            "models": [{
                "alias": "test-provider/claude-test-1",
                "providerId": "prov-1",
                "appType": "claude",
                "model": "claude-test-1"
            }]
        });
        let (router, db) = setup_router_with_db(cfg);

        // 旧 Key 可用
        let req = http::Request::builder()
            .method("GET")
            .uri("/gateway/v1/models")
            .header("authorization", format!("Bearer {KEY}"))
            .body(AxumBody::empty())
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);

        // 轮换 Key：更新配置后旧 Key 立即失效
        let new_cfg = json!({
            "enabled": true,
            "apiKey": "ccs-rotated-key",
            "models": []
        });
        db.set_setting("gateway_config", &new_cfg.to_string())
            .expect("rotate key");
        let req = http::Request::builder()
            .method("GET")
            .uri("/gateway/v1/models")
            .header("authorization", format!("Bearer {KEY}"))
            .body(AxumBody::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
        let body = body_json(resp).await;
        assert_eq!(body["error"]["code"], "invalid_api_key");
    }

    #[tokio::test]
    #[serial]
    async fn core_feature_switch_disabled_returns_403() {
        // 魔改总开关关闭时，即使网关 enabled=true 也返回 403（而非 404 或透传）
        let router = setup_router(true);
        struct ForkGuard;
        impl Drop for ForkGuard {
            fn drop(&mut self) {
                crate::settings::set_fork_features_enabled_for_test(true);
            }
        }
        crate::settings::set_fork_features_enabled_for_test(false);
        let _guard = ForkGuard;
        let resp = router
            .oneshot(post_messages(Some(KEY), "test-provider/claude-test-1"))
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
        let body = body_json(resp).await;
        assert_eq!(body["error"]["code"], "feature_disabled");
    }

    #[tokio::test]
    #[serial]
    async fn duplicate_alias_returns_400() {
        let cfg = json!({
            "enabled": true,
            "apiKey": KEY,
            "models": [
                {
                    "alias": "dup/model",
                    "providerId": "prov-1",
                    "appType": "claude",
                    "model": "claude-test-1"
                },
                {
                    "alias": "dup/model",
                    "providerId": "prov-2",
                    "appType": "claude",
                    "model": "claude-test-2"
                }
            ]
        });
        let router = setup_router_with_cfg(cfg);
        let resp = router
            .oneshot(post_messages(Some(KEY), "dup/model"))
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        let body = body_json(resp).await;
        assert_eq!(body["error"]["code"], "duplicate_alias");
    }

    #[tokio::test]
    #[serial]
    async fn deleted_provider_returns_404() {
        // 默认配置引用 prov-1，但 memory db 中不存在该 provider → 404
        let router = setup_router(true);
        let resp = router
            .oneshot(post_messages(Some(KEY), "test-provider/claude-test-1"))
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
        let body = body_json(resp).await;
        assert_eq!(body["error"]["code"], "provider_not_found");
    }

    #[tokio::test]
    #[serial]
    async fn invalid_json_returns_400() {
        let router = setup_router(true);
        let req = http::Request::builder()
            .method("POST")
            .uri("/gateway/v1/messages")
            .header("authorization", format!("Bearer {KEY}"))
            .header("content-type", "application/json")
            .body(AxumBody::from("this is not json"))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        let body = body_json(resp).await;
        assert_eq!(body["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    #[serial]
    async fn wrong_method_returns_405() {
        let router = setup_router(true);
        let req = http::Request::builder()
            .method("GET")
            .uri("/gateway/v1/messages")
            .header("authorization", format!("Bearer {KEY}"))
            .body(AxumBody::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    #[serial]
    async fn models_endpoint_does_not_leak_provider_key() {
        let cfg = json!({
            "enabled": true,
            "apiKey": KEY,
            "models": [{
                "alias": "test-provider/claude-test-1",
                "providerId": "prov-1",
                "appType": "claude",
                "model": "claude-test-1"
            }]
        });
        let router = setup_router_with_provider(cfg, "prov-1", "claude");
        let req = http::Request::builder()
            .method("GET")
            .uri("/gateway/v1/models")
            .header("authorization", format!("Bearer {KEY}"))
            .body(AxumBody::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), http::StatusCode::OK);
        let body = body_json(resp).await;
        let raw = body.to_string();
        assert!(
            !raw.contains("sk-ant-secret"),
            "models 端点泄露了 provider Key: {raw}"
        );
        assert!(
            !raw.contains("ANTHROPIC_API_KEY"),
            "models 端点泄露了环境变量字段: {raw}"
        );
        // 仍能拿到 alias 与供应商名
        assert_eq!(body["data"][0]["id"], "test-provider/claude-test-1");
        assert_eq!(body["data"][0]["owned_by"], "Test Provider");
    }

    #[tokio::test]
    #[serial]
    async fn error_response_has_stable_code_and_request_id() {
        let router = setup_router(true);
        // 无 Key → 401 missing_api_key + request_id + x-request-id 头
        let resp = router
            .clone()
            .oneshot(post_messages(None, "test-provider/claude-test-1"))
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
        assert!(resp.headers().contains_key("x-request-id"));
        let body = body_json(resp).await;
        assert_eq!(body["error"]["code"], "missing_api_key");
        assert!(
            body["error"]["request_id"]
                .as_str()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            "request_id 缺失"
        );
    }

    #[tokio::test]
    #[serial]
    async fn save_gateway_config_rejects_duplicate_alias() {
        // 配置保存层校验：重复 alias 直接报错
        let cfg = json!({
            "enabled": true,
            "apiKey": KEY,
            "models": [
                {"alias": "a/model", "providerId": "p1", "appType": "claude", "model": "m1"},
                {"alias": "a/model", "providerId": "p2", "appType": "claude", "model": "m2"}
            ]
        });
        let parsed: GatewayConfig = serde_json::from_value(cfg).expect("parse");
        let err = crate::commands::gateway::validate_aliases_unique(&parsed);
        assert!(err.is_err(), "重复 alias 应被拒绝");
        assert!(err.unwrap_err().contains("alias 重复"));
    }
}
