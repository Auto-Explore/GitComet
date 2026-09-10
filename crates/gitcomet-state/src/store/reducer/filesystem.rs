use crate::model::{AppState, Loadable};
use crate::msg::Effect;
use gitcomet_core::domain::{DiffTarget, FileSource};
use gitcomet_core::filesystem::PathChange;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) fn paths_changed(state: &mut AppState, changes: &[PathChange]) -> Vec<Effect> {
    let mut effects = vec![];
    for repo in &mut state.repos {
        let root = &repo.spec.workdir;
        if !changes.iter().any(|change| {
            change
                .old
                .iter()
                .chain(change.new.iter())
                .any(|p| p.starts_with(root))
        }) {
            continue;
        }
        for change in changes {
            let retarget = |path: &Path| -> PathBuf {
                change
                    .retarget(&root.join(path))
                    .and_then(|p| p.strip_prefix(root).ok().map(Path::to_path_buf))
                    .unwrap_or_else(|| path.to_path_buf())
            };
            repo.file_browser.selection.paths = repo
                .file_browser
                .selection
                .paths
                .iter()
                .map(|p| retarget(p))
                .collect();
            repo.file_browser.selection.focused =
                repo.file_browser.selection.focused.as_deref().map(retarget);
            repo.file_browser.selection.anchor =
                repo.file_browser.selection.anchor.as_deref().map(retarget);
            repo.file_browser.expanded_dirs = repo
                .file_browser
                .expanded_dirs
                .iter()
                .map(|p| Arc::new(retarget(p)))
                .collect();
            if let Some(DiffTarget::WorkingTree { path, .. }) = &mut repo.diff_state.diff_target {
                *path = retarget(path);
            }
            for entry in &mut repo.navigation.view_history.entries {
                if entry.source == FileSource::WorkingDirectory {
                    entry.path = retarget(&entry.path);
                }
            }
            for entry in &mut repo.navigation.main_history.entries {
                if let Some(DiffTarget::WorkingTree { path, .. }) = &mut entry.diff_target {
                    *path = retarget(path);
                }
            }
        }
        repo.file_browser.stale = true;
        repo.file_browser.bump_rev();
        repo.diff_state.diff_target_rev = repo.diff_state.diff_target_rev.wrapping_add(1);
        if !matches!(repo.file_browser.entries, Loadable::NotLoaded) {
            effects.push(Effect::LoadFileBrowser {
                repo_id: repo.id,
                source: repo.file_browser.source.clone(),
            });
        }
        effects.push(Effect::LoadStatus { repo_id: repo.id });
    }
    effects
}
