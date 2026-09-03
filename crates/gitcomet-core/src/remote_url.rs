use crate::error::{Error, ErrorKind};

/// Transports Git implements itself. Every other `scheme://` is handed to a
/// `git-remote-<scheme>` helper found on PATH.
const ALLOWED_SCHEMES: [&str; 7] = ["https", "http", "ssh", "git", "file", "ftp", "ftps"];
/// Git's own deprecated spellings of `ssh://`.
const DEPRECATED_SSH_SCHEMES: [&str; 2] = ["git+ssh", "ssh+git"];

/// Validate a user-supplied Git remote source without rejecting local paths or
/// SCP-style URLs such as `git@example.com:org/repo.git`.
pub fn validate_remote_url(url: &str) -> Result<(), Error> {
    let url = url.trim();
    if url.is_empty() {
        return Err(invalid("remote URL cannot be empty"));
    }
    if url.starts_with('-') {
        return Err(invalid(format!(
            "remote URL cannot start with '-': {url:?}"
        )));
    }

    let (scheme, remainder) = git_transport_prefix(url);
    if remainder.starts_with("::") {
        return Err(invalid(format!(
            "Git remote-helper URLs are not supported: `{scheme}::...`"
        )));
    }
    // No `scheme://`: a local path or SCP-style source, which picks no transport.
    if !remainder.starts_with("://") {
        return Ok(());
    }
    let scheme = scheme.to_ascii_lowercase();
    if ALLOWED_SCHEMES.contains(&scheme.as_str())
        || DEPRECATED_SSH_SCHEMES.contains(&scheme.as_str())
    {
        return Ok(());
    }
    Err(invalid(format!(
        "unsupported remote URL scheme `{scheme}` (allowed: {})",
        ALLOWED_SCHEMES.join(", ")
    )))
}

/// The transport name Git would take from `url`, plus the rest. Git's scan is
/// looser than an RFC scheme: the name may be empty or start with a digit.
fn git_transport_prefix(url: &str) -> (&str, &str) {
    let end = url
        .char_indices()
        .find(|&(index, ch)| {
            !(ch.is_ascii_alphanumeric() || (index > 0 && matches!(ch, '+' | '-' | '.')))
        })
        .map_or(url.len(), |(index, _)| index);
    url.split_at(end)
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Backend(message.into()))
}

#[cfg(test)]
mod tests {
    use super::validate_remote_url;

    #[test]
    fn accepts_supported_urls_and_path_forms() {
        for url in [
            "https://example.com/org/repo.git",
            "ssh://git@example.com/org/repo.git",
            "git://example.com/org/repo.git",
            "file:///tmp/repo.git",
            "/tmp/repo.git",
            "git@example.com:org/repo.git",
            "example.com:org/repo.git",
            r"C:\repos\repo.git",
        ] {
            assert!(validate_remote_url(url).is_ok(), "{url:?}");
        }
    }

    #[test]
    fn rejects_remote_helper_names_that_are_not_uri_schemes() {
        // An empty or digit-led helper name still runs `git-remote-<name>`.
        for url in ["::sh -c id", "7z::sh -c whatever", "my-helper::x", "2::x"] {
            assert!(validate_remote_url(url).is_err(), "{url:?}");
        }
    }

    #[test]
    fn rejects_schemes_that_dispatch_to_a_remote_helper() {
        for url in [
            "f://attacker.example/repo",
            "e://x",
            "7z://attacker.example/repo",
            "C://repos/repo.git",
        ] {
            assert!(validate_remote_url(url).is_err(), "{url:?}");
        }
    }

    #[test]
    fn accepts_transports_git_implements_itself() {
        for url in [
            "http://git.internal/team/repo.git",
            "ftp://example.com/repo.git",
            "ftps://example.com/repo.git",
            "git+ssh://example.com/repo.git",
            "ssh+git://example.com/repo.git",
            // An ssh alias named after a scheme, and Git's SCP-style form.
            "git:org/repo.git",
            "ssh:git@example.com/repo.git",
            // A `::` inside an IPv6 literal is not remote-helper syntax.
            "ssh://[::1]/org/repo.git",
        ] {
            assert!(validate_remote_url(url).is_ok(), "{url:?}");
        }
    }

    #[test]
    fn rejects_options_remote_helpers_and_unsupported_schemes() {
        for url in [
            "",
            "--upload-pack=touch /tmp/pwned",
            "ext::sh -c touch /tmp/pwned",
            "hg::example/repo",
            "rsync://example.com/repo.git",
            "hg://example.com/repo.git",
        ] {
            assert!(validate_remote_url(url).is_err(), "{url:?}");
        }
    }
}
