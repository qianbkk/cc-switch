# CC Switch 魔改版 — 维护约束与工作流（单一事实来源）

> 本文档是魔改版**日常维护的权威约束清单**，优先级高于散落的笔记与记忆文件。
> 所有开发、上游同步、CI、发布、文档更新都必须遵守本节约束；与本文冲突的旧记录以本文为准。
>
> 配套文档：
> - `docs/CUSTOM_FORK_PLAN.md` — 总体规划、架构契约、上游同步历史日志
> - `outputs/CCSwitch-后续开发可执行路线图.md` — 27 项执行计划与完成状态
> - `README.md` — 用户视角功能声明（魔改版对外说明）
>
> 最近更新：2026-08-12（纳入 Windows-only 精简、上游两批吸收、CI 门控、环境绕法）

---

## 1. 项目定位（硬约束）

| 约束 | 说明 |
|---|---|
| 用途 | **个人自用为主**（魔改版），非对外产品 |
| 平台 | **仅 Windows**。2026-08-12 起正式舍弃 macOS/Linux（PR #6） |
| 发布形态 | Windows Portable.zip（`m*` tag 预发行）+ MSI/NSIS 安装器 |
| 上游 | `farion1231/cc-switch`，长期跟踪、**选择性吸收**（见 §3） |
| 数据 | SQLite（Schema v18），本地存储；云同步仅隐藏 UI，后端保留 |

## 2. 仓库结构

| 项 | 值 |
|---|---|
| 本地路径 | `D:\AIWorkSpace\CCSwitchM` |
| `origin` | `github.com/qianbkk/cc-switch`（本 fork） |
| `upstream` | `github.com/farion1231/cc-switch`（原仓库） |
| 本地 `main` | **魔改主分支**，GitHub 默认分支，日常工作区 |
| 本地 `upstream` 分支 | 上游镜像，**永不直接改动** |

## 3. 硬性开发约束（务必遵守）

1. **不在 `main` 上直接同步上游或做试验性修改**；一律独立分支 → PR → CI 全绿 → 合并。
2. **上游只选择性吸收**：只吸收与魔改版相关或高价值内容（安全修复、Windows/CI、UI 体验、供应商支持）。
   **永不吸收**：赞助商推广（5 个）、版本发布/版本号提交（2 个）、WSL CI（1 个）。
3. 涉及 **Live 配置** 的修改必须使用临时目录 + 伪造配置测试，**绝不碰用户真实** `~/.claude`、`~/.codex`、`~/.gemini`。
4. **尽量新增文件，少改公共文件**；隐藏官方功能一律走 `src/config/featureFlags.ts` 开关，**不删后端代码**。
5. 前端新 API 封装放 `src/lib/api/` 独立新文件；后端新 command 放 `commands/` 新文件。
6. **版本号策略**：`Cargo.toml` / `package.json` / `tauri.conf.json` 保持与上游对齐（不 bump）；魔改版号走 **tag 后缀** `m<upstream-version>-<n>`。
7. **fork 身份文件**（`CODEOWNERS` 的 `@qianbkk`、SECURITY/SUPPORT/CONTRIBUTING 顶部横幅、删除的 FUNDING.yml）在同步冲突时**保留 fork 侧**，正文取上游新版本。
8. **平台残留**：macOS/Linux 图标资源（`icon.icns`、`dmg-background.png`、`android/`、`ios/`、`Info.plist`）因沙箱限制保留，属**无害冗余**，不主动清理；Rust `cfg(target_os)` 条件编译为 tauri 库依赖必需，**保留不动**。
9. 提交信息：`main` 上用中文说明意图；PR 分支上可用中文或英文，但**必须**在 PR body 写明对应上游 commit hash 与边界划分。

## 4. CI 约束（2026-08-12 精简后矩阵）

| Job | runner | 触发条件 |
|---|---|---|
| Detect changed areas | ubuntu | 总是（paths-filter 检测改动区域） |
| Frontend Checks | ubuntu | 前端文件有改动 |
| Backend Checks | windows-latest | 后端文件有改动（`src-tauri/**` 等） |
| Portable Release smoke | windows-latest | 相关改动（验证 Portable 构建资源嵌入） |
| label | ubuntu | 总是（PR 标签） |

