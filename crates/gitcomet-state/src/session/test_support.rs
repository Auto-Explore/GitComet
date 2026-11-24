use std::{cell::RefCell, path::PathBuf};

thread_local! {
    static SESSION_FILE_PATH_OVERRIDES: RefCell<Vec<Option<PathBuf>>> = const { RefCell::new(Vec::new()) };
}

pub(crate) struct TestSessionFilePathGuard;

pub(crate) fn push_test_session_file_path_override(
    path: impl Into<Option<PathBuf>>,
) -> TestSessionFilePathGuard {
    SESSION_FILE_PATH_OVERRIDES.with(|stack| stack.borrow_mut().push(path.into()));
    TestSessionFilePathGuard
}

impl Drop for TestSessionFilePathGuard {
    fn drop(&mut self) {
        SESSION_FILE_PATH_OVERRIDES.with(|stack| {
            let popped = stack.borrow_mut().pop();
            debug_assert!(popped.is_some(), "session path override stack underflow");
        });
    }
}

pub(super) fn session_file_path_override() -> Option<Option<PathBuf>> {
    SESSION_FILE_PATH_OVERRIDES.with(|stack| stack.borrow().last().cloned())
}
