use super::*;
use std::time::Duration;

pub(crate) fn push_test_state(
    view: &GitCometView,
    state: Arc<AppState>,
    cx: &mut impl gpui::AppContext,
) {
    view.ui_model.update(cx, |model, cx| {
        model.set_state(state, cx);
    });
}

pub(crate) fn sync_store_snapshot(view: &GitCometView, cx: &mut impl gpui::AppContext) {
    push_test_state(view, view.store.snapshot(), cx);
}

#[cfg(target_os = "macos")]
pub(crate) fn apply_state_snapshot_for_test(
    view: &mut GitCometView,
    state: Arc<AppState>,
    cx: &mut gpui::Context<GitCometView>,
) {
    view.apply_state_snapshot(state, cx);
}

pub(crate) fn set_sidebar_width_for_test(
    view: &mut GitCometView,
    width: gpui::Pixels,
    cx: &mut gpui::Context<GitCometView>,
) {
    view.set_sidebar_width_from_pixels(width);
    view.sidebar_render_width = width;
    view.sidebar_width_anim_seq = view.sidebar_width_anim_seq.wrapping_add(1);
    view.sidebar_width_animating = false;
    cx.notify();
}

pub(crate) fn popover_is_open(view: &GitCometView, app: &App) -> bool {
    popover_kind(view, app).is_some()
}

pub(crate) fn command_palette_is_open(view: &GitCometView) -> bool {
    view.command_palette_open
}

/// The Reveal Commit dialog reports two things: the root's flag, and the
/// dialog's own. They only disagree if a close path forgot one of them.
pub(crate) fn reveal_commit_is_open(view: &GitCometView, app: &App) -> bool {
    let dialog_open = view.reveal_commit_dialog.read(app).is_open();
    assert_eq!(
        view.reveal_commit_open, dialog_open,
        "the root flag and the dialog disagree about being open"
    );
    dialog_open
}

/// `(scrolled, max_scroll)` of the repository tab strip, in pixels.
pub(crate) fn repo_tab_scroll(view: &GitCometView, app: &App) -> (Pixels, Pixels) {
    view.repo_tabs_bar.read(app).tab_scroll_for_tests()
}

/// Window-space bounds of the scrollable repository tab strip.
pub(crate) fn repo_tab_strip_viewport(view: &GitCometView, app: &App) -> gpui::Bounds<Pixels> {
    view.repo_tabs_bar.read(app).tab_strip_viewport_for_tests()
}

pub(crate) fn pressed_repo_tab(view: &GitCometView, app: &App) -> Option<RepoId> {
    view.repo_tabs_bar.read(app).pressed_repo_tab_for_tests()
}

pub(crate) fn repo_external_folder_drag_active(view: &GitCometView, app: &App) -> bool {
    view.repo_tabs_bar
        .read(app)
        .external_folder_drag_active_for_tests()
}

pub(crate) fn repo_external_folder_drag_hovered(view: &GitCometView, app: &App) -> bool {
    view.repo_tabs_bar
        .read(app)
        .external_folder_drag_hovered_for_tests()
}

pub(crate) fn external_drag_classification_seq(view: &GitCometView) -> u64 {
    view.external_drag_classification_seq
}

pub(crate) fn add_repo_menu_is_open(view: &GitCometView, app: &App) -> bool {
    matches!(popover_kind(view, app), Some(PopoverKind::AddRepoMenu))
}

pub(crate) fn app_menu_focus_handle(view: &GitCometView, app: &App) -> FocusHandle {
    view.title_bar.read(app).app_menu_focus_handle_for_test()
}

pub(crate) fn titlebar_drag_is_armed(view: &GitCometView, app: &App) -> bool {
    view.title_bar.read(app).title_drag_armed_for_test()
}

pub(in crate::view) fn history_refs_hover_is_open(view: &GitCometView, app: &App) -> bool {
    view.history_refs_hover_host.read(app).is_open_for_tests()
}

