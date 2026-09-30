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
