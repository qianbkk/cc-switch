fn main() {
    let fork_release_tag = std::env::var("CC_SWITCH_FORK_RELEASE_TAG").unwrap_or_default();
    println!("cargo:rustc-env=CC_SWITCH_FORK_RELEASE_TAG={fork_release_tag}");
    println!("cargo:rerun-if-env-changed=CC_SWITCH_FORK_RELEASE_TAG");

    // commit SHA：发布 workflow 显式传入；本地构建 fallback `git rev-parse --short HEAD`
    // （git 不可用时为空字符串，前端显示 dev 而非 dev+空）。
    let commit_sha = std::env::var("CC_SWITCH_COMMIT_SHA").unwrap_or_else(|_| {
        std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| {
                String::from_utf8(o.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            })
            .unwrap_or_default()
    });
    println!("cargo:rustc-env=CC_SWITCH_COMMIT_SHA={commit_sha}");
    println!("cargo:rerun-if-env-changed=CC_SWITCH_COMMIT_SHA");

    // 构建时间（RFC3339 UTC）：发布 workflow 注入，保证可复现；本地为空。
    let build_time = std::env::var("CC_SWITCH_BUILD_TIME").unwrap_or_default();
    println!("cargo:rustc-env=CC_SWITCH_BUILD_TIME={build_time}");
    println!("cargo:rerun-if-env-changed=CC_SWITCH_BUILD_TIME");

    tauri_build::build();

    // Windows: Embed Common Controls v6 manifest for test binaries
    //
    // When running `cargo test`, the generated test executables don't include
    // the standard Tauri application manifest. Without Common Controls v6,
    // `tauri::test` calls fail with STATUS_ENTRYPOINT_NOT_FOUND.
    //
    // This workaround:
    // 1. Embeds the manifest into test binaries via /MANIFEST:EMBED
    // 2. Uses /MANIFEST:NO for the main binary to avoid duplicate resources
    //    (Tauri already handles manifest embedding for the app binary)
    #[cfg(target_os = "windows")]
    {
        let manifest_path = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"),
        )
        .join("common-controls.manifest");
        let manifest_arg = format!("/MANIFESTINPUT:{}", manifest_path.display());

        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg={}", manifest_arg);
        // Avoid duplicate manifest resources in binary builds.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
        println!("cargo:rerun-if-changed={}", manifest_path.display());
    }
}
