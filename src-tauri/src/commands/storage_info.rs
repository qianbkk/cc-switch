//! 数据存储信息命令
//!
//! 只返回**元数据**（路径 / 用途 / 大小 / 记录数 / 类别概览），
//! **绝不读取文件内容**，因此不会泄露任何 API Key、Token、OAuth 凭据等敏感值。
//! 兼容路径不存在、无权限读取、数据库损坏等异常场景：单个条目失败只在该条目上
//! 记录 `error`，不影响整体返回。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::config::get_app_config_dir;

/// 需要统计行数的数据表（不含 key 等敏感内容，仅行数）
const DATA_TABLES: &[&str] = &[
    "providers",
    "provider_endpoints",
    "mcp_servers",
    "prompts",
    "skills",
    "skill_repos",
    "settings",
    "proxy_config",
    "provider_health",
    "proxy_request_logs",
    "model_pricing",
    "stream_check_logs",
    "proxy_live_backup",
    "usage_daily_rollups",
    "session_log_sync",
    "profiles",
];

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StorageItem {
    /// 绝对路径
    pub path: String,
    /// 条目名称（文件/目录名）
    pub name: String,
    /// "file" | "dir"
    pub kind: String,
    /// 类别 key：database / config / settings / backups / logs / skills / other
    pub purpose: String,
    /// 该路径是否存在于磁盘
    pub exists: bool,
    /// 大小（字节）；目录为递归合计。不存在或无法读取时为 null。
    pub size_bytes: Option<u64>,
    /// 记录数（仅 database 条目为各表行数合计；目录条目为文件数）。不可用时为 null。
    pub record_count: Option<u64>,
    /// 读取失败时的错误描述（不存在 / 无权限 / 损坏等），正常时为 null。
    pub error: Option<String>,
    /// 数据库 schema 版本（`PRAGMA user_version`）；仅 database 条目且可读时非 null。
    pub schema_version: Option<i32>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    /// 应用数据根目录
    pub base_dir: String,
    /// 所有条目大小合计（字节）
    pub total_size_bytes: u64,
    pub items: Vec<StorageItem>,
    /// 数据库当前 schema 版本（不可读/不存在时为 null）
    pub db_schema_version: Option<i32>,
    /// 最近一次数据库备份的文件名（backups 目录中最新 `db_backup_*.db`），无备份时为 null
    pub latest_db_backup: Option<String>,
}

fn purpose_label(kind: &str) -> String {
    match kind {
        "database" => "database",
        "config" => "config",
        "settings" => "settings",
        "backups" => "backups",
        "logs" => "logs",
        "skills" => "skills",
        _ => "other",
    }
    .to_string()
}

/// 递归计算目录大小；遇到无权限等错误时跳过该子项并继续，返回 (大小, 条目数, 首个错误)
fn dir_size_and_count(path: &Path) -> (u64, u64, Option<String>) {
    let mut total = 0u64;
    let mut count = 0u64;
    let mut first_error: Option<String> = None;

    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(format!("无法读取目录: {e}"));
                }
                continue;
            }
        };
        for entry in entries.flatten() {
            let p = entry.path();
            match fs::symlink_metadata(&p) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() {
                        // 不跟随符号链接，避免死循环
                        continue;
                    }
                    if meta.is_dir() {
                        stack.push(p);
                    } else {
                        total = total.saturating_add(meta.len());
                        count = count.saturating_add(1);
                    }
                }
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(format!("无法读取条目 {}: {e}", p.display()));
                    }
                }
            }
        }
    }
    (total, count, first_error)
}

fn file_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|m| m.len())
}

/// 统计数据库各表行数；数据库损坏/被占用时返回错误（不 panic）
fn db_table_row_counts(db_path: &Path) -> Result<Vec<(String, u64)>, String> {
    let conn = rusqlite::Connection::open(db_path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut any_ok = false;
    for table in DATA_TABLES {
        let count: rusqlite::Result<i64> =
            conn.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                row.get(0)
            });
        match count {
            Ok(c) => {
                out.push(((*table).to_string(), c.max(0) as u64));
                any_ok = true;
            }
            Err(e) => {
                // 单表失败（可能缺表）不阻塞整体
                log::debug!("storage_info: count {table} failed: {e}");
            }
        }
    }
    if !any_ok {
        // 没有任何一张表可读，视为数据库不可用（损坏或被占用）
        return Err("数据库无法读取（可能已损坏或被占用）".to_string());
    }
    Ok(out)
}

/// 读取数据库 schema 版本（`PRAGMA user_version`）；不可读时返回 None
fn db_schema_version(db_path: &Path) -> Option<i32> {
    let conn = rusqlite::Connection::open(db_path).ok()?;
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .ok()
}

/// 查找 backups 目录中最近一次数据库备份文件名（按修改时间倒序取最新；
/// 修改时间相同时按文件名倒序，因为备份文件名 `db_backup_YYYYMMDD_HHMMSS` 自带时间戳）
fn latest_db_backup_name(backup_dir: &Path) -> Option<String> {
    let entries = fs::read_dir(backup_dir).ok()?;
    let mut backups: Vec<(std::time::SystemTime, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("db_backup_") || !name.ends_with(".db") {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, name))
        })
        .collect();
    if backups.is_empty() {
        return None;
    }
    backups.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    Some(backups.into_iter().next().expect("non-empty").1)
}

