//! Codex `auth.json` 反向同步
//!
//! ## 背景
//! Codex CLI 0.14+ 启动时会自动写回 `~/.codex/auth.json` 的 `OPENAI_API_KEY`，
//! 经常保留某个陈年旧值。这会让 `EditProviderDialog` 通过 `getLiveProviderSettings`
//! 读到陈年值而非 DB 真值（CC Switch 上游 Bug #3646），导致用户编辑保存后
//! 看似"key 没改"。
//!
//! ## 策略
//! 每次 `ProviderService::update` / `switch` 保存或切换 Codex provider 时，本模块
//! 主动把 `auth.json` 里的 `OPENAI_API_KEY` 字段同步成 DB 真值。Codex CLI 再写回
//! 陈年值也无所谓——下次保存会再次纠正。
//!
//! ## 保留字段
//! - `auth_mode`（OpenAI Official 用 OAuth 时是 `chatgpt`，绝不能动）
//! - `tokens`（ChatGPT OAuth 的 id_token / access_token / refresh_token / account_id）
//! - 其他任何用户/Codex 写入的字段
//!
//! ## 失败降级
//! 读 / 写 / 解析任何一步失败都降级为 `log::warn!`，**不阻塞上层保存流程**。

use crate::codex_config::get_codex_auth_path;
use crate::error::AppError;
use log::{debug, info, warn};
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

const OPENAI_API_KEY_FIELD: &str = "OPENAI_API_KEY";

/// 主动把 `~/.codex/auth.json` 的 `OPENAI_API_KEY` 字段同步成 `db_key`。
///
/// - 保留 OAuth tokens / auth_mode 等其他字段
/// - 已匹配则 no-op（避免无谓 IO）
/// - 原子写（先 temp 再 rename）
/// - 解析/读/写失败 → 返回 `Err`，调用方应降级为 warn
///
/// 同步函数——纯文件 IO，无 async 必要。
pub fn sync_codex_auth_with_db(db_key: &str) -> Result<(), AppError> {
    let auth_path = get_codex_auth_path();
    sync_codex_auth_at_path(&auth_path, db_key)
}

/// 高层便捷入口：`ProviderService::update` / `switch` 直接调用此函数。
///
/// - 仅当 `app_type == Codex` 且 provider 有非空 `auth.OPENAI_API_KEY` 时才同步
/// - 失败降级为 `log::warn!`，**绝不返回错误**（保存/切换流程不因本模块失败）
pub fn maybe_sync_codex_auth(
    app_type: crate::app_config::AppType,
    provider: &crate::provider::Provider,
) {
    use crate::app_config::AppType;

    if !matches!(app_type, AppType::Codex) {
        return;
    }
    // 用户在 auth.json 手填的官方 key 一经切换不覆盖
    if crate::settings::preserve_codex_official_auth_on_switch() {
        return;
    }
    let Some(key) = provider
        .settings_config
        .get("auth")
        .and_then(|a| a.get("OPENAI_API_KEY"))
        .and_then(|v| v.as_str())
    else {
        return;
    };
    if key.is_empty() {
        return;
    }
    if let Err(e) = sync_codex_auth_with_db(key) {
        log::warn!("[CodexAuthSync] auth.json 同步失败（保存流程仍继续，不影响 DB）: {e}");
    }
}

