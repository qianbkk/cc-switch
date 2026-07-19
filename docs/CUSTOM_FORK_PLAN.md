# CC Switch 魔改版 — 总体规划与执行状态

> 本文档是本地魔改版（`custom` 分支）的唯一规划文档，供后续会话/开发接续执行。
> 最近更新：2026-07-19（晚）

---

## 1. 项目背景与目标

- 基于上游 [cc-switch](https://github.com/farion1231/cc-switch) 最新源码做本地魔改版。
- **要求长期跟踪上游更新**，因此所有改动遵循低冲突原则（见 §3）。
- 用户决策：14 个功能板块**全部保留**，唯一裁剪是云同步（只隐藏 UI 入口，后端保留）。

## 2. 仓库与分支结构

| 项 | 值 |
|---|---|
| 本地路径 | `/d/AI/A_shared_workspace/ccswitch-issue/cc-switch` |
| remote | `upstream` → github.com/farion1231/cc-switch |
| `main` 分支 | 干净跟踪上游，**永不直接改动** |
| `custom` 分支 | 所有魔改在此进行 |

跟进上游流程：`git fetch upstream` → `main` 上 `git merge upstream/main` → `custom` 上 `git merge main`，冲突集中在少数枢纽文件（lib.rs、commands/mod.rs、SettingsPage.tsx、server.rs）。

## 3. 魔改原则（务必遵守）

1. **尽量新增文件，少改公共文件**——新功能放独立新模块/新组件。
2. 隐藏官方功能一律走 `src/config/featureFlags.ts` 的开关，**不删后端代码**。
3. 前端新 API 封装放 `src/lib/api/` 独立新文件；后端新 command 放 `commands/` 新文件。
4. 不动 `main` 分支；`custom` 上提交信息用中文说明意图。

## 4. 已完成改动（custom 分支 commit 历史）

- `04bf436d` feat: 隐藏云同步入口（featureFlags.ts + SettingsPage.tsx 包裹）
- （commit）docs: 魔改版总体规划（仅本文件）
- （commit）feat(frontend): 统一网关面板 + 三个隐藏功能 UI（含 3 个 bug 修复）
  - 新增 `src/components/settings/GatewaySettingsPanel.tsx`
  - 新增 `src/components/settings/CopilotOptimizerPanel.tsx`（8 开关 + warmupModel；ref 回滚修复）
  - 新增 `src/components/settings/LightweightModeSettings.tsx`
  - 新增 `src/components/settings/ClaudePluginStatusPanel.tsx`（复制 catch 修复）
  - 新增 `src/lib/api/{gateway,copilotOptimizer,lightweight,claudePlugin}.ts`
  - 修改 `src/lib/api/index.ts` re-export
  - 修改 `src/components/settings/SettingsPage.tsx` 挂载 4 组件
  - 修改 `src/i18n/locales/{en,zh,zh-TW,ja}.json` 文案
  - 修 GatewaySettingsPanel 端口为 0 兜底
- （commit）feat(gateway): 后端 4 端点 + alias 路由 + 响应协议反向转换（cargo check 零错误）
  - 新增 `src-tauri/src/commands/gateway.rs`（3 command + KV 持久化）
  - 新增 `src-tauri/src/proxy/gateway.rs`（4 HTTP handler + 鉴权 + 完整 non-streaming 反向转换 + 部分 streaming）
  - 修改 `src-tauri/src/commands/mod.rs`（+pub mod gateway）
  - 修改 `src-tauri/src/proxy/mod.rs`（+pub mod gateway）
  - 修改 `src-tauri/src/proxy/server.rs`（+挂 /gateway/* 4 路由）
  - 修改 `src-tauri/src/lib.rs`（注册 3 command + 启动恢复钩子）

## 5. 设计契约（统一网关）

### 5.1 配置存储
- settings KV 表，键 `gateway_config`，JSON camelCase：
  ```json
  { "enabled": false, "apiKey": "ccs-<48 hex>",
    "models": [ { "alias": "供应商名/模型名", "providerId": "...", "appType": "claude|codex|gemini", "model": "真实模型名" } ] }
  ```

### 5.2 Tauri commands
- `get_gateway_config()` — 无存储时生成默认（enabled=false、key="ccs-"+48 hex）并持久化后返回
- `save_gateway_config({config})` — 保存；enabled 且代理未运行则启动（不做 CLI 接管）
- `regenerate_gateway_key()` — 换新 key 并返回

### 5.3 HTTP 端点
| 路径 | 入站协议 |
|---|---|
| `POST /gateway/v1/messages` | Anthropic Messages |
| `POST /gateway/v1/chat/completions` | OpenAI Chat |
| `POST /gateway/v1/responses` | OpenAI Responses |
| `GET /gateway/v1/models` | OpenAI 风格列表 |

- 鉴权：`Authorization: Bearer <key>` 或 `x-api-key: <key>`，缺/错 401，`enabled=false` 时 403
- alias 精确匹配 body.model；未命中 400 含可用 alias 列表

### 5.4 响应协议转换状态

采用"Anthropic 中间表示"链式（参考 LiteLLM 双跳模式）。

**Non-streaming（✅ 完整实现）**：所有 9 种组合经 Anthropic 中间表示转换；同协议零转换。

| 入站\上游 | Anthropic | OpenAI Chat | OpenAI Responses | Gemini |
|---|---|---|---|---|
| Anthropic | 0 转换 | 链 1 | 链 1 | 链 1 |
| OpenAI Chat | 链 2 | 0 转换 | 链 1 | 链 2 |
| OpenAI Responses | 链 2 | 链 1 | 0 转换 | 链 2 |
| (Gemini 不入站客户端，仅上游) | - | - | - | - |

链 1 = 上游→Anthropic（已有 `openai_to_anthropic`/`responses_to_anthropic`/`gemini_to_anthropic`） + Anthropic→入站
链 2 = 同链 1

**Streaming（⚠️ 部分实现）**：

| 入站\上游 | Anthropic | OpenAI Chat | OpenAI Responses | Gemini |
|---|---|---|---|---|
| Anthropic | ✅ 0 转换 | ⚠️ passthrough+warn | ✅ streaming_responses | ✅ streaming_gemini |
| OpenAI Responses | ✅ streaming_codex_anthropic | ⚠️ passthrough+warn | ✅ 0 转换 | ✅ 链式 |
| OpenAI Chat | ⚠️ passthrough+warn | ✅ 0 转换 | ⚠️ passthrough+warn | ⚠️ passthrough+warn |

✅ = 已复用现有 streaming_* 转换器工作  
⚠️ = 缺 Chat↔Anthropic SSE 转换器，暂透传+warn（响应会被客户端按上游协议解析失败）

## 6. 当前工具链状态

- ✅ Rust 1.95（~/.cargo/bin/cargo）
- ✅ MSVC Build Tools（已装，cargo build 可用）
- ✅ 4 端点 `cargo check` 零错误
- ⏸ 待办：实际 `cargo build` + 端到端冒烟（需要跑应用）
- ⏸ 前端 pnpm install 首次需 `pnpm approve-builds`（之前因 402 没跑端到端）

## 7. 后续待办（按优先级）

### 7.1 Live 配置保护（用户需求 #2）⚠️ 未开始
**症状**：用户在 Claude/Codex/Gemini 的 live 配置文件（如 `~/.codex/config.toml`）做手动修改后，重启 ccswitch 或切换供应商时，修改被接管写盘覆盖。

**根因**（已确认）：
- 所有 live 写都通过 `config::write_json_file`/`write_text_file`/`atomic_write`（`config.rs:274-344`），全部**全量替换**（temp+rename，无 merge）
- `proxy_live_backup` 表只存接管前原始内容（无 hash/mtime），写盘时**不校验用户是否修改过**
- Codex 是唯一用 `toml_edit::DocumentMut`（保留未知字段）的写入器，但其他写者（全量替换）会覆盖其结果
- `services/config.rs` 的旧导入路径已被 `live.rs` 取代，但 `live.rs` 的 `sanitize_claude_settings_for_live` 只删除 cc-switch 自有字段
- 多个写者（`takeover_live_config_strict`、`sync_*_live`、`ProfileService::apply`）都会触发覆盖

**推荐方案（最小侵入）**：
1. 扩展 `proxy_live_backup` 表加 `original_hash` 列（接管前算 SHA256）
2. 新增 `live_protection.rs` 模块封装校验逻辑
3. 接入 `takeover_live_config_strict` 和其他 live 写者前：若文件存在且 hash != backup.original_hash，则**跳过覆盖** + 返回错误 `LiveConfigModifiedByUser`
4. 新增 settings 开关 `protect_user_live_edits`（默认 true）
5. 前端：写失败时弹"用户已修改 live 文件"提示，给"查看 diff / 强制接管 / 取消"三选一

**改动面**：
- 数据库：`schema.rs` 加列 + `dao/proxy.rs` 加 hash 字段；migration 旧数据可填空
- 新增：`src-tauri/src/live_protection.rs`（核心校验 + 错误类型）
- 修改：`services/proxy.rs` `takeover_live_config_strict` + `restore_live_config_for_app_with_fallback_inner`（按开关决定是否校验）+ `sync_*_live` 入口
- 前端：`ProxyPanel.tsx` 增加"实时写入保护"开关；写失败时弹自定义 Toast

**预估**：1 天工作量；改动跨 ~5 个 Rust 文件 + 1 个前端文件

### 7.2 网关响应 SSE 增强 ⚠️ 未开始
当前缺口（5.4 表里 ⚠️ 行）：
- Chat→Anthropic SSE 转换缺
- Anthropic→Chat SSE 转换缺

补齐需新增 `providers/streaming_anthropic_chat.rs`：复用现有 Responses state machine，把"create responses"反向，输出 Chat 的 `data: {...}\n\n`。约 400 行。

**预估**：1-2 天工作量。

### 7.3 按应用禁用代理 UI 优化（用户需求 #1）⚠️ 调研完成，未改动
**调研结论**：后端 per-app API `set_proxy_takeover_for_app(app, false)` 已完整可用，会自动恢复对应 app 的 live 配置。前端 `ProxyPanel` 已有四应用独立 Switch。**95% 已实现，零后端改动**。

**仅 UI 优化**：
- 让 `ProxyPanel` 接管区域常驻可见（不依赖代理运行状态）
- 文案从"应用接管"改为"使用 CC Switch 代理"，更直观
- 主界面 `ProxyToggle` 加可见文字标签
- 补 i18n 文案 `proxy.takeover.useCcswitchProxy`

**预估**：半天工作量。

### 7.4 端到端冒烟 + 用户使用说明 ⚠️ 待办
- 实际跑应用（`pnpm tauri dev` 或 build + install）
- curl 4 端点：401（错 key）/ 403（disabled）/ 200（成功）/ 400（alias 未命中）
- 给用户写完整使用说明文档（Base URL + key + 三协议 curl 示例 + 客户端接入步骤）

## 8. 环境备忘

- Windows + Git Bash（路径用 `/d/...`）；`cargo` 需 `export PATH="$HOME/.cargo/bin:$PATH"`。
- 前端包管理 pnpm；构建脚本被 pnpm 拦截时执行 `pnpm approve-builds`。
- 原始 v3.17.0 快照在 `../cc-switch-v3.17.0`（仅参考，git 里已有 `v3.17.0` 标签，可删）。

## 9. 关键文件速查

| 关注点 | 文件 |
|---|---|
| 网关后端入口 | `src-tauri/src/proxy/gateway.rs`（约 600 行） |
| 网关 command | `src-tauri/src/commands/gateway.rs` |
| 网关前端 | `src/components/settings/GatewaySettingsPanel.tsx` |
| 网关 API 封装 | `src/lib/api/gateway.ts` |
| 隐藏功能 UI | `src/components/settings/{CopilotOptimizerPanel,LightweightModeSettings,ClaudePluginStatusPanel}.tsx` |
| 隐藏功能 API | `src/lib/api/{copilotOptimizer,lightweight,claudePlugin}.ts` |
| featureFlags | `src/config/featureFlags.ts` |
| 代理接管（per-app） | `src-tauri/src/services/proxy.rs:730-914` |
| 接管写盘 | `src-tauri/src/services/proxy.rs:1585-1641` |
| 接管恢复（三层兜底） | `src-tauri/src/services/proxy.rs:1788-1845` |
| proxy_config 表 | `src-tauri/src/database/schema.rs:126-139` |
| proxy_live_backup 表 | `src-tauri/src/database/schema.rs:264-270` |
| 写入器（全量替换） | `src-tauri/src/config.rs:274-344`（atomic_write） |