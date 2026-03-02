//! Focused merge window for standalone `gitgpui-app mergetool` invocation.
//!
//! Opens a GPUI window that displays the 3-way merge result with interactive
//! conflict resolution. The user picks sides for each conflict block, then
//! saves (exit 0) or cancels (exit 1).

use crate::assets::GitGpuiAssets;
use crate::theme::AppTheme;
use crate::view::conflict_resolver::{
    auto_resolve_segments_with_options, conflict_count, next_unresolved_conflict_index,
    parse_conflict_markers, prev_unresolved_conflict_index, resolved_conflict_count, ConflictBlock,
    ConflictChoice, ConflictSegment,
};
use gpui::prelude::*;
use gpui::{
    actions, div, point, px, size, App, Application, Bounds, ClickEvent, FocusHandle, Focusable,
    FontWeight, KeyBinding, Render, ScrollHandle, SharedString, TitlebarOptions, Window,
    WindowBounds, WindowDecorations, WindowOptions,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

const EXIT_SUCCESS: i32 = 0;
const EXIT_CANCELED: i32 = 1;
const EXIT_ERROR: i32 = 2;

// ── Actions ──────────────────────────────────────────────────────────

actions!(
    focused_merge,
    [
        Save,
        Cancel,
        AutoResolve,
        NextConflict,
        PrevConflict,
        PickOurs,
        PickTheirs,
        PickBase,
        PickBoth
    ]
);

// ── Public config ────────────────────────────────────────────────────

/// Configuration for the focused merge window.
#[derive(Clone, Debug)]
pub struct FocusedMergeConfig {
    pub merged_path: PathBuf,
    pub label_local: String,
    pub label_remote: String,
    pub label_base: String,
    /// Pre-merged output text (with conflict markers if conflicts exist).
    pub merged_text: String,
    /// Whether the merge was clean (no conflicts).
    pub is_clean: bool,
    /// Number of conflicts in the merged output.
    pub conflict_count: usize,
}

// ── View state ───────────────────────────────────────────────────────

struct FocusedMergeView {
    segments: Vec<ConflictSegment>,
    active_conflict: usize,
    output_path: PathBuf,
    label_local: String,
    label_remote: String,
    label_base: String,
    saved: bool,
    exit_code: Arc<AtomicI32>,
    focus_handle: FocusHandle,
    scroll_handle: ScrollHandle,
    theme: AppTheme,
}

impl Focusable for FocusedMergeView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl FocusedMergeView {
    fn new(
        config: FocusedMergeConfig,
        exit_code: Arc<AtomicI32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut segments = parse_conflict_markers(&config.merged_text);

        // Auto-resolve trivially resolvable conflicts (identical sides, single-side change).
        auto_resolve_segments_with_options(&mut segments, false);

        let theme = AppTheme::default_for_window_appearance(window.appearance());

        Self {
            segments,
            active_conflict: 0,
            output_path: config.merged_path,
            label_local: config.label_local,
            label_remote: config.label_remote,
            label_base: config.label_base,
            saved: false,
            exit_code,
            focus_handle: cx.focus_handle(),
            scroll_handle: ScrollHandle::new(),
            theme,
        }
    }

    fn total_conflicts(&self) -> usize {
        conflict_count(&self.segments)
    }

    fn resolved_conflicts(&self) -> usize {
        resolved_conflict_count(&self.segments)
    }

    fn all_resolved(&self) -> bool {
        self.total_conflicts() == 0 || self.resolved_conflicts() == self.total_conflicts()
    }

    fn pick_choice(&mut self, choice: ConflictChoice, cx: &mut Context<Self>) {
        let mut conflict_ix = 0usize;
        for seg in &mut self.segments {
            let ConflictSegment::Block(block) = seg else {
                continue;
            };
            if conflict_ix == self.active_conflict {
                if matches!(choice, ConflictChoice::Base) && block.base.is_none() {
                    break; // Can't pick base when there's no base section
                }
                block.choice = choice;
                block.resolved = true;
                break;
            }
            conflict_ix += 1;
        }
        // Advance to next unresolved.
        if let Some(next) = next_unresolved_conflict_index(&self.segments, self.active_conflict) {
            self.active_conflict = next;
        }
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let output = self.build_output();
        if let Err(e) = std::fs::write(&self.output_path, &output) {
            eprintln!(
                "Failed to write merged output to {}: {e}",
                self.output_path.display()
            );
            self.exit_code.store(EXIT_ERROR, Ordering::SeqCst);
            cx.quit();
            return;
        }
        self.saved = true;
        self.exit_code
            .store(saved_exit_code(&self.segments), Ordering::SeqCst);
        cx.quit();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.exit_code.store(EXIT_CANCELED, Ordering::SeqCst);
        cx.quit();
    }

    fn auto_resolve(&mut self, cx: &mut Context<Self>) {
        auto_resolve_segments_with_options(&mut self.segments, true);
        // Reset active conflict to first unresolved.
        self.active_conflict = next_unresolved_conflict_index(&self.segments, 0).unwrap_or(0);
        cx.notify();
    }

    fn navigate_next(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = next_unresolved_conflict_index(&self.segments, self.active_conflict) {
            self.active_conflict = next;
        } else {
            // Wrap through all conflicts.
            let total = self.total_conflicts();
            if total > 0 {
                self.active_conflict = (self.active_conflict + 1) % total;
            }
        }
        cx.notify();
    }

    fn navigate_prev(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = prev_unresolved_conflict_index(&self.segments, self.active_conflict) {
            self.active_conflict = prev;
        } else {
            let total = self.total_conflicts();
            if total > 0 {
                self.active_conflict = if self.active_conflict == 0 {
                    total - 1
                } else {
                    self.active_conflict - 1
                };
            }
        }
        cx.notify();
    }

    /// Build the resolved output text from current segment state.
    fn build_output(&self) -> String {
        let mut out = String::new();
        for seg in &self.segments {
            match seg {
                ConflictSegment::Text(text) => out.push_str(text),
                ConflictSegment::Block(block) => {
                    out.push_str(&block_output_text(
                        block,
                        &self.label_local,
                        &self.label_remote,
                        &self.label_base,
                    ));
                }
            }
        }
        out
    }

    fn get_conflict_block(&self, conflict_ix: usize) -> Option<&ConflictBlock> {
        let mut idx = 0usize;
        for seg in &self.segments {
            if let ConflictSegment::Block(block) = seg {
                if idx == conflict_ix {
                    return Some(block);
                }
                idx += 1;
            }
        }
        None
    }

    fn has_base_for_active(&self) -> bool {
        self.get_conflict_block(self.active_conflict)
            .is_some_and(|b| b.base.is_some())
    }
}

/// Get the chosen text for a conflict block based on its current choice.
fn chosen_block_text(block: &ConflictBlock) -> String {
    match block.choice {
        ConflictChoice::Base => block.base.clone().unwrap_or_default(),
        ConflictChoice::Ours => block.ours.clone(),
        ConflictChoice::Theirs => block.theirs.clone(),
        ConflictChoice::Both => {
            let mut s = block.ours.clone();
            s.push_str(&block.theirs);
            s
        }
    }
}

fn block_output_text(
    block: &ConflictBlock,
    label_local: &str,
    label_remote: &str,
    label_base: &str,
) -> String {
    if block.resolved {
        return chosen_block_text(block);
    }

    render_unresolved_marker_block(block, label_local, label_remote, label_base)
}

fn render_unresolved_marker_block(
    block: &ConflictBlock,
    label_local: &str,
    label_remote: &str,
    label_base: &str,
) -> String {
    let newline = detect_line_ending(block);
    let mut out = String::new();
    out.push_str("<<<<<<< ");
    out.push_str(label_local);
    out.push_str(newline);
    out.push_str(&block.ours);
    // Guard: ensure content ends with a newline so marker starts on its own line.
    if !block.ours.is_empty() && !block.ours.ends_with('\n') {
        out.push_str(newline);
    }
    if let Some(base) = block.base.as_deref() {
        out.push_str("||||||| ");
        out.push_str(label_base);
        out.push_str(newline);
        out.push_str(base);
        if !base.is_empty() && !base.ends_with('\n') {
            out.push_str(newline);
        }
    }
    out.push_str("=======");
    out.push_str(newline);
    out.push_str(&block.theirs);
    if !block.theirs.is_empty() && !block.theirs.ends_with('\n') {
        out.push_str(newline);
    }
    out.push_str(">>>>>>> ");
    out.push_str(label_remote);
    out.push_str(newline);
    out
}

fn detect_line_ending(block: &ConflictBlock) -> &'static str {
    let uses_crlf = block.ours.contains("\r\n")
        || block.theirs.contains("\r\n")
        || block
            .base
            .as_deref()
            .is_some_and(|base| base.contains("\r\n"));
    if uses_crlf {
        "\r\n"
    } else {
        "\n"
    }
}

