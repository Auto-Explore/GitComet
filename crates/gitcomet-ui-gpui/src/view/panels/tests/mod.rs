#![allow(clippy::type_complexity)]

pub(super) use super::main::{
    next_conflict_diff_split_ratio, show_conflict_save_stage_action,
    show_external_mergetool_actions,
};
pub(super) use super::*;
pub(super) use crate::test_support::{lock_clipboard_test, lock_visual_test};
pub(super) use crate::view::panes::main::PreparedSyntaxViewMode;
pub(super) use gitcomet_core::error::{Error, ErrorKind};
pub(super) use gitcomet_core::services::{GitBackend, GitRepository, Result};
pub(super) use gitcomet_state::store::AppStore;
pub(super) use gpui::{Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, point, px};
pub(super) use std::path::Path;
pub(super) use std::sync::Arc;
pub(super) use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) fn simulate_counted_click(
    cx: &mut gpui::VisualTestContext,
    position: gpui::Point<Pixels>,
    click_count: usize,
) {
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count,
    });
}

fn diff_text_click_position_for_offset_range(
    pane: &MainPaneView,
    visible_ix: usize,
    region: DiffTextRegion,
    target: std::ops::Range<usize>,
) -> Option<gpui::Point<Pixels>> {
    let hitbox = pane.diff_text_hitboxes.get(&(visible_ix, region))?;
    let y = hitbox.bounds.center().y;
    (0..2048usize)
        .find_map(|step| {
            let position = point(hitbox.bounds.left() + px(step as f32), y);
            pane.diff_text_offset_for_position_for_tests(visible_ix, region, position)
                .filter(|offset| target.contains(offset))
                .map(|_| position)
        })
        .or_else(|| Some(hitbox.bounds.center()))
}

pub(super) fn wait_for_diff_text_click_position_for_offset_range(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    visible_ix: usize,
    region: DiffTextRegion,
    target: std::ops::Range<usize>,
    description: &str,
) -> gpui::Point<Pixels> {
    let deadline = std::time::Instant::now() + DEFAULT_MAIN_PANE_WAIT_TIMEOUT;
    loop {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        if let Some(position) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            diff_text_click_position_for_offset_range(&pane, visible_ix, region, target.clone())
        }) {
            return position;
        }

        if std::time::Instant::now() >= deadline {
            let snapshot = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                (
                    pane.diff_visible_len(),
                    pane.diff_text_hitboxes
                        .keys()
                        .cloned()
                        .collect::<Vec<(usize, DiffTextRegion)>>(),
                )
            });
            panic!(
                "timed out waiting for {description}: visible_ix={visible_ix} region={region:?} target={target:?} snapshot={snapshot:?}"
            );
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

const _: () = {
    assert!(COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX > 0.0);
    assert!(COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX <= 400.0);
};

#[test]
fn shows_external_mergetool_actions_only_in_normal_mode() {
    assert!(show_external_mergetool_actions(GitCometViewMode::Normal));
    assert!(!show_external_mergetool_actions(
        GitCometViewMode::FocusedMergetool
    ));
}

#[test]
fn shows_save_stage_action_only_in_normal_mode() {
    assert!(show_conflict_save_stage_action(GitCometViewMode::Normal));
    assert!(!show_conflict_save_stage_action(
        GitCometViewMode::FocusedMergetool
    ));
}

#[test]
fn next_conflict_diff_split_ratio_returns_none_when_main_width_is_not_positive() {
    let state = ConflictDiffSplitResizeState {
        start_x: px(10.0),
        start_ratio: 0.5,
    };
    let ratio = next_conflict_diff_split_ratio(state, px(20.0), [px(-4.0), px(-4.0)]);
    assert!(ratio.is_none());
}