pub(in crate::view) fn history_refs_hover_source_bounds(
    view: &GitCometView,
    app: &App,
) -> Option<Bounds<Pixels>> {
    view.history_refs_hover_host
        .read(app)
        .source_bounds_for_tests()
}

pub(in crate::view) fn history_refs_hover_pinned_item_ix(
    view: &GitCometView,
    app: &App,
) -> Option<usize> {
    view.history_refs_hover_host
        .read(app)
        .pinned_item_ix_for_tests()
}

pub(in crate::view) fn history_refs_hover_pinned_item_text(
    view: &GitCometView,
    app: &App,
) -> Option<SharedString> {
    view.history_refs_hover_host
        .read(app)
        .pinned_item_text_for_tests()
}

pub(in crate::view) fn popover_kind(view: &GitCometView, app: &App) -> Option<PopoverKind> {
    view.popover_host.read(app).popover_kind_for_tests()
}

pub(crate) fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

/// Blocks until the store worker has reduced every message dispatched so far.
///
/// GPUI's test executor does not drive the worker thread, so a snapshot read
/// right after a dispatch may predate it. A plain message sent now is reduced
/// strictly after everything already queued (only control messages overtake,
/// and only internal ones), so once its effect is visible, so is every earlier
/// dispatch. The message flips the default tag type, which nothing here reads.
/// Messages the worker itself sends while handling a command's effects can
/// still land later; wait on their own state when a test depends on them.
pub(crate) fn drain_store_worker(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) {
    use gitcomet_state::model::DefaultTagType;
    let store = cx.update(|_window, app| view.read(app).store.clone());
    let sentinel = match store.snapshot().default_tag_type {
        DefaultTagType::Lightweight => DefaultTagType::Annotated,
        DefaultTagType::Annotated => DefaultTagType::Lightweight,
    };
    store.dispatch(gitcomet_state::msg::Msg::SetDefaultTagType(sentinel));
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        redraw(cx);
        cx.run_until_parked();
        if store.snapshot().default_tag_type == sentinel {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the store worker did not reduce the sentinel message"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The repo's `ops_rev`, bumped when a local action begins and again when it
/// finishes. Read after [`drain_store_worker`], it has moved since an earlier
/// read if and only if an action was dispatched in between. The in-flight
/// counters cannot say that: against a backend that never opens a repository
/// an action completes at once with a missing-handle error, so they are back
/// to zero on both sides of the worker.
pub(crate) fn repo_ops_rev(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repo_id: gitcomet_state::model::RepoId,
) -> u64 {
    cx.update(|_window, app| {
        view.read(app)
            .store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map(|repo| repo.ops_rev)
            .expect("the repo under test is in the store")
    })
}

/// Inspect render output inside a frame so GPUI releases arena-owned elements.
/// Return only inspected data; rendered elements must stay inside the callback.
pub(crate) fn inspect_render<R>(
    cx: &mut gpui::VisualTestContext,
    inspect: impl FnOnce(&mut Window, &mut App) -> R,
) -> R {
    let mut result = None;
    cx.draw(
        gpui::point(px(0.0), px(0.0)),
        gpui::size(
            gpui::AvailableSpace::MinContent,
            gpui::AvailableSpace::MinContent,
        ),
        |window, app| {
            result = Some(inspect(window, app));
            gpui::Empty
        },
    );
    result.expect("render inspection should run while drawing the test frame")
}

pub(crate) fn wait_for_native_tooltip(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    redraw(cx);
}

pub(crate) fn tooltip_text(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> Option<SharedString> {
    redraw(cx);
    cx.update(|_window, app| view.read(app).tooltip_text_for_test(app))
}

pub(crate) fn open_repo_panel_visible(view: &GitCometView) -> bool {
    view.open_repo_panel
}

pub(crate) fn show_timezone(view: &GitCometView) -> bool {
    view.show_timezone
}

pub(in crate::view) fn change_tracking_view(view: &GitCometView) -> ChangeTrackingView {
    view.change_tracking_view
}

pub(in crate::view) fn diff_scroll_sync(view: &GitCometView) -> DiffScrollSync {
    view.diff_scroll_sync
}

pub(in crate::view) fn diff_content_mode(view: &GitCometView) -> DiffContentMode {
    view.diff_content_mode
}

pub(in crate::view) fn diff_whitespace_mode(view: &GitCometView) -> DiffWhitespaceMode {
    view.diff_whitespace_mode
}

pub(in crate::view) fn diff_reveal_whitespace_chars(view: &GitCometView) -> bool {
    view.diff_reveal_whitespace_chars
}

pub(in crate::view) fn diff_word_wrap(view: &GitCometView) -> bool {
    view.diff_word_wrap
}

pub(in crate::view) fn diff_show_line_numbers(view: &GitCometView) -> bool {
    view.diff_show_line_numbers
}

/// No-op backend shared by view tests: `open` always fails as unsupported.
pub(crate) struct TestBackend;

impl gitcomet_core::services::GitBackend for TestBackend {
    fn open(
        &self,
        _workdir: &std::path::Path,
    ) -> gitcomet_core::services::Result<Arc<dyn gitcomet_core::services::GitRepository>> {
        Err(gitcomet_core::error::Error::new(
            gitcomet_core::error::ErrorKind::Unsupported("Test backend does not open repositories"),
        ))
    }
}

/// No-op backend for tests that do not touch repositories at all.
pub(crate) struct NoopBackend;

impl gitcomet_core::services::GitBackend for NoopBackend {
    fn open(
        &self,
        _workdir: &std::path::Path,
    ) -> gitcomet_core::services::Result<Arc<dyn gitcomet_core::services::GitRepository>> {
        Err(gitcomet_core::error::Error::new(
            gitcomet_core::error::ErrorKind::Unsupported("no repositories in this test"),
        ))
    }
}

/// Head of a real `tests/unit/helpers.ts` whose file preview once showed
/// garbled colours from row 14 on: a stale parse, not the grammar.
pub(crate) const TS_COMPILER_OPTIONS_HELPERS: &str = r#"import * as assert from 'node:assert/strict'
import * as ts from 'typescript'
import transformer from '../../src'

/*
 * Assertion based test helpers. The reference suite (tests/index.ts) compiles whole files and compares them
 * to tests/references*, these helpers compile small TSX snippets in memory so a test can pin one behaviour.
 */

// Same flavour of options as tests/index.ts: no "use strict" prologue, LF newlines
const baseCompilerOptions: ts.CompilerOptions = {
    jsx: ts.JsxEmit.Preserve,
    strict: false,
    // alwaysStrict=false is deprecated in TypeScript 6 and reported as a diagnostic without this
    ignoreDeprecations: '6.0',
    alwaysStrict: false,
    experimentalDecorators: true,
    target: ts.ScriptTarget.ESNext,
    module: ts.ModuleKind.ESNext,
    newLine: ts.NewLineKind.LineFeed
}

// Compiler options that can be passed as `compilerOptions` to the transform functions
export const es5: ts.CompilerOptions = {target: ts.ScriptTarget.ES5, module: ts.ModuleKind.ESNext}
export const es5CommonJS: ts.CompilerOptions = {target: ts.ScriptTarget.ES5, module: ts.ModuleKind.CommonJS}
export const es2015: ts.CompilerOptions = {target: ts.ScriptTarget.ES2015, module: ts.ModuleKind.ES2015}
export const commonJS: ts.CompilerOptions = {module: ts.ModuleKind.CommonJS}

export interface TranspileResult {
    code: string
    map: string | undefined
    diagnostics: readonly ts.Diagnostic[]
}

/*
 * Compiles `input` with the plugin as an `after` transformer and returns the emitted code untouched,
 * the source map when compilerOptions.sourceMap is set, and the syntactic diagnostics of the input.
 */"#;
