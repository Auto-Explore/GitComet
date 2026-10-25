use super::*;
use crate::view::panes::diff::{
    apply_file_editor_pair_highlights, file_editor_blame_line_for_editor_line,
    file_editor_provider_binding_key,
};
use gitcomet_core::services::GitBackend;
use palette::IntoColor;
use std::path::PathBuf;

#[path = "file_editor_scrolling.rs"]
mod scrolling;

mod close_recovery;

/// A repo whose working tree holds `file_rel` with `contents`, already showing
/// that file in the editor.
fn editor_state(
    repo_id: gitcomet_state::model::RepoId,
    workdir: &Path,
    file_rel: &Path,
) -> Arc<AppState> {
    let mut repo = opening_repo_state(repo_id, workdir);
    repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::working_tree(
        file_rel.to_path_buf(),
        gitcomet_core::domain::DiffArea::Unstaged,
    ));
    repo.diff_state.content_preview = true;
    repo.diff_state.edit_mode = true;
    app_state_with_repo(repo, repo_id)
}

fn two_repo_editor_state(
    first: (gitcomet_state::model::RepoId, &Path, &Path),
    second: (gitcomet_state::model::RepoId, &Path, &Path),
    active_repo: gitcomet_state::model::RepoId,
) -> Arc<AppState> {
    let make_repo =
        |(repo_id, workdir, file_rel): (gitcomet_state::model::RepoId, &Path, &Path)| {
            let mut repo = opening_repo_state(repo_id, workdir);
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::working_tree(
                file_rel.to_path_buf(),
                gitcomet_core::domain::DiffArea::Unstaged,
            ));
            repo.diff_state.content_preview = true;
            repo.diff_state.edit_mode = true;
            repo
        };
    Arc::new(AppState {
        repos: vec![make_repo(first), make_repo(second)],
        active_repo: Some(active_repo),
        ..Default::default()
    })
}

fn unique_workdir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{label}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

mod blame;
mod input_and_layout;
mod lifecycle;
mod loading_and_identity;
mod saving;
mod syntax;