#[test]
fn next_conflict_diff_split_ratio_applies_drag_delta() {
    let state = ConflictDiffSplitResizeState {
        start_x: px(100.0),
        start_ratio: 0.5,
    };
    let ratio = next_conflict_diff_split_ratio(state, px(160.0), [px(300.0), px(300.0)]).unwrap();

    let expected = (0.5 + (60.0 / (300.0 + 300.0 + super::PANE_RESIZE_HANDLE_PX))).clamp(0.1, 0.9);
    assert!((ratio - expected).abs() < 0.0001);
}

#[test]
fn next_conflict_diff_split_ratio_clamps_to_expected_bounds() {
    let state = ConflictDiffSplitResizeState {
        start_x: px(100.0),
        start_ratio: 0.5,
    };
    let min_ratio =
        next_conflict_diff_split_ratio(state, px(-10_000.0), [px(240.0), px(240.0)]).unwrap();
    let max_ratio =
        next_conflict_diff_split_ratio(state, px(10_000.0), [px(240.0), px(240.0)]).unwrap();
    assert_eq!(min_ratio, 0.1);
    assert_eq!(max_ratio, 0.9);
}

#[test]
fn conflict_resolver_strategy_maps_conflict_kinds() {
    use gitcomet_core::conflict_session::ConflictResolverStrategy as S;
    use gitcomet_core::domain::FileConflictKind as K;

    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::BothModified), false),
        Some(S::FullTextResolver),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::BothAdded), false),
        Some(S::FullTextResolver),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::AddedByUs), false),
        Some(S::TwoWayKeepDelete),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::AddedByThem), false),
        Some(S::TwoWayKeepDelete),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::DeletedByUs), false),
        Some(S::TwoWayKeepDelete),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::DeletedByThem), false),
        Some(S::TwoWayKeepDelete),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::BothDeleted), false),
        Some(S::DecisionOnly),
    );
    assert_eq!(MainPaneView::conflict_resolver_strategy(None, false), None);

    // Binary flag overrides any conflict kind to BinarySidePick.
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::BothModified), true),
        Some(S::BinarySidePick),
    );
    assert_eq!(
        MainPaneView::conflict_resolver_strategy(Some(K::DeletedByUs), true),
        Some(S::BinarySidePick),
    );
}

pub(super) struct TestBackend;

impl GitBackend for TestBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported(
            "Test backend does not open repositories",
        )))
    }
}

pub(super) fn set_ready_worktree_preview(
    pane: &mut MainPaneView,
    path: std::path::PathBuf,
    lines: Arc<Vec<String>>,
    source_len: usize,
    cx: &mut gpui::Context<MainPaneView>,
) {
    pane.set_worktree_preview_ready_rows(path, lines.as_ref(), source_len, cx);
    pane.worktree_preview_scroll
        .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
    cx.notify();
}

pub(super) fn build_large_json_array_lines(
    object_count: usize,
    payload_bytes: usize,
) -> Vec<String> {
    assert!(
        object_count >= 2,
        "need at least two objects to build a stable large-JSON test fixture"
    );

    let payload = "x".repeat(payload_bytes);
    let mut lines = Vec::with_capacity(object_count + 2);
    lines.push("[".to_string());
    lines.push(r#"  {"first": true, "count": 1},"#.to_string());
    for ix in 1..object_count - 1 {
        lines.push(format!(
            r#"  {{"line": {ix}, "flag": true, "payload": "{payload}"}},"#
        ));
    }
    lines.push(format!(
        r#"  {{"line": {}, "flag": true, "payload": "{payload}"}}"#,
        object_count - 1
    ));
    lines.push("]".to_string());
    lines
}

pub(super) fn highlights_include_range(
    highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    target: std::ops::Range<usize>,
) -> bool {
    highlights.iter().any(|(range, _)| *range == target)
}

pub(super) fn styled_debug_info(
    styled: &super::CachedDiffStyledText,
) -> (gpui::SharedString, Vec<std::ops::Range<usize>>) {
    (
        styled.text.clone(),
        styled
            .highlights
            .iter()
            .map(|(range, _)| range.clone())
            .collect(),
    )
}

type StyledDebugSpan = (
    std::ops::Range<usize>,
    Option<gpui::Hsla>,
    Option<gpui::Hsla>,
);

pub(super) fn styled_debug_info_with_styles(
    styled: &super::CachedDiffStyledText,
) -> (gpui::SharedString, Vec<StyledDebugSpan>) {
    (
        styled.text.clone(),
        styled
            .highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect(),
    )
}

pub(super) fn file_diff_split_row_ix(
    pane: &MainPaneView,
    region: DiffTextRegion,
    text: &str,
) -> Option<usize> {
    pane.file_diff_cache_rows
        .iter()
        .position(|row| match region {
            DiffTextRegion::SplitLeft => row.old.as_deref() == Some(text),
            DiffTextRegion::SplitRight => row.new.as_deref() == Some(text),
            DiffTextRegion::Inline => false,
        })
}

pub(super) fn file_diff_split_cached_styled<'a>(
    pane: &'a MainPaneView,
    region: DiffTextRegion,
    text: &str,
) -> Option<&'a super::CachedDiffStyledText> {
    let row_ix = file_diff_split_row_ix(pane, region, text)?;
    let key = pane.file_diff_split_cache_key(row_ix, region)?;
    let epoch = pane.file_diff_split_style_cache_epoch(region);
    pane.diff_text_segments_cache_get(key, epoch)
}

