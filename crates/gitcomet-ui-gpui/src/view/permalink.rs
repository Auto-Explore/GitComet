//! Forge permalink generation.
//!
//! Turns a repository's `origin` remote URL (SSH or HTTPS) into a web URL for
//! a specific commit or file on the hosting forge (GitHub, GitLab, Bitbucket,
//! Azure DevOps, Gitea/Codeberg, AWS CodeCommit, or any GitHub-style forge
//! such as a self-hosted Gitea instance).

use gitcomet_core::domain::Remote;

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
pub(super) fn origin_remote<'a>(remotes: &'a [Remote]) -> Option<&'a Remote> {
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

fn parse_remote_url(url: &str) -> Option<ForgeWebBase> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }

    // scp-like syntax: `git@github.com:owner/repo.git`. A scheme URL like
    // `https://…` also splits on ':' (`https`, `//…`), so only enter this
    // branch when the whole URL has no `://`.
    if !url.contains("://") {
        if let Some((user_host, path)) = url.split_once(':') {
            if !user_host.contains('/') && path.contains('/') {
                let host = user_host.rsplit('@').next()?;
                return build_base(host, path, "https");
            }
        }
    }

    // Scheme URLs: https://, http://, git://, ssh://, git+ssh://.
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(
        scheme.as_str(),
        "https" | "http" | "git" | "ssh" | "git+ssh"
    ) {
        return None;
    }
    // Strip userinfo (`git@`) and any port before the path.
    let rest = rest.split('@').next_back()?;
    let (host, path) = rest.split_once('/')?;
    let host = host.split(':').next()?;
    build_base(host, path, &scheme)
}

fn build_base(host: &str, path: &str, scheme: &str) -> Option<ForgeWebBase> {
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
    if let Some(base) = code_commit_base(&host, path) {
        return Some(base);
    }
    let owner_repo = path.strip_suffix(".git").unwrap_or(path).trim_matches('/');
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
    Some(ForgeWebBase {
        kind,
        web_root: format!("{scheme}://{host}/{owner_repo}"),
    })
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
    let owner_repo = path.strip_suffix(".git").unwrap_or(path).trim_matches('/');
    let parts: Vec<&str> = owner_repo.split('/').collect();
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
    let repo = path.strip_suffix(".git").unwrap_or(path).trim_matches('/');
    let repo = repo.strip_prefix("v1/repos/")?;
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
    fn permalinks_reject_empty_arguments() {
        let remotes = [remote("origin", "git@github.com:org/repo.git")];
        assert_eq!(commit_permalink(&remotes, "  "), None);
        assert_eq!(file_permalink(&remotes, "", "a.txt"), None);
        assert_eq!(file_permalink(&remotes, "main", " "), None);
    }
}
