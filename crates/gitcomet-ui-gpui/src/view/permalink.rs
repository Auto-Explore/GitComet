//! Forge permalink generation.
//!
//! Turns a repository's `origin` remote URL (SSH or HTTPS) into a web URL for
//! a specific commit or file on the hosting forge (GitHub, GitLab, Bitbucket,
//! or any GitHub-style forge such as Gitea/Codeberg).

use gitcomet_core::domain::Remote;

/// The forge URL shapes GitComet knows how to generate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ForgeKind {
    GitHub,
    GitLab,
    Bitbucket,
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
        ForgeKind::GitHub | ForgeKind::Generic => {
            format!("{}/commit/{sha}", base.web_root)
        }
        ForgeKind::GitLab => format!("{}/-/commit/{sha}", base.web_root),
        ForgeKind::Bitbucket => format!("{}/commits/{sha}", base.web_root),
    })
}

/// Web permalink for a file at a given reference (commit sha or branch name),
/// e.g. `https://github.com/Auto-Explore/GitComet/blob/<ref>/src/main.rs`.
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
    if !matches!(scheme.as_str(), "https" | "http" | "git" | "ssh" | "git+ssh") {
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
    let owner_repo = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_matches('/');
    if owner_repo.is_empty() || !owner_repo.contains('/') {
        return None;
    }
    let kind = match host.as_str() {
        "github.com" => ForgeKind::GitHub,
        "gitlab.com" => ForgeKind::GitLab,
        "bitbucket.org" => ForgeKind::Bitbucket,
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
        let remotes = [remote("origin", "ssh://git@git.example.com:2222/org/repo.git")];
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
