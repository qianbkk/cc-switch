use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

const FORK_REPOSITORY: &str = "qianbkk/cc-switch";
const FORK_RELEASES_URL: &str = "https://github.com/qianbkk/cc-switch/releases";
const FORK_RELEASES_API: &str =
    "https://api.github.com/repos/qianbkk/cc-switch/releases?per_page=20";
const EMBEDDED_FORK_RELEASE_TAG: &str = env!("CC_SWITCH_FORK_RELEASE_TAG");

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkUpdateInfo {
    pub current_version: String,
    pub available_version: String,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    pub release_url: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    published_at: Option<String>,
    draft: bool,
    prerelease: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ForkVersion {
    upstream: Vec<u64>,
    revision: u64,
}

fn parse_fork_version(tag: &str) -> Option<ForkVersion> {
    let version = tag.strip_prefix('m')?;
    let (upstream, revision) = version.rsplit_once('-')?;
    let upstream = upstream
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if upstream.len() != 3 {
        return None;
    }

    Some(ForkVersion {
        upstream,
        revision: revision.parse().ok()?,
    })
}

fn current_fork_tag(app_version: &str) -> String {
    if parse_fork_version(EMBEDDED_FORK_RELEASE_TAG).is_some() {
        EMBEDDED_FORK_RELEASE_TAG.to_string()
    } else {
        // 本地开发构建没有 m* tag。视为基于当前上游版本的 revision 0，
        // 既能检查真实魔改预发行版，又不会误把上游 release 当成更新源。
        format!("m{app_version}-0")
    }
}

fn select_latest_fork_release<'a>(
    releases: &'a [GitHubRelease],
    current_tag: &str,
) -> Option<&'a GitHubRelease> {
    let current = parse_fork_version(current_tag)?;
    releases
        .iter()
        .filter(|release| !release.draft && release.prerelease)
        .filter_map(|release| {
            let version = parse_fork_version(&release.tag_name)?;
            (version > current).then_some((version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, release)| release)
}

async fn fetch_fork_update(app_version: &str) -> Result<Option<ForkUpdateInfo>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(format!("CC-Switch/{app_version} ({FORK_REPOSITORY})"))
        .build()
        .map_err(|e| format!("初始化魔改版更新客户端失败: {e}"))?;

    let response = client
        .get(FORK_RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("请求魔改版 Release 失败: {e}"))?
        .error_for_status()
        .map_err(|e| format!("魔改版 Release 接口返回错误: {e}"))?;

    let releases = response
        .json::<Vec<GitHubRelease>>()
        .await
        .map_err(|e| format!("解析魔改版 Release 失败: {e}"))?;
    let current_tag = current_fork_tag(app_version);
    let Some(release) = select_latest_fork_release(&releases, &current_tag) else {
        return Ok(None);
    };

    Ok(Some(ForkUpdateInfo {
        current_version: current_tag,
        available_version: release.tag_name.clone(),
        notes: release.body.clone().or_else(|| release.name.clone()),
        pub_date: release.published_at.clone(),
        release_url: release.html_url.clone(),
    }))
}

#[tauri::command]
pub async fn check_fork_update(app: AppHandle) -> Result<Option<ForkUpdateInfo>, String> {
    let app_version = app.package_info().version.to_string();
    fetch_fork_update(&app_version).await
}

#[tauri::command]
pub async fn open_fork_release(
    app: AppHandle,
    release_url: Option<String>,
) -> Result<bool, String> {
    let target = release_url
        .filter(|url| {
            url == FORK_RELEASES_URL
                || url
                    .strip_prefix(FORK_RELEASES_URL)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
        .unwrap_or_else(|| FORK_RELEASES_URL.to_string());

    app.opener()
        .open_url(target, None::<String>)
        .map_err(|e| format!("打开魔改版发布页失败: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{current_fork_tag, parse_fork_version, select_latest_fork_release, GitHubRelease};

    fn release(tag: &str, draft: bool, prerelease: bool) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.to_string(),
            name: Some(tag.to_string()),
            body: None,
            html_url: format!("https://github.com/qianbkk/cc-switch/releases/tag/{tag}"),
            published_at: None,
            draft,
            prerelease,
        }
    }

    #[test]
    fn parses_supported_m_tag() {
        let version = parse_fork_version("m3.18.0-12").expect("parse m tag");
        assert_eq!(version.upstream, vec![3, 18, 0]);
        assert_eq!(version.revision, 12);
    }

    #[test]
    fn rejects_upstream_and_legacy_tags() {
        assert!(parse_fork_version("v3.18.0").is_none());
        assert!(parse_fork_version("custom-v3.18.0-3").is_none());
        assert!(parse_fork_version("m3.18-1").is_none());
    }

    #[test]
    fn selects_highest_newer_fork_release() {
        let releases = vec![
            release("m3.18.0-2", false, true),
            release("m3.19.0-1", false, true),
            release("m3.18.0-10", false, true),
        ];
        let latest = select_latest_fork_release(&releases, "m3.18.0-2").expect("new release");
        assert_eq!(latest.tag_name, "m3.19.0-1");
    }

    #[test]
    fn ignores_drafts_stable_and_legacy_releases() {
        let releases = vec![
            release("m3.18.0-3", true, true),
            release("m3.18.0-4", false, false),
            release("custom-v3.18.0-5", false, true),
        ];
        assert!(select_latest_fork_release(&releases, "m3.18.0-2").is_none());
    }

    #[test]
    fn embedded_or_development_tag_is_parseable() {
        assert!(parse_fork_version(&current_fork_tag("3.18.0")).is_some());
    }
}
