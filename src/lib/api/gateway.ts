import { invoke } from "@tauri-apps/api/core";
import type { AppId } from "./types";

/**
 * 统一网关 (Unified Gateway) API
 *
 * 后端契约（并行开发中）：
 * - get_gateway_config() -> GatewayConfig
 * - save_gateway_config({ config }) -> void
 * - regenerate_gateway_key() -> string（新 key）
 */

/** 网关模型池中的单个模型别名映射 */
export interface GatewayModelEntry {
  /** 对外暴露的别名（`供应商名/模型名`，供应商名内空格与 "/" 替换为 "-"） */
  alias: string;
  /** 供应商 ID */
  providerId: string;
  /** 所属应用类型 */
  appType: Extract<AppId, "claude" | "codex" | "gemini">;
  /** 实际模型名 */
  model: string;
}

/** 统一网关配置 */
export interface GatewayConfig {
  /** 是否启用网关 */
  enabled: boolean;
  /** 网关 API Key */
  apiKey: string;
  /** 模型池 */
  models: GatewayModelEntry[];
  /** 非流式请求总超时（秒），0 = 禁用，默认 600 */
  nonStreamingTimeoutSecs?: number;
  /** 流式首字节超时（秒），0 = 禁用，默认 60 */
  streamingFirstByteTimeoutSecs?: number;
  /** 流式空闲超时（秒），0 = 禁用，默认 120 */
  streamingIdleTimeoutSecs?: number;
}

export const gatewayApi = {
  /** 获取网关配置 */
  async getConfig(): Promise<GatewayConfig> {
    return invoke<GatewayConfig>("get_gateway_config");
  },

  /** 保存网关配置 */
  async saveConfig(config: GatewayConfig): Promise<void> {
    return invoke("save_gateway_config", { config });
  },

  /** 重新生成 API Key（旧 key 立即失效），返回新 key */
  async regenerateKey(): Promise<string> {
    return invoke<string>("regenerate_gateway_key");
  },
};

/**
 * 生成模型别名：`供应商名/模型名`
 * 供应商名里的空格和 "/" 替换为 "-"，避免破坏别名的路径语义。
 */
export function buildGatewayAlias(
  providerName: string,
  modelName: string,
): string {
  const safeProvider = providerName.replace(/[\s/]+/g, "-");
  return `${safeProvider}/${modelName}`;
}
