use gitcomet_core::auth::askpass::PromptAuth;
use gitcomet_core::auth::load_session_passphrases;
use gitcomet_core::services::{CancellationToken, Result};
use std::cell::RefCell;
use std::ffi::OsString;
use std::process::Command;

struct BackgroundGit {
    cancellation: CancellationToken,
    auth: Option<PromptAuth>,
    empty_hooks: tempfile::TempDir,
}

thread_local! {
    static BACKGROUND_GIT: RefCell<Option<BackgroundGit>> = const { RefCell::new(None) };
}

struct BackgroundGitScope(Option<BackgroundGit>);

impl Drop for BackgroundGitScope {
    fn drop(&mut self) {
        BACKGROUND_GIT.with(|slot| slot.replace(self.0.take()));
    }
}

pub(crate) fn with_background_git<T>(
    cancellation: &CancellationToken,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    cancellation.check_cancelled()?;
    let background = BackgroundGit {
        cancellation: cancellation.clone(),
        auth: PromptAuth::from_cached_passphrases(load_session_passphrases()),
        empty_hooks: tempfile::tempdir()?,
    };
    let _scope = BackgroundGitScope(BACKGROUND_GIT.with(|slot| slot.replace(Some(background))));
    let result = run();
    cancellation.check_cancelled()?;
    result
}

pub(crate) fn background_git_enabled() -> bool {
    BACKGROUND_GIT.with(|slot| slot.borrow().is_some())
}

pub(super) fn disable_background_git_hooks(command: &mut Command) {
    BACKGROUND_GIT.with(|slot| {
        if let Some(background) = slot.borrow().as_ref() {
            let mut hooks_path = OsString::from("core.hooksPath=");
            hooks_path.push(background.empty_hooks.path());
            command
                .arg("-c")
                .arg(hooks_path)
                .args(["-c", "hook.reference-transaction.enabled=false"]);
        }
    });
}

pub(super) fn background_git_auth() -> Option<Option<PromptAuth>> {
    BACKGROUND_GIT.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|background| background.auth.clone())
    })
}

pub(super) fn background_git_cancelled() -> bool {
    BACKGROUND_GIT.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|background| background.cancellation.is_cancelled())
    })
}
