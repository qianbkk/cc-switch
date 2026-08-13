//! 统一网关 Tauri Commands
//!
//! 提供 `/gateway/*` 入口所需的设置存取 commands。配置以 JSON 形式存储在
//! settings KV 表的 `gateway_config` 键下（camelCase 字段，与前端契约一致）。
//!
//! - `get_gateway_config`   读取配置，未存时生成默认值并持久化后返回
//! - `save_gateway_config`  保存配置；enabled 且代理未运行则启动（无 CLI 接管）
//! - `regenerate_gateway_key` 重新生成 48 位 hex 的 key 并持久化后返回

use crate::store::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

/// 网关配置中的一条模型映射（与前端 camelCase 契约严格一致）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayModelEntry {
    /// 前端生成的别名（如 "供应商名/模型名"），后端精确匹配
    pub alias: String,
    pub provider_id: String,
    /// 该供应商实际注册到网关的 appType：claude / codex / gemini
    pub app_type: String,
    /// 真实上游模型名（与供应商侧的 model 字段一致）
    pub model: String,
}

/// 整个统一网关配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConfig {
    pub enabled: bool,
    pub api_key: String,
    #[serde(default)]
    pub models: Vec<GatewayModelEntry>,
    /// 非流式请求总超时（秒）。0 = 禁用（不推荐）；默认 600（与普通代理一致）。
    #[serde(default = "default_non_streaming_timeout_secs")]
    pub non_streaming_timeout_secs: u64,
    /// 流式请求首字节超时（秒）。0 = 禁用；默认 60。
    #[serde(default = "default_streaming_first_byte_timeout_secs")]
    pub streaming_first_byte_timeout_secs: u64,
    /// 流式请求空闲超时（秒）：两个数据块之间的最大间隔。0 = 禁用；默认 120。
    #[serde(default = "default_streaming_idle_timeout_secs")]
    pub streaming_idle_timeout_secs: u64,
}

fn default_non_streaming_timeout_secs() -> u64 {
    600
}

fn default_streaming_first_byte_timeout_secs() -> u64 {
    60
}

fn default_streaming_idle_timeout_secs() -> u64 {
    120
}

const GATEWAY_CONFIG_KEY: &str = "gateway_config";
/// 生成的 key 前缀（与前端契约一致）
const API_KEY_PREFIX: &str = "ccs-";

/// 生成 48 位 hex 的新 key（用 uuid v4 三次拼接去 dash 后取前 48 位；
/// 仅做 UI key 用途，不需要密码学强度）
fn generate_api_key() -> Result<String, String> {
    let mut hex = String::with_capacity(48);
    while hex.len() < 48 {
        let id = uuid::Uuid::new_v4().simple().to_string();
        // simple() 已经去掉 - ，共 32 个 hex 字符
        hex.push_str(&id);
    }
    hex.truncate(48);
    Ok(format!("{API_KEY_PREFIX}{hex}"))
}

/// 构造默认配置（首次 `get_gateway_config` 使用）
fn build_default_config() -> GatewayConfig {
    // uuid v4 在支持的平台上不会失败；若真的失败则退化为时间戳占位，仍能确保非空
    let api_key = generate_api_key().unwrap_or_else(|_| {
        format!(
            "{API_KEY_PREFIX}{:048x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0)
        )
    });
    GatewayConfig {
        enabled: false,
        api_key,
        models: Vec::new(),
        non_streaming_timeout_secs: default_non_streaming_timeout_secs(),
        streaming_first_byte_timeout_secs: default_streaming_first_byte_timeout_secs(),
        streaming_idle_timeout_secs: default_streaming_idle_timeout_secs(),
    }
}

/// 从 settings KV 读取配置，缺失时生成默认并落库后返回
fn load_or_init(db: &crate::database::Database) -> Result<GatewayConfig, String> {
    match db.get_setting(GATEWAY_CONFIG_KEY)? {
        Some(json) if !json.trim().is_empty() => {
            // 兼容旧版空数组场景：解析失败时回退到默认而不是直接报错
            match serde_json::from_str::<GatewayConfig>(&json) {
                Ok(cfg) => Ok(cfg),
                Err(e) => {
                    log::warn!("[Gateway] 配置解析失败，使用默认配置: {e}");
                    let cfg = build_default_config();
                    let serialized = serde_json::to_string(&cfg)
                        .map_err(|e| format!("序列化默认配置失败: {e}"))?;
                    db.set_setting(GATEWAY_CONFIG_KEY, &serialized)?;
                    Ok(cfg)
                }
            }
        }
        _ => {
            let cfg = build_default_config();
            let serialized =
                serde_json::to_string(&cfg).map_err(|e| format!("序列化默认配置失败: {e}"))?;
            db.set_setting(GATEWAY_CONFIG_KEY, &serialized)?;
            Ok(cfg)
        }
    }
}

fn persist(db: &crate::database::Database, cfg: &GatewayConfig) -> Result<(), String> {
    let serialized = serde_json::to_string(cfg).map_err(|e| format!("序列化配置失败: {e}"))?;
    db.set_setting(GATEWAY_CONFIG_KEY, &serialized)
        .map_err(|e| format!("写入设置失败: {e}"))
}

fn persist_disabled_after_start_failure(
    db: &crate::database::Database,
    cfg: &GatewayConfig,
) -> Result<(), String> {
    let mut disabled = cfg.clone();
    disabled.enabled = false;
    persist(db, &disabled)
}

/// 获取网关配置；首次未存时生成默认并持久化后返回。
#[tauri::command]
pub async fn get_gateway_config(state: State<'_, AppState>) -> Result<GatewayConfig, String> {
    load_or_init(&state.db)
}

