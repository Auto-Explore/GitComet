pub mod auth;
pub mod conflict_labels;
pub mod conflict_output;
pub mod conflict_session;
pub mod diff;
pub mod domain;
pub mod error;
pub mod file_diff;
pub mod fs_utils;
pub mod git_operation;
pub mod git_ops_trace;
pub mod gitignore;
pub mod hex;
pub mod merge;
pub mod merge_extraction;
pub mod mergetool_trace;
pub mod path_utils;
pub mod platform;
pub mod process;
pub mod remote_url;
pub mod services;
pub mod squash;
pub mod text_utils;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
