#!/usr/bin/env node
/**
 * 版本一致性校验（路线图第 19 项：版本信息单一来源）。
 *
 * 规则：
 * 1. package.json / src-tauri/tauri.conf.json / src-tauri/Cargo.toml 三处
 *    `version` 必须完全一致（三处不同步会导致产物文件名、更新检查、About 页
 *    显示互相矛盾）。
 * 2. 发布构建（env `CC_SWITCH_FORK_RELEASE_TAG` 存在，形如 `m3.19.1-3`）时，
 *    tag 中的基础版本必须等于三处 version——错误 tag 直接阻止发布。
 *
 * 用法：node scripts/check-version-sync.mjs
 * 退出码：0 = 通过；1 = 不一致（打印具体差异）。
 */
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const pkg = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"));
const tauriConf = JSON.parse(
  readFileSync(resolve(root, "src-tauri", "tauri.conf.json"), "utf8"),
);
const cargoToml = readFileSync(
  resolve(root, "src-tauri", "Cargo.toml"),
  "utf8",
);
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

const sources = [
  ["package.json", pkg.version],
  ["src-tauri/tauri.conf.json", tauriConf.version],
  ["src-tauri/Cargo.toml", cargoVersion],
];

const failures = [];
for (const [file, v] of sources) {
  if (!v) {
    failures.push(`${file} 缺少 version`);
  } else if (v !== pkg.version) {
    failures.push(
      `${file} version=${v} 与 package.json(${pkg.version}) 不一致`,
    );
  }
}

// 发布构建：校验 fork tag 基础版本与三处一致
const forkTag = process.env.CC_SWITCH_FORK_RELEASE_TAG;
if (forkTag) {
  const m = forkTag.match(/^m(\d+\.\d+\.\d+)-(\d+)$/);
  if (!m) {
    failures.push(
      `CC_SWITCH_FORK_RELEASE_TAG=${forkTag} 格式非法（应为 m<base>-<rev>，如 m3.19.1-3）`,
    );
  } else if (m[1] !== pkg.version) {
    failures.push(`fork tag 基础版本 ${m[1]} 与应用版本 ${pkg.version} 不一致`);
  }
}

if (failures.length > 0) {
  console.error("❌ 版本一致性校验失败：");
  for (const f of failures) console.error(`   - ${f}`);
  process.exit(1);
}
console.log(
  `✅ 版本一致性通过：${pkg.version}${forkTag ? ` (tag ${forkTag})` : ""}`,
);
