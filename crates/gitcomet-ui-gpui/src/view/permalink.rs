//! Forge permalink generation.
//!
//! Turns a repository's `origin` remote URL (SSH or HTTPS) into a web URL for
//! a specific commit or file on the hosting forge (GitHub, GitLab, Bitbucket,
//! Azure DevOps, Gitea/Codeberg, AWS CodeCommit, or any GitHub-style forge
//! such as a self-hosted Gitea instance), or into the repository page itself.

use gitcomet_core::domain::{Remote, RemoteBranch};

/// The forge URL shapes GitComet knows how to generate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ForgeKind {
    GitHub,
    GitLab,
    Bitbucket,
    /// Azure DevOps (`dev.azure.com` and the legacy `*.visualstudio.com`
    /// hosts). Unlike the other forges, files are addressed with a `path`
    /// query parameter instead of a `blob` path segment.
    AzureDevOps,
    /// Hosted Gitea instances (`codeberg.org`, `gitea.com`, …). Files use the
    /// canonical `src/branch/<ref>` / `src/commit/<sha>` paths.
    Gitea,
    /// AWS CodeCommit. The git remote lives on
    /// `git-codecommit.<region>.amazonaws.com` while the web console is under
    /// `<region>.console.aws.amazon.com/codesuite/codecommit/…`.
    CodeCommit,
    /// Any other host; GitHub-style `/{owner}/{repo}/…` paths are the
    /// de-facto standard shared by Gitea, Codeberg, and friends.
    Generic,
}

/// Parsed web root of a remote, e.g. `https://github.com/Auto-Explore/GitComet`.
#[derive(Debug, Eq, PartialEq)]
struct ForgeWebBase {
    kind: ForgeKind,
    web_root: String,
}

/// The remote web links should be based on: `origin` when present, otherwise
/// the first remote that has a URL.
pub(super) fn origin_remote(remotes: &[Remote]) -> Option<&Remote> {
    remotes
        .iter()
        .find(|remote| remote.name == "origin")
        .or_else(|| remotes.iter().find(|remote| remote.url.is_some()))
}

/// Web permalink for a commit, e.g.
/// `https://github.com/Auto-Explore/GitComet/commit/<sha>`.
pub(super) fn commit_permalink(remotes: &[Remote], sha: &str) -> Option<String> {
    let base = web_base(remotes)?;
    let sha = sha.trim();
    if sha.is_empty() {
        return None;
    }
    Some(match base.kind {
        ForgeKind::GitHub | ForgeKind::Generic | ForgeKind::Gitea | ForgeKind::CodeCommit => {
            format!("{}/commit/{sha}", base.web_root)
        }
        ForgeKind::GitLab => format!("{}/-/commit/{sha}", base.web_root),
        ForgeKind::Bitbucket => format!("{}/commits/{sha}", base.web_root),
        ForgeKind::AzureDevOps => format!("{}/commit/{sha}", base.web_root),
    })
}

/// Whether a branch exists on the remote that web links are based on. A
/// branch-only permalink (`blob/<branch>`) only resolves while the branch is
/// on the forge; for a local-only branch (never pushed) it would point at a
/// nonexistent source, so callers should suppress the permalink action.
/// Matching is done against the permalink remote specifically: a branch pushed
/// to a *different* remote still has no counterpart on the forge the link
/// targets.
pub(super) fn branch_exists_on_permalink_remote(
    remotes: &[Remote],
    remote_branches: &[RemoteBranch],
    branch: &str,
) -> bool {
    let Some(remote) = origin_remote(remotes) else {
        return false;
    };
    remote_branches
        .iter()
        .any(|remote_branch| remote_branch.remote == remote.name && remote_branch.name == branch)
}

/// Web permalink for a file at a given reference (commit sha or branch name),
/// e.g. `https://github.com/Auto-Explore/GitComet/blob/<ref>/src/main.rs` or
/// `https://dev.azure.com/…/_git/repo?path=/src/main.rs&version=GB<ref>`.
/// The path must be repository-relative; backslashes and URL-unsafe characters
/// are normalized/percent-encoded.
pub(super) fn file_permalink(remotes: &[Remote], reference: &str, path: &str) -> Option<String> {
    let base = web_base(remotes)?;
    let reference = reference.trim();
    let path = path.trim();
    if reference.is_empty() || path.is_empty() {
        return None;
    }
    let encoded_path = encode_path(path);
    Some(match base.kind {
        ForgeKind::GitHub | ForgeKind::Generic => {
            format!("{}/blob/{reference}/{encoded_path}", base.web_root)
        }
        ForgeKind::GitLab => format!("{}/-/blob/{reference}/{encoded_path}", base.web_root),
        ForgeKind::Bitbucket => {
            format!("{}/src/{reference}/{encoded_path}", base.web_root)
        }
        ForgeKind::Gitea => {
            // Gitea's canonical file URL distinguishes branches (`src/branch`)
            // from commits (`src/commit`).
            let ref_kind = if is_full_sha(reference) {
                "commit"
            } else {
                "branch"
            };
            format!(
                "{}/src/{ref_kind}/{reference}/{encoded_path}",
                base.web_root
            )
        }
        ForgeKind::CodeCommit => {
            // CodeCommit browses branches via `refs/heads/…` and commits by
            // their id, with `--` separating the reference from the path.
            let browse_ref = if is_full_sha(reference) {
                reference.to_string()
            } else {
                format!("refs/heads/{reference}")
            };
            format!("{}/browse/{browse_ref}/--/{encoded_path}", base.web_root)
        }
        ForgeKind::AzureDevOps => {
            // Azure DevOps addresses the version in a query parameter and
            // needs to know whether the reference is a branch or a commit.
            let encoded_ref = encode_path(reference);
            let version = if is_full_sha(reference) {
                format!("GC{encoded_ref}")
            } else {
                format!("GB{encoded_ref}")
            };
            format!(
                "{}?path=/{encoded_path}&version={version}&_a=contents",
                base.web_root
            )
        }
    })
}

