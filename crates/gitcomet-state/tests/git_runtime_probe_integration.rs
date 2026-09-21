//! Lives in its own test binary because it swaps the process-global Git
//! runtime. Every `AppState::default()` reads that global, so in the lib test
//! binary a concurrent test could see the probe script — or, when exec'ing the
//! freshly written script hits ETXTBSY, an unavailable runtime that makes the
//! reducer drop Git-backed messages.

use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::process::{
    GitExecutablePreference, current_git_executable_preference, current_git_runtime,
    install_git_executable_preference,
};
use gitcomet_core::services::{GitBackend, GitRepository};
use gitcomet_state::model::RepoId;
use gitcomet_state::msg::Msg;
use gitcomet_state::store::AppStore;
use std::fs;
use std::path::Path;
use std::sync::Arc;

struct FailingBackend;

impl GitBackend for FailingBackend {
    fn open(&self, _path: &Path) -> Result<Arc<dyn GitRepository>, Error> {
        Err(Error::new(ErrorKind::Unsupported(
            "git runtime probe test backend",
        )))
    }
}

struct GitRuntimePreferenceResetGuard {
    original: GitExecutablePreference,
}

impl GitRuntimePreferenceResetGuard {
    fn install(preference: GitExecutablePreference) -> Self {
        let original = current_git_executable_preference();
        let _ = install_git_executable_preference(preference);
        Self { original }
    }
}

impl Drop for GitRuntimePreferenceResetGuard {
    fn drop(&mut self) {
        let _ = install_git_executable_preference(self.original.clone());
    }
}

#[cfg(unix)]
fn write_git_runtime_probe_script(script_path: &Path, probe_log: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    fs::write(
        script_path,
        format!(
            "#!/bin/sh\nprintf 'probe\\n' >> '{}'\nprintf 'git version 9.9.9-test\\n'\n",
            probe_log.display()
        ),
    )
    .expect("write git runtime probe script");
    let mut permissions = fs::metadata(script_path)
        .expect("git runtime probe script metadata")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(script_path, permissions)
        .expect("set git runtime probe script permissions");
}

#[cfg(windows)]
fn write_git_runtime_probe_script(script_path: &Path, probe_log: &Path) {
    fs::write(
        script_path,
        format!(
            "@echo off\r\necho probe>>\"{}\"\r\necho git version 9.9.9-test\r\n",
            probe_log.display()
        ),
    )
    .expect("write git runtime probe script");
}

fn git_runtime_probe_count(probe_log: &Path) -> usize {
    fs::read_to_string(probe_log)
        .unwrap_or_default()
        .lines()
        .count()
}

#[test]
fn app_store_dispatch_does_not_reprobe_git_runtime_for_git_messages() {
    let temp = tempfile::tempdir().expect("create tempdir for git runtime probe");
    let probe_log = temp.path().join("git-runtime-probes.log");
    #[cfg(unix)]
    let script_path = temp.path().join("git");
    #[cfg(windows)]
    let script_path = temp.path().join("git.cmd");
    write_git_runtime_probe_script(&script_path, &probe_log);

    let _restore =
        GitRuntimePreferenceResetGuard::install(GitExecutablePreference::Custom(script_path));
    let initial_probe_count = git_runtime_probe_count(&probe_log);
    // Otherwise a failed probe (e.g. ETXTBSY) leaves both counts at 0 and the
    // assertions below pass without testing anything.
    assert!(
        initial_probe_count > 0,
        "installing the probe script should run it once: {:?}",
        current_git_runtime()
    );

    let backend: Arc<dyn GitBackend> = Arc::new(FailingBackend);
    let (store, _event_rx) = AppStore::new(backend);

    assert_eq!(
        git_runtime_probe_count(&probe_log),
        initial_probe_count,
        "creating the store should reuse the installed runtime state without probing again"
    );

    store.dispatch(Msg::ReloadRepo {
        repo_id: RepoId(999),
    });

    assert_eq!(
        git_runtime_probe_count(&probe_log),
        initial_probe_count,
        "dispatch should not re-run `git --version` for regular Git-backed messages"
    );
}
