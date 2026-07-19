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

## 5. 进行中的开发（本轮，未提交）

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
1. **Copilot 优化器面板** — 暴露后端已有的 4 个布尔开关（`get/set_copilot_optimizer_config`）。
2. **轻量模式按钮** — 主界面/设置页入口调 `enter_lightweight_mode`（原先只在托盘菜单）。
3. **Claude 插件状态展示** — 只读展示 `~/.claude/config.json` 的登录向导跳过标记状态（`get_claude_plugin_status` / `is_claude_plugin_applied` / `read_claude_plugin_config`）。

### 5.3 分工与当前状态（2026-07-19）

| 任务 | 执行者 | 状态 |
|---|---|---|
| 后端网关（Rust） | 后端代理（曾做代理可行性评估，id `aa68ec38b91580ffa`） | 进行中 |
| 前端 4 个 UI | 前端代理 `frontend-ui` | 进行中（api 封装与面板组件已出现于工作区） |
| Rust 1.95 工具链 | rustup | ✅ 已装（`~/.cargo/bin`） |
| MSVC Build Tools | 静默安装（后台任务） | ⚠️ 第一次因 UAC 被拒失败(exit 66)，已重试，需用户在 UAC 弹窗点"是" |

### 5.4 待办（本轮收尾清单）
- [ ] 等两个代理完成，收集报告
- [ ] MSVC 装好后：`cd src-tauri && cargo check`（PATH 加 `~/.cargo/bin`），修复编译错误
- [ ] 前端 `./node_modules/.bin/tsc --noEmit` 零错误（pnpm 首次需 `pnpm approve-builds` 放行 esbuild）
- [ ] 手动冒烟：启用网关 → curl 三个端点（鉴权失败/成功/alias 未命中/模型列表）
- [ ] 统一提交到 `custom`（可拆 2-3 个 commit：网关后端 / 网关前端 / 三个隐藏功能 UI）
- [ ] 给用户输出使用说明（Base URL、key、客户端接入示例）

## 6. 后续候选需求（用户提过、未排期）
- 切换供应商时展示配置文件变更 diff 详情（用户误以为已有此功能，官方无）。
- 使用中发现冗余功能再逐个隐藏（走 featureFlags 开关）。
- 网关二期：更多入站协议/更细的路由策略（按前缀/通配符匹配等）。

## 7. 环境备忘

- Windows + Git Bash（zsh 语法输出，路径用 `/d/...`）；`cargo` 需 `export PATH="$HOME/.cargo/bin:$PATH"`。
- 前端包管理 pnpm；构建脚本被 pnpm 拦截时执行 `pnpm approve-builds`。
- 原始 v3.17.0 快照在 `../cc-switch-v3.17.0`（仅参考，git 里已有 `v3.17.0` 标签，可删）。