pub(super) fn file_diff_split_cached_debug(
    pane: &MainPaneView,
    region: DiffTextRegion,
    text: &str,
) -> Option<(gpui::SharedString, Vec<std::ops::Range<usize>>)> {
    file_diff_split_cached_styled(pane, region, text).map(styled_debug_info)
}

pub(super) fn file_diff_inline_ix(
    pane: &MainPaneView,
    kind: gitcomet_core::domain::DiffLineKind,
    text: &str,
) -> Option<usize> {
    pane.file_diff_inline_cache
        .iter()
        .position(|line| line.kind == kind && line.text.as_ref() == text)
}

pub(super) fn file_diff_inline_cached_styled<'a>(
    pane: &'a MainPaneView,
    kind: gitcomet_core::domain::DiffLineKind,
    text: &str,
) -> Option<&'a super::CachedDiffStyledText> {
    let inline_ix = file_diff_inline_ix(pane, kind, text)?;
    let line = pane.file_diff_inline_cache.get(inline_ix)?;
    let epoch = pane.file_diff_inline_style_cache_epoch(line);
    pane.diff_text_segments_cache_get(inline_ix, epoch)
}

pub(super) fn file_diff_inline_cached_debug(
    pane: &MainPaneView,
    kind: gitcomet_core::domain::DiffLineKind,
    text: &str,
) -> Option<(gpui::SharedString, Vec<std::ops::Range<usize>>)> {
    file_diff_inline_cached_styled(pane, kind, text).map(styled_debug_info)
}

pub(super) fn conflict_split_row_ix(
    pane: &MainPaneView,
    side: crate::view::conflict_resolver::ConflictPickSide,
    text: &str,
) -> Option<usize> {
    (0..pane.conflict_resolver.two_way_split_visible_len()).find_map(|visible_ix| {
        let crate::view::conflict_resolver::TwoWaySplitVisibleRow {
            source_row_ix: source_ix,
            row,
            conflict_ix: _conflict_ix,
        } = pane
            .conflict_resolver
            .two_way_split_visible_row(visible_ix)?;
        match side {
            crate::view::conflict_resolver::ConflictPickSide::Ours => {
                (row.old.as_deref() == Some(text)).then_some(source_ix)
            }
            crate::view::conflict_resolver::ConflictPickSide::Theirs => {
                (row.new.as_deref() == Some(text)).then_some(source_ix)
            }
        }
    })
}

pub(super) fn conflict_split_cached_styled<'a>(
    pane: &'a MainPaneView,
    side: crate::view::conflict_resolver::ConflictPickSide,
    text: &str,
) -> Option<&'a super::CachedDiffStyledText> {
    let row_ix = conflict_split_row_ix(pane, side, text)?;
    pane.conflict_diff_segments_cache_split.get(&(row_ix, side))
}

