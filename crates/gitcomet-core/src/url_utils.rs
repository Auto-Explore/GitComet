//! URL parsing and manipulation utilities for Git remotes.
//! Supports HTTPS, HTTP, SSH (URL and SCP-style syntax), and file protocols.

/// Extracts (username, base_url_without_user) from a remote URL.
///
/// Examples:
/// - `https://user@github.com/org/repo.git` -> `(Some("user"), "https://github.com/org/repo.git")`
/// - `https://github.com/org/repo.git` -> `(None, "https://github.com/org/repo.git")`
/// - `ssh://user@host.com:22/path/repo.git` -> `(Some("user"), "ssh://host.com:22/path/repo.git")`
/// - `git@github.com:org/repo.git` -> `(Some("git"), "github.com:org/repo.git")`
pub fn extract_username_and_base_url(url: &str) -> (Option<String>, String) {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return (None, String::new());
    }

    // 1. Standard protocol with scheme: "proto://..."
    if let Some(proto_pos) = trimmed.find("://") {
        let scheme = &trimmed[..proto_pos + 3];
        let rest = &trimmed[proto_pos + 3..];

        // Find the boundary between authority and path: first '/' or '?' or '#'
        let authority_end = rest
            .find('/')
            .unwrap_or_else(|| rest.find('?').unwrap_or(rest.len()));
        let authority = &rest[..authority_end];
        let path_and_query = &rest[authority_end..];

        if let Some(at_pos) = authority.find('@') {
            let user = &authority[..at_pos];
            let host_and_port = &authority[at_pos + 1..];
            let base_url = format!("{scheme}{host_and_port}{path_and_query}");
            return (
                if user.is_empty() {
                    None
                } else {
                    Some(user.to_string())
                },
                base_url,
            );
        } else {
            return (None, trimmed.to_string());
        }
    }

    // 2. SCP-style syntax: "user@host:path/to/repo.git" or "host:path/to/repo.git"
    // Only consider it SCP-style if there is a colon and no leading slash/drive letter
    if let Some(colon_pos) = trimmed.find(':') {
        let before_colon = &trimmed[..colon_pos];
        if !before_colon.contains('/')
            && !before_colon.contains('\\')
            && let Some(at_pos) = before_colon.find('@')
        {
            let user = &before_colon[..at_pos];
            let host = &before_colon[at_pos + 1..];
            let path = &trimmed[colon_pos..];
            let base_url = format!("{host}{path}");
            return (
                if user.is_empty() {
                    None
                } else {
                    Some(user.to_string())
                },
                base_url,
            );
        }
    }

    (None, trimmed.to_string())
}

/// Embeds or removes the username in a Git remote URL.
///
/// If `username` is `Some("...")` (and not empty), injects `username@` into the authority.
/// If `username` is `None` or empty, strips any embedded username.
pub fn embed_username_into_url(url: &str, username: Option<&str>) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let (_, base_url) = extract_username_and_base_url(trimmed);
    let clean_user = username.map(str::trim).filter(|u| !u.is_empty());

    let Some(user) = clean_user else {
        return base_url;
    };

    // 1. Standard protocol with scheme: "proto://..."
    if let Some(proto_pos) = base_url.find("://") {
        let scheme = &base_url[..proto_pos + 3];
        let rest = &base_url[proto_pos + 3..];
        return format!("{scheme}{user}@{rest}");
    }

    // 2. SCP-style syntax: "host:path" -> "user@host:path"
    if let Some(colon_pos) = base_url.find(':') {
        let before_colon = &base_url[..colon_pos];
        if !before_colon.contains('/') && !before_colon.contains('\\') {
            return format!("{user}@{base_url}");
        }
    }

    // 3. Fallback for generic host/path or unparsed string: if no scheme, default to https://
    format!("{user}@{base_url}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_username_and_base_url() {
        // HTTPS with user
        let (user, base) =
            extract_username_and_base_url("https://username@github.com/OrgName/RepoName.git");
        assert_eq!(user.as_deref(), Some("username"));
        assert_eq!(base, "https://github.com/OrgName/RepoName.git");

        // HTTPS without user
        let (user, base) = extract_username_and_base_url("https://github.com/OrgName/RepoName.git");
        assert_eq!(user, None);
        assert_eq!(base, "https://github.com/OrgName/RepoName.git");

        // SSH with scheme and user and port
        let (user, base) =
            extract_username_and_base_url("ssh://git@github.com:22/OrgName/Repo.git");
        assert_eq!(user.as_deref(), Some("git"));
        assert_eq!(base, "ssh://github.com:22/OrgName/Repo.git");

        // SCP-style SSH
        let (user, base) = extract_username_and_base_url("git@github.com:OrgName/Repo.git");
        assert_eq!(user.as_deref(), Some("git"));
        assert_eq!(base, "github.com:OrgName/Repo.git");

        // Empty
        let (user, base) = extract_username_and_base_url("");
        assert_eq!(user, None);
        assert_eq!(base, "");
    }

    #[test]
    fn test_embed_username_into_url() {
        // Add user to https URL
        let url = "https://github.com/OrgName/RepoName.git";
        let with_user = embed_username_into_url(url, Some("username"));
        assert_eq!(
            with_user,
            "https://username@github.com/OrgName/RepoName.git"
        );

        // Replace user in https URL
        let url_with_old_user = "https://olduser@github.com/OrgName/RepoName.git";
        let with_new_user = embed_username_into_url(url_with_old_user, Some("username"));
        assert_eq!(
            with_new_user,
            "https://username@github.com/OrgName/RepoName.git"
        );

        // Remove user from https URL
        let stripped = embed_username_into_url(url_with_old_user, None);
        assert_eq!(stripped, "https://github.com/OrgName/RepoName.git");
        let stripped_empty = embed_username_into_url(url_with_old_user, Some("   "));
        assert_eq!(stripped_empty, "https://github.com/OrgName/RepoName.git");

        // SSH SCP style
        let scp = "git@github.com:OrgName/Repo.git";
        let new_scp = embed_username_into_url(scp, Some("username"));
        assert_eq!(new_scp, "username@github.com:OrgName/Repo.git");

        let stripped_scp = embed_username_into_url(scp, None);
        assert_eq!(stripped_scp, "github.com:OrgName/Repo.git");
    }
}
