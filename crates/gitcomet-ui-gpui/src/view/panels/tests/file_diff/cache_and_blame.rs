use super::*;

mod blame;
mod images;
mod revision_invalidation;
mod search;
mod syntax_documents;
mod yaml_regressions;

/// Opens an unstaged text diff so the diff toolbar (Inline/Split + Blame)
/// renders, and puts the pane in `mode`.
fn push_unstaged_text_diff_for_blame_toggle(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    mode: DiffViewMode,
) {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{fixture_name}",
        std::process::id()
    ));
    let path = PathBuf::from("src/lib.rs");
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        path.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff =
                gitcomet_state::model::Loadable::Ready(Arc::new(gitcomet_core::domain::Diff {
                    target: target.clone(),
                    lines: vec![
                        gitcomet_core::domain::DiffLine {
                            kind: gitcomet_core::domain::DiffLineKind::Context,
                            text: "fn main() {".into(),
                        },
                        gitcomet_core::domain::DiffLine {
                            kind: gitcomet_core::domain::DiffLineKind::Add,
                            text: "    let x = 1;".into(),
                        },
                        gitcomet_core::domain::DiffLine {
                            kind: gitcomet_core::domain::DiffLineKind::Context,
                            text: "}".into(),
                        },
                    ],
                }));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    // Go through the root setter so the root and the pane agree, exactly as the
    // toolbar buttons and the session restore do.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_view_mode(mode, cx);
        });
    });
    draw_and_drain_test_window(cx);
}
