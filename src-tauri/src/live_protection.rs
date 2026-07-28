//! Live 配置保护模块
//!
//! 防止 ccswitch 在用户手动修改 Claude/Codex/Gemini/Grok Build 的 live 配置文件后，
//! 接管写盘或供应商切换时覆盖用户的改动。
//!
//! ## 工作原理
//! 1. 每次接管（takeover）前，备份记录保存 `original_hash`
//!    （接管前对原始 live 文件算 SHA256）。
//! 2. 每次 CC Switch 写盘成功后保存 `managed_hash`。
//! 3. 写盘前用 [`check_user_modified`] 比对磁盘当前 hash 与最近的应用写入 hash：
//!    - 若一致：用户未修改 live，安全覆盖；
//!    - 若不一致：返回 `AppError::LiveConfigModifiedByUser`，**不写盘**，
//!      错误经由 command 层回到前端。
//! 4. 设置开关 `protect_user_live_edits`（默认 true）允许用户整体关闭校验。
//!
//! 详见 `docs/CUSTOM_FORK_PLAN.md` §7.1。

use crate::codex_config::get_codex_config_path;
use crate::config::get_claude_settings_path;
use crate::database::Database;
use crate::error::AppError;
use crate::gemini_config::{get_gemini_env_path, get_gemini_settings_path};
use crate::grok_config::get_grok_config_path;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// settings KV 表中"是否保护用户手动修改的 live 配置"的键名。
///
/// 默认开启（见 `Database::get_protect_user_live_edits`）。
pub const PROTECT_USER_LIVE_EDITS_KEY: &str = "protect_user_live_edits";

/// 把任意应用类型字符串映射到对应的 live 配置文件路径。
///
/// 仅覆盖 Claude/Codex/Gemini/Grok Build 四种应用；其他应用返回 `None`，
/// 让上层直接通过（保护机制不适用）。
///
/// 注意：Codex 真正落盘的入口是 `write_codex_live_atomic`（auth.json + config.toml），
/// 但用户手工编辑的目标是 `config.toml`，所以这里 hash 的就是这个 TOML 文件。
/// `auth.json` 由 Codex 进程自身维护，不在本保护范围内。
pub fn live_file_path(app_type: &str) -> Option<PathBuf> {
    match app_type {
        "claude" => Some(get_claude_settings_path()),
        "codex" => Some(get_codex_config_path()),
        "gemini" => {
            // Gemini 真实存储在 `.env` 文件，但用户手工编辑也主要是这个；
            // settings.json（get_gemini_settings_path）由 ccswitch 全量重建，
            // 且接管流程不直接写入，故选更稳定的 `.env` 作为保护目标。
            let env_path = get_gemini_env_path();
            if env_path.exists() {
                Some(env_path)
            } else {
                Some(get_gemini_settings_path())
            }
        }
        "grokbuild" => Some(get_grok_config_path()),
        _ => None,
    }
}

/// 计算文件的 SHA256 十六进制摘要。
///
/// 文件不存在返回 `None`；读取失败返回 `None`（避免让 IO 抖动阻塞接管流程）。
pub fn compute_file_hash(path: &Path) -> Option<String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            Some(format!("{:x}", hasher.finalize()))
        }
        Err(_) => None,
    }
}

/// 记录最近一次由 CC Switch 写入 live 文件后的 hash。
pub async fn record_managed_hash(db: &Database, app_type: &str) -> Result<(), AppError> {
    let hash = live_file_path(app_type).and_then(|path| compute_file_hash(&path));
    db.set_live_managed_hash(app_type, hash.as_deref()).await
}

/// 读取指定字符串内容的 SHA256 十六进制摘要（仅在测试中使用）。
#[cfg(test)]
pub fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 读取 `settings` 表中 `protect_user_live_edits` 的值。
///
/// 缺省视为 `true`（保护默认开启，符合 §7.1 的产品决策）。
pub fn get_protect_user_live_edits(db: &Database) -> bool {
    // 魔改总开关关闭时，Live 保护整体让路给上游原版行为（直接覆盖写盘）。
    if !crate::settings::fork_features_enabled() {
        return false;
    }
    match db.get_setting(PROTECT_USER_LIVE_EDITS_KEY) {
        Ok(Some(value)) => value == "true" || value == "1",
        // 缺省视为 true：保护默认开启；只有用户显式设置 "false"/"0" 才关闭。
        _ => true,
    }
}

