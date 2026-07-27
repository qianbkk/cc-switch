<div align="center">

# CC Switch (魔改版)

### The All-in-One Manager for Claude Code, Claude Desktop, Codex, Gemini CLI, Grok Build, OpenCode, OpenClaw & Hermes Agent

[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](#download--installation)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)

> 基于 [farion1231/cc-switch](https://github.com/farion1231/cc-switch) 的个人魔改分支。
> 同步上游：`bash scripts/sync-upstream.sh`

</div>

## 什么是魔改版

这是 cc-switch 上游的个人魔改分支，主要用于**本人**日常使用；`m*` tag 会发布 Windows Portable 预发行包。主要改动：

- **统一网关**(`/gateway/*`)：把多个供应商聚合成一个 OpenAI 兼容端点,任意客户端配一次 key+地址就能跨模型调度
- **Live 配置保护**：用户手改 `~/.codex/config.toml` 等 live 文件后，接管/切换不再覆盖,带冲突提示 toast 与一键开关
- **Codex auth.json 反向同步**：修掉 Codex CLI 0.14+ 把陈年 `OPENAI_API_KEY` 写回 `auth.json` 导致 UI 显示旧值的 Bug #3646
- **3 个隐藏功能补 UI**：Copilot 优化器、轻量模式、Claude 插件状态(后端命令本来就在,只是没入口)
- **代理面板常驻可见**：接管开关不再藏进 `AnimatePresence`,文案从"应用接管"改为"使用 CC Switch 代理"
- **隐藏云同步入口**：`ENABLE_CLOUD_SYNC=false`,WebDAV/S3 后端代码保留未删

详细规划见 [docs/CUSTOM_FORK_PLAN.md](docs/CUSTOM_FORK_PLAN.md)。

## 为什么需要 CC Switch

Claude Code、Codex、Gemini CLI 这些工具各有自己的配置格式。原本切换供应商要手改 JSON/TOML/`.env`,MCP 和 Skills 也无法跨工具统一管理。

CC Switch 用一个桌面应用统一管理:50+ 内置供应商预设、一键导入与切换、统一的 MCP/Skills 管理面板、系统托盘快捷切换,数据走 SQLite + 原子写。

- **一个应用管 8 款工具** — Claude Code、Claude Desktop、Codex、Gemini CLI、Grok Build、OpenCode、OpenClaw、Hermes
- **内置预设** — 50+ 含 AWS Bedrock、NVIDIA NIM 等常用 relay
- **统一 MCP & Skills** — 一个面板管 6 款工具的 MCP,GitHub 仓库一键安装 Skills
- **本地代理** — 格式转换、failover、熔断、健康监控,支持 Claude/Codex/Gemini/Grok 单应用接管
- **使用统计** — token 趋势、请求日志、按模型自定义价格

## 截图

| 主界面 | 添加供应商 |
| :---: | :---: |
| ![Main](assets/screenshots/main-zh.png) | ![Add](assets/screenshots/add-zh.png) |

## 魔改版独有功能

### 统一网关

上游的代理每个应用走各自端点。魔改版加了 `/gateway/*`,把多个供应商聚合成**一个 OpenAI 兼容端点**:任一客户端(Claude Code / 自研脚本 / IDE 插件)配一个地址 + 一个 key,就能调所有供应商。

- 4 个路由:`/gateway/v1/messages`、`/gateway/v1/chat/completions`、`/gateway/v1/responses`、`/gateway/v1/models`
- 鉴权:`Authorization: Bearer <key>` 或 `x-api-key: <key>`,启动时自动生成 `ccs-` 前缀 48 位 hex
- 模型通过 `body.model` 精确匹配 alias 表,未命中返回可用列表
- 协议转换走「上游协议 → Anthropic 中间表示 → 入站协议」的链式两步,复用上游已有的 3 个转换器,只补缺失的 Anthropic→Chat 一环就凑齐 3 入站 × 4 上游全部 12 种组合
- 启动时根据 `gateway_config.enabled` 自动恢复,与现存的代理服务独立并存

### Live 配置保护

修用户手改 `~/.codex/config.toml`、`~/.claude/settings.json` 等 live 文件后,被接管或切换供应商覆盖的问题。

- 备份表 `proxy_live_backup` 同时存 `original_hash`(接管前)和 `managed_hash`(CC Switch 最近一次写盘后)
- 写盘前比对磁盘当前 SHA256 与 `managed_hash`,不一致就拒绝写盘并返回 `LiveConfigModifiedByUser`
- 开关 `protect_user_live_edits` 默认开,关掉恢复强制接管
- 前端用 error 前缀匹配(`用户已修改`、`User has modified` 等)触发专用 toast,引导用户关闭保护

### Codex auth.json 反向同步

Codex CLI 0.14+ 启动时把陈年 `OPENAI_API_KEY` 写回 `~/.codex/auth.json`,导致 `getLiveProviderSettings` 读到旧值,UI 改完 key 保存后看似"key 没改"。

魔改版在 `ProviderService::update`/`switch` 时主动把 `auth.json` 的该字段同步成 DB 真值。**只替换一个字段**,保留 `auth_mode` / OAuth tokens(动了会破坏 ChatGPT 登录);失败只 warn 不阻塞保存。

### 三个隐藏功能补 UI

后端命令本来在上游就有(`enter_lightweight_mode`、`get_copilot_optimizer_config`、`get_claude_plugin_status`),魔改版只加前端面板和 API 包装,零后端改动。

## 上游未改动的功能

下面这些直接复用上游,行为与 [上游 README](https://github.com/farion1231/cc-switch) 完全一致:

- 供应商管理 / 切换 / 托盘 / 导入导出
- 本地代理格式转换、failover、熔断、健康监控、per-app 接管
- 统一 MCP / Prompts / Skills 管理
- 使用统计与请求日志
- 会话管理、OpenClaw workspace 编辑
- 主题、深链、自动备份、原子写、i18n

## 快速上手

### 基本使用

1. **添加供应商**:「添加供应商」→ 选择预设或自定义配置
2. **切换供应商**:
   - 主界面: 选中供应商 → 「启用」
   - 系统托盘: 直接点供应商名(立即生效)
3. **生效**: 重启终端或对应 CLI 工具(Claude Code 不用)
4. **回到官方登录**: 添加官方预设,切换后跑 CLI 的登出/登录流程

### 接管某个应用(本地代理)

设置 → Proxy → 打开开关 → 勾选 Claude/Codex/Gemini/Grok 中的目标应用。重启终端后,CC Switch 把指定 app 的请求路由到当前激活的供应商。

### 用统一网关

设置 → 高级 → 「统一网关」→ 生成 key → 在任意 OpenAI 兼容客户端里配:
- base_url: `http://127.0.0.1:15721/gateway/v1`
- api_key: 显示在面板里的 `ccs-...`
- model: alias 表里勾选的模型名

### 同步上游

```bash
bash scripts/sync-upstream.sh
# 等价于:
#   git fetch upstream
#   git merge upstream/main --no-edit
# 冲突时按提示解决
```

## 常见问题

<details>
<summary><strong>支持哪些 AI 工具?</strong></summary>

Claude Code、Claude Desktop、Codex、Gemini CLI、Grok Build、OpenCode、OpenClaw、Hermes 共 8 款。
</details>

<details>
<summary><strong>切换供应商后要重启终端吗?</strong></summary>

大部分要。**Claude Code 例外**,支持热切换不用重启。
</details>

<details>
<summary><strong>为什么不能删当前激活的供应商?</strong></summary>

"最小侵入"原则——删了对应 CLI 工具就废了。系统始终保留至少一个激活配置。不用某个工具可以在设置里隐藏。
</details>

<details>
<summary><strong>macOS 安装?</strong></summary>

代码已签,直接装 `dmg` 即可。
</details>

<details>
<summary><strong>数据存哪?</strong></summary>

- 数据库: `~/.cc-switch/cc-switch.db`(SQLite)
- 本地设置: `~/.cc-switch/settings.json`
- 自动备份: `~/.cc-switch/backups/`(默认保留 10 份)
- Skills: `~/.cc-switch/skills/`
</details>

<details>
<summary><strong>Linux Wayland + NVIDIA 点不动?</strong></summary>

```bash
CC_SWITCH_GDK_BACKEND=wayland ./CC-Switch-*.AppImage
```
</details>

<details>
<summary><strong>网关 401 "网关 key 无效"?</strong></summary>

`Authorization` 头大小写不敏感,但不要混 `Bearer xxx` 和 `x-api-key: xxx` 两个头的不同 key。重新生成 key 走设置面板。
</details>

<details>
<summary><strong>网关 SSE 流式出乱码?</strong></summary>

确认上游 provider 的 `api_format` 和入站协议协商正确。设置面板的 alias 行会显示映射。Anthropic↔Chat 已经补齐,其余走链式转换。如遇残余问题用 `pnpm tauri dev` 看后端 `[Gateway]` 日志。
</details>

## 下载与安装

### 系统要求

- **Windows**: 10+
- **macOS**: 12 (Monterey)+
- **Linux**: Ubuntu 22.04+ / Debian 11+ / Fedora 34+

### Windows

本 fork 的 [Releases](../../releases) 提供 `CC-Switch-m{upstream-version}-{patch}-Windows-Portable.zip`（预发行、免安装）；MSI 等正式安装包请使用上游 Release。

### macOS

```bash
brew install --cask cc-switch    # 推荐
```
或手动下 `CC-Switch-v{version}-macOS.dmg`(已签 notaried)。

### Arch Linux

```bash
paru -S cc-switch-bin
```

### Linux

[Releases](../../releases) 页下 `.deb` / `.rpm` / `.AppImage`。

<details>
<summary><strong>架构概览</strong></summary>

```
┌─────────────────────────────────────────────────────────────┐
│                    前端 (React + TS)                        │
│  Components ── Hooks ── TanStack Query (缓存/同步)          │
└────────────────────────┬────────────────────────────────────┘
                         │ Tauri IPC
┌────────────────────────▼────────────────────────────────────┐
│                  后端 (Tauri + Rust)                        │
│  Commands (API 层) ── Services (业务层) ── DAO (数据层)     │
└─────────────────────────────────────────────────────────────┘
```

**核心模式**:SSOT / 双层存储(SQLite + settings.json) / 双向同步(写 live 与读 live backfill) / 原子写(临时文件 + rename) / 分层架构(Commands → Services → DAO → Database)。

**关键模块**:ProviderService / McpService / ProxyService / SessionManager / ConfigService / SpeedtestService。魔改版新增:统一网关(`proxy/gateway.rs`)、Live 配置保护(`live_protection.rs`)、Codex auth 同步(`services/codex_auth_sync.rs`)。

</details>

<details>
<summary><strong>开发指南</strong></summary>

### 环境

Node.js 18+ / pnpm 8+ / Rust 1.85+ / Tauri CLI 2.8+

### 命令

```bash
pnpm install
pnpm dev                  # 开发(热重载)
pnpm typecheck
pnpm format
pnpm test:unit
pnpm tauri build          # 打包
```

Rust 端:

```bash
cd src-tauri
cargo fmt
cargo clippy
cargo test
```

### 技术栈

**前端**: React 18 · TS · Vite · TailwindCSS · TanStack Query v5 · react-i18next · react-hook-form · zod · shadcn/ui

**后端**: Tauri 2.8 · Rust · serde · tokio · thiserror · axum · hyper

</details>

<details>
<summary><strong>项目结构</strong></summary>

魔改版新增/修改的部分标 **🆕**:

```
src/                        # 前端 (React + TS)
├── components/
│   ├── proxy/              # 代理面板
│   │   └── ProxyPanel.tsx  # 🆕 常驻可见 + live 保护开关
│   └── settings/           # 设置面板
│       ├── GatewaySettingsPanel.tsx       # 🆕
│       ├── CopilotOptimizerPanel.tsx      # 🆕
│       ├── ClaudePluginStatusPanel.tsx    # 🆕
│       └── LightweightModeSettings.tsx    # 🆕
├── config/featureFlags.ts  # 🆕 ENABLE_CLOUD_SYNC 开关
└── lib/api/                # 后端命令前端封装
    ├── gateway.ts          # 🆕
    ├── copilotOptimizer.ts # 🆕
    ├── claudePlugin.ts     # 🆕
    └── lightweight.ts      # 🆕

src-tauri/src/
├── live_protection.rs                   # 🆕 Live 文件 SHA256 保护
├── proxy/
│   ├── gateway.rs                       # 🆕 统一网关
│   └── providers/streaming_anthropic_chat.rs  # 🆕 Chat↔Anthropic SSE
├── services/
│   └── codex_auth_sync.rs               # 🆕
├── commands/gateway.rs                  # 🆕
└── database/schema.rs                   # 🆕 fork 迁移 v15→v16→v17；当前 Schema v18

scripts/sync-upstream.sh                 # 🆕 一键同步上游
docs/CUSTOM_FORK_PLAN.md                 # 🆕 规划文档
```

</details>

## 协议

MIT © Jason Young

魔改版变更在原协议下分发,无任何额外限制。个人 fork 的 `m*` tag 仅提供 Windows Portable 预发行包；macOS、Linux 与正式安装包请使用上游发布版。
