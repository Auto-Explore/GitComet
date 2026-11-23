use crate::error::{Error, ErrorKind};
use std::cell::Cell;
use std::marker::PhantomData;
use std::process::Command;
use std::rc::Rc;

/// A Git transport implemented by Git itself rather than a `git-remote-*`
/// helper discovered on `PATH`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RemoteProtocol {
    Https,
    Ssh,
    Git,
    File,
    Http,
    Ftp,
    Ftps,
    GitSsh,
    SshGit,
}

impl RemoteProtocol {
    pub const ALL: [Self; 9] = [
        Self::Https,
        Self::Ssh,
        Self::Git,
        Self::File,
        Self::Http,
        Self::Ftp,
        Self::Ftps,
        Self::GitSsh,
        Self::SshGit,
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Self::Https => "https",
            Self::Ssh => "ssh",
            Self::Git => "git",
            Self::File => "file",
            Self::Http => "http",
            Self::Ftp => "ftp",
            Self::Ftps => "ftps",
            Self::GitSsh => "git+ssh",
            Self::SshGit => "ssh+git",
        }
    }

    pub fn from_key(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "https" => Some(Self::Https),
            "ssh" => Some(Self::Ssh),
            "git" => Some(Self::Git),
            "file" => Some(Self::File),
            "http" => Some(Self::Http),
            "ftp" => Some(Self::Ftp),
            "ftps" => Some(Self::Ftps),
            "git+ssh" => Some(Self::GitSsh),
            "ssh+git" => Some(Self::SshGit),
            _ => None,
        }
    }
}

/// User-selected allowlist for explicit Git remote URL schemes.
///
/// Schemeless local paths and SCP-style SSH locations are outside this list
/// and remain supported. Remote-helper syntax (`ext::...`, `hg::...`, and
/// similar) is always rejected and cannot be enabled by this policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RemoteUrlPolicy {
    allowed: u16,
}

thread_local! {
    static REMOTE_URL_POLICY: Cell<Option<RemoteUrlPolicy>> = const { Cell::new(None) };
}

pub fn current_remote_url_policy() -> Option<RemoteUrlPolicy> {
    REMOTE_URL_POLICY.get()
}

pub struct ScopedRemoteUrlPolicy {
    previous: Option<RemoteUrlPolicy>,
    thread_bound: PhantomData<Rc<()>>,
}

impl ScopedRemoteUrlPolicy {
    pub fn set(policy: Option<RemoteUrlPolicy>) -> Self {
        Self {
            previous: REMOTE_URL_POLICY.replace(policy),
            thread_bound: PhantomData,
        }
    }
}

impl Drop for ScopedRemoteUrlPolicy {
    fn drop(&mut self) {
        REMOTE_URL_POLICY.set(self.previous);
    }
}

impl RemoteUrlPolicy {
    const fn bit(protocol: RemoteProtocol) -> u16 {
        1 << protocol as u16
    }

    pub const fn none() -> Self {
        Self { allowed: 0 }
    }

    pub fn from_keys<'a>(keys: impl IntoIterator<Item = &'a str>) -> Self {
        let mut policy = Self::none();
        for key in keys {
            if let Some(protocol) = RemoteProtocol::from_key(key) {
                policy.set_allowed(protocol, true);
            }
        }
        policy
    }

    pub const fn allows(self, protocol: RemoteProtocol) -> bool {
        self.allowed & Self::bit(protocol) != 0
    }

    pub fn set_allowed(&mut self, protocol: RemoteProtocol, allowed: bool) {
        let bit = Self::bit(protocol);
        if allowed {
            self.allowed |= bit;
        } else {
            self.allowed &= !bit;
        }
    }

    pub fn with_allowed(mut self, protocol: RemoteProtocol, allowed: bool) -> Self {
        self.set_allowed(protocol, allowed);
        self
    }

    pub fn allowed_protocols(self) -> impl Iterator<Item = RemoteProtocol> {
        RemoteProtocol::ALL
            .into_iter()
            .filter(move |protocol| self.allows(*protocol))
    }
}

impl Default for RemoteUrlPolicy {
    fn default() -> Self {
        Self::none()
            .with_allowed(RemoteProtocol::Https, true)
            .with_allowed(RemoteProtocol::Ssh, true)
            .with_allowed(RemoteProtocol::Git, true)
            .with_allowed(RemoteProtocol::File, true)
    }
}

/// Validate a remote using the secure default allowlist.
pub fn validate_remote_url(url: &str) -> Result<(), Error> {
    validate_remote_url_with_policy(url, RemoteUrlPolicy::default())
}

