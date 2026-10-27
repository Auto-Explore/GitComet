//! How automatic fetch reacts to failures: what it retries, how soon, and what
//! waits for the user. Each rule keeps a broken remote from being contacted
//! again on every interval.

use crate::model::AutoFetchInterval;
use gitcomet_core::error::{Error, ErrorKind, GitFailureId};
use std::time::Duration;

const MAX_RETRY_DELAY: Duration = Duration::from_secs(6 * 60 * 60);
const FIRST_NETWORK_BLOCK: Duration = Duration::from_secs(5 * 60);
const FIRST_BUSY_BLOCK: Duration = Duration::from_secs(15 * 60);
const MAX_HOST_BLOCK: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Failure {
    /// Credentials, a passphrase or a host key need the user.
    SignIn,
    /// The remote repository is gone, or this URL does not lead to it.
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
        matches!(self, Self::Network | Self::Busy)
    }
}

const SIGN_IN: &[&str] = &[
    "authentication failed",
    "permission denied (publickey",
    "host key verification failed",
    "invalid username or password",
    "returned error: 401",
    "returned error: 403",
];
const MISSING: &[&str] = &[
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
    let text = failure_text(error).to_ascii_lowercase();
    let mentions = |phrases: &[&str]| phrases.iter().any(|phrase| text.contains(phrase));
    Some(if mentions(SIGN_IN) {
        Failure::SignIn
    } else if mentions(MISSING) {
        Failure::Missing
    } else if mentions(BUSY) {
        Failure::Busy
    } else if mentions(NETWORK) {
        Failure::Network
    } else {
        Failure::Other
    })
}

fn failure_text(error: &Error) -> String {
    let mut text = error.to_string();
    if let ErrorKind::Git(failure) = error.kind() {
        for output in [failure.stderr(), failure.stdout()] {
            text.push('\n');
            text.push_str(&String::from_utf8_lossy(output));
        }
    }
    text
}

/// Remotes a multi-remote fetch reports as failed (`could not fetch 'name'`);
/// empty when the output does not say which.
pub(super) fn failed_remotes(error: &Error) -> Vec<String> {
    const MARKER: &str = "could not fetch ";
    let mut remotes = Vec::new();
    for line in failure_text(error).lines() {
        let Some(at) = line.to_ascii_lowercase().find(MARKER) else {
            continue;
        };
        let rest = line[at + MARKER.len()..].trim();
        let name = match rest.strip_prefix('\'') {
            Some(quoted) => quoted.split('\'').next(),
            None => rest.split_whitespace().next(),
        };
        if let Some(name) = name.filter(|name| !name.is_empty())
            && !remotes.iter().any(|known| known == name)
        {
            remotes.push(name.to_string());
        }
    }
    remotes
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

/// How long a host waits after its `strikes`-th consecutive network or busy
/// failure: from 5 minutes (15 when busy), doubling, at most an hour.
pub(super) fn host_block(failure: Failure, strikes: u32) -> Duration {
    let first = if failure == Failure::Busy {
        FIRST_BUSY_BLOCK
    } else {
        FIRST_NETWORK_BLOCK
    };
    first
        .saturating_mul(1 << strikes.saturating_sub(1).min(4))
        .min(MAX_HOST_BLOCK)
}

#[cfg(test)]
mod tests;