pub(super) fn styled_has_leading_muted_highlight(
    styled: &super::CachedDiffStyledText,
    comment_prefix_end: usize,
    muted: gpui::Hsla,
) -> bool {
    let has_muted_prefix_start = styled
        .highlights
        .iter()
        .any(|(range, style)| range.start == 0 && style.color == Some(muted));
    let max_muted_end = styled
        .highlights
        .iter()
        .filter(|(range, style)| range.start < comment_prefix_end && style.color == Some(muted))
        .map(|(range, _)| range.end)
        .max()
        .unwrap_or(0);
    has_muted_prefix_start && max_muted_end >= comment_prefix_end
}

pub(super) fn seed_file_diff_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &std::path::Path,
    path: &std::path::Path,
    old_text: &str,
    new_text: &str,
) {
    seed_file_diff_state_with_rev(cx, view, repo_id, workdir, path, 1, old_text, new_text);
}

pub(super) fn seed_file_diff_state_with_rev(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &std::path::Path,
    path: &std::path::Path,
    diff_file_rev: u64,
    old_text: &str,
    new_text: &str,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, workdir);
            set_test_file_status(
                &mut repo,
                path.to_path_buf(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff_file_rev = diff_file_rev;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path.to_path_buf(),
                    Some(old_text.to_string()),
                    Some(new_text.to_string()),
                ),
            )));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });
}

pub(super) fn seed_file_image_diff_state_with_rev(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &std::path::Path,
    path: &std::path::Path,
    diff_file_rev: u64,
    old: Option<&[u8]>,
    new: Option<&[u8]>,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, workdir);
            set_test_file_status(
                &mut repo,
                path.to_path_buf(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff_file_rev = diff_file_rev;
            repo.diff_state.diff_file_image = gitcomet_state::model::Loadable::Ready(Some(
                Arc::new(gitcomet_core::domain::FileDiffImage {
                    path: path.to_path_buf(),
                    old: old.map(|bytes| bytes.to_vec()),
                    new: new.map(|bytes| bytes.to_vec()),
                }),
            ));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });
}

pub(super) fn image_diff_svg_fixture(width: u32, height: u32, fill: &str) -> Vec<u8> {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<rect width="{width}" height="{height}" fill="{fill}"/>
</svg>"##
    )
    .into_bytes()
}

pub(super) fn file_image_diff_cache_debug_snapshot(pane: &MainPaneView) -> String {
    format!(
        "seq={} inflight={:?} repo_id={:?} cache_rev={} signature={:?} cache_target={:?} cache_path={:?} old_ready={} new_ready={} old_svg={:?} new_svg={:?} active_diff_file_rev={:?} active_diff_target={:?} active={}",
        pane.file_image_diff_cache_seq,
        pane.file_image_diff_cache_inflight,
        pane.file_image_diff_cache_repo_id,
        pane.file_image_diff_cache_rev,
        pane.file_image_diff_cache_content_signature,
        pane.file_image_diff_cache_target,
        pane.file_image_diff_cache_path,
        pane.file_image_diff_cache_old.is_some(),
        pane.file_image_diff_cache_new.is_some(),
        pane.file_image_diff_cache_old_svg_path,
        pane.file_image_diff_cache_new_svg_path,
        pane.active_repo().map(|repo| repo.diff_state.diff_file_rev),
        pane.active_repo()
            .and_then(|repo| repo.diff_state.diff_target.clone()),
        pane.is_file_image_diff_view_active(),
    )
}