fn saved_exit_code(segments: &[ConflictSegment]) -> i32 {
    let total = conflict_count(segments);
    let resolved = resolved_conflict_count(segments);
    if total == 0 || total == resolved {
        EXIT_SUCCESS
    } else {
        EXIT_CANCELED
    }
}

/// Truncate text to N lines for display, appending "..." if truncated.
fn truncate_lines(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        text.to_string()
    } else {
        let mut out: String = lines[..max_lines].join("\n");
        out.push_str("\n...");
        out
    }
}

impl Render for FocusedMergeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let total = self.total_conflicts();
        let resolved = self.resolved_conflicts();
        let all_done = self.all_resolved();
        let active = self.active_conflict;
        let has_base = self.has_base_for_active();

        // Status text
        let status_text = if total == 0 {
            "No conflicts — clean merge".to_string()
        } else if all_done {
            format!("All {total} conflict(s) resolved")
        } else {
            format!(
                "{resolved}/{total} resolved — conflict {}/{total}",
                active + 1
            )
        };

        div()
            .id("focused-merge-root")
            .key_context("FocusedMerge")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Save, _window, cx| this.save(cx)))
            .on_action(cx.listener(|this, _: &Cancel, _window, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &AutoResolve, _window, cx| this.auto_resolve(cx)))
            .on_action(cx.listener(|this, _: &NextConflict, _window, cx| this.navigate_next(cx)))
            .on_action(cx.listener(|this, _: &PrevConflict, _window, cx| this.navigate_prev(cx)))
            .on_action(cx.listener(|this, _: &PickOurs, _window, cx| {
                this.pick_choice(ConflictChoice::Ours, cx)
            }))
            .on_action(cx.listener(|this, _: &PickTheirs, _window, cx| {
                this.pick_choice(ConflictChoice::Theirs, cx)
            }))
            .on_action(cx.listener(|this, _: &PickBase, _window, cx| {
                this.pick_choice(ConflictChoice::Base, cx)
            }))
            .on_action(cx.listener(|this, _: &PickBoth, _window, cx| {
                this.pick_choice(ConflictChoice::Both, cx)
            }))
            .size_full()
            .bg(theme.colors.window_bg)
            .text_color(theme.colors.text)
            .font_family("monospace")
            .text_size(px(13.0))
            .flex()
            .flex_col()
            .child(self.render_toolbar(&status_text, all_done, &theme, window, cx))
            .child(self.render_content(active, has_base, &theme, window, cx))
    }
}

