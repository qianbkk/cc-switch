# 根因分析：绿色版新电脑双击 exe 显示"无法访问此页面"

> 日期：2026-08-02
> 涉及：qianbkk/cc-switch fork 的 Windows 绿色压缩版（release-portable.yml 产物）
> 结论：**发布流程用裸 `cargo build --release` 绕过 Tauri CLI，导致发布二进制以 dev 模式编译，运行时加载 `http://localhost:3000`（开发服务器）且未嵌入任何前端资源。**

---

## 一、问题现象

Windows 绿色压缩版（`CC-Switch-m3.19.1-1-Windows-Portable.zip` 等）在新电脑解压后双击 `cc-switch.exe`，窗口显示"无法访问此页面"（对应 WebView 无法连接 `http://localhost:3000`）。

## 二、根因证据链（三层）

### 证据 1：发布二进制内容（构建产物层）

对比三个 exe 的字节串提取结果（`dist/index.html` 引用的资源文件名是否嵌入二进制）：

| 产物 | 构建方式 | exe 内含 `http://localhost:3000/../dist` | exe 内含 `assets/index-*.js/css`（嵌入资源） |
| --- | --- | --- | --- |
| 上游官方 v3.19.1 Portable（正常） | `pnpm tauri build`（Tauri CLI） | 有（config 序列化，正常） | **有**（`/assets/index-CmyF_n_x.js` 等） |
| fork m3.19.1-1 Portable（坏） | `cargo build --release` | 有 | **无** |
| fork m3.18.0-2 Portable（坏） | `cargo build --release` | 有 | **无** |

关键差异：**正常版嵌入了前端资源，坏版一个资源都没嵌入**。这是判定 dev/生产模式最可靠的二进制证据（config 中的 `devUrl` 字段本身总会序列化进二进制，不能作为判据）。

### 证据 2：Tauri CLI 自动注入 feature（源码层）

`tauri-cli`（crates/tauri-cli/src/interface/rust.rs，v2.10.3）：

```rust
pub fn build_options(&self, args: &mut Vec<String>, features: &mut Vec<String>, mobile: bool) {
    features.push("tauri/custom-protocol".into());   // ← `tauri build` 自动注入
    ...
}
```

`tauri dev` 则相反（`dev_options` 用 `--no-default-features` 并把 app 的 default features 中**剔除** `tauri/custom-protocol` 后重放）。即 **custom-protocol 由 CLI 按命令动态注入，不写在 Cargo.toml 里**——这就是官方模板 `tauri = { version = "2", features = [] }` 也能正常工作的原因。

### 证据 3：缺 feature 时 tauri 的 dev 判定与 URL 选择（依赖源码层）

- `tauri` crate build.rs（v2.10.3）第 251-257 行：
  ```rust
  let custom_protocol = has_feature("custom-protocol");
  let dev = !custom_protocol;          // 无 feature → dev = true
  println!("cargo:dev={dev}");
  ```
- `tauri-macros` generate_context!（src/context.rs）：
  ```rust
  dev: cfg!(not(feature = "custom-protocol")),   // dev 模式下不嵌入资源
  ```
- `tauri` 运行时 URL 选择（src/manager/mod.rs `get_app_url`）：
  ```rust
  #[cfg(dev)] let url = self.config.build.dev_url.as_ref();            // → http://localhost:3000
  #[cfg(not(dev))] let url = match frontend_dist { FrontendDist::Url(u) => Some(u), _ => None }; // → 嵌入资源 / tauri://localhost
  ```

`tauri-build`（app 的 build-dependency）通过 `DEP_TAURI_DEV` 读取上述 `cargo:dev` 并给 app crate 设置 `cfg(dev)`，所以裸 `cargo build --release` 时整条链都是 dev 模式。

### 结论（根因）

> `release-portable.yml` 为了跳过安装器打包阶段而直接执行 `cargo build --release`，绕过了 Tauri CLI。缺少 CLI 注入的 `tauri/custom-protocol` feature 后：
> 1. `generate_context!` 不嵌入 dist 前端资源；
> 2. 运行时按 `cfg(dev)` 加载 `devUrl = http://localhost:3000`；
> 3. 新电脑没有 dev server → WebView 报"无法访问此页面"。

用户最初怀疑的"直接 cargo build --release 且未启用 custom-protocol"方向正确，现已由源码 + 工作流 + 构建产物 + 二进制内容四层证据确认。

## 三、修复

1. `release-portable.yml`：构建命令由裸 `cargo build --release` 改为 `pnpm tauri build --no-bundle --target x86_64-pc-windows-msvc`。
   - `tauri build` 自动注入 `tauri/custom-protocol` → 发布版嵌入资源、走 `tauri://localhost` 协议；
   - `--no-bundle` 跳过 msi/updater 产物阶段（`if !options.no_bundle && ...` 才执行 bundle），因此不需要 Windows 代码签名证书与 `TAURI_SIGNING_PRIVATE_KEY`；
   - 前端由 `beforeBuildCommand`（`pnpm build:renderer`）自动构建。
2. 新增 `scripts/verify-embedded-assets.mjs` 自动校验：
   - 解析 `dist/index.html` 引用的资源 → 检查文件存在 → 检查发布 exe 二进制**确实嵌入**这些资源名；
   - 任一缺失即 exit 1，`release-portable.yml` 打包前执行，CI 失败则禁止上传。
3. `ci.yml`：
   - frontend job 增加 `build:renderer` + dist 完整性校验；
   - 新增 `portable-build` job（windows-latest）：完整跑发布构建路径 + exe 嵌入校验，任何 push main 都覆盖。

## 四、防回归要点

- 判据是"资源是否嵌入 exe"，而不是"exe 里有没有 localhost:3000 字符串"（config 序列化总是含 devUrl）。
- 不要在 `src-tauri/Cargo.toml` 的 tauri 依赖上写死 `custom-protocol` feature：那会让 `tauri dev` 也变成非 dev 模式，破坏开发热更新。custom-protocol 应保持由 Tauri CLI 按命令注入。
- 发布绿色版只用 `release-portable.yml`，禁止回归裸 `cargo build`。
