use super::*;
use semver::Version;
#[cfg(not(test))]
use serde::Deserialize;

const UPDATE_CHECK_DISABLE_ENV: &str = "GITCOMET_NO_UPDATE_CHECK";
#[cfg(not(test))]
const UPDATE_CHECK_REPO_ENV: &str = "GITCOMET_UPDATE_REPO";
#[cfg(not(test))]
const DEFAULT_UPDATE_REPO: &str = "GitComet/gitcomet";

#[derive(Clone, Debug, Eq, PartialEq)]
struct UpdateNotice {
    latest_version: String,
    current_version: String,
    releases_url: String,
}

#[cfg(not(test))]
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    html_url: Option<String>,
}

#[cfg(not(test))]
#[derive(Debug, Deserialize)]
struct GitHubTag {
    name: String,
}

#[cfg_attr(test, allow(dead_code))]
#[derive(Clone, Debug, Eq, PartialEq)]
struct GitHubRepo {
    owner: String,
    repo: String,
}

impl GitCometView {
    pub(in crate::view) fn maybe_check_for_updates_on_startup(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.view_mode != GitCometViewMode::Normal
            || std::env::var_os(UPDATE_CHECK_DISABLE_ENV).is_some()
        {
            return;
        }

        #[cfg(test)]
        let _ = cx;

        #[cfg(not(test))]
        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                let notice = smol::unblock(|| fetch_update_notice(env!("CARGO_PKG_VERSION"))).await;
                let Some(notice) = notice else {
                    return;
                };

                let _ = view.update(cx, |this, cx| {
                    this.push_toast_with_link(
                        components::ToastKind::Warning,
                        format!(
                            "A newer GitComet version is available: {} (current {}).",
                            notice.latest_version, notice.current_version
                        ),
                        notice.releases_url,
                        "Open Releases".to_string(),
                        cx,
                    );
                });
            },
        )
        .detach();
    }
}

#[cfg(not(test))]
fn fetch_update_notice(current_version: &'static str) -> Option<UpdateNotice> {
    const UPDATE_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);

    let repo = resolve_update_repo();
    let user_agent = format!(
        "GitComet/{current_version} (+{})",
        env!("CARGO_PKG_REPOSITORY")
    );
    let client = zed_reqwest::blocking::Client::builder()
        .timeout(UPDATE_CHECK_TIMEOUT)
        .build()
        .ok()?;

    let release_response = client
        .get(repo.releases_latest_api_url())
        .header(zed_reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(zed_reqwest::header::USER_AGENT, user_agent.clone())
        .send()
        .ok()?;

    let (release_tag, release_html_url): (Option<String>, Option<String>) =
        if release_response.status().is_success() {
            let release = release_response.json::<GitHubRelease>().ok()?;
            (Some(release.tag_name), release.html_url)
        } else {
            (None, None)
        };

    let tag_names: Vec<String> = client
        .get(repo.tags_api_url())
        .header(zed_reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(zed_reqwest::header::USER_AGENT, user_agent)
        .send()
        .ok()
        .and_then(|response| response.error_for_status().ok())
        .and_then(|response| response.json::<Vec<GitHubTag>>().ok())
        .map(|tags| tags.into_iter().map(|tag| tag.name).collect())
        .unwrap_or_default();

    build_update_notice(
        current_version,
        release_tag.as_deref(),
        release_html_url.as_deref(),
        &tag_names,
        &repo,
    )
}

fn build_update_notice(
    current_version: &str,
    release_tag: Option<&str>,
    release_html_url: Option<&str>,
    tag_names: &[String],
    repo: &GitHubRepo,
) -> Option<UpdateNotice> {
    let current = parse_semver_tag(current_version)?;

    let mut latest: Option<(Version, String)> = release_tag.and_then(|tag| {
        parse_semver_tag(tag).map(|version| {
            let url = release_html_url
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| repo.releases_page_url());
            (version, url)
        })
    });

    for tag_name in tag_names {
        let Some(version) = parse_semver_tag(tag_name) else {
            continue;
        };

        let should_promote = latest
            .as_ref()
            .map(|(best, _)| version > *best)
            .unwrap_or(true);
        if should_promote {
            latest = Some((version, repo.releases_page_url()));
        }
    }

    let Some((latest_version, latest_url)) = latest else {
        return None;
    };

    if latest_version <= current {
        return None;
    }

    Some(UpdateNotice {
        latest_version: latest_version.to_string(),
        current_version: current.to_string(),
        releases_url: latest_url,
    })
}

fn parse_semver_tag(raw: &str) -> Option<Version> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    Version::parse(trimmed)
        .ok()
        .or_else(|| {
            trimmed
                .strip_prefix('v')
                .and_then(|rest| Version::parse(rest).ok())
        })
        .or_else(|| {
            trimmed
                .strip_prefix('V')
                .and_then(|rest| Version::parse(rest).ok())
        })
}