fn web_base(remotes: &[Remote]) -> Option<ForgeWebBase> {
    let url = origin_remote(remotes)?.url.as_deref()?;
    parse_remote_url(url)
}

/// A remote whose URL points at a repository page on a forge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RemoteWebPage {
    pub(super) remote: String,
    pub(super) url: String,
}

impl RemoteWebPage {
    /// The page without its scheme, e.g. `github.com/Auto-Explore/GitComet`.
    pub(super) fn display_address(&self) -> &str {
        self.url
            .split_once("://")
            .map_or(self.url.as_str(), |(_, address)| address)
    }
}

/// The repository page a remote URL points at, e.g.
/// `https://github.com/Auto-Explore/GitComet`.
pub(super) fn remote_web_url(url: &str) -> Option<String> {
    parse_remote_url(url).map(|base| base.web_root)
}

/// Every remote with a web page, `origin` first and the rest in the given
/// order. A page two remotes share (ssh and https of one repo) is listed once,
/// under the first.
pub(super) fn remote_web_pages(remotes: &[Remote]) -> Vec<RemoteWebPage> {
    let origin_first = remotes
        .iter()
        .filter(|remote| remote.name == "origin")
        .chain(remotes.iter().filter(|remote| remote.name != "origin"));
    let mut pages: Vec<RemoteWebPage> = Vec::new();
    for remote in origin_first {
        let Some(url) = remote.url.as_deref().and_then(remote_web_url) else {
            continue;
        };
        if pages.iter().all(|page| page.url != url) {
            pages.push(RemoteWebPage {
                remote: remote.name.clone(),
                url,
            });
        }
    }
    pages
}

fn parse_remote_url(url: &str) -> Option<ForgeWebBase> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }

    // scp-like syntax: `git@github.com:owner/repo.git`. A scheme URL like
    // `https://…` also splits on ':' (`https`, `//…`), so only enter this
    // branch when the whole URL has no `://`.
    if !url.contains("://")
        && let Some((user_host, path)) = url.split_once(':')
        && !user_host.contains('/')
        && path.contains('/')
    {
        let host = user_host.rsplit('@').next()?;
        return build_base(host, None, path, "https");
    }

    // Scheme URLs: https://, http://, git://, ssh:// and git's two aliases for
    // it, git+ssh:// and ssh+git://.
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(
        scheme.as_str(),
        "https" | "http" | "git" | "ssh" | "git+ssh" | "ssh+git"
    ) {
        return None;
    }
    let (authority, path) = rest.split_once('/')?;
    // Userinfo (`git@`, `user:token@`) only ever precedes the host.
    let host_port = authority.rsplit('@').next()?;
    let (host, port) = host_port
        .split_once(':')
        .map_or((host_port, None), |(host, port)| (host, Some(port)));
    build_base(host, web_port(&scheme, port), path, &scheme)
}

/// The port to keep in the web root: an http(s) remote's port is the web
/// server's, while an ssh/git port says nothing about where the web UI is.
fn web_port<'a>(scheme: &str, port: Option<&'a str>) -> Option<&'a str> {
    let port = port.filter(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))?;
    match (scheme, port) {
        ("https", "443") | ("http", "80") => None,
        ("https" | "http", port) => Some(port),
        _ => None,
    }
}

fn build_base(host: &str, port: Option<&str>, path: &str, scheme: &str) -> Option<ForgeWebBase> {
    let host = host.trim().to_ascii_lowercase();
    let path = path.trim();
    if host.is_empty() || (host != "localhost" && !host.contains('.')) {
        return None;
    }
    // Azure DevOps and AWS CodeCommit remotes use a different web-root shape
    // than the GitHub-style `/{owner}/{repo}` layout, so they are handled
    // exclusively here and never fall through to the generic path.
    if is_azure_devops_host(&host) {
        return azure_devops_base(&host, path);
    }
    if host.starts_with("git-codecommit.") {
        return code_commit_base(&host, path);
    }
    let owner_repo = repo_path(path);
    if owner_repo.is_empty() || !owner_repo.contains('/') {
        return None;
    }
    let kind = match host.as_str() {
        "github.com" => ForgeKind::GitHub,
        "gitlab.com" => ForgeKind::GitLab,
        "bitbucket.org" => ForgeKind::Bitbucket,
        "codeberg.org" | "gitea.com" | "code.forgejo.org" => ForgeKind::Gitea,
        _ => ForgeKind::Generic,
    };
    // Keep the remote's own scheme: an http-only self-hosted forge stays
    // reachable over http rather than getting an https URL that may not exist.
    let scheme = if matches!(scheme, "http" | "https") {
        scheme
    } else {
        "https"
    };
    let port = port.map(|port| format!(":{port}")).unwrap_or_default();
    Some(ForgeWebBase {
        kind,
        web_root: format!("{scheme}://{host}{port}/{owner_repo}"),
    })
}

