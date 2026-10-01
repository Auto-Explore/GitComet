#[path = "support/extension_formats.rs"]
mod formats;

#[test]
fn sha256_open_stage_commit_and_uncommitted_blame() {
    formats::open_stage_commit_and_uncommitted_blame("sha256", "files");
}

#[test]
fn sha256_history_loose_commit_graph_and_packs() {
    formats::history_loose_commit_graph_and_packs("sha256", "files");
}

#[test]
fn sha256_submodules() {
    formats::submodules_and_unconfigured_gitlinks("sha256", "files");
}

#[test]
fn sha256_status_diff_commit_reflog_and_file_history() {
    formats::status_diff_commit_reflog_and_file_history("sha256", "files");
}

#[test]
fn revision_lookups_resolve_in_both_object_formats() {
    // SHA-1 files repositories take gix's rev-parse directly; SHA-256 does not.
    for hash in ["sha1", "sha256"] {
        formats::revision_lookups(hash, "files");
    }
}