impl FocusedMergeView {
    fn render_toolbar(
        &self,
        status_text: &str,
        all_done: bool,
        theme: &AppTheme,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let save_bg = if all_done {
            theme.colors.success
        } else {
            theme.colors.accent
        };

        div()
            .id("toolbar")
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .bg(theme.colors.surface_bg)
            .border_b_1()
            .border_color(theme.colors.border)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            // Title
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(14.0))
                    .child("Merge Conflict Resolver"),
            )
            // Spacer
            .child(div().flex_grow())
            // Status
            .child(
                div()
                    .text_color(theme.colors.text_muted)
                    .text_size(px(12.0))
                    .child(SharedString::from(status_text.to_string())),
            )
            // Auto-resolve button
            .child(
                div()
                    .id("btn-auto")
                    .px(px(10.0))
                    .py(px(4.0))
                    .bg(theme.colors.surface_bg_elevated)
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .on_click(|_event: &ClickEvent, _window, cx| {
                        cx.dispatch_action(&AutoResolve);
                    })
                    .child("Auto"),
            )
            // Cancel button
            .child(
                div()
                    .id("btn-cancel")
                    .px(px(10.0))
                    .py(px(4.0))
                    .bg(theme.colors.surface_bg_elevated)
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .on_click(|_event: &ClickEvent, _window, cx| {
                        cx.dispatch_action(&Cancel);
                    })
                    .child("Cancel"),
            )
            // Save button
            .child(
                div()
                    .id("btn-save")
                    .px(px(10.0))
                    .py(px(4.0))
                    .bg(save_bg)
                    .text_color(gpui::rgba(0xffffffff))
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .font_weight(FontWeight::BOLD)
                    .on_click(|_event: &ClickEvent, _window, cx| {
                        cx.dispatch_action(&Save);
                    })
                    .child(if all_done {
                        "Save"
                    } else {
                        "Save (unresolved)"
                    }),
            )
    }

    fn render_content(
        &self,
        active_conflict: usize,
        has_base: bool,
        theme: &AppTheme,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut children: Vec<gpui::AnyElement> = Vec::new();
        let mut conflict_ix = 0usize;

        for seg in &self.segments {
            match seg {
                ConflictSegment::Text(text) => {
                    if !text.is_empty() {
                        children.push(self.render_text_segment(text, theme).into_any_element());
                    }
                }
                ConflictSegment::Block(block) => {
                    let is_active = conflict_ix == active_conflict;
                    children.push(
                        self.render_conflict_block(
                            block,
                            conflict_ix,
                            is_active,
                            has_base && is_active,
                            theme,
                        )
                        .into_any_element(),
                    );
                    conflict_ix += 1;
                }
            }
        }

        div()
            .id("content-scroll")
            .flex_grow()
            .overflow_y_scroll()
            .track_scroll(&self.scroll_handle)
            .px(px(16.0))
            .py(px(8.0))
            .children(children)
    }

    fn render_text_segment(&self, text: &str, theme: &AppTheme) -> impl IntoElement {
        let display = truncate_lines(text, 20);
        div()
            .w_full()
            .py(px(2.0))
            .text_color(theme.colors.text_muted)
            .text_size(px(12.0))
            .child(SharedString::from(display))
    }

    fn render_conflict_block(
        &self,
        block: &ConflictBlock,
        conflict_ix: usize,
        is_active: bool,
        show_base_button: bool,
        theme: &AppTheme,
    ) -> impl IntoElement {
        let border_color = if is_active {
            theme.colors.accent
        } else if block.resolved {
            theme.colors.success
        } else {
            theme.colors.danger
        };

        let resolved_label = if block.resolved {
            match block.choice {
                ConflictChoice::Base => "Base",
                ConflictChoice::Ours => "Ours",
                ConflictChoice::Theirs => "Theirs",
                ConflictChoice::Both => "Both",
            }
        } else {
            "Unresolved"
        };

        let ours_display = truncate_lines(&block.ours, 30);
        let theirs_display = truncate_lines(&block.theirs, 30);
        let ours_bg = if block.resolved && block.choice == ConflictChoice::Ours {
            with_alpha(theme.colors.success, 0.15)
        } else {
            with_alpha(theme.colors.accent, 0.05)
        };
        let theirs_bg = if block.resolved && block.choice == ConflictChoice::Theirs {
            with_alpha(theme.colors.success, 0.15)
        } else {
            with_alpha(theme.colors.accent, 0.05)
        };

        let conflict_id = format!("conflict-{conflict_ix}");

        let mut block_el = div()
            .id(SharedString::from(conflict_id))
            .w_full()
            .my(px(6.0))
            .p(px(8.0))
            .bg(theme.colors.surface_bg)
            .border_l_4()
            .border_color(border_color)
            .rounded_r(px(4.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            // Header: "Conflict N — Ours / Unresolved"
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(12.0))
                            .child(SharedString::from(format!("Conflict {}", conflict_ix + 1))),
                    )
                    .child(
                        div()
                            .px(px(6.0))
                            .py(px(1.0))
                            .rounded(px(3.0))
                            .bg(if block.resolved {
                                theme.colors.success
                            } else {
                                theme.colors.danger
                            })
                            .text_color(gpui::rgba(0xffffffff))
                            .text_size(px(10.0))
                            .child(resolved_label),
                    ),
            )
            // Two-column diff: Ours | Theirs
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .w_full()
                    // Ours column
                    .child(
                        div()
                            .flex_1()
                            .p(px(6.0))
                            .bg(ours_bg)
                            .rounded(px(3.0))
                            .overflow_x_hidden()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(theme.colors.text_muted)
                                    .font_weight(FontWeight::BOLD)
                                    .mb(px(2.0))
                                    .child(SharedString::from(format!(
                                        "LOCAL ({})",
                                        self.label_local
                                    ))),
                            )
                            .child(div().text_size(px(12.0)).whitespace_nowrap().child(
                                SharedString::from(if ours_display.is_empty() {
                                    "(empty)".to_string()
                                } else {
                                    ours_display
                                }),
                            )),
                    )
                    // Theirs column
                    .child(
                        div()
                            .flex_1()
                            .p(px(6.0))
                            .bg(theirs_bg)
                            .rounded(px(3.0))
                            .overflow_x_hidden()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(theme.colors.text_muted)
                                    .font_weight(FontWeight::BOLD)
                                    .mb(px(2.0))
                                    .child(SharedString::from(format!(
                                        "REMOTE ({})",
                                        self.label_remote
                                    ))),
                            )
                            .child(div().text_size(px(12.0)).whitespace_nowrap().child(
                                SharedString::from(if theirs_display.is_empty() {
                                    "(empty)".to_string()
                                } else {
                                    theirs_display
                                }),
                            )),
                    ),
            );

        // Pick buttons (only for active conflict)
        if is_active {
            let mut buttons = div()
                .flex()
                .flex_row()
                .gap(px(4.0))
                .mt(px(4.0))
                .child(self.pick_button("Ours (b)", ConflictChoice::Ours, block, theme))
                .child(self.pick_button("Theirs (c)", ConflictChoice::Theirs, block, theme));

            if show_base_button {
                buttons =
                    buttons.child(self.pick_button("Base (a)", ConflictChoice::Base, block, theme));
            }

            buttons =
                buttons.child(self.pick_button("Both (d)", ConflictChoice::Both, block, theme));

            block_el = block_el.child(buttons);
        }

        block_el
    }

    fn pick_button(
        &self,
        label: &'static str,
        choice: ConflictChoice,
        block: &ConflictBlock,
        theme: &AppTheme,
    ) -> impl IntoElement {
        let is_selected = block.resolved && block.choice == choice;
        let bg = if is_selected {
            theme.colors.accent
        } else {
            theme.colors.surface_bg_elevated
        };
        let text_color = if is_selected {
            gpui::rgba(0xffffffff)
        } else {
            theme.colors.text
        };

        let id = SharedString::from(format!("pick-{}-{}", label, self.active_conflict));

        div()
            .id(id)
            .px(px(8.0))
            .py(px(3.0))
            .bg(bg)
            .text_color(text_color)
            .border_1()
            .border_color(theme.colors.border)
            .rounded(px(3.0))
            .cursor_pointer()
            .text_size(px(11.0))
            .on_click(move |_: &ClickEvent, _window, cx| {
                match choice {
                    ConflictChoice::Base => cx.dispatch_action(&PickBase),
                    ConflictChoice::Ours => cx.dispatch_action(&PickOurs),
                    ConflictChoice::Theirs => cx.dispatch_action(&PickTheirs),
                    ConflictChoice::Both => cx.dispatch_action(&PickBoth),
                };
            })
            .child(label)
    }
}