/// 构建应用数据目录下的存储信息
fn collect_storage_info() -> StorageInfo {
    let base_dir = get_app_config_dir();
    let mut items: Vec<StorageItem> = Vec::new();

    // 1) 数据库
    let db_schema_version = {
        let db_path = base_dir.join("cc-switch.db");
        let mut item = StorageItem {
            path: db_path.to_string_lossy().to_string(),
            name: "cc-switch.db".to_string(),
            kind: "file".to_string(),
            purpose: purpose_label("database"),
            exists: db_path.exists(),
            size_bytes: None,
            record_count: None,
            error: None,
            schema_version: None,
        };
        if item.exists {
            item.size_bytes = file_size(&db_path);
            item.schema_version = db_schema_version(&db_path);
            match db_table_row_counts(&db_path) {
                Ok(rows) => {
                    item.record_count = Some(rows.iter().map(|(_, c)| *c).sum());
                }
                Err(e) => {
                    item.error = Some(format!("数据库读取失败（可能已损坏）: {e}"));
                }
            }
        } else {
            item.error = Some("数据库文件不存在".to_string());
        }
        let schema_version = item.schema_version;
        items.push(item);
        schema_version
    };

    // 2) 固定条目：config.json / settings.json / backups / logs / skills / crash.log
    let fixed_entries: Vec<(String, String, String)> = vec![
        (
            "config.json".to_string(),
            purpose_label("config"),
            "file".to_string(),
        ),
        (
            "settings.json".to_string(),
            purpose_label("settings"),
            "file".to_string(),
        ),
        (
            "backups".to_string(),
            purpose_label("backups"),
            "dir".to_string(),
        ),
        ("logs".to_string(), purpose_label("logs"), "dir".to_string()),
        (
            "skills".to_string(),
            purpose_label("skills"),
            "dir".to_string(),
        ),
        (
            "crash.log".to_string(),
            purpose_label("logs"),
            "file".to_string(),
        ),
    ];
    for (name, purpose, kind) in &fixed_entries {
        let path = base_dir.join(name);
        let exists = path.exists();
        let mut item = StorageItem {
            path: path.to_string_lossy().to_string(),
            name: name.clone(),
            kind: kind.clone(),
            purpose: purpose.clone(),
            exists,
            size_bytes: None,
            record_count: None,
            error: None,
            schema_version: None,
        };
        if exists {
            if kind == "dir" {
                let (size, count, err) = dir_size_and_count(&path);
                item.size_bytes = Some(size);
                item.record_count = Some(count);
                item.error = err;
            } else {
                item.size_bytes = file_size(&path);
            }
        } else {
            item.error = Some("路径不存在".to_string());
        }
        items.push(item);
    }

    // 3) 兜底：顶层其他文件/目录（未知条目归为 other）
    if let Ok(entries) = fs::read_dir(&base_dir) {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        seen.insert("cc-switch.db".to_string());
        for (name, _p, _k) in &fixed_entries {
            seen.insert(name.clone());
        }
        let mut extra: Vec<StorageItem> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if seen.contains(&name) {
                continue;
            }
            let p = entry.path();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let mut item = StorageItem {
                path: p.to_string_lossy().to_string(),
                name,
                kind: if is_dir {
                    "dir".to_string()
                } else {
                    "file".to_string()
                },
                purpose: purpose_label("other"),
                exists: true,
                size_bytes: None,
                record_count: None,
                error: None,
                schema_version: None,
            };
            if is_dir {
                let (size, count, err) = dir_size_and_count(&p);
                item.size_bytes = Some(size);
                item.record_count = Some(count);
                item.error = err;
            } else {
                item.size_bytes = file_size(&p);
            }
            extra.push(item);
        }
        extra.sort_by_key(|b| std::cmp::Reverse(b.size_bytes.unwrap_or(0)));
        items.extend(extra);
    }

    let total_size_bytes = items.iter().map(|i| i.size_bytes.unwrap_or(0)).sum();
    let latest_db_backup = latest_db_backup_name(&base_dir.join("backups"));

    StorageInfo {
        base_dir: base_dir.to_string_lossy().to_string(),
        total_size_bytes,
        items,
        db_schema_version,
        latest_db_backup,
    }
}

/// 获取数据存储信息（元数据，不含任何敏感内容）
#[tauri::command]
pub async fn get_storage_info() -> Result<StorageInfo, String> {
    Ok(collect_storage_info())
}

/// 判断 target 是否位于 base 目录内（或等于 base），路径按大小写不敏感规范化比较
fn is_within_dir(base: &Path, target: &Path) -> bool {
    fn norm_key(p: &Path) -> String {
        let mut key = p.to_string_lossy().replace('\\', "/");
        while key.len() > 1 && key.ends_with('/') {
            key.pop();
        }
        #[cfg(windows)]
        {
            key = key.to_lowercase();
        }
        key
    }
    let base_key = norm_key(base);
    let target_key = norm_key(target);
    target_key == base_key || target_key.starts_with(&format!("{base_key}/"))
}

/// 打开应用数据目录下的某个条目（文件/目录）。
/// 仅允许打开 app config dir 以内的路径，防止任意路径操作。
#[tauri::command]
pub async fn open_storage_item(handle: AppHandle, path: String) -> Result<bool, String> {
    let base = get_app_config_dir();
    let target = PathBuf::from(path);

    if !is_within_dir(&base, &target) {
        return Err("路径不在应用数据目录内，已拒绝打开".to_string());
    }

    if !target.exists() {
        return Err(format!("路径不存在: {}", target.display()));
    }

    handle
        .opener()
        .open_path(target.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| format!("打开路径失败: {e}"))?;
    Ok(true)
}
