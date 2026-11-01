use super::{
    GitAuthKind, SessionPassphrase, StagedGitAuth, lock_or_recover, session_passphrase_slot,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// A credential request actually observed by the helper, without its secret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthChallenge {
    pub kind: GitAuthKind,
    pub prompt: String,
    pub key_path: Option<PathBuf>,
    pub key_fingerprint: Option<[u8; 32]>,
}

impl AuthChallenge {
    pub fn from_prompt(kind: GitAuthKind, prompt: impl Into<String>) -> Self {
        let prompt = prompt.into();
        let key_path = (kind == GitAuthKind::Passphrase)
            .then(|| key_path_from_prompt(&prompt))
            .flatten();
        let key_fingerprint = key_path.as_deref().and_then(key_fingerprint);
        Self {
            kind,
            prompt,
            key_path,
            key_fingerprint,
        }
    }

    /// A FIDO security key asking for its PIN rather than a key passphrase.
    pub fn is_pin(&self) -> bool {
        self.kind == GitAuthKind::Passphrase
            && self.prompt.to_ascii_lowercase().contains("enter pin")
    }
}

/// Supplying credentials does not prove either key unlock or Git success.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthOutcome {
    NotRequested,
    NeedsInput(AuthChallenge),
    Supplied,
    Verified,
    Rejected(AuthChallenge),
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PassphraseValidation {
    Verified,
    Rejected,
    Unknown,
}

fn key_path_from_prompt(prompt: &str) -> Option<PathBuf> {
    // OpenSSH authentication truncates its displayed key path at 100 bytes.
    // Never guess at a truncated, relative, or custom prompt's identity.
    let prompt = prompt.trim();
    let raw = if let Some(raw) = prompt.strip_prefix("Enter passphrase for key '") {
        let raw = raw.strip_suffix("':")?;
        if raw.len() >= 100 {
            return None;
        }
        raw
    } else if let Some(raw) = prompt.strip_prefix("Enter passphrase for \"") {
        raw.strip_suffix("\":")?
    } else if let Some(raw) = prompt.strip_prefix("Enter passphrase for key ") {
        let raw = raw.strip_suffix(':')?;
        if raw.len() >= 100 {
            return None;
        }
        raw
    } else {
        let raw = prompt.strip_prefix("Enter passphrase for ")?;
        raw.strip_suffix(':')?
    };
    if raw.contains(['\n', '\r', '\0']) {
        return None;
    }
    let path = local_key_path(raw);
    path.is_absolute()
        .then(|| std::fs::canonicalize(path).ok())
        .flatten()
        .map(crate::path_utils::strip_windows_verbatim_prefix)
}

/// Git for Windows' bundled ssh names keys by their MSYS path (`/c/Users/...`
/// or `/cygdrive/c/...`), which is not absolute to Windows itself.
#[cfg(windows)]
fn local_key_path(raw: &str) -> PathBuf {
    let drive_path = raw
        .strip_prefix("/cygdrive/")
        .or_else(|| raw.strip_prefix('/'))
        .and_then(|rest| {
            let mut chars = rest.chars();
            let drive = chars.next().filter(char::is_ascii_alphabetic)?;
            let tail = chars.as_str();
            (tail.is_empty() || tail.starts_with('/')).then(|| {
                format!(
                    "{}:/{}",
                    drive.to_ascii_uppercase(),
                    tail.trim_start_matches('/')
                )
            })
        });
    PathBuf::from(drive_path.as_deref().unwrap_or(raw))
}

#[cfg(not(windows))]
fn local_key_path(raw: &str) -> PathBuf {
    PathBuf::from(raw)
}

pub(super) fn key_fingerprint(path: &Path) -> Option<[u8; 32]> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return None;
    }
    Some(Sha256::digest(std::fs::read(path).ok()?).into())
}

/// Called only for a current, successfully validated challenge. A key-content
/// fingerprint invalidates this session entry when the private key changes.
pub fn remember_verified_passphrase(challenge: &AuthChallenge, auth: &StagedGitAuth) -> bool {
    let Some(path) = challenge.key_path.as_ref() else {
        return false;
    };
    if auth.kind != GitAuthKind::Passphrase || auth.secret.is_empty() {
        return false;
    }
    let Some(fingerprint) = key_fingerprint(path) else {
        return false;
    };
    if challenge.key_fingerprint != Some(fingerprint) {
        return false;
    }
    lock_or_recover(session_passphrase_slot()).insert(
        challenge.prompt.clone(),
        SessionPassphrase {
            secret: auth.secret.clone(),
            key: Some((path.clone(), fingerprint)),
        },
    );
    true
}

/// `challenge` was just observed, so its fingerprint is the key's current one.
pub(super) fn challenge_is_verified(challenge: &AuthChallenge, secret: Option<&str>) -> bool {
    let (Some(path), Some(fingerprint)) = (challenge.key_path.as_ref(), challenge.key_fingerprint)
    else {
        return false;
    };
    lock_or_recover(session_passphrase_slot())
        .values()
        .any(|entry| {
            secret.is_none_or(|secret| secret == entry.secret.as_str())
                && entry
                    .key
                    .as_ref()
                    .is_some_and(|(key, digest)| key == path && *digest == fingerprint)
        })
}

pub(super) fn forget_challenge(challenge: &AuthChallenge) {
    lock_or_recover(session_passphrase_slot()).retain(|prompt, entry| {
        prompt != &challenge.prompt
            && !entry
                .key
                .as_ref()
                .is_some_and(|(path, _)| Some(path) == challenge.key_path.as_ref())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_complete_absolute_key_prompts_can_be_locally_verified() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("key with spaces");
        std::fs::write(&key, b"key fixture").unwrap();
        for prompt in [
            format!("Enter passphrase for key '{}': ", key.display()),
            format!("Enter passphrase for \"{}\": ", key.display()),
        ] {
            assert_eq!(
                AuthChallenge::from_prompt(GitAuthKind::Passphrase, prompt).key_path,
                Some(crate::path_utils::canonicalize_or_original(key.clone())),
            );
        }
        for prompt in [
            "Enter passphrase for key 'relative':".to_string(),
            format!("Enter passphrase for key '{}':", "x".repeat(100)),
            "custom PIN request".into(),
        ] {
            assert!(
                AuthChallenge::from_prompt(GitAuthKind::Passphrase, prompt)
                    .key_path
                    .is_none()
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn git_for_windows_drive_paths_identify_the_local_key() {
        // Git for Windows' bundled ssh names keys as /c/Users/... in prompts.
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("id_test");
        std::fs::write(&key, b"key fixture").unwrap();
        let windows = key.to_string_lossy().replace('\\', "/");
        let (drive, rest) = windows.split_once(":/").unwrap();
        let msys = format!("/{}/{rest}", drive.to_ascii_lowercase());
        assert_eq!(
            AuthChallenge::from_prompt(
                GitAuthKind::Passphrase,
                format!("Enter passphrase for key '{msys}': "),
            )
            .key_path,
            Some(crate::path_utils::canonicalize_or_original(key)),
        );
    }
}
