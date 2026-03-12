//! Focused diff window for standalone `gitcomet-app difftool` invocation.
//!
//! Opens a GPUI window that displays a unified diff with color-coded lines.
//! The user reviews the diff and closes the window (exit 0).

use crate::assets::GitCometAssets;
use crate::launch_guard::run_with_panic_guard;
use crate::theme::{AppTheme, with_alpha};
use gpui::prelude::*;
use gpui::{
    App, Application, Bounds, FocusHandle, Focusable, FontWeight, KeyBinding, Render, ScrollHandle,
    SharedString, TitlebarOptions, Window, WindowBounds, WindowDecorations, WindowOptions, actions,
    div, point, px, size,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

// ── Actions ──────────────────────────────────────────────────────────

actions!(focused_diff, [Close]);
const FOCUSED_DIFF_EXIT_ERROR: i32 = 2;

// ── Public config ────────────────────────────────────────────────────

/// Configuration for the focused diff window.
#[derive(Clone, Debug)]
pub struct FocusedDiffConfig {
    pub label_left: String,
    pub label_right: String,
    pub display_path: Option<String>,
    /// The unified diff text to display.
    pub diff_text: String,
}

// ── View state ───────────────────────────────────────────────────────

struct FocusedDiffView {
    lines: Vec<DiffLine>,
    title: String,
    exit_code: Arc<AtomicI32>,
    focus_handle: FocusHandle,
    scroll_handle: ScrollHandle,
    theme: AppTheme,
}

#[derive(Clone, Debug)]
struct DiffLine {
    kind: DiffLineKind,
    content: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffLineKind {
    Header,
    HunkHeader,
    Add,
    Remove,
    Context,
}

impl FocusedDiffView {
    fn new(
        config: FocusedDiffConfig,
        exit_code: Arc<AtomicI32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let lines = parse_diff_lines(&config.diff_text);
        let title = config
            .display_path
            .unwrap_or_else(|| format!("{} vs {}", config.label_left, config.label_right));

        let theme = AppTheme::default_for_window_appearance(window.appearance());

        Self {
            lines,
            title,
            exit_code,
            focus_handle: cx.focus_handle(),
            scroll_handle: ScrollHandle::new(),
            theme,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.exit_code.store(0, Ordering::SeqCst);
        cx.quit();
    }
}

impl Focusable for FocusedDiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn parse_diff_lines(text: &str) -> Vec<DiffLine> {
    text.lines()
        .map(|line| {
            let kind = if line.starts_with("diff ")
                || line.starts_with("index ")
                || line.starts_with("--- ")
                || line.starts_with("+++ ")
            {
                DiffLineKind::Header
            } else if line.starts_with("@@") {
                DiffLineKind::HunkHeader
            } else if line.starts_with('+') {
                DiffLineKind::Add
            } else if line.starts_with('-') {
                DiffLineKind::Remove
            } else {
                DiffLineKind::Context
            };
            DiffLine {
                kind,
                content: line.to_string(),
            }
        })
        .collect()
}

impl Render for FocusedDiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let line_count = self.lines.len();

        div()
            .id("focused-diff-root")
            .key_context("FocusedDiff")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, _window, cx| this.close(cx)))
            .size_full()
            .bg(theme.colors.window_bg)
            .text_color(theme.colors.text)
            .font_family("monospace")
            .text_size(px(13.0))
            .flex()
            .flex_col()
            // Toolbar
            .child(
                div()
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
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(14.0))
                            .child(SharedString::from(self.title.clone())),
                    )
                    .child(div().flex_grow())
                    .child(
                        div()
                            .text_color(theme.colors.text_muted)
                            .text_size(px(12.0))
                            .child(SharedString::from(format!("{line_count} lines"))),
                    )
                    .child(
                        div()
                            .id("btn-close")
                            .px(px(10.0))
                            .py(px(4.0))
                            .bg(theme.colors.accent)
                            .text_color(gpui::rgba(0xffffffff))
                            .rounded(px(2.0))
                            .cursor_pointer()
                            .font_weight(FontWeight::BOLD)
                            .on_click(|_: &gpui::ClickEvent, _window, cx| {
                                cx.dispatch_action(&Close);
                            })
                            .child("Close"),
                    ),
            )
            // Diff content
            .child(
                div()
                    .id("diff-scroll")
                    .flex_grow()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll_handle)
                    .px(px(16.0))
                    .py(px(4.0))
                    .children(
                        self.lines
                            .iter()
                            .enumerate()
                            .map(|(i, line)| render_diff_line(i, line, &theme)),
                    ),
            )
    }
}