pub(super) fn draw_and_drain_test_window(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

pub(super) const ALL_DIFF_SCROLL_SYNC_MODES: [DiffScrollSync; 4] = [
    DiffScrollSync::Both,
    DiffScrollSync::Vertical,
    DiffScrollSync::Horizontal,
    DiffScrollSync::None,
];

#[derive(Clone, Copy, Debug)]
pub(super) enum ScrollSyncAxis {
    Horizontal,
    Vertical,
}

impl ScrollSyncAxis {
    pub(super) const ALL: [Self; 2] = [Self::Horizontal, Self::Vertical];

    pub(super) const fn component(self, offset: gpui::Point<Pixels>) -> Pixels {
        match self {
            Self::Horizontal => offset.x,
            Self::Vertical => offset.y,
        }
    }

    pub(super) const fn includes(self, mode: DiffScrollSync) -> bool {
        match self {
            Self::Horizontal => mode.includes_horizontal(),
            Self::Vertical => mode.includes_vertical(),
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        }
    }

    pub(super) fn offset(self, magnitude: Pixels) -> gpui::Point<Pixels> {
        match self {
            Self::Horizontal => point(-magnitude, px(0.0)),
            Self::Vertical => point(px(0.0), -magnitude),
        }
    }
}

pub(super) fn uniform_list_offset(handle: &gpui::UniformListScrollHandle) -> gpui::Point<Pixels> {
    handle.0.borrow().base_handle.offset()
}

pub(super) fn uniform_list_max_offset(
    handle: &gpui::UniformListScrollHandle,
) -> gpui::Size<Pixels> {
    handle.0.borrow().base_handle.max_offset().into()
}

pub(super) fn set_uniform_list_offset(
    handle: &gpui::UniformListScrollHandle,
    offset: gpui::Point<Pixels>,
) {
    handle.0.borrow().base_handle.set_offset(offset);
}

pub(super) fn reset_uniform_list_offsets(handles: &[&gpui::UniformListScrollHandle]) {
    for handle in handles {
        set_uniform_list_offset(handle, point(px(0.0), px(0.0)));
    }
}

pub(super) fn set_diff_scroll_sync_for_test(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    mode: DiffScrollSync,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_scroll_sync(mode, cx);
        });
    });
    draw_and_drain_test_window(cx);
}

pub(super) fn set_diff_content_mode_for_test(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    mode: DiffContentMode,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(mode, cx);
        });
    });
    draw_and_drain_test_window(cx);
}

