//! How automatic fetch reacts to failures: what it retries, how soon, and what
//! waits for the user. Each rule keeps a broken remote from being contacted
//! again on every interval.

use crate::model::{AUTO_FETCH_QUIET_MAX_MULTIPLIER, AutoFetchInterval};
use gitcomet_core::error::{Error, ErrorKind, GitFailureId};
use std::time::Duration;

const MAX_RETRY_DELAY: Duration = Duration::from_secs(6 * 60 * 60);
const FIRST_NETWORK_BLOCK: Duration = Duration::from_secs(5 * 60);
const FIRST_BUSY_BLOCK: Duration = Duration::from_secs(15 * 60);
const MAX_HOST_BLOCK: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Failure {
    /// Credentials, a passphrase or a host key need the user. SSH signs in
    /// before it picks a repository and Git stores credentials per host, so
    /// no other repository on the host would get further.
    SignIn,
    /// The remote repository is gone, this URL does not lead to it, or these
    /// credentials may not read it (403).
    Missing,
    /// The host could not be reached.
    Network,
    /// The server refused under load: rate limits and 5xx responses.
    Busy,
    Other,
}

impl Failure {
    /// Retrying cannot succeed until the user acts.
    pub(super) fn pauses(self) -> bool {
        matches!(self, Self::SignIn | Self::Missing)
    }

    /// The host is at fault, so every repository on it waits.
    pub(super) fn blocks_host(self) -> bool {
        matches!(self, Self::SignIn | Self::Network | Self::Busy)
    }
}

const SIGN_IN: &[&str] = &[
    "authentication failed",
    "permission denied (publickey",
    "host key verification failed",
    "invalid username or password",
    "returned error: 401",
    // A background fetch may not ask for credentials, so Git gives up.
    "unable to get password from user",
    "could not read username",
    "could not read password",
];
const MISSING: &[&str] = &[
    "returned error: 403",
    "repository not found",
    "does not appear to be a git repository",
    "returned error: 404",
];
const BUSY: &[&str] = &[
    "returned error: 429",
    "too many requests",
    "rate limit",
    "returned error: 50",
    "http 50",
];
const NETWORK: &[&str] = &[
    "could not resolve host",
    "temporary failure in name resolution",
    "name or service not known",
    "connection refused",
    "connection timed out",
    "operation timed out",
    "network is unreachable",
    "no route to host",
    "failed to connect",
    "connection reset",
    "remote end hung up unexpectedly",
    "early eof",
    "ssl_connect",
    "gnutls_handshake",
];

/// `None` for a cancelled fetch, which is not a failure.
pub(super) fn classify(error: &Error) -> Option<Failure> {
    match error.kind() {
        ErrorKind::Cancelled => return None,
        ErrorKind::Git(failure) if failure.id() == GitFailureId::Timeout => {
            return Some(Failure::Network);
        }
        _ => {}
    }
    if crate::store::reducer::detect_auth_prompt_kind(error).is_some() {
        return Some(Failure::SignIn);
    }
    Some(classify_output(&failure_text(error)))
}

fn classify_output(text: &str) -> Failure {
    let text = text.to_ascii_lowercase();
    let mentions = |phrases: &[&str]| phrases.iter().any(|phrase| text.contains(phrase));
    if mentions(SIGN_IN) {
        Failure::SignIn
    } else if mentions(MISSING) {
        Failure::Missing
    } else if mentions(BUSY) {
        Failure::Busy
    } else if mentions(NETWORK) {
        Failure::Network
    } else {
        Failure::Other
    }
}

fn failure_text(error: &Error) -> String {
    match error.kind() {
        ErrorKind::Git(_) => format!("{error}\n{}", git_output(error)),
        _ => error.to_string(),
    }
}

/// What Git printed, or the message of an error that did not come from Git.
fn git_output(error: &Error) -> String {
    match error.kind() {
        ErrorKind::Git(failure) => [failure.stderr(), failure.stdout()]
            .map(String::from_utf8_lossy)
            .join("\n"),
        _ => error.to_string(),
    }
}

/// Each remote a multi-remote fetch reports as failed (`could not fetch
/// 'name'`), classified by what Git printed for it just before; empty when
/// the output names none.
pub(super) fn remote_failures(error: &Error) -> Vec<(String, Failure)> {
    const MARKER: &str = "could not fetch ";
    let mut failures: Vec<(String, Failure)> = Vec::new();
    let mut output = String::new();
    for line in git_output(error).lines() {
        let Some(at) = line.to_ascii_lowercase().find(MARKER) else {
            output.push_str(line);
            output.push('\n');
            continue;
        };
        let rest = line[at + MARKER.len()..].trim();
        let name = match rest.strip_prefix('\'') {
            Some(quoted) => quoted.split('\'').next(),
            None => rest.split_whitespace().next(),
        };
        if let Some(name) = name.filter(|name| !name.is_empty())
            && !failures.iter().any(|(known, _)| known == name)
        {
            failures.push((name.to_string(), classify_output(&output)));
        }
        output.clear();
    }
    failures
}

/// The host a remote URL connects to, or `None` for a repository on this
/// machine.
pub(super) fn remote_host(url: &str) -> Option<String> {
    let url = url.trim();
    let authority = match url.split_once("://") {
        Some((scheme, _)) if scheme.eq_ignore_ascii_case("file") => return None,
        Some((_, rest)) => rest.split('/').next()?,
        // scp-like `[user@]host:path`. A colon after a path separator, or
        // after a drive letter, is part of a local path.
        None => {
            let (authority, _) = url.split_once(':')?;
            if authority.len() == 1 || authority.contains(['/', '\\']) {
                return None;
            }
            authority
        }
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Consecutive failures stretch the interval to 2×, 4× and then 8×, never
/// past six hours unless the interval itself is longer.
pub(super) fn retry_delay(interval: AutoFetchInterval, failures: u32) -> Duration {
    let interval = interval.duration();
    interval
        .saturating_mul(1 << failures.min(3))
        .min(interval.max(MAX_RETRY_DELAY))
}

/// Inactive repositories whose last `quiet` fetches found nothing new wait 2×
/// and then 4× the interval: most open repositories change rarely.
pub(super) fn quiet_delay(interval: AutoFetchInterval, quiet: u32) -> Duration {
    interval
        .duration()
        .saturating_mul((1 << quiet.min(31)).min(AUTO_FETCH_QUIET_MAX_MULTIPLIER))
}

/// How long a host waits after its `strikes`-th consecutive failure: from 5
/// minutes (15 when busy), doubling, at most an hour. `None` after a sign-in
/// failure: until a manual fetch reaches the host.
pub(super) fn host_block(failure: Failure, strikes: u32) -> Option<Duration> {
    let first = match failure {
        Failure::SignIn => return None,
        Failure::Busy => FIRST_BUSY_BLOCK,
        _ => FIRST_NETWORK_BLOCK,
    };
    Some(
        first
            .saturating_mul(1 << strikes.saturating_sub(1).min(4))
            .min(MAX_HOST_BLOCK),
    )
}

#[cfg(test)]
mod tests;