fn with_alpha(color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    gpui::Rgba {
        r: color.r,
        g: color.g,
        b: color.b,
        a: alpha,
    }
}

// ── Public entry point ───────────────────────────────────────────────

/// Launch a focused GPUI merge window.
///
/// Returns the process exit code:
/// - 0: user saved the resolved output
/// - 1: user canceled/closed, or saved with unresolved conflicts
/// - 2: save failed due to I/O error
pub fn run_focused_merge(config: FocusedMergeConfig) -> i32 {
    let exit_code = Arc::new(AtomicI32::new(EXIT_CANCELED)); // Default: cancel/unresolved
    let exit_code_for_app = exit_code.clone();

    Application::new()
        .with_assets(GitGpuiAssets)
        .run(move |cx: &mut App| {
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            cx.bind_keys([
                KeyBinding::new("ctrl-s", Save, Some("FocusedMerge")),
                KeyBinding::new("cmd-s", Save, Some("FocusedMerge")),
                KeyBinding::new("escape", Cancel, Some("FocusedMerge")),
                KeyBinding::new("ctrl-shift-a", AutoResolve, Some("FocusedMerge")),
                KeyBinding::new("f3", NextConflict, Some("FocusedMerge")),
                KeyBinding::new("f2", PrevConflict, Some("FocusedMerge")),
                KeyBinding::new("alt-down", NextConflict, Some("FocusedMerge")),
                KeyBinding::new("alt-up", PrevConflict, Some("FocusedMerge")),
                KeyBinding::new("b", PickOurs, Some("FocusedMerge")),
                KeyBinding::new("c", PickTheirs, Some("FocusedMerge")),
                KeyBinding::new("a", PickBase, Some("FocusedMerge")),
                KeyBinding::new("d", PickBoth, Some("FocusedMerge")),
            ]);

            let exit_code_clone = exit_code_for_app.clone();
            let bounds = Bounds::centered(None, size(px(1000.0), px(700.0)), cx);

            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(600.0), px(400.0))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("GitGpui — Merge".into()),
                        appears_transparent: false,
                        traffic_light_position: Some(point(px(9.0), px(9.0))),
                    }),
                    app_id: Some("gitgpui-merge".to_string()),
                    window_decorations: Some(WindowDecorations::Server),
                    is_movable: true,
                    is_resizable: true,
                    ..Default::default()
                },
                move |window, cx| {
                    cx.new(|cx| {
                        let view = FocusedMergeView::new(config, exit_code_clone, window, cx);
                        cx.focus_self(window);
                        view
                    })
                },
            )
            .unwrap();

            cx.activate(true);
        });

    exit_code.load(Ordering::SeqCst)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_output_clean_merge() {
        let segments = vec![ConflictSegment::Text("hello world\n".to_string())];
        let view_output = build_output_from_segments(&segments);
        assert_eq!(view_output, "hello world\n");
    }

    #[test]
    fn build_output_resolved_ours() {
        let segments = vec![
            ConflictSegment::Text("before\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base\n".to_string()),
                ours: "ours\n".to_string(),
                theirs: "theirs\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
            ConflictSegment::Text("after\n".to_string()),
        ];
        assert_eq!(
            build_output_from_segments(&segments),
            "before\nours\nafter\n"
        );
    }

    #[test]
    fn build_output_resolved_theirs() {
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "A\n".to_string(),
            theirs: "B\n".to_string(),
            choice: ConflictChoice::Theirs,
            resolved: true,
        })];
        assert_eq!(build_output_from_segments(&segments), "B\n");
    }

    #[test]
    fn build_output_resolved_both() {
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "A\n".to_string(),
            theirs: "B\n".to_string(),
            choice: ConflictChoice::Both,
            resolved: true,
        })];
        assert_eq!(build_output_from_segments(&segments), "A\nB\n");
    }

    #[test]
    fn build_output_resolved_base() {
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("BASE\n".to_string()),
            ours: "A\n".to_string(),
            theirs: "B\n".to_string(),
            choice: ConflictChoice::Base,
            resolved: true,
        })];
        assert_eq!(build_output_from_segments(&segments), "BASE\n");
    }

    #[test]
    fn build_output_unresolved_keeps_conflict_markers_two_way() {
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours line\n".to_string(),
            theirs: "theirs line\n".to_string(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];
        let output = build_output_from_segments_with_labels(&segments, "LOCAL", "REMOTE", "BASE");
        assert!(output.contains("<<<<<<< LOCAL\n"), "output: {output}");
        assert!(output.contains("=======\n"), "output: {output}");
        assert!(output.contains(">>>>>>> REMOTE\n"), "output: {output}");
        assert!(output.contains("ours line\n"), "output: {output}");
        assert!(output.contains("theirs line\n"), "output: {output}");
    }

    #[test]
    fn build_output_unresolved_keeps_conflict_markers_diff3() {
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("base line\n".to_string()),
            ours: "ours line\n".to_string(),
            theirs: "theirs line\n".to_string(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];
        let output = build_output_from_segments_with_labels(&segments, "LOCAL", "REMOTE", "BASE");
        assert!(output.contains("<<<<<<< LOCAL\n"), "output: {output}");
        assert!(output.contains("||||||| BASE\n"), "output: {output}");
        assert!(output.contains("=======\n"), "output: {output}");
        assert!(output.contains(">>>>>>> REMOTE\n"), "output: {output}");
    }

    #[test]
    fn build_output_unresolved_uses_crlf_when_block_uses_crlf() {
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("base line\r\n".to_string()),
            ours: "ours line\r\n".to_string(),
            theirs: "theirs line\r\n".to_string(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];
        let output = build_output_from_segments_with_labels(&segments, "LOCAL", "REMOTE", "BASE");
        assert!(output.contains("<<<<<<< LOCAL\r\n"), "output: {output:?}");
        assert!(output.contains("||||||| BASE\r\n"), "output: {output:?}");
        assert!(output.contains("=======\r\n"), "output: {output:?}");
        assert!(output.contains(">>>>>>> REMOTE\r\n"), "output: {output:?}");
    }

    #[test]
    fn truncate_lines_short() {
        assert_eq!(truncate_lines("a\nb\nc", 5), "a\nb\nc");
    }

    #[test]
    fn truncate_lines_long() {
        assert_eq!(truncate_lines("1\n2\n3\n4\n5\n6", 3), "1\n2\n3\n...");
    }

    #[test]
    fn chosen_block_text_base_missing() {
        let block = ConflictBlock {
            base: None,
            ours: "ours".to_string(),
            theirs: "theirs".to_string(),
            choice: ConflictChoice::Base,
            resolved: false,
        };
        assert_eq!(chosen_block_text(&block), "");
    }

    #[test]
    fn saved_exit_code_clean_merge_is_success() {
        let segments = vec![ConflictSegment::Text("clean\n".to_string())];
        assert_eq!(saved_exit_code(&segments), EXIT_SUCCESS);
    }

    #[test]
    fn saved_exit_code_all_conflicts_resolved_is_success() {
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".to_string()),
            ours: "ours\n".to_string(),
            theirs: "theirs\n".to_string(),
            choice: ConflictChoice::Ours,
            resolved: true,
        })];
        assert_eq!(saved_exit_code(&segments), EXIT_SUCCESS);
    }

    #[test]
    fn saved_exit_code_unresolved_conflicts_are_canceled() {
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".to_string()),
            ours: "ours\n".to_string(),
            theirs: "theirs\n".to_string(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];
        assert_eq!(saved_exit_code(&segments), EXIT_CANCELED);
    }

    #[test]
    fn build_output_unresolved_content_without_trailing_newline() {
        // Content missing trailing newline should still produce well-formed markers.
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours no newline".to_string(),
            theirs: "theirs no newline".to_string(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];
        let output = build_output_from_segments_with_labels(&segments, "L", "R", "B");
        // Each marker must start at the beginning of its own line.
        assert!(
            output.contains("<<<<<<< L\nours no newline\n=======\n"),
            "output: {output:?}"
        );
        assert!(
            output.contains("theirs no newline\n>>>>>>> R\n"),
            "output: {output:?}"
        );
    }

    #[test]
    fn build_output_unresolved_diff3_content_without_trailing_newline() {
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some("base no newline".to_string()),
            ours: "ours".to_string(),
            theirs: "theirs".to_string(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];
        let output = build_output_from_segments_with_labels(&segments, "L", "R", "B");
        assert!(
            output.contains("||||||| B\nbase no newline\n=======\n"),
            "output: {output:?}"
        );
    }

    #[test]
    fn build_output_unresolved_crlf_content_without_trailing_newline() {
        // CRLF detection should still work and marker newlines should use CRLF.
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours\r\nmore".to_string(), // CRLF in middle, no trailing newline
            theirs: "theirs".to_string(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];
        let output = build_output_from_segments_with_labels(&segments, "L", "R", "B");
        // Detected CRLF from ours content, so guard should insert \r\n.
        assert!(
            output.contains("<<<<<<< L\r\n"),
            "start marker: {output:?}"
        );
        assert!(output.contains("more\r\n=======\r\n"), "separator: {output:?}");
        assert!(
            output.contains("theirs\r\n>>>>>>> R\r\n"),
            "end marker: {output:?}"
        );
    }

    #[test]
    fn build_output_unresolved_empty_ours_and_theirs() {
        // Empty content sections should produce well-formed markers with no content.
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: String::new(),
            theirs: String::new(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];
        let output = build_output_from_segments_with_labels(&segments, "L", "R", "B");
        assert_eq!(output, "<<<<<<< L\n=======\n>>>>>>> R\n");
    }

    #[test]
    fn build_output_unresolved_empty_base_section() {
        let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some(String::new()),
            ours: "ours\n".to_string(),
            theirs: "theirs\n".to_string(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];
        let output = build_output_from_segments_with_labels(&segments, "L", "R", "B");
        assert!(
            output.contains("||||||| B\n=======\n"),
            "empty base: {output:?}"
        );
    }

    #[test]
    fn build_output_mixed_resolved_and_unresolved_blocks() {
        let segments = vec![
            ConflictSegment::Text("header\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "A\n".to_string(),
                theirs: "B\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: true,
            }),
            ConflictSegment::Text("middle\n".to_string()),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "C\n".to_string(),
                theirs: "D\n".to_string(),
                choice: ConflictChoice::Theirs,
                resolved: false, // Unresolved
            }),
            ConflictSegment::Text("footer\n".to_string()),
        ];
        let output = build_output_from_segments_with_labels(&segments, "L", "R", "B");
        // First block resolved to ours: "A\n"
        assert!(output.starts_with("header\nA\nmiddle\n"), "output: {output:?}");
        // Second block unresolved: markers around C/D
        assert!(
            output.contains("<<<<<<< L\nC\n=======\nD\n>>>>>>> R\n"),
            "output: {output:?}"
        );
        assert!(output.ends_with("footer\n"), "output: {output:?}");
    }

    #[test]
    fn build_output_multiple_consecutive_unresolved_blocks() {
        let segments = vec![
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "a1\n".to_string(),
                theirs: "b1\n".to_string(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: "a2\n".to_string(),
                theirs: "b2\n".to_string(),
                choice: ConflictChoice::Theirs,
                resolved: false,
            }),
        ];
        let output = build_output_from_segments_with_labels(&segments, "L", "R", "B");
        let expected = "<<<<<<< L\na1\n=======\nb1\n>>>>>>> R\n\
                        <<<<<<< L\na2\n=======\nb2\n>>>>>>> R\n";
        assert_eq!(output, expected);
    }

    /// Helper to build output without needing a full view.
    fn build_output_from_segments(segments: &[ConflictSegment]) -> String {
        build_output_from_segments_with_labels(segments, "LOCAL", "REMOTE", "BASE")
    }

    fn build_output_from_segments_with_labels(
        segments: &[ConflictSegment],
        label_local: &str,
        label_remote: &str,
        label_base: &str,
    ) -> String {
        let mut out = String::new();
        for seg in segments {
            match seg {
                ConflictSegment::Text(text) => out.push_str(text),
                ConflictSegment::Block(block) => out.push_str(&block_output_text(
                    block,
                    label_local,
                    label_remote,
                    label_base,
                )),
            }
        }
        out
    }
}
