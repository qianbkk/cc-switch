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
    hyper_client::ProxyResponse,
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
use http_body_util::BodyExt;
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

fn auth_error_response(status: StatusCode, msg: &str) -> Response {
    let body = Json(json!({
        "error": {
            "message": msg,
            "type": "invalid_request_error",
        }
    }));
    (status, body).into_response()
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
            "网关配置加载失败",
        ))
    })?;

    // 魔改总开关关闭时，统一网关整体停用（配置保留，重新打开即恢复）。
    if !crate::settings::fork_features_enabled() {
        return Err(Box::new(auth_error_response(
            StatusCode::FORBIDDEN,
            "魔改功能已在设置中整体关闭，统一网关不可用",
        )));
    }

    if !cfg.enabled {
        return Err(Box::new(auth_error_response(
            StatusCode::FORBIDDEN,
            "网关未启用，请在设置中打开「统一网关」开关",
        )));
    }

    let client_key = match extract_client_key(headers) {
        Some(k) if !k.is_empty() => k,
        _ => {
            return Err(Box::new(auth_error_response(
                StatusCode::UNAUTHORIZED,
                "缺少鉴权头（Authorization: Bearer <key> 或 x-api-key: <key>）",
            )));
        }
    };

    if client_key != cfg.api_key {
        return Err(Box::new(auth_error_response(
            StatusCode::UNAUTHORIZED,
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
    auth_error_response(StatusCode::BAD_REQUEST, &format!("alias 未命中；{joined}"))
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
                "读取供应商失败",
            ))
        })?
        .ok_or_else(|| {
            Box::new(auth_error_response(
                StatusCode::NOT_FOUND,
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

/// 构建单 provider 候选的 `RequestForwarder`（与「failover 关闭 +
/// max_retries=0」语义一致：网关只尝试一家、不启用超时与熔断探测）
fn build_forwarder(state: &ProxyState, provider: &Provider) -> RequestForwarder {
    RequestForwarder::new(
        state.provider_router.clone(),
        0, // non_streaming_timeout (disabled)
        state.status.clone(),
        state.current_providers.clone(),
        state.gemini_shadow.clone(),
        state.codex_chat_history.clone(),
        state.failover_manager.clone(),
        state.app_handle.clone(),
        provider.id.clone(),
        String::new(), // session_id (unused for gateway)
        false,         // session_client_provided
        0,             // streaming_first_byte_timeout (disabled)
        0,             // streaming_idle_timeout (disabled)
        RectifierConfig::default(),
        OptimizerConfig::default(),
        CopilotOptimizerConfig::default(),
        0, // max_retries => single attempt
    )
}

/// 实际发起单 provider 转发
async fn forward_with_single_provider(
    state: &ProxyState,
    provider: Provider,
    app_type: AppType,
    endpoint: &str,
    body: Value,
    headers: HeaderMap,
    extensions: Extensions,
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
    let forwarder = build_forwarder(state, &provider);
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
/// streaming 走"上游 SSE 协议 → Anthropic SSE → 入站 SSE"两步链式。已实现的转换路径：
/// - OpenAI Responses → Anthropic SSE → OpenAI Responses（流透传，因为双向同协议）
/// - OpenAI Responses → Anthropic SSE → OpenAI Responses 客户端（流正常）
/// - Gemini → Anthropic SSE → 入站（流正常）
/// - 缺口：Chat→Anthropic SSE、Anthropic→Chat SSE（暂 passthrough + warn）。
async fn proxy_response_to_axum(
    resp: ProxyResponse,
    inbound: InboundProtocol,
    upstream_format: Option<String>,
    is_streaming: bool,
) -> Response {
    let status = resp.status();
    let headers = resp.headers().clone();

    if is_streaming {
        // 流式路径：直接拿 stream，构造链式 SSE 转换。
        let upstream_stream = resp.bytes_stream().map(|item| match item {
            Ok(b) => Ok::<Bytes, std::io::Error>(b),
            Err(e) => Err(e),
        });
        let body_stream = match build_inbound_sse_stream(
            Box::pin(upstream_stream),
            upstream_format.as_deref(),
            inbound,
        ) {
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
    let bytes = match resp.bytes().await {
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
    let body = Json(json!({
        "error": {
            "message": error.to_string(),
            "type": "invalid_request_error",
        }
    }));
    (status, body).into_response()
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

    let body_bytes = match req_body.collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => {
            return error_to_response(ProxyError::Internal(format!("read body: {e}")));
        }
    };
    let mut body: Value = match serde_json::from_slice(&body_bytes) {
        Ok(b) => b,
        Err(e) => {
            return error_to_response(ProxyError::Internal(format!("parse body: {e}")));
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
            return auth_error_response(StatusCode::BAD_REQUEST, "请求体缺少 model 字段");
        }
    };

    let entry = match resolve_alias(&cfg, &model_in_body) {
        Ok(e) => e,
        Err(list) => return alias_not_found_response(list),
    };

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
        &state, provider, app_type, endpoint, body, headers, extensions,
    )
    .await
    {
        Ok((resp, upstream_format, _is_streaming_request)) => {
            proxy_response_to_axum(resp, inbound, upstream_format, is_streaming).await
        }
        Err(e) => error_to_response(e),
    }
}

// =====================================================================
// 冒烟测试：路由级端到端（鉴权 / alias 路由 / 模型列表）
// =====================================================================

#[cfg(test)]
mod tests {
    use crate::database::Database;
    use crate::proxy::server::ProxyServer;
    use crate::proxy::types::ProxyConfig;
    use axum::body::Body as AxumBody;
    use http_body_util::BodyExt as _;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tower::ServiceExt;

    const KEY: &str = "ccs-testkey";

    fn setup_router(enabled: bool) -> axum::Router {
        let db = Arc::new(Database::memory().expect("memory db"));
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
        db.set_setting("gateway_config", &cfg.to_string())
            .expect("save gateway config");
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
    async fn missing_auth_returns_401() {
        let router = setup_router(true);
        let resp = router
            .oneshot(post_messages(None, "test-provider/claude-test-1"))
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
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
    async fn disabled_gateway_returns_403() {
        let router = setup_router(false);
        let resp = router
            .oneshot(post_messages(Some(KEY), "test-provider/claude-test-1"))
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
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
}
