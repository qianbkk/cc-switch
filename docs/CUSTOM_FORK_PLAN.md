# CC Switch 魔改版 — 总体规划与执行状态

> 本文档是本地魔改版（`main` 分支，原 custom 分支）的唯一规划文档，供后续会话/开发接续执行。
> 最近更新：2026-07-26（合入上游 v3.18.0，Schema v18；分支重命名 custom→main）

---

## 1. 项目背景与目标

- 基于上游 [cc-switch](https://github.com/farion1231/cc-switch) 最新源码做本地魔改版。
- **要求长期跟踪上游更新**，因此所有改动遵循低冲突原则（见 §3）。
- 用户决策：14 个功能板块**全部保留**，唯一裁剪是云同步（只隐藏 UI 入口，后端保留）。

## 2. 仓库与分支结构

| 项 | 值 |
|---|---|
| 本地路径 | `/d/AI/A_shared_workspace/ccswitch-issue/cc-switch` |
| remote `origin` | github.com/qianbkk/cc-switch（本 fork） |
| remote `upstream` | github.com/farion1231/cc-switch（原仓库） |
| 本地 `main` 分支 | **魔改主分支**，GitHub 默认分支，daily 工作区 |
| 本地 `upstream` 分支 | 上游镜像，跟 `farion1231/main` 同步，**永不直接改动** |
| 旧 `custom` 分支 | 2026-07-26 已合并/重命名为 main |

跟进上游流程：`bash scripts/sync-upstream.sh` 会自动 git fetch → `upstream` 上 merge upstream/main → `main` 上 merge upstream，冲突集中在少数枢纽文件（lib.rs、commands/mod.rs、SettingsPage.tsx、server.rs）。

## 3. 魔改原则（务必遵守）

1. **尽量新增文件，少改公共文件**——新功能放独立新模块/新组件。
2. 隐藏官方功能一律走 `src/config/featureFlags.ts` 的开关，**不删后端代码**。
3. 前端新 API 封装放 `src/lib/api/` 独立新文件；后端新 command 放 `commands/` 新文件。
4. 不动 `upstream` 分支；`main` 上提交信息用中文说明意图。

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
- `b131556d` feat(live-protection): 保护用户手动修改的 live 配置
  - 为 Live 备份增加原始 hash，并新增默认开启的保护开关。
  - 接入 Claude/Codex/Gemini/Grok Build 的接管、同步和切换写盘入口。
  - 前端提供保护开关和用户修改冲突提示。
- `705fa20e` feat(gateway): 补齐 Chat↔Anthropic SSE 流式转换
  - 新增 `streaming_anthropic_chat.rs`，覆盖文本、工具调用、usage、错误和非流式 JSON 兜底。
  - 网关三种入站协议与 Anthropic、OpenAI Chat、OpenAI Responses、Gemini 上游之间的流式链路已补齐。
- 当前工作树收尾：补充网关路由级鉴权/alias/model 冒烟测试；统一 `tower` 到 Axum 0.7 使用的 0.5 版本；Live 备份增加 `managed_hash`，避免把 CC Switch 自己的写盘误判为用户修改。

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

**Streaming（✅ 已补齐）**：

| 入站\上游 | Anthropic | OpenAI Chat | OpenAI Responses | Gemini |
|---|---|---|---|---|
| Anthropic | ✅ 0 转换 | ✅ Chat→Anthropic | ✅ streaming_responses | ✅ streaming_gemini |
| OpenAI Responses | ✅ streaming_codex_anthropic | ✅ Chat→Anthropic→Responses | ✅ 0 转换 | ✅ 链式 |
| OpenAI Chat | ✅ Anthropic→Chat | ✅ 0 转换 | ✅ Responses→Anthropic→Chat | ✅ Gemini→Anthropic→Chat |

✅ = 已有转换器或链式转换可用；Gemini 只作为上游协议，不作为统一网关入站协议。

## 6. 当前工具链状态

> 最近一次完整同步验证见本文末尾的同步记录；当前剩余待办是启动应用后的真实端到端冒烟。

- ✅ MSVC Build Tools 可用，Rust `cargo check` / `cargo build` 已验证
- ✅ 统一网关路由级鉴权、alias、model 冒烟测试已覆盖
- ✅ 前端 TypeScript 类型检查已验证
- ✅ 前端 Vitest 同时发现 `tests/**` 与 `src/**` 测试
- ⏸ 待办：启动应用后的真实端到端冒烟（需要真实上游或 mock upstream）

> 说明：测试总数会随上游同步增长，不在本节固定记录；以命令当次输出和下方按日期记录的验证快照为准。

## 7. 后续待办（按优先级）

### 7.1 Live 配置保护（用户需求 #2）✅ 已完成
**症状**：用户在 Claude/Codex/Gemini 的 live 配置文件（如 `~/.codex/config.toml`）做手动修改后，重启 ccswitch 或切换供应商时，修改被接管写盘覆盖。

**根因**（已确认）：
- 所有 live 写都通过 `config::write_json_file`/`write_text_file`/`atomic_write`（`config.rs:274-344`），全部**全量替换**（temp+rename，无 merge）
- `proxy_live_backup` 表只存接管前原始内容（无 hash/mtime），写盘时**不校验用户是否修改过**
- Codex 是唯一用 `toml_edit::DocumentMut`（保留未知字段）的写入器，但其他写者（全量替换）会覆盖其结果
- `services/config.rs` 的旧导入路径已被 `live.rs` 取代，但 `live.rs` 的 `sanitize_claude_settings_for_live` 只删除 cc-switch 自有字段
- 多个写者（`takeover_live_config_strict`、`sync_*_live`、`ProfileService::apply`）都会触发覆盖

**推荐方案（最小侵入）**：
1. `proxy_live_backup` 表同时保存 `original_hash` 和最近一次应用写入的 `managed_hash`。
2. `live_protection.rs` 封装校验逻辑，默认开启 `protect_user_live_edits`。
3. 接入接管、热切换和代理运行期间的同步写盘；只在当前文件 hash 不等于最近一次 CC Switch 写入 hash 时拒绝覆盖。
4. 前端提供保护开关和用户修改冲突提示；用户关闭保护后可恢复强制接管行为。

**改动面**：
- 数据库：`schema.rs` 加列 + `dao/proxy.rs` 加 hash 字段；migration 旧数据可填空
- 新增：`src-tauri/src/live_protection.rs`（核心校验 + 错误类型）
- 修改：`services/proxy.rs` `takeover_live_config_strict` + `restore_live_config_for_app_with_fallback_inner`（按开关决定是否校验）+ `sync_*_live` 入口
- 前端：`ProxyPanel.tsx` 增加"实时写入保护"开关；写失败时弹自定义 Toast

**预估**：1 天工作量；改动跨 ~5 个 Rust 文件 + 1 个前端文件

### 7.2 网关响应 SSE 增强 ✅ 已完成
新增 `providers/streaming_anthropic_chat.rs`，完成 Chat↔Anthropic SSE 转换，并接入网关全部协议链路。

**预估**：1-2 天工作量。

### 7.3 按应用禁用代理 UI 优化（用户需求 #1）✅ 已完成
**调研结论**：后端 per-app API `set_proxy_takeover_for_app(app, false)` 已完整可用，会自动恢复对应 app 的 live 配置。前端 `ProxyPanel` 已有四应用独立 Switch。**95% 已实现，零后端改动**。

**已完成 UI 优化**：
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
| Live 配置保护 | `src-tauri/src/live_protection.rs` |
| 写入器（全量替换） | `src-tauri/src/config.rs:274-344`（atomic_write） |

## 9.5. Release 流程（魔改版专用）

**目标**：在自己的 fork 仓库 `qianbkk/cc-switch` 发布 Windows 绿色版（Portable.zip），无需代码签名证书，给自己和朋友用即可。

**为什么不复用上游 `release.yml`**：上游在 push `v*` tag 时跑完整 5 平台矩阵 + Apple/Windows 签名 + Tauri signing key，硬编码依赖 GitHub Secrets，没配就会失败。

**专用 workflow**：`.github/workflows/release-portable.yml`
- **trigger**: push tag 匹配 `m*`（如 `m3.18.0-1`、`m3.19.0-1`...）
- **runner**: 单机 `windows-2022`
- **产物**: `CC-Switch-{tag}-Windows-Portable.zip`（仅可执行 exe + `portable.ini` 标记）
- **不签**: 跳过 `.msi` 安装器、Apple notarization、Tauri signing key；用环境变量 `CSC_IDENTITY_AUTO_DISCOVERY=false` 阻止 Tauri 找证书

**Tag 命名规则**：

- `m<upstream-version>-<patch>`
- 例：基于 upstream v3.18.0 的第 1 个魔改版：`m3.18.0-1`
- 合并 v3.19 后：`m3.19.0-1`...
- `m` 前缀明确是 fork 用的魔改版本,**绝不与上游 `v3.x.y` 撞名**
- 历史 `custom-v3.18.0-*` 标签保留（已下载用户的链接继续有效），但**不再发布新 `custom-*`**

**发布命令**：

```bash
# 在 main 分支(魔改主分支)上、状态干净时
git checkout main
git tag m3.18.0-1
git push origin m3.18.0-1
```

GitHub Actions 自动 build → 8-15 分钟 → 在 [Releases 页](https://github.com/qianbkk/cc-switch/releases) 生成 pre-release → 下载 Portable.zip 解压双击 `CC Switch.exe` 即可。

**版本号策略**：`Cargo.toml` / `package.json` / `tauri.conf.json` 保持 `3.18.0` 不变；魔改版版本号走 **tag 后缀**（`-1`、`-2`...）—— 这样永远跟上游版本对齐，避免合并后产生版本号分歧需要回填。

**之前发的 `custom-v3.18.0-3` 处理**：保持原 release 不删（用户已下载链接有效），加 deprecation 注释（可选）。

**Fork Actions 启用**：Settings → Actions → General → "Allow all actions and reusable workflows"（默认 fork 是关闭的）。

**配额**：GitHub 每月免费 2000 分钟，单次 Windows build 约 8-15 分钟（首次拉依赖 ~20 分钟），足够个人使用。

## 9.6. Fork 仓库规范（与上游的边界）

上游仓库的社区文件默认都指向 farion1231，直接继承到 fork 里会出错（PR 等不到
审核、赞助按钮指向别人、安全漏洞报错地方）。已做如下 fork 化改造：

| 文件 | 改动 | 原因 |
|---|---|---|
| `.github/CODEOWNERS` | `@farion1231` → `@qianbkk` | 否则本仓库 PR 永远等不到可能的审核 |
| `.github/FUNDING.yml` | 已删除 | 魔改版已剔除赞助内容，不该再挂 Sponsor 按钮 |
| `.github/ISSUE_TEMPLATE/config.yml` | 重写入口 | 安全问题走本仓库；上游问题引导去上游 |
| `SECURITY.md` | 顶部加 fork 横幅 | 区分「魔改独有代码漏洞」和「上游代码漏洞」的报告去向 |
| `SUPPORT.md` | 顶部加 fork 横幅 | 引导：怀疑魔改导致的先关总开关自测 |
| `CONTRIBUTING.md` | 顶部加 fork 横幅 | 引导：改进产品本身请提给上游，魔改部分才提这里 |
| `.github/workflows/stale.yml` | 注释掉 cron | 个人 fork 的 issue 很少，不该被机器人每天自动关 |
| `.github/workflows/claude.yml` | job 级 env 中转 + step 级判空 | fork 没有 `CLAUDE_CODE_OAUTH_TOKEN`，缺了会红；判空后跳过，配上自动生效 |

**踩过的坑**：`secrets` 上下文在 **job 级 `if` 和 step 级 `if` 里都不可用**。
直接写 `if: secrets.X != ''` 会让整个 workflow 文件语法失效——GitHub 会在
每次 push 时生成一条 event=push 的失败记录（workflow 名显示成文件路径，就是这个症状）。
正确写法是先在 job 级 `env` 里中转：

```yaml
jobs:
  claude:
    env:
      HAS_CLAUDE_TOKEN: ${{ secrets.CLAUDE_CODE_OAUTH_TOKEN != '' }}
    steps:
      - if: env.HAS_CLAUDE_TOKEN == 'true'
```

`env` 才是 step 级 `if` 支持的上下文。

**维护提醒**：这些文件被改过，未来 `sync-upstream.sh` 合并上游时可能冲突。
冲突时的处理原则是**保留 fork 侧的身份信息**（CODEOWNERS 的 @qianbkk、各横幅），
正文部分取上游的新版本。

## 9.7. 魔改功能总开关

**位置**：设置 → 通用 → 魔改功能 → 「启用魔改功能」

**作用**：一键回退到上游原版行为，用于排查「问题是不是魔改引入的」。

| 层 | 关闭时的行为 |
|---|---|
| 统一网关 | `authorize()` 前置守卫直接返回 403 |
| Live 配置保护 | `get_protect_user_live_edits()` 返回 false，写盘不再校验 hash |
| Codex auth 反向同步 | `maybe_sync_codex_auth()` 提前 return |
| 魔改 UI | 4 个魔改面板（网关 / Copilot 优化 / 轻量模式 / Claude 插件状态）隐藏 |
| 被隐藏的上游入口 | 云同步入口重新出现（`ENABLE_CLOUD_SYNC \|\| !forkEnabled`） |

**数据**：一律保留，重新打开即恢复。这不是卸载，只是"暂时按原版跑"。

**实现**：
- 后端 `AppSettings.fork_features_enabled`（默认 true）+ `settings::fork_features_enabled()` 访问器
- 各魔改入口统一调这个访问器做守卫
- 前端 `settings.forkFeaturesEnabled`，`SettingsPage` 用 `forkEnabled` 控制面板显隐

## 10. 上游同步日志

### 2026-07-26 合入上游 v3.18.0（878c26f3）

**冲突解决**：4 处（与计划完全一致）
1. `README.md` → 全取 ours（fork 魔改版）
2. `src-tauri/src/database/mod.rs` → `SCHEMA_VERSION = 18`
3. `src-tauri/src/database/schema.rs` → 迁移编号重排：原 `15→16`（Codex 用量重建）改名为 `17→18`；新增 match 分支 `17 =>`；fork 的 `15→16`/`16→17`（original_hash/managed_hash）保留。两个测试都保留：ours `migrate_v15_to_v17_adds_live_backup_hash_columns`（断言 `SCHEMA_VERSION`）、upstream 改名为 `migrate_v17_to_v18_resets_only_codex_session_usage`（初始版本号 17，最终断言改 `SCHEMA_VERSION`）
4. `src-tauri/src/services/proxy.rs` → 两处 Grok 接管 hunk：上游 `grok_live_config_supports_takeover` 守卫包裹 fork 的 `record_managed_hash`；严格模式分支保留 `is_ok()` 结构

**Schema 状态**：v18。本地已迁移 DB（v17→v18）启动时一次性重建 Codex 用量。

**自上游带来的关键能力**：
- Windows 切换闪窗卡死修复
- 日志持久化 + 密钥脱敏
- 前端白屏留痕（`FrontendErrorBoundary` + `frontendLogger`）
- Codex 工具参数兼容
- xAI/Grok OAuth 全家桶（`subscription_grok.rs`、`xai_oauth_auth.rs`、`XaiOAuthSection.tsx`、预设库）
- Grok 接管：官方登录态守卫 `grok_live_config_supports_takeover`

**fork 独有保留**：
- 统一网关 `/gateway/*`（4 路由、ccs- 鉴权）
- Live 保护 `record_managed_hash` + 冲突 toast
- Codex auth.json 反向同步
- 3 隐藏功能补 UI

**与上游意外冲突**：`GROKBUILD_OFFICIAL_PROVIDER_ID` 在上游首次被引用但通过 `database::dao::providers_seed` 定义，合并后两个引用点（`commands/provider.rs`、`services/provider/live.rs`）需要从 `crate::database` 顶层取，**已在 `database/mod.rs` 的 `dao::providers_seed` re-export 列表中显式导出**（一处非冲突 fix，编译无该常量失败 6 处）。

**验证**：
- `cargo test --lib` 2202 通过 / 0 失败 / 2 忽略（迁移测试 5 项全绿，含重命名的 `migrate_v17_to_v18_resets_only_codex_session_usage`）
- `cargo check` 干净
- `tsc --noEmit` 干净
- 前端 `pnpm test:unit`：73 文件 462 通过 2 失败 —— 仅 `tests/integration/App.test.tsx` MSW 集成测试的两个用例在全套并发下超 5s 默认超时（单跑全过）。已确认非本次合并回归。

### 2026-07-26 同步上游 b0482320（878c26f3..b0482320）

**新增 4 commit**：

| SHA | 类型 | 内容 |
|---|---|---|
| `b0482320` | chore | 5 家赞助商域名换 `.ai` 后缀（PackyCode、RightCode、ClaudeAPI→apito.ai、APINebula、SudoCode）+ 删 1 个失效 endpoint |
| `9cf4ae41` | feat | 内置定价表加 `claude-opus-5`（$5/$25 MTok），仅 `schema.rs` +2 行，无 schema bump |
| `876e9f89` | feat | 恢复 AICoding (aicoding.sh) 合作伙伴预设覆盖 7 个 app + 加进 3 README + zh/en/ja locale |
| `414b7150` | ci | release 产物镜像到 Cloudflare R2（dl.ccswitch.io），新增 `scripts/generate-download-manifest.mjs`；无 R2 secrets 时自动跳过 |

**冲突解决**（共 3 文件，README.md 6 处、其余 0）：

1. **`README.md`** — 上游把赞助商列表塞进 fork 既有的 6 处段落（特性列表 / 截图区 / 三个魔改功能小节末尾 / 同步脚本段）。**全取 ours**，上游赞助段全剔除，魔改立场一致；AICoding 自然也在其中一并删。
2. `README_DE.md` / `README_JA.md` / `README_ZH.md` — git 报"自动合并"，但 `876e9f89` 把 AICoding `<tr>` 块加进了三语赞助商表。手动从 `README_ZH.md` L84-87、`README_JA.md` L84-87 删除 AICoding 块（DE 此 commit 未涉 AICoding，无需动）。

**Schema 状态**：仍 v18，新 commit 无 schema 改动。

**自上游带来的关键能力**：
- 5 家赞助商域名 `.com → .ai` 修复（防止失效 link）
- `claude-opus-5` 价格条目（用户用量面板可自定义或默认同步）
- Cloudflare R2 镜像（不影响 fork，仅上游官方下载用得到）

**fork 独有保留**：
- 顶部 fork 声明块（前 4 README 仍在）
- 4 块魔改功能（统一网关 / Live 保护 / Codex auth 反向同步 / 3 隐藏 UI）
- AICoding 合作伙伴**预设保留**（fork 的 7 个 presets 文件已含 AICoding 选项），仅从 3 个 README 删除推广段落

**GitHub 状态**：
- `upstream/main` HEAD = `b0482320`
- `origin/upstream` HEAD = `b0482320`
- `origin/main` HEAD = `a76e355c`（merge commit），已推

**可视化**：`D:\AI\A_shared_workspace\.claude-viz\ccsync-b0482320-viz.html`

**验证**：
- `cargo check` Finished `dev` profile in **15.80s**（0 error）
- `tsc --noEmit` 0 output（干净）

---

## 临时接续记录（2026-07-27）

已完成并提交：文档事实修订、陈旧 PRD/设计稿归档、Vitest 补回 `src/**` 测试发现、App 集成测试局部超时预算、Codex `auth.json` 损坏内容保护与跨平台原子写复用。

后续在其他设备上优先处理：

1. **Vitest open handle**：关键前端用例 62/62 通过，但进程在输出总结后偶发不主动退出；排查 Tauri window mocks、MSW `list_profiles` 未处理请求及监听器清理。
2. **完整验证**：重新执行 `pnpm test:unit`、`cargo test --lib`，必要时再跑完整 `cargo test`。
3. **前端构建环境问题**：常规 `vite build` 曾在清理 `dist` 时触发 WorkBuddy safe-delete shim 超时；源码 transform 已完成，可在普通终端/CI 复验正式构建。
4. **网关状态一致性**：`save_gateway_config` 当前先持久化 `enabled=true` 再启动代理；启动失败时应考虑回滚配置或调整事务顺序。
5. **测试覆盖补强**：补 Live 保护状态机测试，以及网关真实 mock upstream 的非流式/SSE 协议矩阵测试。

当前已知验证：TypeScript 类型检查通过；Codex auth 定点测试 8/8 通过；关键前端测试 62/62 用例通过；Rust 格式与 `git diff --check` 通过。
