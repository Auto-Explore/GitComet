use super::*;

#[gpui::test]
fn structural_context_menu_opens_without_installed_extensions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        assert!(root.read(app).extension_window.is_none());
        root.read(app).main_pane.clone().update(app, |pane, cx| {
            pane.open_structural_menu(
                Vec::new(),
                Some("selected text".into()),
                point(px(100.0), px(100.0)),
                window,
                cx,
            );
        });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        assert!(crate::view::test_support::popover_is_open(
            root.read(app),
            app
        ));
    });
}

#[gpui::test]
fn prepared_structural_patch_renders_without_installed_extensions(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::domain::{Diff, RepoSpec};
    use gitcomet_core::patch_action::{DiffActionSnapshot, prepare};
    use gitcomet_state::model::RepoState;
    let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
    let (root, app_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let preview = app_cx.update(|_, app| {
        assert!(root.read(app).extension_window.is_none());
        root.read(app).main_pane.clone().update(app, |pane, cx| {
            let target = DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged);
            let diff = Diff::from_unified(target.clone(), "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-old\n+new\n");
            let prepared = prepare(
                DiffActionSnapshot {
                    diff: diff.clone(),
                    old: Arc::from("old\n"),
                    new: Arc::from("new\n"),
                    policy: Arc::from([]),
                },
                PatchOperation::Stage,
                &SourceLineSelection { old: [1].into(), new: [1].into() },
                "old\n",
                "new\n",
            ).unwrap();
            let expected = prepared.patch().to_vec();
            let mut repo = RepoState::new_opening(RepoId(812), RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet-native-patch-review"),
            });
            repo.open = Loadable::Ready(());
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff = Loadable::Ready(Arc::new(diff));
            let origin = PatchOrigin {
                repo_id: repo.id,
                lifetime: repo.lifetime(),
                view: None,
                file_rev: repo.diff_state.diff_file_rev,
                generation: repo.diff_state.diff_target_rev,
                cache_seq: pane.file_diff_cache_seq,
                target,
                operation: PatchOperation::Stage,
            };
            pane.state = Arc::new(AppState { active_repo: Some(repo.id), repos: vec![repo], ..AppState::test_default() });
            pane.diff_engine_status = DiffEngineStatus::Difftastic { syntactic_changes: true, text_differs: true };
            pane.patch_review = Some(PatchReview {
                sequence: 1,
                origin,
                cancellation: CancellationToken::new(),
                state: ReviewState::Preparing,
                focus: cx.focus_handle(),
                focus_pending: true,
            });
            pane.complete_structural_patch(1, Ok(prepared), cx);
            match &pane.patch_review.as_ref().unwrap().state {
                ReviewState::Ready { pane, prepared, .. } => {
                    assert_eq!(prepared.patch(), expected);
                    pane.clone()
                }
                ReviewState::Error(message) => panic!("Patch preview failed: {message}"),
                ReviewState::Preparing => panic!("Patch preparation did not complete"),
            }
        })
    });
    app_cx.run_until_parked();
    let mounted = preview.clone();
    let (_holder, cx) = cx.add_window_view(move |_, _| PatchPreviewHolder(mounted));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let ready = cx.update(|window, app| {
            let _ = window.draw(app);
            let pane = preview.read(app);
            let renderer = pane.renderer().unwrap().read(app);
            assert_eq!(renderer.effective_diff_engine(), DiffEngine::LineByLine);
            assert_eq!(renderer.diff_view, DiffViewMode::Inline);
            assert_eq!(
                renderer.main_pane_surface().body,
                crate::view::panes::main::MainPaneBody::Patch
            );
            assert!(!renderer.store.policy.allow_edit);
            assert!(!renderer.store.policy.allow_stage);
            pane.exact_patch_ready(app)
        });
        if ready {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Exact patch never became ready"
        );
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    cx.run_until_parked();
    assert!(cx.debug_bounds("diff_body_container").is_some());
    cx.update(|_, app| {
        let renderer = preview.read(app).renderer().unwrap().read(app);
        for text in ["old", "new"] {
            assert!(
                renderer
                    .diff_text_hitboxes
                    .values()
                    .any(|hitbox| hitbox.painted_text.contains(text)),
                "Patch text {text:?} was not painted"
            );
        }
    });
}

struct PatchPreviewHolder(Entity<DiffPaneView>);

impl Render for PatchPreviewHolder {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}
