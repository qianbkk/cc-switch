//! 网关与代理统一的大小限制常量（单一事实来源）。
//!
//! 各文件禁止再写魔法数字；需要限制大小的地方统一引用本模块常量。
//! 本模块与 `content_encoding.rs`（解压炸弹防护）配合，覆盖：
//! 请求体、压缩响应体、解压后响应体、流式单事件四类上限。

/// 请求体上限（字节）。对应 server.rs 中 axum 的 `DefaultBodyLimit`，
/// 也是网关/代理手动收集请求体时的硬上限。
pub const MAX_REQUEST_BODY_BYTES: usize = 200 * 1024 * 1024; // 200 MiB

/// 响应体上限（字节，压缩后线上字节数）。
///
/// 透传路径与手动收集路径共用：非流式 JSON 收集与 SSE 透传前的
/// 缓冲区累积都受此约束。历史上即 128 MiB，保持兼容。
pub const MAX_RESPONSE_BODY_BYTES: usize = 128 * 1024 * 1024; // 128 MiB

/// 解压后响应体上限（字节）。
///
/// 压缩炸弹防护：`content_encoding::decompress_body_with_limit` 在输出
/// 超过此值时立即中止。与压缩态上限同值，避免 gzip 压缩比带来的放大。
pub const MAX_DECOMPRESSED_BODY_BYTES: usize = 128 * 1024 * 1024; // 128 MiB

/// 流式 SSE 单事件上限（字节）。
///
/// 单个 SSE 事件（`\n\n` 分隔的一个块）的 data 部分不允许超过此值，
/// 防止异常/恶意上游发送超大单事件导致下游解析器内存膨胀。
/// 正常 LLM 流式事件（文本增量、tool_use、usage）远小于此值。
pub const MAX_SSE_EVENT_BYTES: usize = 16 * 1024 * 1024; // 16 MiB
