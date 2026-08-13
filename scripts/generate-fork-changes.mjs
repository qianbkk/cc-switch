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
const baseFlag = args.indexOf("--base");
let base = baseFlag >= 0 && args[baseFlag + 1] ? args[baseFlag + 1] : null;

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

const commitsTotal = git(["rev-list", "--count", "main", "--not", base]);
const commitsNonMerge = git([
  "rev-list",
  "--count",
  "--no-merges",
  "main",
  "--not",
  base,
]);
const shortstat = git(["diff", "--shortstat", base, "main"]);

if (commitsTotal === null || commitsNonMerge === null || shortstat === null) {
  console.error("❌ git 数据采集失败（rev-list / diff 不可用）");
  process.exit(1);
}

// 解析 " 119 files changed, 11367 insertions(+), 2788 deletions(-)"
const filesM = shortstat.match(/(\d+) files? changed/);
const addM = shortstat.match(/(\d+) insertions?\(\+\)/);
const delM = shortstat.match(/(\d+) deletions?\(-\)/);
const diffFiles = filesM ? filesM[1] : "0";
const diffAdded = addM ? addM[1] : "0";
const diffRemoved = delM ? delM[1] : "0";

const statsDate = new Date().toISOString().slice(0, 10);

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