pub(super) fn conflict_compare_repo_state(
    repo_id: gitcomet_state::model::RepoId,
    workdir: &std::path::Path,
    file_rel: &std::path::Path,
    base_text: &str,
    ours_text: &str,
    theirs_text: &str,
    current_text: &str,
) -> gitcomet_state::model::RepoState {
    let mut repo = opening_repo_state(repo_id, workdir);
    set_test_file_status(
        &mut repo,
        file_rel.to_path_buf(),
        gitcomet_core::domain::FileStatusKind::Conflicted,
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    set_test_conflict_file(
        &mut repo,
        file_rel,
        base_text,
        ours_text,
        theirs_text,
        current_text,
    );
    repo
}

pub(super) fn assert_file_preview_ctrl_a_ctrl_c_copies_all(
    cx: &mut gpui::TestAppContext,
    repo_id: gitcomet_state::model::RepoId,
    workdir: std::path::PathBuf,
    file_rel: std::path::PathBuf,
    status_kind: gitcomet_core::domain::FileStatusKind,
    lines: Arc<Vec<String>>,
) {
    let _clipboard_guard = lock_clipboard_test();
    let expected = lines.join("\n");
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    // Create the file on disk so is_file_preview_active() can detect it.
    let _ = std::fs::create_dir_all(&workdir);
    std::fs::write(workdir.join(&file_rel), lines.join("\n")).expect("write preview fixture file");
    let deleted_preview_source_path = (status_kind
        == gitcomet_core::domain::FileStatusKind::Deleted)
        .then(|| workdir.join(".deleted_preview_source.txt"));
    if let Some(source_path) = deleted_preview_source_path.as_ref() {
        std::fs::write(source_path, &expected).expect("write deleted preview source fixture");
    }

    // Push state through the model first; the observer will clear stale
    // worktree_preview on diff-target change.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                status_kind,
                gitcomet_core::domain::DiffArea::Staged,
            );
            if let Some(source_path) = deleted_preview_source_path.as_ref() {
                repo.diff_state.diff_preview_text_file = gitcomet_state::model::Loadable::Ready(
                    Some(Arc::new(gitcomet_core::domain::DiffPreviewTextFile {
                        path: source_path.clone(),
                        side: gitcomet_core::domain::DiffPreviewTextSide::Old,
                    })),
                );
                repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);
            }

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    // Set preview data in a separate update so it runs after the observer
    // has cleared the stale preview state.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let workdir = workdir.clone();
            let file_rel = file_rel.clone();
            let lines = Arc::clone(&lines);
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    workdir.join(&file_rel),
                    lines,
                    expected.len(),
                    cx,
                );
            });
        });
    });

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let focus = main_pane.read(app).diff_panel_focus_handle.clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "file preview copy selection state",
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_line_count() == Some(lines.len())
                && matches!(
                    pane.worktree_preview,
                    gitcomet_state::model::Loadable::Ready(_)
                )
        },
        |pane| {
            format!(
                "active={} preview={:?} preview_path={:?} source_path={:?} line_count={:?}",
                pane.is_file_preview_active(),
                pane.worktree_preview,
                pane.worktree_preview_path,
                pane.worktree_preview_source_path,
                pane.worktree_preview_line_count(),
            )
        },
    );

    cx.simulate_keystrokes("ctrl-a ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(expected)
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

pub(super) fn assert_markdown_file_preview_toggle_visible(
    cx: &mut gpui::TestAppContext,
    repo_id: gitcomet_state::model::RepoId,
    workdir: std::path::PathBuf,
    file_rel: std::path::PathBuf,
    status_kind: gitcomet_core::domain::FileStatusKind,
    old_text: Option<&str>,
    new_text: Option<&str>,
    create_worktree_file: bool,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create markdown preview workdir");
    if create_worktree_file {
        let contents = new_text.or(old_text).unwrap_or_default();
        std::fs::write(workdir.join(&file_rel), contents).expect("write markdown preview fixture");
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                status_kind,
                gitcomet_core::domain::DiffArea::Staged,
            );
            let preview_side = match status_kind {
                gitcomet_core::domain::FileStatusKind::Added
                | gitcomet_core::domain::FileStatusKind::Untracked => {
                    Some(gitcomet_core::domain::DiffPreviewTextSide::New)
                }
                gitcomet_core::domain::FileStatusKind::Deleted => {
                    Some(gitcomet_core::domain::DiffPreviewTextSide::Old)
                }
                _ => None,
            };
            if let Some(side) = preview_side {
                let preview_source_path = workdir.join(format!(
                    ".markdown_preview_source_{}_{}.md",
                    repo_id.0,
                    match side {
                        gitcomet_core::domain::DiffPreviewTextSide::New => "new",
                        gitcomet_core::domain::DiffPreviewTextSide::Old => "old",
                    }
                ));
                let contents = match side {
                    gitcomet_core::domain::DiffPreviewTextSide::New => new_text.unwrap_or_default(),
                    gitcomet_core::domain::DiffPreviewTextSide::Old => old_text.unwrap_or_default(),
                };
                std::fs::write(&preview_source_path, contents)
                    .expect("write markdown preview source fixture");
                repo.diff_state.diff_preview_text_file = gitcomet_state::model::Loadable::Ready(
                    Some(Arc::new(gitcomet_core::domain::DiffPreviewTextFile {
                        path: preview_source_path,
                        side,
                    })),
                );
            }

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown file preview activation",
        |pane| {
            let rendered_preview_kind = crate::view::diff_target_rendered_preview_kind(
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.as_ref()),
            );
            let toggle_kind = crate::view::main_diff_rendered_preview_toggle_kind(
                false,
                pane.is_file_preview_active(),
                rendered_preview_kind,
            );
            pane.is_file_preview_active()
                && toggle_kind == Some(RenderedPreviewKind::Markdown)
                && pane
                    .rendered_preview_modes
                    .get(RenderedPreviewKind::Markdown)
                    == RenderedPreviewMode::Rendered
        },
        |pane| {
            let rendered_preview_kind = crate::view::diff_target_rendered_preview_kind(
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.as_ref()),
            );
            let toggle_kind = crate::view::main_diff_rendered_preview_toggle_kind(
                false,
                pane.is_file_preview_active(),
                rendered_preview_kind,
            );
            format!(
                "active_repo={:?} diff_target={:?} is_file_preview_active={} toggle_kind={toggle_kind:?} markdown_mode={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.is_file_preview_active(),
                pane.rendered_preview_modes
                    .get(RenderedPreviewKind::Markdown),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let rendered_preview_kind = crate::view::diff_target_rendered_preview_kind(
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.as_ref()),
        );
        let toggle_kind = crate::view::main_diff_rendered_preview_toggle_kind(
            false,
            pane.is_file_preview_active(),
            rendered_preview_kind,
        );
        assert!(
            pane.is_file_preview_active(),
            "expected markdown {status_kind:?} target to use single-file preview mode"
        );
        assert_eq!(
            toggle_kind,
            Some(RenderedPreviewKind::Markdown),
            "expected markdown {status_kind:?} target to request the main preview toggle"
        );
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered,
            "expected markdown {status_kind:?} target to default to Preview mode"
        );
    });
    assert!(
        cx.debug_bounds("markdown_diff_view_toggle").is_some(),
        "expected markdown Preview/Text toggle for {status_kind:?} file preview"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown preview fixture");
}