/// Validate a user-supplied Git remote source without rejecting local paths or
/// SCP-style URLs such as `git@example.com:org/repo.git`.
pub fn validate_remote_url_with_policy(url: &str, policy: RemoteUrlPolicy) -> Result<(), Error> {
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
    let Some(protocol) = RemoteProtocol::from_key(&scheme) else {
        return Err(invalid(format!(
            "unsupported remote URL scheme `{scheme}` (allowed: {})",
            allowed_scheme_list(policy)
        )));
    };
    if !policy.allows(protocol) {
        return Err(invalid(format!(
            "remote URL scheme `{scheme}` is blocked by Settings > Security / Privacy > Allowed remote protocols (allowed: {})",
            allowed_scheme_list(policy)
        )));
    }
    Ok(())
}

pub fn restrict_native_git_transports(
    command: &mut Command,
    policy: RemoteUrlPolicy,
    explicit_source: Option<&str>,
) {
    use RemoteProtocol::{File, Ftp, Ftps, Git, GitSsh, Http, Https, Ssh, SshGit};

    const NATIVE_PROTOCOLS: [(&str, &[RemoteProtocol]); 7] = [
        ("file", &[File]),
        ("ssh", &[Ssh, GitSsh, SshGit]),
        ("git", &[Git]),
        ("http", &[Http]),
        ("https", &[Https]),
        ("ftp", &[Ftp]),
        ("ftps", &[Ftps]),
    ];
    const ALLOW_PROTOCOL: &str = "GIT_ALLOW_PROTOCOL";

    let schemeless_transport = explicit_source.and_then(schemeless_native_git_transport);
    let mut blocked = Vec::new();
    for (name, protocols) in NATIVE_PROTOCOLS {
        let accepts_schemeless = schemeless_transport == Some(name)
            || (explicit_source.is_none() && matches!(name, "file" | "ssh"));
        if !protocols.iter().any(|&protocol| policy.allows(protocol)) && !accepts_schemeless {
            command.args(["-c", &format!("protocol.{name}.allow=never")]);
            blocked.push(name);
        }
    }

    let caller_allowlist = command
        .get_envs()
        .find_map(|(key, value)| (key == ALLOW_PROTOCOL).then_some(value))
        .map_or_else(
            || std::env::var_os(ALLOW_PROTOCOL),
            |value| value.map(ToOwned::to_owned),
        );
    if let Some(caller_allowlist) = caller_allowlist {
        let allowed = caller_allowlist
            .to_str()
            .unwrap_or_default()
            .split(':')
            .filter(|name| {
                !blocked.contains(name) && NATIVE_PROTOCOLS.iter().any(|(native, _)| native == name)
            })
            .collect::<Vec<_>>()
            .join(":");
        command.env(ALLOW_PROTOCOL, allowed);
    }
}

fn schemeless_native_git_transport(source: &str) -> Option<&'static str> {
    let source = source.trim();
    let (_, remainder) = git_transport_prefix(source);
    if remainder.starts_with("://") || remainder.starts_with("::") {
        return None;
    }
    let has_windows_drive = cfg!(windows)
        && source
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        && source.as_bytes().get(1) == Some(&b':');
    let is_local = has_windows_drive
        || source
            .find(':')
            .is_none_or(|colon| source.find('/').is_some_and(|slash| slash < colon));
    Some(if is_local { "file" } else { "ssh" })
}

