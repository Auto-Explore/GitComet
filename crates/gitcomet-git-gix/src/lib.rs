mod backend;
#[doc(hidden)]
pub mod command_trace;
mod ignore;
mod open;
mod refs;
mod repo;
#[cfg(any(test, feature = "test-support"))]
mod test_support;
mod util;

pub use backend::GixBackend;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub use test_support::{
    allow_test_repo_local_mergetool_command, install_test_git_command_environment,
};

/// Optional decoded retention and caller-pinned allocations, in bytes, followed
/// by optional retained row count. Pinned bytes may overlap retained bytes.
#[doc(hidden)]
pub fn shared_history_memory() -> (usize, usize, usize) {
    repo::shared_history_memory()
}
