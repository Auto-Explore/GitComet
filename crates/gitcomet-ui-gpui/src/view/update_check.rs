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

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), derive(Deserialize))]
struct GitHubRelease {
    tag_name: String,
    #[cfg_attr(not(test), serde(default))]
    html_url: Option<String>,
}

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
                let notice = fetch_update_notice(env!("CARGO_PKG_VERSION"), resolve_update_repo()).await;
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
async fn fetch_update_notice(
    current_version: &'static str,
    repo: GitHubRepo,
) -> Option<UpdateNotice> {
    smol::unblock(move || fetch_update_notice_blocking(current_version, repo)).await
}

#[cfg(not(test))]
fn fetch_update_notice_blocking(
    current_version: &'static str,
    repo: GitHubRepo,
) -> Option<UpdateNotice> {
    const UPDATE_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);

    let user_agent = format!(
        "GitComet/{current_version} (+{})",
        env!("CARGO_PKG_REPOSITORY")
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(UPDATE_CHECK_TIMEOUT)
        .build()
        .ok()?;

    let release: GitHubRelease = client
        .get(repo.releases_latest_api_url())
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .ok()
        .and_then(|response| response.error_for_status().ok())
        .and_then(|response| response.json::<GitHubRelease>().ok())?;

    build_update_notice(current_version, &release, &repo)
}

fn build_update_notice(
    current_version: &str,
    release: &GitHubRelease,
    repo: &GitHubRepo,
) -> Option<UpdateNotice> {
    let current = parse_semver_tag(current_version)?;
    let latest_version = parse_semver_tag(&release.tag_name)?;
    if !latest_version.pre.is_empty() {
        return None;
    }
    let latest_url = release
        .html_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| repo.releases_page_url());

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

    fn releases_page_url(&self) -> String {
        format!("https://github.com/{}/{}/releases", self.owner, self.repo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github_release(tag_name: &str, html_url: Option<&str>) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag_name.to_string(),
            html_url: html_url.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn parse_semver_tag_accepts_plain_and_prefixed_versions() {
        assert_eq!(parse_semver_tag("1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_semver_tag("v1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_semver_tag("V1.2.3"), Some(Version::new(1, 2, 3)));
    }

    #[test]
    fn build_update_notice_returns_none_when_release_is_not_newer() {
        let repo = GitHubRepo::from_slug("Auto-Explore/GitComet");
        let release = github_release("v0.1.0", None);
        assert!(build_update_notice("0.1.0", &release, &repo).is_none());
    }

    #[test]
    fn build_update_notice_returns_notice_when_new_release_exists() {
        let repo = GitHubRepo::from_slug("Auto-Explore/GitComet");
        let notice = build_update_notice(
            "0.2.0",
            &github_release("v0.2.1", Some("https://example.invalid/releases/0.2.1")),
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
        let notice = build_update_notice("0.2.0", &github_release("0.2.1", None), &repo)
            .expect("update notice expected");
        assert_eq!(
            notice.releases_url,
            "https://github.com/Auto-Explore/GitComet/releases"
        );
    }

    #[test]
    fn build_update_notice_returns_none_for_non_stable_release_tag() {
        let repo = GitHubRepo::from_slug("Auto-Explore/GitComet");
        let release = github_release(
            "v0.3.0-beta.1",
            Some("https://example.invalid/releases/0.3.0-beta.1"),
        );
        assert!(build_update_notice("0.2.0", &release, &repo).is_none());
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
