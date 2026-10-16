//! The backend's integration tests in one binary. Each module was a separate
//! test crate before; nextest still runs every test in its own process.

// Its callers re-run Unix-only tests in a child process.
#[cfg(unix)]
#[path = "../support/child_test.rs"]
mod child_test;
#[path = "../support/extension_formats.rs"]
#[allow(dead_code)]
mod formats;
#[path = "../support/source_audit.rs"]
mod source_audit;
#[path = "../support/test_git_env.rs"]
mod test_git_env;

mod annex;
mod apply_file_change;
mod backend;
mod blame;
mod cherry_pick;
mod commit_signature;
mod comparison;
mod conflict_base_checkout;
mod git_cli_scoreboard;
mod git_ops_trace;
mod gitignore;
mod large_files;
mod log;
mod maintenance;
mod operation_identity;
mod patch;
mod refs;
mod refs_api_guard;
mod refs_review_regressions;
mod reftable;
mod remote_management;
mod revert;
mod sha256;
mod squash;
mod ssh_signing_passphrase;
mod status;
mod structural_patch;
mod submodules;
mod tag_push;
mod text_encoding;
mod upstream;
mod upstream_divergence;
mod worktrees;
