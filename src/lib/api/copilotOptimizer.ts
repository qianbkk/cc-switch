import { invoke } from "@tauri-apps/api/core";

/**
 * Copilot 请求优化器 API
 *
 * 后端契约：
 * - get_copilot_optimizer_config() -> CopilotOptimizerConfig
 * - set_copilot_optimizer_config({ config }) -> boolean
 *
 * 字段与 src-tauri/src/proxy/types.rs 的 CopilotOptimizerConfig 对齐（camelCase）。
 */

/** Copilot 优化器配置 */
export interface CopilotOptimizerConfig {
  /** 总开关 */
  enabled: boolean;
  /** x-initiator 请求分类 */
  requestClassification: boolean;
  /** Tool result 消息合并 */
  toolResultMerging: boolean;
  /** Compact 请求识别 */
  compactDetection: boolean;
  /** 确定性 Request ID */
  deterministicRequestId: boolean;
  /** Subagent 检测 */
  subagentDetection: boolean;
  /** Warmup 小模型降级 */
  warmupDowngrade: boolean;
  /** Warmup 降级使用的模型 */
  warmupModel: string;
  /** 请求前剥离 thinking / redacted_thinking block */
  stripThinking: boolean;
}

export const copilotOptimizerApi = {
  async getConfig(): Promise<CopilotOptimizerConfig> {
    return invoke<CopilotOptimizerConfig>("get_copilot_optimizer_config");
  },

  async setConfig(config: CopilotOptimizerConfig): Promise<boolean> {
    return invoke<boolean>("set_copilot_optimizer_config", { config });
  },
};
