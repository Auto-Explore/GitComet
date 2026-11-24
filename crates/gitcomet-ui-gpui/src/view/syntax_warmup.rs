use super::GitCometView;
use gitcomet_state::model::Loadable;
use gitcomet_syntax::{DiffSyntaxLanguage, diff_syntax_language_for_path, warm_highlight_spec};
use rustc_hash::FxHashMap;
use std::cmp::Reverse;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const TRACKED_PATH_SAMPLE: NonZeroUsize = NonZeroUsize::new(128).unwrap();
const CHANGED_PATH_SAMPLE: usize = 64;

impl GitCometView {
    pub(super) fn warm_active_repository_syntax(&mut self, cx: &gpui::Context<Self>) {
        if !self.startup_work.initial_frame_complete
            || !crate::ui_runtime::current().uses_background_compute()
        {
            return;
        }
        let Some(repo) = self.active_repo() else {
            return;
        };
        let id = repo.id;
        let lifetime = repo.lifetime_id();
        if repo.is_backend_parked()
            || self
                .startup_work
                .syntax_warmups
                .get(&id)
                .is_some_and(|(previous, _)| *previous == lifetime)
        {
            return;
        }
        // Status has already opened the shared Git index. On a clean worktree it
        // supplies no paths itself, so sample the existing index across its span.
        let (Loadable::Ready(staged), Loadable::Ready(unstaged)) =
            (&repo.staged_status, &repo.worktree_status)
        else {
            return;
        };
        let staged = Arc::clone(staged);
        let unstaged = Arc::clone(unstaged);
        let store = Arc::clone(&self.store);
        let requested_state = Arc::downgrade(&self.state);
        let sample =
            cx.background_executor()
                .spawn_with_priority(gpui::Priority::Low, async move {
                    let tracked = {
                        let Some(repo) = store.attached_repository(id, lifetime.get()) else {
                            return false;
                        };
                        repo.sample_tracked_file_paths(TRACKED_PATH_SAMPLE)
                            .unwrap_or_default()
                    };
                    let changed = staged
                        .iter()
                        .take(CHANGED_PATH_SAMPLE)
                        .chain(unstaged.iter().take(CHANGED_PATH_SAMPLE))
                        .map(|file| file.path.as_path());
                    for language in observed_languages(changed, &tracked) {
                        warm_highlight_spec(language);
                        smol::future::yield_now().await;
                    }
                    true
                });
        let task = cx.spawn(async move |view, cx| {
            if sample.await {
                return;
            }
            let _ = view.update(cx, |view, cx| {
                if let std::collections::hash_map::Entry::Occupied(entry) =
                    view.startup_work.syntax_warmups.entry(id)
                    && entry.get().0 == lifetime
                {
                    drop(entry.remove());
                    // A reactivation may already have published while the worker was parked.
                    if !requested_state.ptr_eq(&Arc::downgrade(&view.state)) {
                        view.warm_active_repository_syntax(cx);
                    }
                }
            });
        });
        self.startup_work
            .syntax_warmups
            .insert(id, (lifetime, task));
    }
}

fn observed_languages<'a>(
    changed: impl Iterator<Item = &'a Path>,
    tracked: &[PathBuf],
) -> Vec<DiffSyntaxLanguage> {
    let mut counts = FxHashMap::<_, (usize, usize)>::default();
    for language in changed.filter_map(diff_syntax_language_for_path) {
        counts.entry(language).or_default().0 += 1;
    }
    for language in tracked.iter().filter_map(diff_syntax_language_for_path) {
        counts.entry(language).or_default().1 += 1;
    }
    let mut languages: Vec<_> = counts.into_iter().collect();
    languages.sort_unstable_by_key(|&(language, count)| (Reverse(count), language));
    languages
        .into_iter()
        .map(|(language, _)| language)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_state::model::{AppState, RepoId, RepoState};
    use gitcomet_state::store::AppStore;

    #[gpui::test]
    fn a_repository_parked_before_sampling_can_warm_when_reattached(cx: &mut gpui::TestAppContext) {
        use crate::view::test_support::TestBackend;
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let directory = tempfile::tempdir().unwrap();
        let id = RepoId(1);
        let mut repo = RepoState::new_opening(
            id,
            gitcomet_core::domain::RepoSpec {
                workdir: directory.path().into(),
            },
        );
        repo.open = Loadable::Ready(());
        repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
        repo.staged_status = Loadable::Ready(Default::default());
        repo.worktree_status = Loadable::Ready(Default::default());
        store.replace_snapshot_for_test(Arc::new(AppState {
            active_repo: Some(id),
            repos: vec![repo],
            ..AppState::test_default()
        }));
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new(store.clone(), events, None, window, cx)
        });
        let request = |cx: &mut gpui::VisualTestContext| {
            view.update(cx, |view, cx| {
                view.startup_work.initial_frame_complete = true;
                crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
                    view.warm_active_repository_syntax(cx);
                });
                assert!(view.startup_work.syntax_warmups.contains_key(&id));
            });
            cx.run_until_parked();
        };
        request(cx);
        assert!(!view.read_with(cx, |view, _| {
            view.startup_work.syntax_warmups.contains_key(&id)
        }));
        crate::view::test_support::install_working_tree_repositories(&store);
        request(cx);
        assert!(view.read_with(cx, |view, _| {
            view.startup_work.syntax_warmups.contains_key(&id)
        }));
    }

    #[test]
    fn changed_languages_precede_frequent_tracked_languages() {
        let changed = ["file.ts", "lib.rs", "main.rs", "photo.png"]
            .into_iter()
            .map(Path::new);
        let tracked = ["one.c", "two.c", "lib.rs", "script.py", "photo.png"].map(PathBuf::from);
        assert_eq!(
            observed_languages(changed, &tracked),
            [
                DiffSyntaxLanguage::Rust,
                DiffSyntaxLanguage::TypeScript,
                DiffSyntaxLanguage::C,
                DiffSyntaxLanguage::Python
            ]
        );
    }
}
