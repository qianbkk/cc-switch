import { invoke } from "@tauri-apps/api/core";

/**
 * Claude 插件状态 API
 *
 * 后端契约：
 * - get_claude_plugin_status() -> { exists, path }
 * - is_claude_plugin_applied() -> boolean
 * - read_claude_plugin_config() -> string | null
 */

/** Claude 插件配置文件状态 */
export interface ClaudePluginStatus {
  /** 配置文件是否存在 */
  exists: boolean;
  /** 配置文件绝对路径 */
  path: string;
}

export const claudePluginApi = {
  /** 获取 ~/.claude/config.json 状态 */
  async getStatus(): Promise<ClaudePluginStatus> {
    return invoke<ClaudePluginStatus>("get_claude_plugin_status");
  },

  /** 是否已写入登录向导跳过标记 */
  async isApplied(): Promise<boolean> {
    return invoke<boolean>("is_claude_plugin_applied");
  },

  /** 读取配置原文（不存在时返回 null） */
  async readConfig(): Promise<string | null> {
    return invoke<string | null>("read_claude_plugin_config");
  },
};