#[cfg(not(test))]
fn resolve_update_repo() -> GitHubRepo {
    std::env::var(UPDATE_CHECK_REPO_ENV)
        .ok()
        .as_deref()
        .and_then(parse_repo_slug)
        .or_else(|| parse_repo_slug(env!("CARGO_PKG_REPOSITORY")))
        .unwrap_or_else(|| GitHubRepo::from_slug(DEFAULT_UPDATE_REPO))
}

#[cfg(not(test))]
fn parse_repo_slug(raw: &str) -> Option<GitHubRepo> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(repo) = parse_github_repo_from_url(trimmed) {
        return Some(repo);
    }

    if trimmed.split('/').count() == 2 {
        return Some(GitHubRepo::from_slug(trimmed));
    }

    None
}

fn parse_github_repo_from_url(raw: &str) -> Option<GitHubRepo> {
    let without_scheme = raw
        .strip_prefix("https://github.com/")
        .or_else(|| raw.strip_prefix("http://github.com/"))
        .or_else(|| raw.strip_prefix("git@github.com:"))
        .or_else(|| raw.strip_prefix("ssh://git@github.com/"))?;

    Some(GitHubRepo::from_slug(without_scheme))
}

impl GitHubRepo {
    fn from_slug(raw: &str) -> Self {
        let mut normalized = raw.trim().trim_end_matches('/').to_string();
        if let Some(stripped) = normalized.strip_suffix(".git") {
            normalized = stripped.to_string();
        }

        let mut parts = normalized.splitn(2, '/');
        let owner = parts.next().unwrap_or_default().trim().to_string();
        let repo = parts.next().unwrap_or_default().trim().to_string();

        Self { owner, repo }
    }

    #[cfg(not(test))]
    fn releases_latest_api_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            self.owner, self.repo
        )
    }

    #[cfg(not(test))]
    fn tags_api_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/tags?per_page=20",
            self.owner, self.repo
        )
    }

    fn releases_page_url(&self) -> String {
        format!("https://github.com/{}/{}/releases", self.owner, self.repo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_semver_tag_accepts_plain_and_prefixed_versions() {
        assert_eq!(parse_semver_tag("1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_semver_tag("v1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_semver_tag("V1.2.3"), Some(Version::new(1, 2, 3)));
    }

    #[test]
    fn build_update_notice_returns_none_when_release_is_not_newer() {
        let repo = GitHubRepo::from_slug("Auto-Explore/GitComet");
        let tags = vec!["0.0.9".to_string()];
        assert!(build_update_notice("0.1.0", Some("v0.1.0"), None, &tags, &repo).is_none());
    }

    #[test]
    fn build_update_notice_returns_notice_when_new_release_exists() {
        let repo = GitHubRepo::from_slug("Auto-Explore/GitComet");
        let notice = build_update_notice(
            "0.2.0",
            Some("v0.2.1"),
            Some("https://example.invalid/releases/0.2.1"),
            &[],
            &repo,
        )
        .expect("update notice expected");
        assert_eq!(notice.current_version, "0.2.0");
        assert_eq!(notice.latest_version, "0.2.1");
        assert_eq!(
            notice.releases_url,
            "https://example.invalid/releases/0.2.1"
        );
    }

    #[test]
    fn build_update_notice_falls_back_to_repo_releases_page_when_no_release_url() {
        let repo = GitHubRepo::from_slug("Auto-Explore/GitComet");
        let notice = build_update_notice("0.2.0", Some("0.2.1"), None, &[], &repo)
            .expect("update notice expected");
        assert_eq!(
            notice.releases_url,
            "https://github.com/Auto-Explore/GitComet/releases"
        );
    }

    #[test]
    fn build_update_notice_promotes_newer_tag_over_older_release() {
        let repo = GitHubRepo::from_slug("Auto-Explore/GitComet");
        let tags = vec!["v0.2.0".to_string()];
        let notice = build_update_notice("0.1.0", Some("v0.1.0"), None, &tags, &repo)
            .expect("update notice expected");
        assert_eq!(notice.latest_version, "0.2.0");
        assert_eq!(
            notice.releases_url,
            "https://github.com/Auto-Explore/GitComet/releases"
        );
    }

    #[test]
    fn parse_github_repo_from_url_supports_https_and_ssh_forms() {
        assert_eq!(
            parse_github_repo_from_url("https://github.com/Auto-Explore/GitComet.git"),
            Some(GitHubRepo {
                owner: "Auto-Explore".to_string(),
                repo: "GitComet".to_string(),
            })
        );
        assert_eq!(
            parse_github_repo_from_url("git@github.com:Auto-Explore/GitComet.git"),
            Some(GitHubRepo {
                owner: "Auto-Explore".to_string(),
                repo: "GitComet".to_string(),
            })
        );
    }
}
