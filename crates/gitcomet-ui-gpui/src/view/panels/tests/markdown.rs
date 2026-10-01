use super::*;
use crate::view::mod_helpers::MarkdownSearchSurface;
use crate::view::panes::main::DiffWrapVisualRow;

/// `debug_bounds` takes a `&'static str`; tests that build a selector from a
/// row index need one that outlives the call.
fn leaked_selector(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

/// Source offsets of every picture a document carries, in order.
///
/// A picture's element id and debug selector are both keyed on this, so it is
/// how a test names the picture it wants to look at.
fn picture_offsets(
    document: &crate::view::markdown_preview::MarkdownPreviewDocument,
) -> Vec<usize> {
    document
        .rows
        .iter()
        .flat_map(|row| row.inline_images.iter())
        .map(|inline| inline.source_byte)
        .collect()
}

/// A worktree markdown file, seeded and opened in the rendered preview.
///
/// Every rendered-preview test needs the same seven steps: write the file, push
/// a repo state that lists it, wait for the diff target to settle, hand the
/// pane a ready source preview and a parsed document, and draw. Spelling that
/// out per test hid what each one was actually about.
struct RenderedPreviewFixture {
    workdir: std::path::PathBuf,
    document: Arc<crate::view::markdown_preview::MarkdownPreviewDocument>,
}

impl RenderedPreviewFixture {
    fn open(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        repo_id: gitcomet_state::model::RepoId,
        name: &str,
        source: &str,
    ) -> Self {
        Self::open_with_status(
            cx,
            view,
            repo_id,
            name,
            source,
            gitcomet_core::domain::FileStatusKind::Untracked,
        )
    }

    /// The status matters where the preview's gutter does: an added or removed
    /// file draws a change bar, an untracked one does not.
    fn open_with_status(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        repo_id: gitcomet_state::model::RepoId,
        name: &str,
        source: &str,
        status: gitcomet_core::domain::FileStatusKind,
    ) -> Self {
        let workdir = open_rendered_markdown_preview(cx, view, repo_id, name, source, status);
        let document = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            match &pane.worktree_markdown.document {
                gitcomet_state::model::Loadable::Ready(document) => Arc::clone(document),
                other => panic!("expected a ready preview, got {other:?}"),
            }
        });
        Self { workdir, document }
    }

    /// Document index of the first row whose text is exactly `text`.
    fn row_ix(&self, text: &str) -> usize {
        self.document
            .rows
            .iter()
            .position(|row| row.text.as_ref() == text)
            .unwrap_or_else(|| {
                panic!(
                    "no row reads {text:?}; rows: {:?}",
                    self.document
                        .rows
                        .iter()
                        .map(|row| row.text.as_ref())
                        .collect::<Vec<_>>()
                )
            })
    }

    /// Source offsets of every picture the document carries, in order.
    fn picture_offsets(&self) -> Vec<usize> {
        picture_offsets(&self.document)
    }

    fn cleanup(self) {
        std::fs::remove_dir_all(&self.workdir).expect("cleanup preview fixture");
    }
}

/// Seed a rendered worktree markdown preview for `source` and draw it.
fn open_rendered_markdown_preview(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    name: &str,
    source: &str,
    status: gitcomet_core::domain::FileStatusKind,
) -> std::path::PathBuf {
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_{name}", std::process::id()));
    let file_rel = std::path::PathBuf::from("docs/preview.md");
    let abs_path = workdir.join(&file_rel);
    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        file_rel.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture parent dir"))
        .expect("create preview workdir");
    std::fs::write(&abs_path, source.as_bytes()).expect("write preview fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                status,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        view,
        "rendered markdown preview target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| format!("repo={:?}", pane.active_repo().map(|repo| repo.id)),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::clone(&preview_lines),
                    source.len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.worktree_markdown.path = Some(abs_path.clone());
                pane.worktree_markdown.source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown.document = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(source)
                        .expect("preview fixture parses"),
                ));
                pane.worktree_markdown.inflight = None;
                cx.notify();
            });
        });
    });

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    workdir
}

fn table_row_ixs(fixture: &RenderedPreviewFixture) -> Vec<usize> {
    fixture
        .document
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            matches!(
                row.kind,
                crate::view::markdown_preview::MarkdownPreviewRowKind::TableRow { .. }
            )
        })
        .map(|(ix, _)| ix)
        .collect()
}