pub(super) fn app_state_with_repo(
    repo: gitcomet_state::model::RepoState,
    repo_id: gitcomet_state::model::RepoId,
) -> Arc<AppState> {
    Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    })
}

pub(super) fn opening_repo_state(
    repo_id: gitcomet_state::model::RepoId,
    workdir: &Path,
) -> gitcomet_state::model::RepoState {
    gitcomet_state::model::RepoState::new_opening(
        repo_id,
        gitcomet_core::domain::RepoSpec {
            workdir: workdir.to_path_buf(),
        },
    )
}

pub(super) fn push_test_state(
    this: &super::super::GitCometView,
    state: Arc<AppState>,
    cx: &mut impl gpui::AppContext,
) {
    this._ui_model.update(cx, |model, cx| {
        model.set_state(state, cx);
    });
}

/// Sets `repo.status` with a single file and `repo.diff_state.diff_target` in one call.
/// Covers `Staged` (file in `staged`, empty `unstaged`) and `Unstaged` (reverse).
pub(super) fn set_test_file_status(
    repo: &mut gitcomet_state::model::RepoState,
    path: impl Into<std::path::PathBuf>,
    kind: gitcomet_core::domain::FileStatusKind,
    area: gitcomet_core::domain::DiffArea,
) {
    set_test_file_status_with_conflict(repo, path, kind, None, area);
}

/// Like `set_test_file_status` but for conflicted files with `BothModified` conflict kind.
pub(super) fn set_test_conflict_status(
    repo: &mut gitcomet_state::model::RepoState,
    path: impl Into<std::path::PathBuf>,
    area: gitcomet_core::domain::DiffArea,
) {
    set_test_file_status_with_conflict(
        repo,
        path,
        gitcomet_core::domain::FileStatusKind::Conflicted,
        Some(gitcomet_core::domain::FileConflictKind::BothModified),
        area,
    );
}

