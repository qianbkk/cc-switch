//! 版本信息单一来源（路线图第 19 项）。
//!
//! 统一「Fork 完整版本」的解析与展示规则，About 页、更新检查、详情页共用：
//!
//! - **发布构建**（编译时注入了 `CC_SWITCH_FORK_RELEASE_TAG`，形如 `m3.19.2-1`）：
//!   `display_version` 与 `update_tag` 都是完整 tag；build.rs 同时注入
//!   commit SHA 与构建时间（由发布 workflow 设置）。
//! - **本地开发构建**（无发布 tag）：`display_version` 显示 `dev+<short-sha>`
//!   （不伪装成已发布修订版）；`update_tag` 退化为 `m<base>-0`，使更新检查
//!   仍可比较真实魔改预发行版，又不会误把上游 release 当成更新源。

use serde::Serialize;

const EMBEDDED_FORK_RELEASE_TAG: &str = env!("CC_SWITCH_FORK_RELEASE_TAG");
const EMBEDDED_COMMIT_SHA: &str = env!("CC_SWITCH_COMMIT_SHA");
const EMBEDDED_BUILD_TIME: &str = env!("CC_SWITCH_BUILD_TIME");

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkVersionInfo {
    /// 上游基础版本（如 `3.19.2`），取自应用运行时版本
    pub base_version: String,
    /// 展示版本：发布构建为完整 `m<base>-<rev>` tag；本地构建为 `dev+<short-sha>`
    pub display_version: String,
    /// 更新检查用当前 tag：发布构建为完整 tag；本地构建为 `m<base>-0`
    pub update_tag: String,
    /// 是否为发布构建（注入了 `m*` 发布 tag）
    pub is_release: bool,
    /// commit short sha（git 不可用时为空）
    pub commit_sha: String,
    /// 构建时间 RFC3339（发布构建由 workflow 注入；本地为空）
    pub build_time: String,
}

/// 统一版本解析：所有展示/比较 Fork 版本的地方都应走这里。
pub fn fork_version_info(app_version: &str) -> ForkVersionInfo {
    let is_release = !EMBEDDED_FORK_RELEASE_TAG.is_empty();
    let display_version = if is_release {
        EMBEDDED_FORK_RELEASE_TAG.to_string()
    } else if EMBEDDED_COMMIT_SHA.is_empty() {
        "dev".to_string()
    } else {
        format!("dev+{}", EMBEDDED_COMMIT_SHA)
    };
    let update_tag = if is_release {
        EMBEDDED_FORK_RELEASE_TAG.to_string()
    } else {
        format!("m{app_version}-0")
    };
    ForkVersionInfo {
        base_version: app_version.to_string(),
        display_version,
        update_tag,
        is_release,
        commit_sha: EMBEDDED_COMMIT_SHA.to_string(),
        build_time: EMBEDDED_BUILD_TIME.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_build_uses_embedded_tag() {
        // 发布路径逻辑独立于 env 实测：直接断言结构函数的行为（tag 注入在编译期）。
        // 这里通过构造逻辑验证——env 常量无法在测试中修改，仅验证 dev 分支与结构。
        let info = fork_version_info("3.19.2");
        // 无论编译环境如何，字段结构完整、base_version 正确
        assert_eq!(info.base_version, "3.19.2");
        assert!(!info.display_version.is_empty());
        assert!(!info.update_tag.is_empty());
        // update_tag 必须以 m 开头（两种分支都是 m 前缀）
        assert!(info.update_tag.starts_with('m'));
    }

    #[test]
    fn update_tag_always_parseable_for_updates() {
        // update_tag 必须能被 parse_fork_version 解析（更新检查依赖）
        let info = fork_version_info("3.19.2");
        let tag = &info.update_tag;
        let version = tag.strip_prefix('m').expect("m 前缀");
        let (upstream, revision) = version.rsplit_once('-').expect("revision 分隔");
        assert_eq!(upstream, "3.19.2");
        let _rev: u64 = revision.parse().expect("revision 是数字");
    }
}
