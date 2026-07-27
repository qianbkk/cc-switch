# 2026-07-26 — 魔改版 vs 原版 可视化集成（历史设计）

> **归档说明（2026-07-27）**：功能已经实施，但最终方案与本文不同：HTML 实际位于 `src-tauri/assets/FORK_CHANGES.html`，通过 `include_str!` 编译进二进制，`open_fork_changes_html` 将内容写入临时文件后由系统浏览器打开；前端调用封装位于 `src/lib/api/settings.ts`。本文保留为设计演进记录，不作为当前路径或 API 的依据。

## 1. 目标

把"魔改版 vs 上游原版改动区别"做成**详细自包含 HTML**,集成到 cc-switch 魔改版桌面应用里,让用户(或任何用此 fork 的人)能在 Settings → 关于里**一键查看**:

1. 上游版本基线(例如 v3.18.0)
2. fork 独有的 4 大块魔改(统一网关 / Live 保护 / Codex auth 反向同步 / 3 隐藏功能 UI)
3. 上游 → fork 已吸收 / 未吸收 改动对比
4. 历次合并记录 + 上游后续待同步
5. 部署细节 / 使用方法
6. 链接到 GitHub repo / fork README

后续每次打新 tag(m3.18.0-1 → m3.18.0-2 → m3.19.0-1 ...)时,**只改一个 HTML 源文件 + 重新发版即可**,HTML 100% 跟随 binary 走。

## 2. 架构

```
[src/assets/FORK_CHANGES.html]            ← 单一源,手写维护
                │
                │ Vite build (默认 copy)
                ▼
[dist/assets/FORK_CHANGES.html]           ← 打包产物
                │
                │ Tauri bundler 嵌入 binary
                ▼
[app.path().resource_dir()/assets/FORK_CHANGES.html]
                │
                │ Rust command: get_fork_changes_html_url()
                ▼ 返回 String 绝对路径
                │
[AboutSection button onClick]
                │
                │ @tauri-apps/plugin-opener.openPath(url)
                ▼
[系统默认浏览器渲染 HTML]
```

## 3. 文件清单(改动 6 个,新增 2 个)

### 3.1 新增

| 路径 | 内容 |
|---|---|
| `src/assets/FORK_CHANGES.html` | 单文件,自包含(无外链 CDN/JS),中文,4-6 章节。**唯一需要随版本手工维护的文件** |

### 3.2 改动

| 路径 | 改动 |
|---|---|
| `src-tauri/src/commands/misc.rs` (或新文件) | 新加 `pub fn get_fork_changes_html_url(app: tauri::AppHandle) -> Result<String, AppError>` command,从 `app.path().resource_dir()` + `assets/FORK_CHANGES.html` 返回绝对路径 |
| `src-tauri/src/lib.rs` | 注册新 command 到 `tauri::generate_handler!` |
| `src/lib/api/misc.ts` (或新文件 `src/lib/api/changelog.ts`) | 加 `getForkChangesHtmlUrl(): Promise<string>` API 包装 |
| `src/lib/api/index.ts` | re-export 新 API |
| `src/components/settings/AboutSection.tsx` | 加 `<Button>查看魔改详情</Button>`,onClick: `const path = await api.getForkChangesHtmlUrl(); await openPath(path);` |
| `src/i18n/locales/{zh,en,ja,de}.json` | 加 `about.forkChangesButton` 文案 key |

## 4. 关键设计决策

### 4.1 为什么用系统默认浏览器而不是内嵌 iframe

- 内嵌 iframe 在 Tauri 里要处理 CSP / asset protocol / 同源策略,样式不能 100% 控制
- 系统浏览器拿到完整样式 + 字体 + 颜色,体验更好
- 用户可全屏 / 缩放 / 搜索 / 选中复制

### 4.2 HTML 资源定位:用 `app.path().resource_dir()`

Tauri 2.x 的标准做法:`app.path().resource_dir()` 在 dev 模式返回 `src-tauri/target/.../resources`,在生产模式返回 binary 旁边的 resources 目录。Vite build 时 `assets/` 默认 copy 到 dist/assets,Tauri bundler 自动把 `dist/` 整体当 resources 拷进去。

所以最终路径是 `resource_dir.join("assets/FORK_CHANGES.html")`。

### 4.3 不缓存,每次都读真实文件

命令直接返回绝对路径,不缓存 HTML 内容。改 HTML → 重新 build → 用户拿到的就是新内容。

### 4.4 HTML 必须自包含(无外链)

- 所有 CSS inline
- 所有 emoji / 字体 / 图标用 Unicode + 内嵌 base64 或 inline SVG
- 无 `<script src=...>`
- 无 `<link href=...>`(外部样式)
- 离线可双击 .html 直接打开

这样:
- 浏览器打开时不依赖网络
- 即使 fork 仓库下线,HTML 仍能正常显示

### 4.5 HTML 内容结构(4-6 章节)

参考 `~/.claude-viz/ccsync-b0482320-viz.html` 风格,新 HTML 章节:

1. **Header**:版本号 / fork 身份声明
2. **上游基线 + 历次同步**(v3.18.0 起点 → b0482320 已合 → 当前 fork HEAD)
3. **fork 独有的 4 块魔改**(每块独立卡片)
4. **上游吸收 / 未吸收 改动对比**(表格形式,每条 commit 一行)
5. **使用 / 部署 / 接入指南**(base_url + key + 4 路由 curl 示例)
6. **Footer**:文件 SHA + 仓库链接

