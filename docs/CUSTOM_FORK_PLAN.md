# CC Switch 魔改版 — 总体规划与执行状态

> 本文档是本地魔改版（`custom` 分支）的唯一规划文档，供后续会话/开发接续执行。
> 最近更新：2026-07-19

---

## 1. 项目背景与目标

- 基于上游 [cc-switch](https://github.com/farion1231/cc-switch) 最新源码做本地魔改版。
- **要求长期跟踪上游更新**，因此所有改动遵循低冲突原则（见 §3）。
- 用户决策：14 个功能板块**全部保留**，唯一裁剪是云同步（只隐藏 UI 入口，后端保留）。

## 2. 仓库与分支结构

| 项 | 值 |
|---|---|
| 本地路径 | `/d/AI/A_shared_workspace/ccswitch-issue/cc-switch`（Windows: `D:\AI\A_shared_workspace\ccswitch-issue\cc-switch`） |
| remote | `upstream` → github.com/farion1231/cc-switch |
| `main` 分支 | 干净跟踪上游，**永不直接改动** |
| `custom` 分支 | 所有魔改在此进行 |

**跟进上游流程**：`git fetch upstream` → `main` 上 `git merge upstream/main` → `custom` 上 `git merge main`，冲突集中在少数枢纽文件（lib.rs、commands/mod.rs、SettingsPage.tsx、server.rs）。

## 3. 魔改原则（务必遵守）

1. **尽量新增文件，少改公共文件**——新功能放独立新模块/新组件。
2. 隐藏官方功能一律走 `src/config/featureFlags.ts` 的开关，**不删后端代码**。
3. 前端新 API 封装放 `src/lib/api/` 独立新文件；后端新 command 放 `commands/` 新文件。
4. 不动 `main` 分支；`custom` 上提交信息用中文说明意图。

## 4. 已完成的改动

### 4.1 隐藏云同步入口（已提交 `04bf436d`）
- 新增 `src/config/featureFlags.ts`：`ENABLE_CLOUD_SYNC = false`（改回 `true` 即恢复）。
- `SettingsPage.tsx` 的 cloudSync Accordion 用该开关包裹。
- 后端 WebDAV/S3 代码全部保留，未配置则无后台行为。

## 5. 进行中的开发（2026-07-19 暂停，**额度用尽，已写接续指南**）

> 当前真实状态：本轮开发因 API 额度（402，重置时间今晚 21:27 UTC+8）暂停，**后端网关 0 进度**，**前端 4 个 UI 已基本完成待修**，**MSVC 已装好**。下一会话可直接对照本文 §10 续做。

### 5.1 统一网关（单 key 用遍所有供应商模型）⭐ 核心新功能

**需求**（用户原话归纳）：导入多个供应商的 key 和模型后，勾选要暴露的模型，生成一个专属 key；任意客户端用这个 key + 本地地址即可调用所有勾选的模型；模型名要能看出属于哪个供应商。

**设计契约**（前后端并行开发的接口约定，勿改字段名）：

- 配置存 settings KV 表，键 `gateway_config`，JSON camelCase：
  ```json
  { "enabled": false, "apiKey": "ccs-<48位hex>",
    "models": [ { "alias": "供应商名/模型名", "providerId": "...", "appType": "claude|codex|gemini", "model": "真实模型名" } ] }
  ```
- alias 由前端生成（供应商名中空格和 `/` 替换为 `-`），后端只做精确匹配。
- Tauri commands（`src-tauri/src/commands/gateway.rs`）：
  - `get_gateway_config()` — 无存储时生成默认值并持久化
  - `save_gateway_config({config})` — 保存；enabled 且代理未运行则启动（不做 CLI 接管）
  - `regenerate_gateway_key()` — 换新 key 并返回
- HTTP 端点（挂现有代理服务器，新模块 `proxy/gateway.rs`）：
  - `POST /gateway/v1/messages`（Anthropic 协议入站）
  - `POST /gateway/v1/chat/completions`（OpenAI Chat 入站）
  - `POST /gateway/v1/responses`（OpenAI Responses 入站）
  - `GET /gateway/v1/models`（OpenAI 风格模型列表，id=alias）
  - 鉴权：`Authorization: Bearer <key>` 或 `x-api-key`；错误 401；`enabled=false` 时 403
  - 未命中 alias 返回 400 并列出可用 alias
- 路由：alias → (providerId, 真实 app_type) → 单供应商候选走现有 forwarder（复用 key 替换/协议转换/SSE/usage 统计/熔断）。
- 入站协议由路径决定、上游协议由 provider 的 api_format 决定；缺失的转换链（如 Chat 入站→Anthropic 上游）通过组合现有 transform 补齐。
- 启动恢复：应用启动时若 gateway enabled 则确保代理服务器运行。

### 5.2 三个隐藏功能补 UI（用户确认全要）
1. **Copilot 优化器面板** — 暴露后端已有开关（`get/set_copilot_optimizer_config`，实际有 8 个 boolean + 1 个 warmupModel 字符串字段，不是最初的"4 开关"；前端已按全字段实现）。
2. **轻量模式按钮** — 主界面/设置页入口调 `enter_lightweight_mode`（原先只在托盘菜单）。
3. **Claude 插件状态展示** — 只读展示 `~/.claude/config.json` 的登录向导跳过标记状态（`get_claude_plugin_status` / `is_claude_plugin_applied` / `read_claude_plugin_config`）。

### 5.3 分工与当前状态（截至 2026-07-19 18:20）

| 任务 | 状态 |
|---|---|
| 后端网关（Rust） | ⛔ **未开始**（代理因 402 额度用尽终止，0 文件落盘） |
| 前端 4 个 UI | ✅ **基本完成**（4 组件 + 4 API 封装 + SettingsPage 挂载 + i18n 文案）；已知 3 个 bug，见 §9 |
| Rust 1.95 工具链 | ✅ 已装（`~/.cargo/bin`，自动加入 PATH） |
| MSVC Build Tools | ✅ 已装（重试后 exit 0，可 cargo build） |
| `cargo check` 验证 | ⏸ 等后端写完再做 |
| 前端 tsc 验证 | ⏸ 需先 `pnpm approve-builds` + `pnpm install` 完成 |

## 6. 后续候选需求（用户提过、未排期）
- 切换供应商时展示配置文件变更 diff 详情（用户误以为已有此功能，官方无）。
- 使用中发现冗余功能再逐个隐藏（走 featureFlags 开关）。
- 网关二期：更多入站协议/更细的路由策略（按前缀/通配符匹配等）。

## 7. 环境备忘

- Windows + Git Bash（zsh 语法输出，路径用 `/d/...`）；`cargo` 需 `export PATH="$HOME/.cargo/bin:$PATH"`。
- 前端包管理 pnpm；构建脚本被 pnpm 拦截时执行 `pnpm approve-builds`。
- 原始 v3.17.0 快照在 `../cc-switch-v3.17.0`（仅参考，git 里已有 `v3.17.0` 标签，可删）。

## 8. 前端已完成内容清单（待后端配合）

### 8.1 新增组件（4 个）
- `src/components/settings/GatewaySettingsPanel.tsx`（约 400 行）：启用开关、key 展示/复制/重新生成（带确认）、按应用分组的供应商列表、模型拉取/勾选/手动添加、curl 示例
- `src/components/settings/CopilotOptimizerPanel.tsx`：8 个布尔开关 + warmupModel 字符串字段（按 enabled 总开关自动灰显）
- `src/components/settings/LightweightModeSettings.tsx`：轻量模式入口按钮
- `src/components/settings/ClaudePluginStatusPanel.tsx`：显示 config.json 路径、登录向导跳过标记徽标、查看原文弹窗

### 8.2 新增 API 封装（4 个）
- `src/lib/api/gateway.ts`：`get/save/regenerate` 三个函数；导出 `GatewayConfig`、`GatewayModelEntry`、`AppId` 类型；`buildGatewayAlias(name, model)` 工具
- `src/lib/api/copilotOptimizer.ts`：`getConfig/setConfig`
- `src/lib/api/lightweight.ts`：`enter`
- `src/lib/api/claudePlugin.ts`：`status/applied/read`

### 8.3 改动
- `src/lib/api/index.ts`：re-export 4 个新模块
- `src/components/settings/SettingsPage.tsx`：导入并挂载 4 个新组件（Gateway/Copilot/ClaudePlugin 挂在 advanced Tab 的同名 AccordionItem；Lightweight 挂在 general Tab）
- 4 个 i18n 文件（en/zh/zh-TW/ja）：新增 `settings.advanced.gateway*` / `copilotOptimizer.*` / `claudePlugin.*` / `lightweightMode.*` 文案

## 9. 前端已知 bug（修复后即可提交）

来自前端代理离场前的自评：

1. **CopilotOptimizerPanel 错误回滚失效**：第 109-110 行 `warmupModel` 先本地 `setConfig({ ...config, warmupModel: e.target.value })`，但 onBlur 第 110 行捕获的 `prev` 已是新值，保存失败时无法正确回滚。
   修法：onChange 阶段把 `prev` 通过 ref 保存，或者在 onBlur 内重新读 ref。
2. **GatewaySettingsPanel 端口为 0**：代理端口获取失败时显示 `http://127.0.0.1:0/gateway`。应回退到默认端口或显示"未配置代理"提示。
3. **ClaudePluginStatusPanel 复制失败未捕获**：复制按钮的 `copyText` 调用未 try/catch，可能抛出未被捕获的 promise rejection。

## 10. 接续指南（下次开会话或额度恢复后，照此执行）

### 10.1 一句话目标
**先修前端 3 个 bug → 验证 tsc 零错误 → 实现后端网关 → cargo check 零错误 → 端到端冒烟 → 分批提交 → 给用户使用说明。**

### 10.2 推荐执行顺序
1. **修前端 bug**（小，直接改）
   - `CopilotOptimizerPanel.tsx`：用 ref 保存 prev；onBlur 前先存 prev
   - `GatewaySettingsPanel.tsx`：端口获取失败时给个 toast 或回退显示
   - `ClaudePluginStatusPanel.tsx`：包一层 try/catch + toast
2. **前端 tsc 验证**：`./node_modules/.bin/tsc --noEmit`（若失败先 `pnpm install` 再 `pnpm approve-builds`）
3. **提交前端**：`git add -A && git commit -m "feat(frontend): 统一网关面板 + 三个隐藏功能 UI"`（不要带 docs/，那个单独已提交）
4. **后端网关开发**：参考 §5.1 契约，重点文件清单：
   - 新增 `src-tauri/src/commands/gateway.rs`：3 个 command
   - 在 `src-tauri/src/commands/mod.rs` 加 `pub mod gateway;`
   - 在 `src-tauri/src/lib.rs` 的 `invoke_handler![]` 注册 3 个命令
   - 新增 `src-tauri/src/proxy/gateway.rs`：4 个 HTTP 端点 + 鉴权 + alias 路由
   - 在 `src-tauri/src/proxy/server.rs` 的 build_router 处挂载 `/gateway/*` 子路由
   - 在 `src-tauri/src/proxy/mod.rs` 加 `pub mod gateway;`
   - 应用启动恢复逻辑在 `lib.rs` 中现有 proxy 恢复代码附近加一小段（若 gateway enabled 则确保代理启动）
   - 模型转换参考 §5.1；缺失的 Chat→Anthropic 入站链可通过组合 `responses_request_to_anthropic` + OpenAI Chat→Responses 间接实现，或者补一条直接转换
5. **后端验证**：`cd src-tauri && cargo check`，修复编译错误
6. **端到端冒烟**（需要实际跑应用）：
   - 启用网关 → 后端 `cargo build` → `cargo tauri dev`（或手工构建并跑 .msi）
   - curl 4 个端点验证：401/200/400
7. **分批提交**：建议 3 个 commit
   - `feat(gateway): 统一网关设置面板`
   - `feat(settings): 三个隐藏功能 UI`
   - `feat(gateway): 后端三协议入口 + alias 路由`
8. **给用户输出使用说明**：Base URL、key 示例、3 协议接入 curl 示例