/// A remote's repository path without its slashes or `.git` suffix, so
/// `/org/repo.git/` and `org/repo` name the same repository.
fn repo_path(path: &str) -> &str {
    let path = path.trim_matches('/');
    path.strip_suffix(".git").unwrap_or(path)
}

/// The hosts that host Azure DevOps git repositories: the current
/// `dev.azure.com` (with the SSH-only `ssh.dev.azure.com`) and the legacy
/// `*.visualstudio.com` accounts (with their `vs-ssh.visualstudio.com` SSH
/// host).
fn is_azure_devops_host(host: &str) -> bool {
    host == "dev.azure.com"
        || host == "ssh.dev.azure.com"
        || host == "vs-ssh.visualstudio.com"
        || host.ends_with(".visualstudio.com")
}

/// Web root for an Azure DevOps remote. HTTPS remotes carry the same
/// `/{org}/{project}/_git/{repo}` path as the web UI, while SSH remotes use a
/// `v3/{org}/{project}/{repo}` path and the legacy `*.visualstudio.com` hosts
/// keep the organization in the hostname.
fn azure_devops_base(host: &str, path: &str) -> Option<ForgeWebBase> {
    let parts: Vec<&str> = repo_path(path).split('/').collect();
    let web_root = match (host, parts.as_slice()) {
        ("dev.azure.com", [org, project, "_git", repo]) => {
            format!("https://dev.azure.com/{org}/{project}/_git/{repo}")
        }
        ("ssh.dev.azure.com", ["v3", org, project, repo]) => {
            format!("https://dev.azure.com/{org}/{project}/_git/{repo}")
        }
        ("vs-ssh.visualstudio.com", ["v3", org, project, repo]) => {
            format!("https://{org}.visualstudio.com/{project}/_git/{repo}")
        }
        (legacy_host, [project, "_git", repo]) if legacy_host.ends_with(".visualstudio.com") => {
            format!("https://{legacy_host}/{project}/_git/{repo}")
        }
        _ => return None,
    };
    Some(ForgeWebBase {
        kind: ForgeKind::AzureDevOps,
        web_root,
    })
}

/// Web root for an AWS CodeCommit remote. The git host is
/// `git-codecommit.<region>.amazonaws.com` and the web console lives at
/// `<region>.console.aws.amazon.com/codesuite/codecommit/repositories/<repo>`,
/// which mirrors the remote's `v1/repos/<repo>` path.
fn code_commit_base(host: &str, path: &str) -> Option<ForgeWebBase> {
    let region = host
        .strip_prefix("git-codecommit.")?
        .strip_suffix(".amazonaws.com")?;
    if region.is_empty() || region.contains('.') {
        return None;
    }
    let repo = repo_path(path).strip_prefix("v1/repos/")?;
    if repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some(ForgeWebBase {
        kind: ForgeKind::CodeCommit,
        web_root: format!(
            "https://{region}.console.aws.amazon.com/codesuite/codecommit/repositories/{repo}"
        ),
    })
}

