use crate::process::{
    GitExecutablePreference, current_git_executable_preference, install_git_executable_path,
    install_git_executable_preference,
};
use std::process::Command;

const CHILD_ENV: &str = "GITCOMET_LOCALIZED_GIT_TEST_CHILD";

/// Runs `test` (the caller's test `name`) again in a child process whose
/// locale is not C and whose backend Git is `script`. The Git executable is
/// process-wide, so no other test may share the process.
pub fn run(module: &str, name: &str, script: &str, test: impl FnOnce()) {
    if std::env::var_os(CHILD_ENV).is_none() {
        let mut child = Command::new(std::env::current_exe().expect("current test executable"));
        child.env(CHILD_ENV, "1").env("LC_ALL", "POSIX");
        super::child_test::run(&mut child, module, name);
        return;
    }
    let dir = tempfile::tempdir().expect("create localized Git directory");
    let wrapper = dir.path().join("localized-git");
    super::script::write_executable(&wrapper, script);
    let _git = Installed::new(&wrapper);
    test();
}

struct Installed {
    original: GitExecutablePreference,
}

impl Installed {
    fn new(wrapper: &std::path::Path) -> Self {
        let original = current_git_executable_preference();
        let runtime = install_git_executable_path(Some(wrapper.to_path_buf()));
        assert!(
            runtime.is_available(),
            "localized Git test wrapper must be executable: {:?}",
            runtime.availability
        );
        Self { original }
    }
}

impl Drop for Installed {
    fn drop(&mut self) {
        let _ = install_git_executable_preference(self.original.clone());
    }
}