fn ensure_gateway_enable_allowed(
    fork_features_enabled: bool,
    gateway_enabled: bool,
) -> Result<(), String> {
    if gateway_enabled && !fork_features_enabled {
        return Err("核心运行时魔改已关闭，无法启用统一网关".to_string());
    }
    Ok(())
}

fn gateway_start_allowed(fork_features_enabled: bool, gateway_enabled: bool) -> bool {
    fork_features_enabled && gateway_enabled
}

/// 保存网关配置。
///
/// - 核心运行时魔改关闭时，允许保存禁用状态和模型映射，但拒绝启用网关
/// - 保存到 settings KV
/// - 若 `enabled=true` 且代理当前未运行，则启动共享代理（不带 Live 接管）
/// - 启动失败时保留 key / 模型映射，但把持久化的 `enabled` 回滚为 false，
///   避免 UI 与实际运行状态分裂
#[tauri::command]
pub async fn save_gateway_config(
    config: GatewayConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let fork_features_enabled = crate::settings::fork_features_enabled();
    ensure_gateway_enable_allowed(fork_features_enabled, config.enabled)?;

    // 1. 持久化
    persist(&state.db, &config)?;

    // 2. 若启用且代理未运行，则启动共享代理服务器
    if gateway_start_allowed(fork_features_enabled, config.enabled)
        && !state.proxy_service.is_running().await
    {
        match state.proxy_service.start().await {
            Ok(info) => {
                log::info!(
                    "[Gateway] 启用时自动启动代理: {}:{}",
                    info.address,
                    info.port
                );
            }
            Err(e) => {
                log::error!("[Gateway] 启用时启动代理失败: {e}");
                if let Err(rollback_error) =
                    persist_disabled_after_start_failure(&state.db, &config)
                {
                    return Err(format!("{e}；同时回滚网关启用状态失败: {rollback_error}"));
                }
                return Err(e);
            }
        }
    }

    Ok(())
}

/// 重新生成一个 48 位 hex key 并持久化，返回新 key 字符串。
#[tauri::command]
pub async fn regenerate_gateway_key(state: State<'_, AppState>) -> Result<String, String> {
    let mut cfg = load_or_init(&state.db)?;
    let new_key = generate_api_key()?;
    cfg.api_key = new_key.clone();
    persist(&state.db, &cfg)?;
    Ok(new_key)
}

/// 应用启动恢复时调用：核心运行时魔改开启且网关 enabled 时确保代理已启动。
///
/// 由 `lib.rs` 在现有代理恢复代码附近调用，幂等多次调用安全。关闭核心开关时
/// 保留网关配置，但不会仅因为网关配置而拉起共享代理。
pub async fn ensure_gateway_started_on_startup(state: &AppState) {
    let fork_features_enabled = crate::settings::fork_features_enabled();
    if !fork_features_enabled {
        log::debug!("[Gateway] 启动恢复：核心运行时魔改已关闭，保留配置但跳过启动");
        return;
    }

    let cfg = match load_or_init(&state.db) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[Gateway] 启动恢复：读取配置失败（忽略）: {e}");
            return;
        }
    };

    if !cfg.enabled {
        log::debug!("[Gateway] 启动恢复：网关未启用，跳过");
        return;
    }

    if state.proxy_service.is_running().await {
        log::debug!("[Gateway] 启动恢复：代理已在运行，跳过");
        return;
    }

    match state.proxy_service.start().await {
        Ok(info) => {
            log::info!(
                "[Gateway] 启动恢复：代理已启动 {}:{}",
                info.address,
                info.port
            );
        }
        Err(e) => {
            log::error!("[Gateway] 启动恢复：启动代理失败（忽略）: {e}");
        }
    }
}

/// 仅由 `proxy/gateway.rs` 使用的访问器：从 DB 读最新配置（含 key），
/// 用于 HTTP 入口的鉴权。
pub fn load_gateway_config(
    db: &crate::database::Database,
) -> Result<GatewayConfig, crate::error::AppError> {
    load_or_init(db).map_err(crate::error::AppError::Database)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabling_gateway_requires_core_runtime_features() {
        assert!(ensure_gateway_enable_allowed(true, true).is_ok());
        assert!(ensure_gateway_enable_allowed(true, false).is_ok());
        assert!(ensure_gateway_enable_allowed(false, false).is_ok());
        assert_eq!(
            ensure_gateway_enable_allowed(false, true).unwrap_err(),
            "核心运行时魔改已关闭，无法启用统一网关"
        );
    }

    #[test]
    fn gateway_start_requires_both_runtime_features_and_gateway_enabled() {
        assert!(gateway_start_allowed(true, true));
        assert!(!gateway_start_allowed(false, true));
        assert!(!gateway_start_allowed(true, false));
        assert!(!gateway_start_allowed(false, false));
    }

    #[test]
    fn start_failure_disables_gateway_but_preserves_user_config() {
        let db = crate::database::Database::memory().expect("memory database");
        let config = GatewayConfig {
            enabled: true,
            api_key: "ccs-test-key".to_string(),
            models: vec![GatewayModelEntry {
                alias: "demo/model".to_string(),
                provider_id: "provider-1".to_string(),
                app_type: "codex".to_string(),
                model: "model-1".to_string(),
            }],
            non_streaming_timeout_secs: 600,
            streaming_first_byte_timeout_secs: 60,
            streaming_idle_timeout_secs: 120,
        };
        persist(&db, &config).expect("persist enabled config");

        persist_disabled_after_start_failure(&db, &config).expect("rollback enabled state");

        let stored = load_or_init(&db).expect("load rolled back config");
        assert!(!stored.enabled);
        assert_eq!(stored.api_key, config.api_key);
        assert_eq!(stored.models.len(), 1);
        assert_eq!(stored.models[0].alias, "demo/model");
    }
}