fn render_diff_line(index: usize, line: &DiffLine, theme: &AppTheme) -> impl IntoElement {
    let (text_color, bg) = match line.kind {
        DiffLineKind::Header => (theme.colors.text_muted, None),
        DiffLineKind::HunkHeader => (theme.colors.accent, None),
        DiffLineKind::Add => (
            theme.colors.success,
            Some(with_alpha(theme.colors.success, 0.08)),
        ),
        DiffLineKind::Remove => (
            theme.colors.danger,
            Some(with_alpha(theme.colors.danger, 0.08)),
        ),
        DiffLineKind::Context => (theme.colors.text, None),
    };

    let line_num = format!("{:>4} ", index + 1);

    let mut el = div()
        .w_full()
        .flex()
        .flex_row()
        .child(
            div()
                .text_color(theme.colors.text_muted)
                .text_size(px(11.0))
                .min_w(px(40.0))
                .child(SharedString::from(line_num)),
        )
        .child(
            div()
                .flex_grow()
                .text_color(text_color)
                .whitespace_nowrap()
                .child(SharedString::from(line.content.clone())),
        );

    if let Some(bg) = bg {
        el = el.bg(bg);
    }

    el
}

// ── Public entry point ───────────────────────────────────────────────

/// Launch a focused GPUI diff window.
///
/// Returns process exit code (0 on success, 2 when the window fails to launch).
pub fn run_focused_diff(config: FocusedDiffConfig) -> i32 {
    #[cfg(target_os = "macos")]
    {
        let count = metal::Device::all().len();
        if count == 0 {
            eprintln!(
                "Failed to launch focused diff window: no compatible Metal graphics device is available in this macOS session."
            );
            return FOCUSED_DIFF_EXIT_ERROR;
        }
    }

    let exit_code = Arc::new(AtomicI32::new(0));
    let exit_code_for_app = exit_code.clone();

    if let Err(err) = run_with_panic_guard("focused diff GPUI launch", move || {
        Application::new()
            .with_assets(GitCometAssets)
            .run(move |cx: &mut App| {
                cx.on_window_closed(|cx| {
                    if cx.windows().is_empty() {
                        cx.quit();
                    }
                })
                .detach();

                cx.bind_keys([
                    KeyBinding::new("escape", Close, Some("FocusedDiff")),
                    KeyBinding::new("q", Close, Some("FocusedDiff")),
                    KeyBinding::new("ctrl-w", Close, Some("FocusedDiff")),
                    KeyBinding::new("cmd-w", Close, Some("FocusedDiff")),
                ]);

                let exit_code_clone = exit_code_for_app.clone();
                let bounds = Bounds::centered(None, size(px(900.0), px(650.0)), cx);

                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        window_min_size: Some(size(px(500.0), px(300.0))),
                        titlebar: Some(TitlebarOptions {
                            title: Some("GitComet — Diff".into()),
                            appears_transparent: false,
                            traffic_light_position: Some(point(px(9.0), px(9.0))),
                        }),
                        app_id: Some("gitcomet-diff".to_string()),
                        window_decorations: Some(WindowDecorations::Server),
                        is_movable: true,
                        is_resizable: true,
                        ..Default::default()
                    },
                    move |window, cx| {
                        cx.new(|cx| {
                            let view = FocusedDiffView::new(config, exit_code_clone, window, cx);
                            cx.focus_self(window);
                            view
                        })
                    },
                )
                .expect("failed to open focused diff window");

                cx.activate(true);
            });
    }) {
        eprintln!("Failed to launch focused diff window: {err}");
        return FOCUSED_DIFF_EXIT_ERROR;
    }

    exit_code.load(Ordering::SeqCst)
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_diff_lines_classifies_correctly() {
        let diff = "\
diff --git a/f b/f
index 1234567..abcdef0 100644
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 context
-removed
+added
";
        let lines = parse_diff_lines(diff);

        assert_eq!(lines[0].kind, DiffLineKind::Header); // diff --git
        assert_eq!(lines[1].kind, DiffLineKind::Header); // index
        assert_eq!(lines[2].kind, DiffLineKind::Header); // ---
        assert_eq!(lines[3].kind, DiffLineKind::Header); // +++
        assert_eq!(lines[4].kind, DiffLineKind::HunkHeader); // @@
        assert_eq!(lines[5].kind, DiffLineKind::Context); // context
        assert_eq!(lines[6].kind, DiffLineKind::Remove); // -removed
        assert_eq!(lines[7].kind, DiffLineKind::Add); // +added
    }

    #[test]
    fn parse_empty_diff() {
        let lines = parse_diff_lines("");
        assert!(lines.is_empty());
    }

    #[test]
    fn parse_no_diff_only_context() {
        let lines = parse_diff_lines("hello\nworld\n");
        assert!(lines.iter().all(|l| l.kind == DiffLineKind::Context));
    }
}