pub(super) fn set_test_file_status_with_conflict(
    repo: &mut gitcomet_state::model::RepoState,
    path: impl Into<std::path::PathBuf>,
    kind: gitcomet_core::domain::FileStatusKind,
    conflict: Option<gitcomet_core::domain::FileConflictKind>,
    area: gitcomet_core::domain::DiffArea,
) {
    let path = path.into();
    let file_status = gitcomet_core::domain::FileStatus {
        path: path.clone(),
        kind,
        conflict,
    };
    let (staged, unstaged) = match area {
        gitcomet_core::domain::DiffArea::Staged => (vec![file_status], vec![]),
        gitcomet_core::domain::DiffArea::Unstaged => (vec![], vec![file_status]),
    };
    repo.status = gitcomet_state::model::Loadable::Ready(
        gitcomet_core::domain::RepoStatus { staged, unstaged }.into(),
    );
    repo.status_rev = repo.status_rev.wrapping_add(1);
    repo.has_unstaged_conflicts = matches!(
        (area, kind, conflict),
        (
            gitcomet_core::domain::DiffArea::Unstaged,
            gitcomet_core::domain::FileStatusKind::Conflicted,
            Some(_)
        )
    );
    repo.diff_state.diff_target =
        Some(gitcomet_core::domain::DiffTarget::WorkingTree { path, area });
    repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);
}

/// Sets `repo.conflict_state.conflict_file_path` and `repo.conflict_state.conflict_file`.
pub(super) fn set_test_conflict_file(
    repo: &mut gitcomet_state::model::RepoState,
    path: impl Into<std::path::PathBuf>,
    base: impl Into<String>,
    ours: impl Into<String>,
    theirs: impl Into<String>,
    current: impl Into<String>,
) {
    let path = path.into();
    repo.conflict_state.conflict_file_path = Some(path.clone());
    repo.conflict_state.conflict_file =
        gitcomet_state::model::Loadable::Ready(Some(gitcomet_state::model::ConflictFile {
            path: path.into(),
            base_bytes: None,
            ours_bytes: None,
            theirs_bytes: None,
            current_bytes: None,
            base: Some(base.into().into()),
            ours: Some(ours.into().into()),
            theirs: Some(theirs.into().into()),
            current: Some(current.into().into()),
        }));
}

pub(super) fn focus_diff_panel(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) {
    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let focus = main_pane.read(app).diff_panel_focus_handle.clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
}

pub(super) const DEFAULT_MAIN_PANE_WAIT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(12);
pub(super) const BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(20);

pub(super) fn wait_for_main_pane_condition<T, Ready, Snapshot>(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    description: &str,
    is_ready: Ready,
    snapshot: Snapshot,
) where
    T: std::fmt::Debug,
    Ready: Fn(&MainPaneView) -> bool,
    Snapshot: Fn(&MainPaneView) -> T,
{
    wait_for_main_pane_condition_with_timeout(
        cx,
        view,
        description,
        DEFAULT_MAIN_PANE_WAIT_TIMEOUT,
        is_ready,
        snapshot,
    );
}

pub(super) fn wait_for_main_pane_condition_with_timeout<T, Ready, Snapshot>(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    description: &str,
    timeout: std::time::Duration,
    is_ready: Ready,
    snapshot: Snapshot,
) where
    T: std::fmt::Debug,
    Ready: Fn(&MainPaneView) -> bool,
    Snapshot: Fn(&MainPaneView) -> T,
{
    let deadline = std::time::Instant::now() + timeout;
    loop {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        let ready = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            is_ready(pane)
        });
        if ready {
            return;
        }
        if std::time::Instant::now() >= deadline {
            let snapshot = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                snapshot(pane)
            });
            panic!("timed out waiting for {description}: {snapshot:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

pub(super) fn wait_for_file_image_diff_cache<Ready>(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    description: &str,
    is_ready: Ready,
) where
    Ready: Fn(&MainPaneView) -> bool,
{
    wait_for_main_pane_condition(
        cx,
        view,
        description,
        |pane| {
            pane.file_image_diff_cache_inflight.is_none()
                && pane.is_file_image_diff_view_active()
                && is_ready(pane)
        },
        file_image_diff_cache_debug_snapshot,
    );
}

mod conflict;
mod file_diff;
mod file_preview;
mod file_status;
mod large_file_diff;
mod markdown;
mod shortcuts;