/// Whether a reference is a full 40-hex-digit git commit id. Forges like
/// Azure DevOps and Gitea need to distinguish branches from commits in the
/// URL (`GB`/`GC`, `src/branch`/`src/commit`).
fn is_full_sha(reference: &str) -> bool {
    reference.len() == 40 && reference.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Percent-encode every character outside the RFC 3986 unreserved set (plus
/// `/`, which separates path segments). Backslashes from Windows path
/// rendering are normalized to forward slashes.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b'\\' => out.push('/'),
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(byte & 0x0F) as usize]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(name: &str, url: &str) -> Remote {
        Remote {
            name: name.to_string(),
            url: Some(url.to_string()),
        }
    }

    #[test]
    fn origin_is_preferred_over_other_remotes() {
        let remotes = [
            remote("upstream", "https://github.com/other/repo.git"),
            remote("origin", "git@github.com:Auto-Explore/GitComet.git"),
        ];
        assert_eq!(
            origin_remote(&remotes).map(|r| r.name.as_str()),
            Some("origin")
        );
    }

    #[test]
    fn falls_back_to_first_remote_with_url_when_origin_is_missing() {
        let remotes = [
            remote("upstream", "git@github.com:other/repo.git"),
            remote("mirror", "https://example.com/mirror.git"),
        ];
        assert_eq!(
            origin_remote(&remotes).map(|r| r.name.as_str()),
            Some("upstream")
        );
        assert_eq!(origin_remote(&[]), None);
    }

    fn remote_branch(remote: &str, name: &str) -> RemoteBranch {
        RemoteBranch {
            remote: remote.to_string(),
            name: name.to_string(),
            target: gitcomet_core::domain::CommitId("abc123".into()),
        }
    }

    #[test]
    fn branch_exists_on_permalink_remote_when_pushed_to_origin() {
        let remotes = [remote("origin", "git@github.com:org/repo.git")];
        let remote_branches = [
            remote_branch("origin", "main"),
            remote_branch("origin", "feature/x"),
        ];
        assert!(branch_exists_on_permalink_remote(
            &remotes,
            &remote_branches,
            "feature/x"
        ));
    }

    #[test]
    fn local_only_branch_is_not_on_permalink_remote() {
        let remotes = [remote("origin", "git@github.com:org/repo.git")];
        let remote_branches = [remote_branch("origin", "main")];
        assert!(!branch_exists_on_permalink_remote(
            &remotes,
            &remote_branches,
            "permalink-copy"
        ));
        assert!(!branch_exists_on_permalink_remote(&remotes, &[], "main"));
        assert!(!branch_exists_on_permalink_remote(
            &[],
            &remote_branches,
            "main"
        ));
    }

    #[test]
    fn branch_pushed_to_another_remote_does_not_count_for_the_permalink_remote() {
        let remotes = [remote("origin", "git@github.com:org/repo.git")];
        let remote_branches = [remote_branch("backup", "feature")];
        // The permalink is based on `origin`, where the branch has no
        // counterpart, so the link would not resolve.
        assert!(!branch_exists_on_permalink_remote(
            &remotes,
            &remote_branches,
            "feature"
        ));
    }

    #[test]
    fn branch_is_checked_against_the_fallback_remote_when_origin_is_missing() {
        let remotes = [remote("upstream", "git@github.com:org/repo.git")];
        let remote_branches = [remote_branch("upstream", "feature")];
        assert!(branch_exists_on_permalink_remote(
            &remotes,
            &remote_branches,
            "feature"
        ));
        assert!(!branch_exists_on_permalink_remote(
            &remotes,
            &remote_branches,
            "other"
        ));
    }

    #[test]
    fn commit_permalink_for_ssh_github_remote() {
        let remotes = [remote("origin", "git@github.com:Auto-Explore/GitComet.git")];
        assert_eq!(
            commit_permalink(&remotes, "abc123").as_deref(),
            Some("https://github.com/Auto-Explore/GitComet/commit/abc123")
        );
    }

    #[test]
    fn commit_permalink_for_https_github_remote_without_git_suffix() {
        let remotes = [remote("origin", "https://github.com/Auto-Explore/GitComet")];
        assert_eq!(
            commit_permalink(&remotes, "abc123").as_deref(),
            Some("https://github.com/Auto-Explore/GitComet/commit/abc123")
        );
    }

    #[test]
    fn commit_permalink_for_gitlab_uses_dash_dash_paths() {
        let remotes = [remote("origin", "git@gitlab.com:group/subgroup/repo.git")];
        assert_eq!(
            commit_permalink(&remotes, "abc123").as_deref(),
            Some("https://gitlab.com/group/subgroup/repo/-/commit/abc123")
        );
    }

    #[test]
    fn commit_permalink_for_bitbucket_uses_commits_path() {
        let remotes = [remote("origin", "https://bitbucket.org/team/repo.git")];
        assert_eq!(
            commit_permalink(&remotes, "abc123").as_deref(),
            Some("https://bitbucket.org/team/repo/commits/abc123")
        );
    }

    #[test]
    fn commit_permalink_for_self_hosted_forge_uses_github_paths() {
        let remotes = [remote(
            "origin",
            "ssh://git@git.example.com:2222/org/repo.git",
        )];
        assert_eq!(
            commit_permalink(&remotes, "abc123").as_deref(),
            Some("https://git.example.com/org/repo/commit/abc123")
        );
    }

    #[test]
    fn commit_permalink_keeps_http_scheme() {
        let remotes = [remote("origin", "http://github.com/org/repo.git")];
        assert_eq!(
            commit_permalink(&remotes, "abc123").as_deref(),
            Some("http://github.com/org/repo/commit/abc123")
        );
    }

    #[test]
    fn web_urls_keep_an_http_port_but_drop_an_ssh_port() {
        // Gitea's default port; the web UI is served on it too.
        let gitea = [remote("origin", "http://localhost:3000/org/repo.git")];
        assert_eq!(
            commit_permalink(&gitea, "abc123").as_deref(),
            Some("http://localhost:3000/org/repo/commit/abc123")
        );
        let https = [remote(
            "origin",
            "https://git.example.com:8443/org/repo.git",
        )];
        assert_eq!(
            commit_permalink(&https, "abc123").as_deref(),
            Some("https://git.example.com:8443/org/repo/commit/abc123")
        );
        let default_port = [remote("origin", "https://github.com:443/org/repo.git")];
        assert_eq!(
            commit_permalink(&default_port, "abc123").as_deref(),
            Some("https://github.com/org/repo/commit/abc123")
        );
    }

    #[test]
    fn userinfo_is_stripped_from_the_host_only() {
        let token = [remote(
            "origin",
            "https://user:token@github.com/org/repo.git",
        )];
        assert_eq!(
            commit_permalink(&token, "abc123").as_deref(),
            Some("https://github.com/org/repo/commit/abc123")
        );
        let at_in_path = [remote("origin", "https://git.example.com/org/repo@v2.git")];
        assert_eq!(
            commit_permalink(&at_in_path, "abc123").as_deref(),
            Some("https://git.example.com/org/repo@v2/commit/abc123")
        );
    }

    #[test]
    fn file_permalink_encodes_path_and_uses_blob_path() {
        let remotes = [remote("origin", "git@github.com:Auto-Explore/GitComet.git")];
        assert_eq!(
            file_permalink(&remotes, "main", "src/my file#1.rs").as_deref(),
            Some("https://github.com/Auto-Explore/GitComet/blob/main/src/my%20file%231.rs")
        );
    }

    #[test]
    fn file_permalink_normalizes_backslashes_to_forward_slashes() {
        let remotes = [remote("origin", "git@github.com:org/repo.git")];
        assert_eq!(
            file_permalink(&remotes, "abc123", r"crates\gitcomet\src\lib.rs").as_deref(),
            Some("https://github.com/org/repo/blob/abc123/crates/gitcomet/src/lib.rs")
        );
    }

    #[test]
    fn file_permalink_for_gitlab_and_bitbucket() {
        let gitlab = [remote("origin", "git@gitlab.com:group/repo.git")];
        assert_eq!(
            file_permalink(&gitlab, "main", "a/b.txt").as_deref(),
            Some("https://gitlab.com/group/repo/-/blob/main/a/b.txt")
        );
        let bitbucket = [remote("origin", "git@bitbucket.org:team/repo.git")];
        assert_eq!(
            file_permalink(&bitbucket, "main", "a/b.txt").as_deref(),
            Some("https://bitbucket.org/team/repo/src/main/a/b.txt")
        );
    }

    #[test]
    fn commit_permalink_for_azure_devops_https_remote() {
        let remotes = [remote(
            "origin",
            "https://org@dev.azure.com/org/project/_git/repo",
        )];
        assert_eq!(
            commit_permalink(&remotes, "abc123").as_deref(),
            Some("https://dev.azure.com/org/project/_git/repo/commit/abc123")
        );
    }

    #[test]
    fn commit_permalink_for_azure_devops_ssh_remote() {
        let remotes = [remote(
            "origin",
            "git@ssh.dev.azure.com:v3/org/project/repo",
        )];
        assert_eq!(
            commit_permalink(&remotes, "abc123").as_deref(),
            Some("https://dev.azure.com/org/project/_git/repo/commit/abc123")
        );
    }

    #[test]
    fn commit_permalink_for_legacy_visualstudio_remotes() {
        let https = [remote(
            "origin",
            "https://org.visualstudio.com/project/_git/repo",
        )];
        assert_eq!(
            commit_permalink(&https, "abc123").as_deref(),
            Some("https://org.visualstudio.com/project/_git/repo/commit/abc123")
        );
        let ssh = [remote(
            "origin",
            "git@vs-ssh.visualstudio.com:v3/org/project/repo",
        )];
        assert_eq!(
            commit_permalink(&ssh, "abc123").as_deref(),
            Some("https://org.visualstudio.com/project/_git/repo/commit/abc123")
        );
    }

    #[test]
    fn file_permalink_for_azure_devops_branch_reference() {
        let remotes = [remote(
            "origin",
            "git@ssh.dev.azure.com:v3/org/project/repo",
        )];
        assert_eq!(
            file_permalink(&remotes, "main", "src/main.rs").as_deref(),
            Some(
                "https://dev.azure.com/org/project/_git/repo?path=/src/main.rs&version=GBmain&_a=contents"
            )
        );
    }

    #[test]
    fn file_permalink_for_azure_devops_commit_reference() {
        let remotes = [remote(
            "origin",
            "https://dev.azure.com/org/project/_git/repo",
        )];
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            file_permalink(&remotes, sha, "src/main.rs").as_deref(),
            Some(
                "https://dev.azure.com/org/project/_git/repo?path=/src/main.rs&version=GC0123456789abcdef0123456789abcdef01234567&_a=contents"
            )
        );
    }

    #[test]
    fn file_permalink_for_azure_devops_encodes_path() {
        let remotes = [remote(
            "origin",
            "https://org.visualstudio.com/project/_git/repo",
        )];
        assert_eq!(
            file_permalink(&remotes, "feature/x", "src/my file#1.rs").as_deref(),
            Some(
                "https://org.visualstudio.com/project/_git/repo?path=/src/my%20file%231.rs&version=GBfeature/x&_a=contents"
            )
        );
    }

    #[test]
    fn azure_devops_remotes_with_unexpected_shapes_are_rejected() {
        // Missing the `_git` segment means the remote is not an Azure DevOps
        // repo, so no permalink should be produced instead of a broken one.
        let remotes = [remote("origin", "https://dev.azure.com/org/project/repo")];
        assert_eq!(commit_permalink(&remotes, "abc123"), None);
        let ssh = [remote("origin", "git@ssh.dev.azure.com:v3/org/repo")];
        assert_eq!(commit_permalink(&ssh, "abc123"), None);
    }

    #[test]
    fn commit_permalink_for_gitea_and_codeberg() {
        let gitea = [remote("origin", "git@gitea.com:org/repo.git")];
        assert_eq!(
            commit_permalink(&gitea, "abc123").as_deref(),
            Some("https://gitea.com/org/repo/commit/abc123")
        );
        let codeberg = [remote("origin", "https://codeberg.org/org/repo.git")];
        assert_eq!(
            commit_permalink(&codeberg, "abc123").as_deref(),
            Some("https://codeberg.org/org/repo/commit/abc123")
        );
    }

    #[test]
    fn file_permalink_for_gitea_uses_src_branch_and_src_commit() {
        let remotes = [remote("origin", "git@codeberg.org:org/repo.git")];
        assert_eq!(
            file_permalink(&remotes, "main", "src/lib.rs").as_deref(),
            Some("https://codeberg.org/org/repo/src/branch/main/src/lib.rs")
        );
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            file_permalink(&remotes, sha, "src/lib.rs").as_deref(),
            Some(
                "https://codeberg.org/org/repo/src/commit/0123456789abcdef0123456789abcdef01234567/src/lib.rs"
            )
        );
    }

    #[test]
    fn commit_permalink_for_code_commit() {
        let remotes = [remote(
            "origin",
            "https://git-codecommit.eu-west-1.amazonaws.com/v1/repos/my-repo",
        )];
        assert_eq!(
            commit_permalink(&remotes, "abc123").as_deref(),
            Some(
                "https://eu-west-1.console.aws.amazon.com/codesuite/codecommit/repositories/my-repo/commit/abc123"
            )
        );
    }

    #[test]
    fn file_permalink_for_code_commit() {
        let remotes = [remote(
            "origin",
            "ssh://git-codecommit.us-east-1.amazonaws.com/v1/repos/my-repo",
        )];
        assert_eq!(
            file_permalink(&remotes, "main", "src/lib.rs").as_deref(),
            Some(
                "https://us-east-1.console.aws.amazon.com/codesuite/codecommit/repositories/my-repo/browse/refs/heads/main/--/src/lib.rs"
            )
        );
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            file_permalink(&remotes, sha, "src/lib.rs").as_deref(),
            Some(
                "https://us-east-1.console.aws.amazon.com/codesuite/codecommit/repositories/my-repo/browse/0123456789abcdef0123456789abcdef01234567/--/src/lib.rs"
            )
        );
    }

    #[test]
    fn code_commit_remotes_with_unexpected_shapes_are_rejected() {
        // Missing region in the git host or a repo id containing slashes.
        let no_region = [remote(
            "origin",
            "https://git-codecommit.amazonaws.com/v1/repos/repo",
        )];
        assert_eq!(commit_permalink(&no_region, "abc123"), None);
        let nested_repo = [remote(
            "origin",
            "https://git-codecommit.us-east-1.amazonaws.com/v1/repos/a/b",
        )];
        assert_eq!(commit_permalink(&nested_repo, "abc123"), None);
    }

    #[test]
    fn permalinks_reject_local_paths_and_unsupported_remotes() {
        assert_eq!(commit_permalink(&[], "abc123"), None);
        let local = [remote("origin", "/srv/git/repo.git")];
        assert_eq!(commit_permalink(&local, "abc123"), None);
        let windows_drive = [remote("origin", "C:/git/repo.git")];
        assert_eq!(commit_permalink(&windows_drive, "abc123"), None);
        let no_path = [remote("origin", "https://github.com/owner")];
        assert_eq!(commit_permalink(&no_path, "abc123"), None);
        let no_url = [Remote {
            name: "origin".to_string(),
            url: None,
        }];
        assert_eq!(commit_permalink(&no_url, "abc123"), None);
    }

    #[test]
    fn remote_web_url_is_the_repository_page() {
        for (url, expected) in [
            (
                "git@github.com:Auto-Explore/GitComet.git",
                "https://github.com/Auto-Explore/GitComet",
            ),
            (
                "https://github.com/Auto-Explore/GitComet",
                "https://github.com/Auto-Explore/GitComet",
            ),
            (
                "https://user:token@github.com/org/repo.git",
                "https://github.com/org/repo",
            ),
            (
                "ssh://git@git.example.com:2222/org/repo.git",
                "https://git.example.com/org/repo",
            ),
            (
                "https://git.example.com:8443/org/repo.git",
                "https://git.example.com:8443/org/repo",
            ),
            (
                "http://localhost:3000/org/repo.git",
                "http://localhost:3000/org/repo",
            ),
            (
                "git@gitlab.com:group/subgroup/repo.git",
                "https://gitlab.com/group/subgroup/repo",
            ),
            (
                "git@ssh.dev.azure.com:v3/org/project/repo",
                "https://dev.azure.com/org/project/_git/repo",
            ),
            (
                "https://git-codecommit.eu-west-1.amazonaws.com/v1/repos/my-repo",
                "https://eu-west-1.console.aws.amazon.com/codesuite/codecommit/repositories/my-repo",
            ),
        ] {
            assert_eq!(remote_web_url(url).as_deref(), Some(expected), "{url}");
        }
    }

    #[test]
    fn remote_web_url_rejects_local_and_unsupported_urls() {
        for url in [
            "",
            "/srv/git/repo.git",
            "C:/git/repo.git",
            "file:///srv/git/repo.git",
            "https://github.com/owner",
        ] {
            assert_eq!(remote_web_url(url), None, "{url}");
        }
    }

    #[test]
    fn remote_web_pages_put_origin_first_then_keep_order() {
        let remotes = [
            remote("backup", "https://gitlab.com/org/backup.git"),
            remote("origin", "git@github.com:org/repo.git"),
            remote("upstream", "git@github.com:other/repo.git"),
        ];
        let pages = remote_web_pages(&remotes);
        let names: Vec<&str> = pages.iter().map(|page| page.remote.as_str()).collect();
        assert_eq!(names, ["origin", "backup", "upstream"]);
        assert_eq!(pages[0].url, "https://github.com/org/repo");
    }

    #[test]
    fn remote_web_pages_skip_remotes_without_a_web_page() {
        let remotes = [
            Remote {
                name: "no-url".to_string(),
                url: None,
            },
            remote("local", "/srv/git/repo.git"),
            remote("upstream", "https://github.com/org/repo.git"),
        ];
        let pages = remote_web_pages(&remotes);
        assert_eq!(
            pages,
            [RemoteWebPage {
                remote: "upstream".to_string(),
                url: "https://github.com/org/repo".to_string(),
            }]
        );
        assert!(remote_web_pages(&[]).is_empty());
    }

    #[test]
    fn remote_web_pages_list_a_shared_page_once() {
        let remotes = [
            remote("https", "https://github.com/org/repo.git"),
            remote("origin", "git@github.com:org/repo.git"),
        ];
        let pages = remote_web_pages(&remotes);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].remote, "origin");
    }

    #[test]
    fn repo_path_strips_slashes_and_one_git_suffix() {
        for (path, expected) in [
            ("org/repo", "org/repo"),
            ("org/repo.git", "org/repo"),
            ("/org/repo.git/", "org/repo"),
            ("org/repo/", "org/repo"),
            ("org/repo.git.git", "org/repo.git"),
            ("org/.github", "org/.github"),
            ("/", ""),
        ] {
            assert_eq!(repo_path(path), expected, "{path}");
        }
    }

    #[test]
    fn web_port_keeps_only_a_non_default_http_port() {
        for (scheme, port, expected) in [
            ("https", Some("8443"), Some("8443")),
            ("http", Some("3000"), Some("3000")),
            ("http", Some("443"), Some("443")),
            ("https", Some("443"), None),
            ("http", Some("80"), None),
            ("ssh", Some("2222"), None),
            ("git", Some("9418"), None),
            ("git+ssh", Some("22"), None),
            ("https", Some(""), None),
            ("https", Some("80a"), None),
            ("https", None, None),
        ] {
            assert_eq!(web_port(scheme, port), expected, "{scheme} {port:?}");
        }
    }

    #[test]
    fn remote_web_url_lowercases_scheme_and_host_but_keeps_the_path() {
        assert_eq!(
            remote_web_url("HTTPS://GitHub.COM/Org/Repo.git").as_deref(),
            Some("https://github.com/Org/Repo")
        );
        assert_eq!(
            remote_web_url("git@GitLab.com:Group/Repo.git").as_deref(),
            Some("https://gitlab.com/Group/Repo")
        );
    }

    #[test]
    fn remote_web_url_trims_surrounding_whitespace() {
        assert_eq!(
            remote_web_url("  git@github.com:org/repo.git\n").as_deref(),
            Some("https://github.com/org/repo")
        );
    }

    #[test]
    fn remote_web_url_serves_git_and_ssh_schemes_over_https() {
        for url in [
            "git://github.com/org/repo.git",
            "ssh://github.com/org/repo.git",
            "git+ssh://git@github.com/org/repo.git",
            "ssh+git://git@github.com/org/repo.git",
        ] {
            assert_eq!(
                remote_web_url(url).as_deref(),
                Some("https://github.com/org/repo"),
                "{url}"
            );
        }
    }

    #[test]
    fn remote_web_url_accepts_scp_syntax_without_a_user() {
        assert_eq!(
            remote_web_url("github.com:org/repo.git").as_deref(),
            Some("https://github.com/org/repo")
        );
    }

    #[test]
    fn remote_web_url_ignores_trailing_slashes() {
        for url in [
            "https://github.com/org/repo/",
            "https://github.com/org/repo.git/",
            "ssh://git@github.com/org/repo.git/",
        ] {
            assert_eq!(
                remote_web_url(url).as_deref(),
                Some("https://github.com/org/repo"),
                "{url}"
            );
        }
    }

    #[test]
    fn remote_web_url_never_carries_credentials() {
        for url in [
            "https://user:token@github.com/org/repo.git",
            "https://user:p@ss@github.com/org/repo.git",
            "https://user:p%40ss@github.com/org/repo.git",
            "http://token@git.example.com:8080/org/repo.git",
            "ssh://user:token@github.com/org/repo.git",
            "https://token@dev.azure.com/org/project/_git/repo",
        ] {
            let web = remote_web_url(url).unwrap_or_else(|| panic!("{url} has a page"));
            assert!(
                !web.contains("token") && !web.contains("ss@") && !web.contains("user"),
                "{url} leaked into {web}"
            );
        }
    }

    #[test]
    fn remote_web_url_rejects_hosts_without_a_domain() {
        for url in [
            "git@gitserver:org/repo.git",
            "ssh://git@gitserver/org/repo.git",
            "https://intranet/org/repo",
        ] {
            assert_eq!(remote_web_url(url), None, "{url}");
        }
        assert_eq!(
            remote_web_url("ssh://git@localhost:2222/org/repo.git").as_deref(),
            Some("https://localhost/org/repo"),
            "localhost is the one dotless host allowed"
        );
    }

    #[test]
    fn remote_web_url_rejects_unsupported_schemes() {
        for url in [
            "ftp://github.com/org/repo.git",
            "rsync://github.com/org/repo.git",
            "file:///srv/git/org/repo.git",
            "codecommit::us-east-1://my-repo",
        ] {
            assert_eq!(remote_web_url(url), None, "{url}");
        }
    }

    #[test]
    fn remote_web_url_needs_an_owner_and_a_repository() {
        for url in [
            "git@github.example.com:repo.git",
            "https://github.example.com/repo.git",
            "https://github.example.com/",
            "https://github.example.com",
        ] {
            assert_eq!(remote_web_url(url), None, "{url}");
        }
    }

    #[test]
    fn remote_web_url_keeps_deep_gitlab_subgroups() {
        assert_eq!(
            remote_web_url("https://gitlab.com/a/b/c/d.git").as_deref(),
            Some("https://gitlab.com/a/b/c/d")
        );
    }

    #[test]
    fn remote_web_url_maps_every_azure_devops_shape() {
        for (url, expected) in [
            (
                "https://org@dev.azure.com/org/project/_git/repo",
                "https://dev.azure.com/org/project/_git/repo",
            ),
            (
                "git@ssh.dev.azure.com:v3/org/project/repo",
                "https://dev.azure.com/org/project/_git/repo",
            ),
            (
                "git@vs-ssh.visualstudio.com:v3/org/project/repo",
                "https://org.visualstudio.com/project/_git/repo",
            ),
            (
                "https://org.visualstudio.com/project/_git/repo",
                "https://org.visualstudio.com/project/_git/repo",
            ),
        ] {
            assert_eq!(remote_web_url(url).as_deref(), Some(expected), "{url}");
        }
        assert_eq!(
            remote_web_url("https://dev.azure.com/org/project/repo"),
            None
        );
    }

    #[test]
    fn remote_web_url_maps_code_commit_over_https_and_ssh() {
        let console =
            "https://eu-west-1.console.aws.amazon.com/codesuite/codecommit/repositories/my-repo";
        for url in [
            "https://git-codecommit.eu-west-1.amazonaws.com/v1/repos/my-repo",
            "ssh://git-codecommit.eu-west-1.amazonaws.com/v1/repos/my-repo",
        ] {
            assert_eq!(remote_web_url(url).as_deref(), Some(console), "{url}");
        }
    }

    #[test]
    fn remote_web_pages_skip_an_origin_without_a_web_page() {
        let remotes = [
            remote("origin", "/srv/git/repo.git"),
            remote("upstream", "https://github.com/org/repo.git"),
        ];
        let names: Vec<String> = remote_web_pages(&remotes)
            .into_iter()
            .map(|page| page.remote)
            .collect();
        assert_eq!(names, ["upstream"]);
    }

    #[test]
    fn remote_web_pages_treat_host_case_as_the_same_page() {
        let remotes = [
            remote("origin", "git@GitHub.com:org/repo.git"),
            remote("https", "https://github.com/org/repo"),
        ];
        assert_eq!(remote_web_pages(&remotes).len(), 1);
    }

    #[test]
    fn remote_web_pages_keep_the_first_of_two_remotes_sharing_a_page() {
        let remotes = [
            remote("alpha", "https://github.com/org/repo.git"),
            remote("beta", "git@github.com:org/repo.git"),
        ];
        let pages = remote_web_pages(&remotes);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].remote, "alpha");
    }

    #[test]
    fn display_address_keeps_the_port() {
        let page = RemoteWebPage {
            remote: "gitea".to_string(),
            url: "http://localhost:3000/org/repo".to_string(),
        };
        assert_eq!(page.display_address(), "localhost:3000/org/repo");
    }

    #[test]
    fn file_permalink_keeps_an_https_port() {
        let remotes = [remote(
            "origin",
            "https://git.example.com:8443/org/repo.git",
        )];
        assert_eq!(
            file_permalink(&remotes, "main", "a.txt").as_deref(),
            Some("https://git.example.com:8443/org/repo/blob/main/a.txt")
        );
    }

    #[test]
    fn display_address_drops_the_scheme() {
        let page = RemoteWebPage {
            remote: "origin".to_string(),
            url: "https://github.com/org/repo".to_string(),
        };
        assert_eq!(page.display_address(), "github.com/org/repo");
    }

    #[test]
    fn permalinks_reject_empty_arguments() {
        let remotes = [remote("origin", "git@github.com:org/repo.git")];
        assert_eq!(commit_permalink(&remotes, "  "), None);
        assert_eq!(file_permalink(&remotes, "", "a.txt"), None);
        assert_eq!(file_permalink(&remotes, "main", " "), None);
    }
}