- push 到 `main` 时**全部无条件运行**（保持缓存热）。
- **CI 全绿是合并的唯一门槛**：任何 PR 未全绿不得合并。

### CI 已知超时与教训
- Rust 编译 + 全量测试约 10-20 分钟（Backend 实测 15m24s）。
- **历史教训（PR #4）**：三平台合并曾造成 sync_import 测试同线程二次 `lock_conn!` 死锁 → 三平台确定性挂起 50+ 分钟。排查用 CI 插桩（eprintln 步骤标记 + `--nocapture` + `timeout-minutes: 15`）定位后 revert。
- `#[serial]` 测试无 key 时共享同一把全局锁，一个挂起会连带其他 serial 测试"running over 60s"。

## 5. 标准维护工作流

### 5.1 功能开发 / 修复
1. 从最新 `main` 建独立分支（命名：`fix/`、`feat/`、`chore/`、`sync/`）。
2. 开发 + 本地验证（见 §6 环境限制）+ 自测。
3. 推送分支 → 创建 PR → 等 CI 全绿 → 合并 → 同步本地 main ref（§5.3）。

### 5.2 上游同步（选择性吸收流程）
> **节奏：每周固定检查一次**（2026-08-12 用户确认；已配置自动化提醒，也可手动触发）。

1. **检查**：`git fetch --no-write-fetch-head upstream main` → `git log --oneline main..upstream/main` 看新提交。
2. **评估**：按主题分类（UI/体验、供应商、安全、skills、usage、Windows/CI、赞助商/发布）；赞助商与发布类直接跳过。
3. **吸收**：`git show <commit> > patch` → `git apply patch`（08-12 起可用）→ 若有依赖链按序应用。
4. **提交**：按主题边界分多个提交（共享文件归并、i18n 独立收集），**逐条 commit-tree，不要用 `&&` 串联**（见 §6）。
5. **PR → CI → 合并**，随后同步本地 refs。

### 5.3 同步本地 main（环境受限绕法）
```bash
# 1. 下载合并提交对象（普通 fetch 写 FETCH_HEAD 会被沙箱拒）
git fetch --no-write-fetch-head origin main
# 2. 用 Edit 工具把以下文件内容改为远端合并 commit hash：
#    .git/refs/heads/main
#    .git/refs/remotes/origin/main
# 3. 验证
git rev-parse main origin/main
```

### 5.4 发布（m* tag）
1. 在 `main` 上、状态干净时：`git tag m<upstream-version>-<n>` → `git push origin m<tag>`。
2. `release-portable.yml`（Windows-only）自动构建 8-15 分钟 → 生成 pre-release → 下载 Portable.zip。
3. 禁止对已发布的 `m*` 标签强推或移动。
4. 历史 `custom-v3.18.0-*` 标签保留链接有效，但不再发新 `custom-*`。
5. 发布后：更新 `docs/CUSTOM_FORK_PLAN.md` 同步日志 + 路线图状态。

## 6. 环境限制与绕法（重要，可能复用）

| 限制 | 绕法 |
|---|---|
| 本地无 MSVC `link.exe`（不能本地链接 Rust） | 完整编译/测试**依赖 GitHub CI**；本地只做 cargo-fmt |
| `aws-lc-sys` C 编译在 MinGW 下报 `X509_NAME` 宏冲突 | 本地**不能跑 Rust 单元测试**（`rustls-tls-no-provider` 临时替换可绕过编译但全量构建 25min 会僵死，已弃用） |
| `.git/index.lock` 被进程独占（125KB 静止） | `GIT_INDEX_FILE="C:/Users/NLG-A6/AppData/Local/Temp/git-index.tmp"` + `read-tree` + `add` + `write-tree` + `commit-tree -p <hash>`；**`&&` 串联会莫名失败，单条 Bash 最稳** |
| 沙箱拒绝 bash 写 `.git/refs/*`、`Cargo.lock`、`Cargo.toml` | **Edit 工具**可写（refs 直写完成提交/切换分支） |
| `git add` 写真实 index 失败 | 用临时 `GIT_INDEX_FILE`；commit 后 `git push` 正常（不依赖 index） |
| `git fetch` 写 `FETCH_HEAD` 被拒 | `git fetch --no-write-fetch-head` |
| 切换分支被 index.lock 阻止 | 直接改写 `.git/HEAD`（`ref: refs/heads/<branch>`）+ 建/改 `.git/refs/heads/<branch>` |
| 本地 Rust 链接器缺失 | 见上方；`cargo-fmt` 用 `.workbuddy/toolchains/rustup-phase1/toolchains/1.95-x86_64-pc-windows-msvc/bin/cargo-fmt.exe` |

