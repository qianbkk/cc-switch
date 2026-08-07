//! Live 配置保护模块
//!
//! 防止 CC Switch 在普通供应商切换、保存、同步或代理接管时，静默覆盖用户
//! 在 Claude/Codex/Gemini/Grok Build live 配置文件中的外部修改。
//!
//! 普通写盘基线保存在 `live_managed_state`；代理接管继续使用
//! `proxy_live_backup.managed_hash`。两者必须分离，因为备份行是否存在本身就是
//! “当前由代理接管”的状态信号。

use crate::codex_config::get_codex_config_path;
use crate::config::get_claude_settings_path;
use crate::database::Database;
use crate::error::AppError;
use crate::gemini_config::{get_gemini_env_path, get_gemini_settings_path};
use crate::grok_config::get_grok_config_path;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// settings KV 表中“是否保护用户手动修改的 live 配置”的键名。
pub const PROTECT_USER_LIVE_EDITS_KEY: &str = "protect_user_live_edits";

/// 会覆盖受管 Live 文件的写盘原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveWriteReason {
    ProviderSwitch,
    ProviderSave,
    ProviderSync,
    ProxyRestore,
    InternalMigration,
}

impl LiveWriteReason {
    fn checks_for_external_modification(self) -> bool {
        matches!(
            self,
            Self::ProviderSwitch | Self::ProviderSave | Self::ProviderSync
        )
    }

    fn records_normal_managed_state(self) -> bool {
        matches!(
            self,
            Self::ProviderSwitch
                | Self::ProviderSave
                | Self::ProviderSync
                | Self::ProxyRestore
                | Self::InternalMigration
        )
    }
}

/// 返回受保护的 live 文件集合。
///
/// Gemini 同时管理 `.env` 与 `settings.json`，因此两者作为一个组合指纹保护；
/// 其他应用各保护一个主配置文件。未覆盖的应用返回空集合。
pub fn live_file_paths(app_type: &str) -> Vec<PathBuf> {
    match app_type {
        "claude" => vec![get_claude_settings_path()],
        "codex" => vec![get_codex_config_path()],
        "gemini" => vec![get_gemini_env_path(), get_gemini_settings_path()],
        "grokbuild" => vec![get_grok_config_path()],
        _ => Vec::new(),
    }
}

/// 兼容旧调用方：返回该应用的主 live 文件路径。
pub fn live_file_path(app_type: &str) -> Option<PathBuf> {
    live_file_paths(app_type).into_iter().next()
}

