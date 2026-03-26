use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread::ThreadId;

pub const GITCOMET_AUTH_KIND_ENV: &str = "GITCOMET_AUTH_KIND";
pub const GITCOMET_AUTH_USERNAME_ENV: &str = "GITCOMET_AUTH_USERNAME";
pub const GITCOMET_AUTH_SECRET_ENV: &str = "GITCOMET_AUTH_SECRET";

pub const GITCOMET_AUTH_KIND_USERNAME_PASSWORD: &str = "username_password";
pub const GITCOMET_AUTH_KIND_PASSPHRASE: &str = "passphrase";
pub const GITCOMET_AUTH_KIND_HOST_VERIFICATION: &str = "host_verification";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitAuthKind {
    UsernamePassword,
    Passphrase,
    HostVerification,
}

#[derive(Clone, Eq, PartialEq)]
pub struct StagedGitAuth {
    pub kind: GitAuthKind,
    pub username: Option<String>,
    pub secret: String,
}

#[derive(Clone, Eq, PartialEq)]
struct PendingStagedGitAuth {
    auth: StagedGitAuth,
    owner_thread: Option<ThreadId>,
}

impl std::fmt::Debug for StagedGitAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const REDACTED: &str = "<redacted>";

        f.debug_struct("StagedGitAuth")
            .field("kind", &self.kind)
            .field("username", &self.username.as_ref().map(|_| REDACTED))
            .field("secret", &REDACTED)
            .finish()
    }
}

fn staged_git_auth_slot() -> &'static Mutex<Option<PendingStagedGitAuth>> {
    static SLOT: OnceLock<Mutex<Option<PendingStagedGitAuth>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Lock a mutex, recovering from poison if a prior holder panicked.
fn lock_or_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn clear_staged_git_auth() {
    let mut guard = lock_or_recover(staged_git_auth_slot());
    *guard = None;
}

pub fn stage_git_auth(auth: StagedGitAuth) {
    let mut guard = lock_or_recover(staged_git_auth_slot());
    *guard = Some(PendingStagedGitAuth {
        auth,
        owner_thread: None,
    });
}

pub fn stage_git_auth_for_current_thread(auth: StagedGitAuth) {
    let mut guard = lock_or_recover(staged_git_auth_slot());
    *guard = Some(PendingStagedGitAuth {
        auth,
        owner_thread: Some(std::thread::current().id()),
    });
}

pub fn take_staged_git_auth() -> Option<StagedGitAuth> {
    let mut guard = lock_or_recover(staged_git_auth_slot());
    let current = std::thread::current().id();
    match guard.as_ref() {
        Some(pending)
            if pending.owner_thread.is_none() || pending.owner_thread == Some(current) =>
        {
            guard.take().map(|pending| pending.auth)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{GitAuthKind, StagedGitAuth};

    #[test]
    fn staged_git_auth_debug_redacts_sensitive_fields() {
        let auth = StagedGitAuth {
            kind: GitAuthKind::UsernamePassword,
            username: Some("alice".to_string()),
            secret: "token-123".to_string(),
        };

        let rendered = format!("{auth:?}");
        assert!(rendered.contains("StagedGitAuth"));
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("alice"));
        assert!(!rendered.contains("token-123"));
    }
}