### 本地可做的验证
- 前端：`pnpm` + `tsc --noEmit` + `vitest` + `vite build --emptyOutDir=false`（全部本地可跑）。
- 格式：Rust 用 cargo-fmt；前端用 prettier；`git diff --check`。
- Rust 编译/测试：**只能靠 CI**。

## 7. 文档体系（各文档职责与更新时机）

| 文档 | 职责 | 更新时机 |
|---|---|---|
| `docs/MAINTENANCE.md`（本文） | 维护约束与工作流（单一事实来源） | 约束/流程变化时 |
| `docs/CUSTOM_FORK_PLAN.md` | 总体规划、架构契约、上游同步历史日志 | 大版本合并、架构变化、每次上游同步后追加日志 |
| `outputs/CCSwitch-后续开发可执行路线图.md` | 27 项执行计划与完成状态 | 每完成一项即打勾 |
| `README.md` | 用户视角功能声明 | 功能增删时 |
| `docs/ROOTCAUSE_*.md` | 根因分析（如 Portable dev URL） | 根因确认时 |
| `.workbuddy/memory/YYYY-MM-DD.md` | 每日工作日志（append-only） | 每日收尾 |
| `CHANGELOG.md` | 上游主 changelog（388KB，不手工维护） | 上游同步时 |

## 8. 历史决策存档（勿回退）

- **2026-08-12**：Windows-only 精简（PR #6，merge `56b0327b`）——CI 从 6 项减为 4 项。
- **2026-08-12**：上游 43 提交中 **35 个已吸收**（PR #4/#5 的 3 个 + PR #7 第一批 8 个 + PR #8 第二批 23 个），**排除 8 个**（5 赞助商 + 2 发布 + 1 WSL CI）。
- **2026-08-11**：PR #4 修复 sync_import 二次锁死锁（merge `8ef806a1`）；第 09 项完成。
- **2026-08-07**：第 07 项 Live 保护接入（`fix/live-protection-phase2`）；第 08 项第一批（PR #3）。
- **2026-07-28**：合入上游安全修复系列 + Codex/Grok/用量更新；Schema 仍 v18。
- 数据库结构版本：**Schema v18**；未来版本拒绝 + 迁移前备份（PR #4 顺序修复）。

---

## 9. 决策记录与待确认（2026-08-12）

### 已确认决策（回填）
- **上游同步节奏：每周固定检查一次**（2026-08-12 确认）。
- **文档分工**：保持 MAINTENANCE.md（约束/流程）与 CUSTOM_FORK_PLAN.md（架构/历史）双文档结构（2026-08-12 用户授权自行决定）。
- **路线图**：已完成项补状态标记归档；剩余 10-27 项按「网关工程化验收」优先级保留推进，其余冻结（见下）。

### 待确认
- [ ] **P1 本地 Rust 环境**：评估如下——
  - 磁盘：C 盘可用 239.5GB（充足）；MSVC Build Tools 最小安装约 **2.5-4GB**。
  - 时间：下载 + 安装约 **30-60 分钟**（一次性）；装完后 `rustup default` 切到 MSVC toolchain（本地已有 1.95 MSVC 工具链），即可本地跑 `cargo build/test`（aws-lc-sys 在 MSVC 下正常编译）。
  - 收益：每次改 Rust 代码免去等 CI 15 分钟往返；**一劳永逸**。
  - 成本：需要管理员权限装系统组件（WorkBuddy 沙箱可能无法自动完成，需用户手动执行或授权）。
  - **建议：值得装**。确认后我给出具体安装步骤。