/// 同上，但允许注入路径（用于测试）
pub fn sync_codex_auth_at_path(auth_path: &Path, db_key: &str) -> Result<(), AppError> {
    // 1. 读 auth.json（不存在或空 → 用空对象）
    let mut data: Value = if auth_path.exists() {
        match fs::read_to_string(auth_path) {
            Ok(text) if !text.trim().is_empty() => match serde_json::from_str::<Value>(&text) {
                Ok(v) if v.is_object() => v,
                Ok(_) => {
                    warn!(
                        "[CodexAuthSync] auth.json 顶层不是 object，使用空对象覆盖（原始内容丢失）: {}",
                        auth_path.display()
                    );
                    Value::Object(Map::new())
                }
                Err(e) => {
                    warn!(
                        "[CodexAuthSync] auth.json 解析失败（{}），使用空对象覆盖: {}",
                        e,
                        auth_path.display()
                    );
                    Value::Object(Map::new())
                }
            },
            Ok(_) => Value::Object(Map::new()), // 空文件
            Err(e) => return Err(AppError::io(auth_path, e)),
        }
    } else {
        Value::Object(Map::new())
    };

    // 2. 检查是否已匹配（no-op 优化）
    let current_key = data
        .get(OPENAI_API_KEY_FIELD)
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if current_key == db_key {
        debug!("[CodexAuthSync] auth.json OPENAI_API_KEY 已匹配 DB，跳过写盘");
        return Ok(());
    }

    // 3. 仅替换 OPENAI_API_KEY 字段，保留其他所有字段
    let obj = data
        .as_object_mut()
        .expect("[CodexAuthSync] data was just checked to be object");
    obj.insert(
        OPENAI_API_KEY_FIELD.to_string(),
        Value::String(db_key.to_string()),
    );

    // 4. 原子写：先写临时文件再 rename
    let parent = auth_path.parent().ok_or_else(|| {
        AppError::Message(format!(
            "auth.json path has no parent: {}",
            auth_path.display()
        ))
    })?;
    if !parent.exists() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    let file_name = auth_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("auth.json");
    // 加 pid 避免同一进程的两次同步并发；加时间戳让重试有唯一名
    let temp_name = format!(
        ".{}.tmp.{}.{}",
        file_name,
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let temp_path = parent.join(&temp_name);

    let json_text = serde_json::to_string_pretty(&data)
        .map_err(|e| AppError::Message(format!("[CodexAuthSync] serialize failed: {e}")))?;
    fs::write(&temp_path, json_text.as_bytes()).map_err(|e| AppError::io(&temp_path, e))?;
    fs::rename(&temp_path, auth_path).map_err(|e| AppError::io(auth_path, e))?;

    info!("[CodexAuthSync] auth.json OPENAI_API_KEY 已同步为 DB（避免 Bug #3646）");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        // 跨平台临时目录 + pid + nanos 避免并行测试冲突
        let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let mut p = env::temp_dir();
        p.push(format!(
            "cc-switch-codex-auth-{}-{}-{}",
            name,
            std::process::id(),
            nanos
        ));
        p
    }

    fn cleanup(path: &Path) {
        let _ = fs::remove_file(path);
    }

    #[test]
    fn creates_when_missing() {
        let path = tmp_path("create");
        cleanup(&path);
        assert!(!path.exists());

        sync_codex_auth_at_path(&path, "sk-newkey123").unwrap();
        assert!(path.exists());

        let data: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(data["OPENAI_API_KEY"], "sk-newkey123");
        cleanup(&path);
    }

    #[test]
    fn preserves_oauth_tokens() {
        let path = tmp_path("tokens");
        let original = r#"{
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {
                "access_token": "AT_TOKEN",
                "refresh_token": "RT_TOKEN",
                "account_id": "ACC_ID"
            },
            "last_refresh": "2026-07-18T00:00:00Z"
        }"#;
        fs::write(&path, original).unwrap();

        sync_codex_auth_at_path(&path, "sk-newkey123").unwrap();

        let data: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(data["OPENAI_API_KEY"], "sk-newkey123");
        // OAuth 字段全部保留
        assert_eq!(data["auth_mode"], "chatgpt");
        assert_eq!(data["tokens"]["access_token"], "AT_TOKEN");
        assert_eq!(data["tokens"]["refresh_token"], "RT_TOKEN");
        assert_eq!(data["tokens"]["account_id"], "ACC_ID");
        assert_eq!(data["last_refresh"], "2026-07-18T00:00:00Z");
        cleanup(&path);
    }

    #[test]
    fn noop_when_key_matches() {
        let path = tmp_path("noop");
        let original = r#"{"OPENAI_API_KEY": "sk-samekey", "tokens": {"a": "b"}}"#;
        fs::write(&path, original).unwrap();

        let before = fs::read(&path).unwrap();
        let mtime_before = fs::metadata(&path).unwrap().modified().unwrap();
        // 等一小段时间确保 mtime 会变化（如果写盘发生）
        std::thread::sleep(std::time::Duration::from_millis(50));

        sync_codex_auth_at_path(&path, "sk-samekey").unwrap();

        let after = fs::read(&path).unwrap();
        let mtime_after = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after, "file content must not change");
        assert_eq!(
            mtime_before, mtime_after,
            "file mtime must not change (no write happened)"
        );
        cleanup(&path);
    }

    #[test]
    fn overwrites_stale_key() {
        let path = tmp_path("stale");
        fs::write(
            &path,
            r#"{"OPENAI_API_KEY": "sk-old", "tokens": {"a": "b"}}"#,
        )
        .unwrap();

        sync_codex_auth_at_path(&path, "sk-new").unwrap();

        let data: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(data["OPENAI_API_KEY"], "sk-new");
        assert_eq!(data["tokens"]["a"], "b", "其他字段必须保留");
        cleanup(&path);
    }

    #[test]
    fn handles_malformed_json() {
        let path = tmp_path("malformed");
        fs::write(&path, "this is not json { broken").unwrap();

        // 不 panic, 降级为警告，写盘
        sync_codex_auth_at_path(&path, "sk-new").unwrap();

        let data: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(data["OPENAI_API_KEY"], "sk-new");
        cleanup(&path);
    }

    #[test]
    fn handles_empty_file() {
        let path = tmp_path("empty");
        fs::write(&path, "").unwrap();

        sync_codex_auth_at_path(&path, "sk-new").unwrap();

        let data: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(data["OPENAI_API_KEY"], "sk-new");
        cleanup(&path);
    }

    #[test]
    fn handles_top_level_array() {
        // 非预期的顶层类型（有人手残写了 [...] 当 auth.json）也要安全降级
        let path = tmp_path("array");
        fs::write(&path, "[1, 2, 3]").unwrap();

        sync_codex_auth_at_path(&path, "sk-new").unwrap();

        let data: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(data.is_object(), "顶层被规范为 object");
        assert_eq!(data["OPENAI_API_KEY"], "sk-new");
        cleanup(&path);
    }

    #[test]
    fn preserves_arbitrary_custom_fields() {
        // 防御未来 Codex 新增字段
        let path = tmp_path("custom");
        let original = r#"{
            "auth_mode": "apikey",
            "OPENAI_API_KEY": "old",
            "custom_field": "must_remain",
            "nested": {"a": 1, "b": [2, 3]},
            "future_field_v999": {"x": true}
        }"#;
        fs::write(&path, original).unwrap();

        sync_codex_auth_at_path(&path, "sk-new").unwrap();

        let data: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(data["OPENAI_API_KEY"], "sk-new");
        assert_eq!(data["auth_mode"], "apikey");
        assert_eq!(data["custom_field"], "must_remain");
        assert_eq!(data["nested"]["a"], 1);
        assert_eq!(data["nested"]["b"][1], 3);
        assert_eq!(data["future_field_v999"]["x"], true);
        cleanup(&path);
    }
}