## 5. 数据流

### 5.1 启动 → 无任何操作

HTML 在启动时**不**主动加载,直到用户点按钮。

### 5.2 用户点按钮

```
AboutSection.handleClick()
  └→ api.getForkChangesHtmlUrl()
        └→ invoke('get_fork_changes_html_url')
              └→ Rust: app.path().resource_dir()? + "assets/FORK_CHANGES.html"
              └→ return absolute path string
  └→ openPath(path)  // tauri-plugin-opener
        └→ 系统默认浏览器渲染
```

### 5.3 文件不存在时

Rust 端用 `std::fs::metadata` 检查文件存在性,不存在返回 `AppError::Message("魔改说明 HTML 缺失...")`,前端 toast 提示用户。

## 6. API 设计

### 6.1 Rust

```rust
#[tauri::command]
pub fn get_fork_changes_html_url(app: tauri::AppHandle) -> Result<String, AppError> {
    let resource_dir = app.path().resource_dir()
        .map_err(|e| AppError::Message(format!("无法解析 resource_dir: {e}")))?;
    let path = resource_dir.join("assets").join("FORK_CHANGES.html");
    if !path.exists() {
        return Err(AppError::Message(format!(
            "魔改说明 HTML 不存在: {}",
            path.display()
        )));
    }
    Ok(path.to_string_lossy().into_owned())
}
```

### 6.2 TypeScript

```typescript
// src/lib/api/changelog.ts
import { invoke } from "@tauri-apps/api/core";

export const changelogApi = {
  async getForkChangesHtmlUrl(): Promise<string> {
    return invoke<string>("get_fork_changes_html_url");
  },
};
```

### 6.3 前端调用

```tsx
// AboutSection.tsx (新增片段)
import { openPath } from "@tauri-apps/plugin-opener";
import { changelogApi } from "@/lib/api";

async function handleViewForkChanges() {
  try {
    const path = await changelogApi.getForkChangesHtmlUrl();
    await openPath(path);
  } catch (err) {
    toast.error(`打开魔改说明失败: ${err}`);
  }
}

<Button onClick={handleViewForkChanges} variant="outline" size="sm">
  <FileText className="mr-2 h-4 w-4" />
  {t("about.forkChangesButton")}
</Button>
```

## 7. i18n 文案

**4 语言各加一条**:

```json
// zh.json
"about": {
  "forkChangesButton": "查看魔改详情"
}

// en.json
"about": {
  "forkChangesButton": "View Fork Changes"
}

// ja.json
"about": {
  "forkChangesButton": "魔改の詳細を見る"
}

// de.json
"about": {
  "forkChangesButton": "Fork-Änderungen anzeigen"
}
```

## 8. 更新流程

用户(后续日常):

```bash
# 1. 改 src/assets/FORK_CHANGES.html(描述新魔改 / 新同步状态)
# 2. git commit + push
# 3. 打新 tag + push,触发 release-portable.yml
git checkout main
git tag m3.18.0-2
git push origin main m3.18.0-2
# 4. CI 跑完后,GitHub Releases 出现新 zip,内置最新版 HTML
```

**用户视角**:每次升级应用(`m3.18.0-1` → `m3.18.0-2`),点"查看魔改详情"自动看到最新版对比。

## 9. 测试

### 9.1 编译验证

```bash
cd src-tauri && cargo check
npx tsc --noEmit
```

### 9.2 dev 模式手动验证

```bash
pnpm tauri dev
# 点 Settings → 关于 → "查看魔改详情"
# 期望:系统默认浏览器打开,显示完整 HTML
```

### 9.3 离线可访问验证

```bash
# 断网后双击
# 期望:HTML 仍然能完整渲染(无 CDN/外链)
```

### 9.4 集成回归

跑 `cargo test --lib` 确认新增 command 不破坏现有 2202 个测试。

## 10. 风险与备案

| 风险 | 备案 |
|---|---|
| Windows 上 openPath 关联错误应用 | 改用 `openUrl("file://...")` 强制浏览器 |
| HTML 体积膨胀失控 | 单一文件,设上限 200 KB |
| 后续魔改多了忘记更新 HTML | 加 README 提示:"魔改后请同步更新 src/assets/FORK_CHANGES.html" |
| 用户字体看不清 | 强制最小 17px body / 22-28px h2 |

## 11. 实施顺序

1. 创建 `src/assets/FORK_CHANGES.html`(核心内容,首要)
2. Rust command + lib.rs 注册
3. 前端 API 包装 + AboutSection 按钮
4. 4 语言 i18n 文案
5. `cargo check` + `tsc --noEmit` 验证
6. commit + push 触发 CI
7. CI 通过后打 `m3.18.0-1` tag(已在跑,等完成)

## 12. 文件路径最终约定

| 项 | 路径 |
|---|---|
| 唯一源文件 | `src/assets/FORK_CHANGES.html` |
| Rust command | `src-tauri/src/commands/misc.rs` 内(若已存在,否则新建该文件) |
| TS API | `src/lib/api/changelog.ts`(新文件) |
| 按钮位置 | `src/components/settings/AboutSection.tsx` |