/// 计算文件的 SHA256 十六进制摘要。
///
/// 文件不存在或读取失败返回 `None`。
pub fn compute_file_hash(path: &Path) -> Option<String> {
    match std::fs::read(path) {
        Ok(bytes) => Some(hash_bytes(&bytes)),
        Err(_) => None,
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn compute_paths_fingerprint(paths: &[PathBuf]) -> Result<String, AppError> {
    let mut hasher = Sha256::new();
    for path in paths {
        let path_label = path.to_string_lossy();
        hasher.update((path_label.len() as u64).to_le_bytes());
        hasher.update(path_label.as_bytes());
        match std::fs::read(path) {
            Ok(bytes) => {
                hasher.update([1]);
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update(&bytes);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update([0]);
            }
            Err(error) => return Err(AppError::io(path, error)),
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn protected_files_display(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

/// 记录代理接管路径最近一次由 CC Switch 写入主 live 文件后的 hash。
pub async fn record_managed_hash(db: &Database, app_type: &str) -> Result<(), AppError> {
    let hash = live_file_path(app_type).and_then(|path| compute_file_hash(&path));
    db.set_live_managed_hash(app_type, hash.as_deref()).await
}

/// 读取指定字符串内容的 SHA256 十六进制摘要（仅在测试中使用）。
#[cfg(test)]
pub fn hash_content(content: &str) -> String {
    hash_bytes(content.as_bytes())
}

/// 读取 `settings` 表中 `protect_user_live_edits` 的值。
pub fn get_protect_user_live_edits(db: &Database) -> bool {
    if !crate::settings::fork_features_enabled() {
        return false;
    }
    match db.get_setting(PROTECT_USER_LIVE_EDITS_KEY) {
        Ok(Some(value)) => value == "true" || value == "1",
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

/// 检查代理接管路径中的用户外部修改。
///
/// 接管备份存在时以 `managed_hash`（回退 `original_hash`）为基线；没有备份时，
/// 再回退到普通写盘基线，使“普通切换后手改 → 首次接管”也不会被静默覆盖。
pub async fn check_user_modified(db: &Database, app_type: &str) -> Result<(), AppError> {
    let paths = live_file_paths(app_type);
    if paths.is_empty() || !get_protect_user_live_edits(db) {
        return Ok(());
    }

    let backup_expected = match db.get_live_backup(app_type).await {
        Ok(Some(backup)) => expected_live_hash(
            backup.managed_hash.as_deref(),
            backup.original_hash.as_deref(),
        )
        .map(str::to_string),
        Ok(None) => None,
        Err(error) => {
            log::warn!("读取 {app_type} live 备份失败，尝试普通托管基线: {error}");
            None
        }
    };
    let using_takeover_baseline = backup_expected.is_some();
    let expected = match backup_expected {
        Some(hash) => Some(hash),
        None => db.get_live_managed_state_hash(app_type)?,
    };
    let Some(expected) = expected.filter(|hash| !hash.trim().is_empty()) else {
        return Ok(());
    };

    // 旧接管记录只保存主文件 hash；普通托管状态保存所有受管文件的组合指纹。
    let actual = if using_takeover_baseline {
        compute_file_hash(&paths[0])
    } else {
        Some(compute_paths_fingerprint(&paths)?)
    };
    if live_hash_was_modified(Some(expected.trim()), actual.as_deref()) {
        Err(AppError::LiveConfigModifiedByUser {
            app_type: app_type.to_string(),
            path: protected_files_display(&paths),
            expected_hash: Some(expected),
            actual_hash: actual,
        })
    } else {
        Ok(())
    }
}

/// 普通切换/保存/同步路径的统一受保护写盘入口。
///
/// 只有写盘成功后才更新托管指纹；写盘失败时基线保持不变。恢复与内部迁移可通过
/// `LiveWriteReason` 显式绕过普通冲突检测，但仍不会伪造成功基线。
pub fn record_normal_managed_state(db: &Database, app_type: &str) -> Result<(), AppError> {
    let paths = live_file_paths(app_type);
    if paths.is_empty() {
        return Ok(());
    }
    let managed_hash = compute_paths_fingerprint(&paths)?;
    db.set_live_managed_state_hash(app_type, &managed_hash)
}

/// 用户明确确认覆盖后，接受当前磁盘内容为新的冲突基线。
///
/// 这不是关闭保护：调用方重试写盘时会重新计算磁盘指纹；若确认后文件再次变化，
/// 仍会再次触发冲突。代理接管已有备份时，同时更新其主文件基线。
pub async fn accept_current_live_state(db: &Database, app_type: &str) -> Result<(), AppError> {
    let paths = live_file_paths(app_type);
    if paths.is_empty() {
        return Err(AppError::InvalidInput(format!(
            "不支持的 Live 配置类型: {app_type}"
        )));
    }

    let managed_fingerprint = compute_paths_fingerprint(&paths)?;
    if db.get_live_backup(app_type).await?.is_some() {
        let primary_hash = compute_file_hash(&paths[0]);
        db.set_live_managed_hash(app_type, primary_hash.as_deref())
            .await?;
    }
    db.set_live_managed_state_hash(app_type, &managed_fingerprint)
}

pub fn protected_live_write<T>(
    db: &Database,
    app_type: &str,
    reason: LiveWriteReason,
    write: impl FnOnce() -> Result<T, AppError>,
) -> Result<T, AppError> {
    let paths = live_file_paths(app_type);
    if reason.checks_for_external_modification()
        && reason.records_normal_managed_state()
        && !paths.is_empty()
        && get_protect_user_live_edits(db)
    {
        if let Some(expected) = db.get_live_managed_state_hash(app_type)? {
            let actual = compute_paths_fingerprint(&paths)?;
            if expected.trim() != actual {
                return Err(AppError::LiveConfigModifiedByUser {
                    app_type: app_type.to_string(),
                    path: protected_files_display(&paths),
                    expected_hash: Some(expected),
                    actual_hash: Some(actual),
                });
            }
        }
    }

    let result = write()?;

    if reason.records_normal_managed_state() && !paths.is_empty() {
        record_normal_managed_state(db, app_type)?;
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use serial_test::serial;
    use std::env;
    use tempfile::TempDir;

    struct TempHome {
        _dir: TempDir,
        previous: Option<std::ffi::OsString>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("create temp home");
            let previous = env::var_os("CC_SWITCH_TEST_HOME");
            env::set_var("CC_SWITCH_TEST_HOME", dir.path());
            Self {
                _dir: dir,
                previous,
            }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => env::set_var("CC_SWITCH_TEST_HOME", value),
                None => env::remove_var("CC_SWITCH_TEST_HOME"),
            }
        }
    }

    #[test]
    fn hash_content_is_stable() {
        let h1 = hash_content("hello world");
        let h2 = hash_content("hello world");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn compute_file_hash_returns_none_for_missing() {
        let p = std::env::temp_dir().join("cc-switch-live-protect-missing-file");
        let _ = std::fs::remove_file(&p);
        assert!(compute_file_hash(&p).is_none());
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
    fn missing_or_different_actual_hash_is_modified() {
        assert!(live_hash_was_modified(Some("expected"), None));
        assert!(live_hash_was_modified(Some("expected"), Some("different")));
        assert!(!live_hash_was_modified(Some("expected"), Some("expected")));
    }

    #[test]
    #[serial]
    fn protected_write_records_baseline_only_after_success() {
        let _home = TempHome::new();
        let db = Database::memory().expect("database");
        let path = get_claude_settings_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create config dir");

        let error = protected_live_write(&db, "claude", LiveWriteReason::ProviderSave, || {
            Err::<(), _>(AppError::Message("write failed".to_string()))
        })
        .expect_err("failed write must stay failed");
        assert!(error.to_string().contains("write failed"));
        assert_eq!(
            db.get_live_managed_state_hash("claude")
                .expect("read state"),
            None
        );

        protected_live_write(&db, "claude", LiveWriteReason::ProviderSave, || {
            std::fs::write(&path, "{\"managed\":1}").map_err(|error| AppError::io(&path, error))?;
            Ok(())
        })
        .expect("successful write");
        assert!(db
            .get_live_managed_state_hash("claude")
            .expect("read state")
            .is_some());
    }

    #[test]
    #[serial]
    fn protected_write_rejects_external_modification() {
        let _home = TempHome::new();
        let db = Database::memory().expect("database");
        let path = get_claude_settings_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create config dir");

        protected_live_write(&db, "claude", LiveWriteReason::ProviderSwitch, || {
            std::fs::write(&path, "{\"managed\":1}").map_err(|error| AppError::io(&path, error))?;
            Ok(())
        })
        .expect("initial managed write");
        std::fs::write(&path, "{\"user\":2}").expect("external edit");

        let error = protected_live_write(&db, "claude", LiveWriteReason::ProviderSwitch, || {
            std::fs::write(&path, "{\"managed\":3}").map_err(|error| AppError::io(&path, error))?;
            Ok(())
        })
        .expect_err("external edit must block overwrite");
        assert!(matches!(error, AppError::LiveConfigModifiedByUser { .. }));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read live"),
            "{\"user\":2}"
        );
    }

    #[test]
    #[serial]
    fn accepting_current_state_allows_one_retry_but_not_later_edits() {
        let _home = TempHome::new();
        let db = Database::memory().expect("database");
        let path = get_claude_settings_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create config dir");

        protected_live_write(&db, "claude", LiveWriteReason::ProviderSwitch, || {
            std::fs::write(&path, "{\"managed\":1}").map_err(|error| AppError::io(&path, error))?;
            Ok(())
        })
        .expect("initial managed write");
        std::fs::write(&path, "{\"user\":2}").expect("external edit");

        futures::executor::block_on(accept_current_live_state(&db, "claude"))
            .expect("accept current state");
        protected_live_write(&db, "claude", LiveWriteReason::ProviderSwitch, || {
            std::fs::write(&path, "{\"managed\":3}").map_err(|error| AppError::io(&path, error))?;
            Ok(())
        })
        .expect("confirmed retry");

        std::fs::write(&path, "{\"user\":4}").expect("later external edit");
        let error = protected_live_write(&db, "claude", LiveWriteReason::ProviderSwitch, || Ok(()))
            .expect_err("later edit must be protected again");
        assert!(matches!(error, AppError::LiveConfigModifiedByUser { .. }));
    }

    #[test]
    #[serial]
    fn codex_and_grokbuild_reject_external_modification() {
        let _home = TempHome::new();
        let db = Database::memory().expect("database");

        for (app_type, path) in [
            ("codex", get_codex_config_path()),
            ("grokbuild", get_grok_config_path()),
        ] {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("create config dir");
            protected_live_write(&db, app_type, LiveWriteReason::ProviderSync, || {
                std::fs::write(&path, "managed = true\n")
                    .map_err(|error| AppError::io(&path, error))?;
                Ok(())
            })
            .expect("initial managed write");

            std::fs::write(&path, "user_edit = true\n").expect("external edit");
            let error =
                protected_live_write(&db, app_type, LiveWriteReason::ProviderSave, || Ok(()))
                    .expect_err("external edit must be protected");
            assert!(matches!(error, AppError::LiveConfigModifiedByUser { .. }));
            assert_eq!(
                std::fs::read_to_string(&path).expect("read live"),
                "user_edit = true\n"
            );
        }
    }

    #[test]
    #[serial]
    fn gemini_fingerprint_covers_env_and_settings() {
        let _home = TempHome::new();
        let db = Database::memory().expect("database");
        let env_path = get_gemini_env_path();
        let settings_path = get_gemini_settings_path();
        std::fs::create_dir_all(env_path.parent().expect("parent")).expect("create config dir");

        protected_live_write(&db, "gemini", LiveWriteReason::ProviderSave, || {
            std::fs::write(&env_path, "GEMINI_API_KEY=managed")
                .map_err(|error| AppError::io(&env_path, error))?;
            std::fs::write(&settings_path, "{\"theme\":\"dark\"}")
                .map_err(|error| AppError::io(&settings_path, error))?;
            Ok(())
        })
        .expect("managed Gemini write");

        std::fs::write(&settings_path, "{\"theme\":\"light\"}").expect("external settings edit");
        let error = protected_live_write(&db, "gemini", LiveWriteReason::ProviderSave, || Ok(()))
            .expect_err("settings.json edit must be protected");
        assert!(matches!(error, AppError::LiveConfigModifiedByUser { .. }));
    }
}