/// Press at `from`, drag to `to`, release.
///
/// A click and a drag are different gestures: the press begins the selection,
/// the move extends it, and only the release ends it.
fn drag_preview_selection(
    cx: &mut gpui::VisualTestContext,
    from: gpui::Point<Pixels>,
    to: gpui::Point<Pixels>,
) {
    cx.simulate_mouse_move(from, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: from,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_mouse_move(
        to,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_event(gpui::MouseUpEvent {
        position: to,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
    });
    cx.run_until_parked();
}

/// Whatever the preview's selection would put on the clipboard.
fn copied_preview_selection(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> Option<String> {
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.copy_selected_diff_text_to_clipboard(cx)
        });
    });
    cx.read_from_clipboard().and_then(|item| item.text())
}

/// Show `old`/`new` as the rendered markdown diff in `mode`.
fn open_rendered_markdown_diff_in(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    name: &str,
    old_text: &str,
    new_text: &str,
    mode: DiffViewMode,
) -> std::path::PathBuf {
    let workdir = open_rendered_markdown_diff(cx, view, repo_id, name, old_text, new_text);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = mode;
                cx.notify();
            });
        });
    });
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }
    workdir
}

fn row_ix_with_text(
    doc: &crate::view::markdown_preview::MarkdownPreviewDocument,
    text: &str,
) -> usize {
    doc.rows
        .iter()
        .position(|row| row.text.as_ref() == text)
        .unwrap_or_else(|| panic!("no row reads {text:?}"))
}

/// Seed a working-tree markdown diff and show it rendered, inline.
fn open_rendered_markdown_diff(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    name: &str,
    old_text: &str,
    new_text: &str,
) -> std::path::PathBuf {
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_{name}", std::process::id()));
    let file_rel = std::path::PathBuf::from("docs/long.md");
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        file_rel.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");
    seed_file_diff_state(cx, view, repo_id, &workdir, &file_rel, old_text, new_text);
    wait_for_main_pane_condition(
        cx,
        view,
        "rendered markdown diff target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| format!("repo={:?}", pane.active_repo().map(|repo| repo.id)),
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_markdown.cache_repo_id = Some(repo_id);
                pane.diff_markdown.cache_rev = 1;
                pane.diff_markdown.cache_target = Some(target.clone());
                pane.diff_markdown.preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::build_markdown_diff_preview(old_text, new_text)
                        .expect("markdown diff preview should parse"),
                ));
                pane.diff_markdown.inflight = None;
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.diff_view = DiffViewMode::Inline;
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);
    workdir
}

// ── Review findings: rendered markdown diff interactions ─────────────────

/// A point on a link in row `row_ix` of `region`, probed across its hitbox.
fn point_on_link_in_region(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    row_ix: usize,
    region: DiffTextRegion,
) -> gpui::Point<Pixels> {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let bounds = pane
            .diff_text_hitbox_bounds_for_tests(row_ix, region)
            .unwrap_or_else(|| panic!("row {row_ix} of {region:?} is drawn"));
        let mut y = bounds.top() + px(2.0);
        while y < bounds.bottom() {
            let mut x = bounds.left();
            while x < bounds.right() {
                let position = point(x, y);
                if pane
                    .markdown_preview_link_span_at(row_ix, region, position)
                    .is_some()
                {
                    return position;
                }
                x += px(2.0);
            }
            y += px(4.0);
        }
        panic!("no link in row {row_ix} of {region:?}")
    })
}

fn popover_kind(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> Option<PopoverKind> {
    cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    })
}

fn close_popover(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) {
    cx.update(|_window, app| {
        let host = view.read(app).popover_host.clone();
        host.update(app, |host, cx| host.close_popover(cx));
    });
    draw_and_drain_test_window(cx);
}

// ── Known rendering bugs: regression tests written before the fixes ──────

fn draw_frames(cx: &mut gpui::VisualTestContext, frames: usize) {
    for _ in 0..frames {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }
}

/// A conflicted `conflict.md` in the merge tool's rendered preview, parsed.
fn open_conflict_markdown_preview(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &std::path::Path,
    [base, ours, theirs]: [&str; 3],
) {
    let file_rel = std::path::PathBuf::from("conflict.md");
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, workdir);
            set_test_conflict_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            let merged = format!("<<<<<<< ours\n{ours}=======\n{theirs}>>>>>>> theirs\n");
            set_test_conflict_file(&mut repo, file_rel.clone(), base, ours, theirs, &merged);
            repo.conflict_state.conflict_file_load_mode =
                gitcomet_state::model::ConflictFileLoadMode::Full;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_frames(cx, 1);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Preview;
                cx.notify();
            });
        });
    });
    draw_frames(cx, 3);
}

mod images;
mod links;
mod rendering;
mod selection_search;
mod task_editing;
