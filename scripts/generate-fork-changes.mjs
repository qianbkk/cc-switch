#!/usr/bin/env node
/**
 * 魔改详情页生成器（路线图第 20 项）。
 *
 * 从模板 `scripts/fork-changes/template.html` 生成
 * `src-tauri/assets/FORK_CHANGES.html`，动态数据全部来自 Git 现场数据：
 *
 * - Fork 独有提交数 / 非合并提交数：`git rev-list --count [--no-merges] main --not <base>`
 * - 差异文件数 / 新增 / 删除行数：`git diff --shortstat <base> main`
 * - 统计口径日期：生成当日
 *
 * 基线默认 `upstream/main`（对比上游口径）；不存在时 fallback `origin/main`
 * 并打印警告（如 CI 未 fetch 上游）。可用 `--base <ref>` 显式指定。
 *
 * 用法：
 *   node scripts/generate-fork-changes.mjs            # 默认 upstream/main
 *   node scripts/generate-fork-changes.mjs --base origin/main
 *
 * 退出码：0 = 生成成功且无残留占位符；1 = git 数据不可用或生成失败。
 */
import { readFileSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const TEMPLATE = resolve(root, "scripts", "fork-changes", "template.html");
const OUTPUT = resolve(root, "src-tauri", "assets", "FORK_CHANGES.html");

// ---- git 数据采集（任一失败则整体失败，禁止生成残缺页面）----

function git(args) {
  try {
    return execFileSync("git", args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      maxBuffer: 64 * 1024 * 1024,
    }).trim();
  } catch (e) {
    return null;
  }
}

const args = process.argv.slice(2);
const flagOf = (name) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? args[i + 1] : null;
};
const baseFlag = flagOf("--base");
// 统计对象（"当前 main"）：本地默认 main 分支；CI 的 PR 场景 checkout 是
// merge ref（无 main 分支），必须显式传 --head origin/main 才能与本地口径一致。
const headFlag = flagOf("--head") || "main";
let base = baseFlag;

if (!base) {
  // 默认基线：upstream/main；不存在则退回 origin/main（口径不同，明确警告）
  if (git(["rev-parse", "--verify", "upstream/main"])) {
    base = "upstream/main";
  } else if (git(["rev-parse", "--verify", "origin/main"])) {
    base = "origin/main";
    console.warn(
      "⚠️ 未找到 upstream/main，使用 origin/main 作为对比基线（口径不同）。",
    );
  } else {
    console.error("❌ 无法确定对比基线（既无 upstream/main 也无 origin/main）");
    process.exit(1);
  }
} else if (!git(["rev-parse", "--verify", `${base}^{commit}`])) {
  console.error(`❌ 基线 ${base} 不存在`);
  process.exit(1);
}
if (!git(["rev-parse", "--verify", `${headFlag}^{commit}`])) {
  console.error(`❌ 统计对象 ${headFlag} 不存在（PR 场景请传 --head origin/main）`);
  process.exit(1);
}

// 自我引用修复：若 head 提交是"纯生成物刷新提交"（只修改 FORK_CHANGES.html，
// 且提交信息以 docs(fork-changes) 开头），统计时回退到其 parent。
// 否则刷新提交本身会使提交数 +1：本地生成时 head 还是旧提交，提交刷新后
// CI 复现 head 已是新提交，数字永远差 1 → freshness 校验必失败。
function effectiveHead(headRef) {
  const msg = git(["log", "-1", "--format=%s", headRef]) || "";
  if (!msg.startsWith("docs(fork-changes)")) return headRef;
  const files = git(["show", "--name-only", "--format=", headRef]);
  if (files === null) return headRef;
  const touched = files.split("\n").filter((l) => l.trim() !== "");
  const onlyHtml =
    touched.length > 0 &&
    touched.every((f) => f === "src-tauri/assets/FORK_CHANGES.html");
  if (!onlyHtml) return headRef;
  const parent = git(["rev-parse", `${headRef}^`]);
  if (parent) {
    console.log(`  （head ${headRef} 为纯生成物刷新提交，按 ${parent} 统计）`);
    return parent;
  }
  return headRef;
}
const head = effectiveHead(headFlag);

const commitsTotal = git(["rev-list", "--count", head, "--not", base]);
const commitsNonMerge = git([
  "rev-list",
  "--count",
  "--no-merges",
  head,
  "--not",
  base,
]);
const shortstat = git(["diff", "--shortstat", base, head]);
// 口径日期取统计对象最近提交日期（而非“今天”）：保证生成器完全确定性，
// 任意时间重复生成结果一致，CI 的 freshness 校验才可靠。
const statsDate = git(["log", "-1", "--format=%cs", head]);
// 基线短 hash 写入页面：CI 复现时按页面记录的口径生成（消除上游漂移）
const baseCommit = git(["rev-parse", "--short", `${base}^{commit}`]);

if (
  commitsTotal === null ||
  commitsNonMerge === null ||
  shortstat === null ||
  statsDate === null ||
  baseCommit === null
) {
  console.error("❌ git 数据采集失败（rev-list / diff / log / rev-parse 不可用）");
  process.exit(1);
}

// 解析 " 119 files changed, 11367 insertions(+), 2788 deletions(-)"
const filesM = shortstat.match(/(\d+) files? changed/);
const addM = shortstat.match(/(\d+) insertions?\(\+\)/);
const delM = shortstat.match(/(\d+) deletions?\(-\)/);
const diffFiles = filesM ? filesM[1] : "0";
const diffAdded = addM ? addM[1] : "0";
const diffRemoved = delM ? delM[1] : "0";

// ---- 模板渲染 ----

let html;
try {
  html = readFileSync(TEMPLATE, "utf8");
} catch (e) {
  console.error(`❌ 读取模板失败: ${e.message}`);
  process.exit(1);
}

const placeholders = {
  COMMITS_TOTAL: commitsTotal,
  COMMITS_NON_MERGE: commitsNonMerge,
  DIFF_FILES: diffFiles,
  DIFF_ADDED: diffAdded,
  DIFF_REMOVED: diffRemoved,
  STATS_DATE: statsDate,
  BASE_COMMIT: baseCommit,
};

for (const [key, value] of Object.entries(placeholders)) {
  html = html.split(`{{${key}}}`).join(value);
}

// 生成物不应残留任何占位符
const leftover = html.match(/\{\{[A-Z_]+\}\}/g);
if (leftover) {
  console.error(
    `❌ 模板存在未替换占位符: ${[...new Set(leftover)].join(", ")}`,
  );
  process.exit(1);
}

writeFileSync(OUTPUT, html, "utf8");
console.log(
  `✅ 已生成 ${OUTPUT}\n` +
    `   基线 ${base}：${commitsTotal} 提交（${commitsNonMerge} 非合并）、` +
    `${diffFiles} 文件、+${diffAdded}/-${diffRemoved}（口径 ${statsDate}）`,
);
