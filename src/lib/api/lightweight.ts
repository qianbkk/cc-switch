import { invoke } from "@tauri-apps/api/core";

/**
 * 轻量模式 API
 *
 * 后端契约：enter_lightweight_mode() -> void
 * 进入后主窗口关闭，可从系统托盘恢复。
 */
export const lightweightApi = {
  /** 进入轻量模式（关闭窗口，仅保留托盘） */
  async enter(): Promise<void> {
    return invoke("enter_lightweight_mode");
  },
};
