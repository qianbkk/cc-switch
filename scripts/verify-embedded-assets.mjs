#!/usr/bin/env node
// Verifies that a release binary embeds the built frontend assets.
//
// Background: the portable release workflow must NOT be built with a plain
// `cargo build --release`, because that bypasses the Tauri CLI, which is what
// injects the `tauri/custom-protocol` feature. Without it the binary is
// compiled in "dev" mode: no frontend assets are embedded and the app loads
// `http://localhost:3000` (the dev server) at runtime — on a fresh machine
// that URL is unreachable and the app shows "无法访问此页面".
//
// This script is the automated gate for that regression:
//   1. Parses the asset names referenced by dist/index.html.
//   2. Asserts the referenced files actually exist in dist/assets.
//   3. Asserts the release executable's byte stream contains those asset
//      names (embedded assets are stored by their path key, e.g.
//      `assets/index-<hash>.js`, so the name appears as a plain string).
//   4. Exits non-zero if anything is missing, so CI refuses to publish a
//      broken package.
//
// Usage:
//   node scripts/verify-embedded-assets.mjs [--exe <path>] [--dist <dir>] [--check-dist-only]
//
// --check-dist-only  only validates dist integrity (fast; used by CI frontend job).

import { readFileSync, existsSync, readdirSync, statSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const args = process.argv.slice(2);
const getArg = (flag, fallback) => {
  const idx = args.indexOf(flag);
  return idx >= 0 && args[idx + 1] ? args[idx + 1] : fallback;
};

const checkDistOnly = args.includes("--check-dist-only");
const distDir = resolve(ROOT, getArg("--dist", "dist"));
let exePath = getArg("--exe", "");

if (!existsSync(distDir)) {
  console.error(`[verify-embedded-assets] dist directory not found: ${distDir}`);
  console.error("Run `pnpm build:renderer` first.");
  process.exit(1);
}

// 1) Collect asset file names referenced by index.html
const indexPath = join(distDir, "index.html");
if (!existsSync(indexPath)) {
  console.error(`[verify-embedded-assets] missing ${indexPath}`);
  process.exit(1);
}
const indexHtml = readFileSync(indexPath, "utf8");
const referenced = [...indexHtml.matchAll(/(?:src|href)="\.\/([^"]+)"/g)].map(
  (m) => m[1],
);
if (referenced.length === 0) {
  console.error(
    `[verify-embedded-assets] no ./assets/* references found in index.html — is this a real vite build?`,
  );
  process.exit(1);
}

// 2) Assert each referenced file exists on disk
const missingOnDisk = [];
for (const rel of referenced) {
  const full = join(distDir, rel);
  if (!existsSync(full) || !statSync(full).isFile()) {
    missingOnDisk.push(rel);
  }
}
if (missingOnDisk.length > 0) {
  console.error(
    `[verify-embedded-assets] dist is incomplete, referenced files missing:\n  ${missingOnDisk.join("\n  ")}`,
  );
  process.exit(1);
}

console.log(
  `[verify-embedded-assets] dist OK: ${referenced.length} asset(s) referenced and present.`,
);

if (checkDistOnly) {
  process.exit(0);
}

// 3) Locate the release executable
const candidates = exePath
  ? [resolve(ROOT, exePath)]
  : [
      join(ROOT, "src-tauri/target/x86_64-pc-windows-msvc/release/cc-switch.exe"),
      join(ROOT, "src-tauri/target/release/cc-switch.exe"),
      join(ROOT, "src-tauri/target/x86_64-pc-windows-msvc/debug/cc-switch.exe"),
      join(ROOT, "src-tauri/target/debug/cc-switch.exe"),
    ];
const found = candidates.find((p) => existsSync(p));
if (!found) {
  console.error(
    `[verify-embedded-assets] cc-switch executable not found. Tried:\n  ${candidates.join("\n  ")}`,
  );
  process.exit(1);
}
exePath = found;

// 4) Assert the executable contains every referenced asset name.
//    In a custom-protocol (embedded) build, generate_context! embeds the dist
//    files and stores them keyed by their path (e.g. `assets/index-x.js`),
//    so the name must appear as a literal byte sequence. A dev-mode build
//    embeds nothing and will fail this check.
console.log(`[verify-embedded-assets] checking exe: ${exePath}`);
const exeBuf = readFileSync(exePath);
const missingInExe = [];
for (const rel of referenced) {
  // index.html is always embedded as `index.html`; assets are embedded with
  // their relative path.
  const key = rel.replace(/^\.\//, "");
  if (!exeBuf.includes(Buffer.from(key, "utf8"))) {
    missingInExe.push(key);
  }
}

if (missingInExe.length > 0) {
  console.error(
    `[verify-embedded-assets] FAIL: the executable does NOT embed these frontend assets:\n  ${missingInExe.join("\n  ")}`,
  );
  console.error(
    "This usually means the build ran without the `custom-protocol` feature " +
      "(e.g. plain `cargo build --release`), producing a dev-mode binary that " +
      "loads http://localhost:3000 at runtime. Refusing to publish.",
  );
  process.exit(1);
}

console.log(
  `[verify-embedded-assets] PASS: all ${referenced.length} frontend asset(s) are embedded in the executable.`,
);