fn allowed_scheme_list(policy: RemoteUrlPolicy) -> String {
    let allowed = policy
        .allowed_protocols()
        .map(RemoteProtocol::key)
        .collect::<Vec<_>>()
        .join(", ");
    if allowed.is_empty() {
        "none".to_string()
    } else {
        allowed
    }
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
    use super::{
        RemoteProtocol, RemoteUrlPolicy, restrict_native_git_transports, validate_remote_url,
        validate_remote_url_with_policy,
    };
    use std::process::Command;

    #[test]
    fn native_protocol_exceptions_apply_only_to_the_schemeless_source_family() {
        for (source, permitted, blocked) in [
            ("./repo.git", "file", "ssh"),
            ("git@example.test:repo.git", "ssh", "file"),
        ] {
            let mut command = Command::new("git");
            command.env_remove("GIT_ALLOW_PROTOCOL");
            restrict_native_git_transports(&mut command, RemoteUrlPolicy::none(), Some(source));
            let prohibited = format!("protocol.{blocked}.allow=never");
            let accepted = format!("protocol.{permitted}.allow=never");
            assert!(command.get_args().any(|arg| arg == prohibited.as_str()));
            assert!(!command.get_args().any(|arg| arg == accepted.as_str()));
        }
    }

    #[test]
    fn native_restrictions_preserve_caller_protocol_limits() {
        let mut command = Command::new("git");
        command.env("GIT_ALLOW_PROTOCOL", "https:http:file:fixture-helper");
        let policy = RemoteUrlPolicy::none().with_allowed(RemoteProtocol::Https, true);
        restrict_native_git_transports(&mut command, policy, Some("https://example.test/repo"));
        assert_eq!(
            command
                .get_envs()
                .find_map(|(key, value)| (key == "GIT_ALLOW_PROTOCOL").then_some(value))
                .flatten(),
            Some(std::ffi::OsStr::new("https"))
        );

        command.env("GIT_ALLOW_PROTOCOL", "file");
        restrict_native_git_transports(&mut command, policy, Some("https://example.test/repo"));
        assert_eq!(
            command
                .get_envs()
                .find_map(|(key, value)| (key == "GIT_ALLOW_PROTOCOL").then_some(value))
                .flatten(),
            Some(std::ffi::OsStr::new(""))
        );
    }

    #[test]
    fn explicitly_enabled_ssh_alias_uses_the_native_ssh_transport() {
        let policy = RemoteUrlPolicy::none().with_allowed(RemoteProtocol::GitSsh, true);
        let mut command = Command::new("git");
        command.env_remove("GIT_ALLOW_PROTOCOL");
        restrict_native_git_transports(&mut command, policy, Some("git+ssh://example.test/repo"));
        assert!(
            !command
                .get_args()
                .any(|arg| arg == "protocol.ssh.allow=never")
        );
        assert!(
            command
                .get_args()
                .any(|arg| arg == "protocol.file.allow=never")
        );
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "GIT_ALLOW_PROTOCOL" && value.is_none())
        );
    }

    #[test]
    fn secure_defaults_accept_only_the_default_protocols_and_path_forms() {
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

        for url in [
            "http://git.internal/team/repo.git",
            "ftp://example.com/repo.git",
            "ftps://example.com/repo.git",
            "git+ssh://example.com/repo.git",
            "ssh+git://example.com/repo.git",
        ] {
            assert!(validate_remote_url(url).is_err(), "{url:?}");
        }
    }

    #[test]
    fn explicitly_allowed_builtin_protocols_are_accepted() {
        let policy = RemoteUrlPolicy::default()
            .with_allowed(RemoteProtocol::Http, true)
            .with_allowed(RemoteProtocol::Ftp, true)
            .with_allowed(RemoteProtocol::Ftps, true)
            .with_allowed(RemoteProtocol::GitSsh, true)
            .with_allowed(RemoteProtocol::SshGit, true);

        for url in [
            "http://git.internal/team/repo.git",
            "ftp://example.com/repo.git",
            "ftps://example.com/repo.git",
            "git+ssh://example.com/repo.git",
            "ssh+git://example.com/repo.git",
        ] {
            assert!(
                validate_remote_url_with_policy(url, policy).is_ok(),
                "{url:?}"
            );
        }
    }

    #[test]
    fn policy_round_trips_known_keys_and_ignores_unknown_ones() {
        let policy = RemoteUrlPolicy::from_keys(["HTTPS", "http", "unknown"]);
        assert!(policy.allows(RemoteProtocol::Https));
        assert!(policy.allows(RemoteProtocol::Http));
        assert!(!policy.allows(RemoteProtocol::Ssh));
        assert_eq!(
            policy
                .allowed_protocols()
                .map(RemoteProtocol::key)
                .collect::<Vec<_>>(),
            ["https", "http"]
        );
    }

    #[test]
    fn rejects_remote_helper_names_even_when_every_builtin_is_allowed() {
        let policy = RemoteUrlPolicy::from_keys(RemoteProtocol::ALL.map(RemoteProtocol::key));
        // An empty or digit-led helper name still runs `git-remote-<name>`.
        for url in ["::sh -c id", "7z::sh -c whatever", "my-helper::x", "2::x"] {
            assert!(
                validate_remote_url_with_policy(url, policy).is_err(),
                "{url:?}"
            );
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
    fn accepts_schemeless_locations_independently_of_policy() {
        let policy = RemoteUrlPolicy::none();
        for url in [
            "git:org/repo.git",
            "ssh:git@example.com/repo.git",
            "git@example.com:org/repo.git",
            "/tmp/repo.git",
        ] {
            assert!(
                validate_remote_url_with_policy(url, policy).is_ok(),
                "{url:?}"
            );
        }

        // A `::` inside an IPv6 literal is not remote-helper syntax, but the
        // explicit SSH scheme still follows the user's policy.
        assert!(validate_remote_url_with_policy("ssh://[::1]/org/repo.git", policy).is_err());
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
