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

/// Whether a reference is a full 40- or 64-hex-digit git commit id. Forges like
/// Azure DevOps and Gitea need to distinguish branches from commits in the
/// URL (`GB`/`GC`, `src/branch`/`src/commit`).
fn is_full_sha(reference: &str) -> bool {
    matches!(reference.len(), 40 | 64) && reference.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Percent-encode every character outside the RFC 3986 unreserved set (plus
/// `/`, which separates path segments). Backslashes from Windows path
/// rendering are normalized to forward slashes.
fn encode_path(path: &str) -> String {
    let path = if path.contains('\\') {
        std::borrow::Cow::Owned(path.replace('\\', "/"))
    } else {
        std::borrow::Cow::Borrowed(path)
    };
    gitcomet_core::url_encoding::encode_path(path.as_bytes()).collect()
}

#[cfg(test)]
mod tests;