/// 写入 `settings` 表中 `protect_user_live_edits` 的值。
pub fn set_protect_user_live_edits(db: &Database, enabled: bool) -> Result<(), AppError> {
    db.set_setting(
        PROTECT_USER_LIVE_EDITS_KEY,
        if enabled { "true" } else { "false" },
    )
}

fn expected_live_hash<'a>(
    managed_hash: Option<&'a str>,
    original_hash: Option<&'a str>,
) -> Option<&'a str> {
    managed_hash
        .filter(|hash| !hash.trim().is_empty())
        .or_else(|| original_hash.filter(|hash| !hash.trim().is_empty()))
        .map(str::trim)
}

fn live_hash_was_modified(expected: Option<&str>, actual: Option<&str>) -> bool {
    expected.is_some_and(|expected| actual != Some(expected))
}

/// 检查用户是否在接管后修改了指定应用的 live 文件。
///
/// 规则：
/// 1. `app_type` 不在保护范围内（live 文件路径不存在）→ `Ok(())` 跳过；
/// 2. 设置开关关闭 → `Ok(())` 跳过；
/// 3. 没有备份记录（即 ccswitch 此前从未接管过该应用）→ `Ok(())` 跳过
///    （首次接管无"用户原始版"可对照）；
/// 4. `managed_hash` 为空时回退到 `original_hash`（兼容升级前的备份）；
/// 5. 两个 hash 都为空（迁移期老数据）→ `Ok(())` 跳过；
/// 6. 磁盘当前 hash 与期望 hash 一致 → `Ok(())` 通过；
/// 7. 否则 → `Err(AppError::LiveConfigModifiedByUser { ... })`，
///    **调用方必须不写盘并把错误回传前端**。
pub async fn check_user_modified(db: &Database, app_type: &str) -> Result<(), AppError> {
    let Some(path) = live_file_path(app_type) else {
        return Ok(());
    };

    if !get_protect_user_live_edits(db) {
        return Ok(());
    }

    let backup = match db.get_live_backup(app_type).await {
        Ok(Some(b)) => b,
        // 没备份 = 之前没接管过，本次是首次接管，没有"原版"可比对。
        Ok(None) => return Ok(()),
        Err(e) => {
            log::warn!("读取 {app_type} live 备份失败，跳过保护校验: {e}");
            return Ok(());
        }
    };

    let expected = expected_live_hash(
        backup.managed_hash.as_deref(),
        backup.original_hash.as_deref(),
    );
    if expected.is_none() {
        // 迁移期老数据：备份无 hash。无法校验，放行（保持历史行为）。
        return Ok(());
    }

    let actual = compute_file_hash(&path);
    if live_hash_was_modified(expected, actual.as_deref()) {
        Err(AppError::LiveConfigModifiedByUser {
            app_type: app_type.to_string(),
            path: path.display().to_string(),
            expected_hash: expected.map(str::to_string),
            actual_hash: actual,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn hash_content_is_stable() {
        let h1 = hash_content("hello world");
        let h2 = hash_content("hello world");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA256 hex
    }

    #[test]
    fn compute_file_hash_returns_none_for_missing() {
        let p = std::env::temp_dir().join("cc-switch-live-protect-missing-file");
        let _ = std::fs::remove_file(&p);
        assert!(compute_file_hash(&p).is_none());
    }

    #[test]
    fn compute_file_hash_matches_content_hash() {
        let dir = std::env::temp_dir().join("cc-switch-live-protect-roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("settings.json");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(b"{\"x\":1}").unwrap();
        let h_file = compute_file_hash(&p).unwrap();
        let h_str = hash_content("{\"x\":1}");
        assert_eq!(h_file, h_str);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn managed_hash_takes_precedence_over_original_hash() {
        assert_eq!(
            expected_live_hash(Some(" managed "), Some("original")),
            Some("managed")
        );
    }

    #[test]
    fn empty_managed_hash_falls_back_to_original_hash() {
        assert_eq!(
            expected_live_hash(Some("  "), Some(" original ")),
            Some("original")
        );
    }

    #[test]
    fn missing_expected_hash_allows_legacy_backup() {
        assert_eq!(expected_live_hash(None, Some("  ")), None);
        assert!(!live_hash_was_modified(None, Some("actual")));
    }

    #[test]
    fn missing_or_different_actual_hash_is_modified() {
        assert!(live_hash_was_modified(Some("expected"), None));
        assert!(live_hash_was_modified(Some("expected"), Some("different")));
        assert!(!live_hash_was_modified(Some("expected"), Some("expected")));
    }
}
