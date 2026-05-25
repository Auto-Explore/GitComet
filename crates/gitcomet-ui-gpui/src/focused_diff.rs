//! Focused diff window for standalone `gitcomet difftool` invocation.
//!
//! Opens a GPUI window that displays a unified diff with color-coded lines.
//! The user reviews the diff and closes the window (exit 0).

use crate::assets::GitCometAssets;
use crate::launch_guard::run_with_panic_guard;
use crate::theme::AppTheme;
use gitcomet_state::session;
use gpui::prelude::*;
use gpui::{
    App, Bounds, FocusHandle, Focusable, FontWeight, KeyBinding, Pixels, Render, ScrollHandle,
    SharedString, TitlebarOptions, Window, WindowBounds, WindowDecorations, WindowOptions, actions,
    div, point,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

// ── Actions ──────────────────────────────────────────────────────────

actions!(focused_diff, [Close]);
const FOCUSED_DIFF_EXIT_ERROR: i32 = 2;
const FOCUSED_DIFF_MIN_WIDTH_PX: f32 = 500.0;
const FOCUSED_DIFF_MIN_HEIGHT_PX: f32 = 300.0;
const FOCUSED_DIFF_DEFAULT_WIDTH_PX: f32 = 900.0;
const FOCUSED_DIFF_DEFAULT_HEIGHT_PX: f32 = 650.0;

actions!(
    focused_diff_scale,
    [IncreaseUiScale, DecreaseUiScale, ResetUiScale]
);

fn focused_diff_min_size_for_percent(percent: u32) -> gpui::Size<Pixels> {
    crate::ui_scale::design_size_from_percent(
        FOCUSED_DIFF_MIN_WIDTH_PX,
        FOCUSED_DIFF_MIN_HEIGHT_PX,
        percent,
    )
}

fn focused_diff_default_size_for_percent(percent: u32) -> gpui::Size<Pixels> {
    crate::ui_scale::design_size_from_percent(
        FOCUSED_DIFF_DEFAULT_WIDTH_PX,
        FOCUSED_DIFF_DEFAULT_HEIGHT_PX,
        percent,
    )
}

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
    diff_whitespace_mode: FocusedDiffWhitespaceMode,
    exit_code: Arc<AtomicI32>,
    focus_handle: FocusHandle,
    scroll_handle: ScrollHandle,
    theme: AppTheme,
    ui_font_family: String,
    editor_font_family: String,
    use_font_ligatures: bool,
    ui_scale_percent: u32,
}

#[derive(Clone, Debug)]
struct DiffLine {
    kind: DiffLineKind,
    visual_kind: DiffLineKind,
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FocusedDiffWhitespaceMode {
    #[default]
    Show,
    Ignore,
}

impl FocusedDiffWhitespaceMode {
    fn from_key(raw: &str) -> Option<Self> {
        match raw {
            "show" => Some(Self::Show),
            "ignore" => Some(Self::Ignore),
            _ => None,
        }
    }

    const fn key(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Ignore => "ignore",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Show => "Show",
            Self::Ignore => "Ignore",
        }
    }

    const fn toggled(self) -> Self {
        match self {
            Self::Show => Self::Ignore,
            Self::Ignore => Self::Show,
        }
    }
}

impl FocusedDiffView {
    fn new(
        config: FocusedDiffConfig,
        exit_code: Arc<AtomicI32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let ui_session = session::load();
        let diff_whitespace_mode = ui_session
            .diff_whitespace_mode
            .as_deref()
            .and_then(FocusedDiffWhitespaceMode::from_key)
            .unwrap_or_default();
        let lines = parse_diff_lines(&config.diff_text, diff_whitespace_mode);
        let title = config
            .display_path
            .unwrap_or_else(|| format!("{} vs {}", config.label_left, config.label_right));

        let theme = AppTheme::default_for_window_appearance(window.appearance());
        let ui_scale = crate::ui_scale::current_or_initialize_from_session(&ui_session, cx);
        let font_preferences =
            crate::font_preferences::current_or_initialize_from_session(window, &ui_session, cx);

        Self {
            lines,
            title,
            diff_whitespace_mode,
            exit_code,
            focus_handle: cx.focus_handle(),
            scroll_handle: ScrollHandle::new(),
            theme,
            ui_font_family: crate::font_preferences::applied_ui_font_family(
                &font_preferences.ui_font_family,
            ),
            editor_font_family: crate::font_preferences::applied_editor_font_family(
                &font_preferences.editor_font_family,
            ),
            use_font_ligatures: font_preferences.use_font_ligatures,
            ui_scale_percent: ui_scale.percent,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.exit_code.store(0, Ordering::SeqCst);
        cx.quit();
    }

    fn set_diff_whitespace_mode(
        &mut self,
        mode: FocusedDiffWhitespaceMode,
        cx: &mut Context<Self>,
    ) {
        if self.diff_whitespace_mode == mode {
            return;
        }
        self.diff_whitespace_mode = mode;
        apply_visual_diff_line_kinds(self.lines.as_mut_slice(), mode);
        let _ = session::persist_ui_settings(session::UiSettings {
            diff_whitespace_mode: Some(mode.key().to_string()),
            ..session::UiSettings::default()
        });
        cx.notify();
    }

    fn set_ui_scale_percent(&mut self, percent: u32, window: &mut Window, cx: &mut Context<Self>) {
        let percent = crate::ui_scale::set_current(cx, percent).percent;
        if self.ui_scale_percent == percent {
            return;
        }

        self.ui_scale_percent = percent;
        crate::ui_scale::apply_to_window(window, percent);
        crate::app::ensure_window_respects_min_size(
            window,
            focused_diff_min_size_for_percent(percent),
        );
        let _ = session::persist_ui_settings(session::UiSettings {
            ui_scale_percent: Some(percent),
            ..session::UiSettings::default()
        });
        cx.notify();
    }
}

impl Focusable for FocusedDiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn parse_diff_lines(text: &str, whitespace_mode: FocusedDiffWhitespaceMode) -> Vec<DiffLine> {
    let mut lines = text
        .lines()
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
                visual_kind: kind,
                content: line.to_string(),
            }
        })
        .collect::<Vec<_>>();
    apply_visual_diff_line_kinds(lines.as_mut_slice(), whitespace_mode);
    lines
}

fn diff_line_content(line: &DiffLine) -> &str {
    match line.kind {
        DiffLineKind::Add => line.content.strip_prefix('+').unwrap_or(&line.content),
        DiffLineKind::Remove => line.content.strip_prefix('-').unwrap_or(&line.content),
        DiffLineKind::Context => line.content.strip_prefix(' ').unwrap_or(&line.content),
        DiffLineKind::Header | DiffLineKind::HunkHeader => &line.content,
    }
}

fn push_non_whitespace(text: &str, out: &mut String) {
    out.extend(text.chars().filter(|ch| !ch.is_whitespace()));
}

fn is_no_newline_marker(line: &DiffLine) -> bool {
    line.content.starts_with("\\ No newline")
}

fn is_whitespace_group_line(line: &DiffLine) -> bool {
    matches!(line.kind, DiffLineKind::Remove | DiffLineKind::Add) || is_no_newline_marker(line)
}

fn apply_visual_diff_line_kinds(
    lines: &mut [DiffLine],
    whitespace_mode: FocusedDiffWhitespaceMode,
) {
    for line in lines.iter_mut() {
        line.visual_kind = line.kind;
    }
    if whitespace_mode == FocusedDiffWhitespaceMode::Show {
        return;
    }

    let mut ix = 0usize;
    while ix < lines.len() {
        if !matches!(lines[ix].kind, DiffLineKind::Remove | DiffLineKind::Add) {
            ix += 1;
            continue;
        }

        let group_start = ix;
        let mut old_stripped = String::new();
        let mut new_stripped = String::new();
        while ix < lines.len() && is_whitespace_group_line(&lines[ix]) {
            match lines[ix].kind {
                DiffLineKind::Remove => {
                    push_non_whitespace(diff_line_content(&lines[ix]), &mut old_stripped)
                }
                DiffLineKind::Add => {
                    push_non_whitespace(diff_line_content(&lines[ix]), &mut new_stripped)
                }
                DiffLineKind::Header | DiffLineKind::HunkHeader | DiffLineKind::Context => {}
            }
            ix += 1;
        }

        if old_stripped == new_stripped {
            for line in &mut lines[group_start..ix] {
                line.visual_kind = DiffLineKind::Context;
            }
        }
    }
}

impl Render for FocusedDiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let line_count = self.lines.len();
        let scaled_px = |value| crate::ui_scale::design_px_from_window(value, window);
        let next_whitespace_mode = self.diff_whitespace_mode.toggled();

        div()
            .id("focused-diff-root")
            .key_context("FocusedDiff")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, _window, cx| this.close(cx)))
            .on_action(cx.listener(|this, _: &IncreaseUiScale, window, cx| {
                this.set_ui_scale_percent(
                    crate::ui_scale::step_up(this.ui_scale_percent),
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &DecreaseUiScale, window, cx| {
                this.set_ui_scale_percent(
                    crate::ui_scale::step_down(this.ui_scale_percent),
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &ResetUiScale, window, cx| {
                this.set_ui_scale_percent(crate::ui_scale::DEFAULT_UI_SCALE_PERCENT, window, cx);
            }))
            .size_full()
            .bg(theme.colors.window_bg)
            .text_color(theme.colors.text)
            .font(gpui::Font {
                family: self.ui_font_family.clone().into(),
                features: crate::font_preferences::applied_font_features(self.use_font_ligatures),
                fallbacks: None,
                weight: FontWeight::default(),
                style: gpui::FontStyle::default(),
            })
            .text_size(scaled_px(13.0))
            .flex()
            .flex_col()
            // Toolbar
            .child(
                div()
                    .w_full()
                    .px(scaled_px(12.0))
                    .py(scaled_px(8.0))
                    .bg(theme.colors.surface_bg)
                    .border_b_1()
                    .border_color(theme.colors.border)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(scaled_px(8.0))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_size(scaled_px(14.0))
                            .child(SharedString::from(self.title.clone())),
                    )
                    .child(div().flex_grow())
                    .child(
                        div()
                            .text_color(theme.colors.text_muted)
                            .text_size(scaled_px(12.0))
                            .child(SharedString::from(format!("{line_count} lines"))),
                    )
                    .child(
                        div()
                            .id("btn-whitespace-mode")
                            .px(scaled_px(10.0))
                            .py(scaled_px(4.0))
                            .border_1()
                            .border_color(theme.colors.border)
                            .rounded(scaled_px(2.0))
                            .cursor_pointer()
                            .on_click(cx.listener(
                                move |this, _e: &gpui::ClickEvent, _window, cx| {
                                    this.set_diff_whitespace_mode(next_whitespace_mode, cx);
                                },
                            ))
                            .child(SharedString::from(format!(
                                "Whitespace: {}",
                                self.diff_whitespace_mode.label()
                            ))),
                    )
                    .child(
                        div()
                            .id("btn-close")
                            .px(scaled_px(10.0))
                            .py(scaled_px(4.0))
                            .bg(theme.colors.accent)
                            .text_color(theme.colors.accent_text)
                            .rounded(scaled_px(2.0))
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
                    .font_family(self.editor_font_family.clone())
                    .px(scaled_px(16.0))
                    .py(scaled_px(4.0))
                    .children(
                        self.lines
                            .iter()
                            .enumerate()
                            .map(|(i, line)| render_diff_line(i, line, &theme, window)),
                    ),
            )
    }
}

fn render_diff_line(
    index: usize,
    line: &DiffLine,
    theme: &AppTheme,
    window: &Window,
) -> impl IntoElement {
    let (text_color, bg) = match line.visual_kind {
        DiffLineKind::Header => (theme.colors.text_muted, None),
        DiffLineKind::HunkHeader => (theme.colors.accent, None),
        DiffLineKind::Add => (theme.colors.diff_add_text, Some(theme.colors.diff_add_bg)),
        DiffLineKind::Remove => (
            theme.colors.diff_remove_text,
            Some(theme.colors.diff_remove_bg),
        ),
        DiffLineKind::Context => (theme.colors.text, None),
    };

    let line_num = format!("{:>4} ", index + 1);
    let scaled_px = |value| crate::ui_scale::design_px_from_window(value, window);

    let mut el = div()
        .w_full()
        .flex()
        .flex_row()
        .child(
            div()
                .text_color(theme.colors.text_muted)
                .text_size(scaled_px(11.0))
                .min_w(scaled_px(40.0))
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

fn bind_focused_diff_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Close, Some("FocusedDiff")),
        KeyBinding::new("q", Close, Some("FocusedDiff")),
        KeyBinding::new("ctrl-w", Close, Some("FocusedDiff")),
        KeyBinding::new("cmd-w", Close, Some("FocusedDiff")),
        KeyBinding::new("secondary-+", IncreaseUiScale, Some("FocusedDiff")),
        KeyBinding::new("secondary-=", IncreaseUiScale, Some("FocusedDiff")),
        KeyBinding::new("secondary--", DecreaseUiScale, Some("FocusedDiff")),
        KeyBinding::new("secondary-0", ResetUiScale, Some("FocusedDiff")),
    ]);
}

// ── Public entry point ───────────────────────────────────────────────

/// Launch a focused GPUI diff window.
///
/// Returns process exit code (0 on success, 2 when the window fails to launch).
pub fn run_focused_diff(config: FocusedDiffConfig) -> i32 {
    if let Err(err) = crate::app::ensure_graphics_device_available("focused diff GPUI launch") {
        eprintln!("Failed to launch focused diff window: {err}");
        return FOCUSED_DIFF_EXIT_ERROR;
    }

    let exit_code = Arc::new(AtomicI32::new(0));
    let exit_code_for_app = exit_code.clone();

    if let Err(err) = run_with_panic_guard("focused diff GPUI launch", move || {
        crate::app::application()
            .with_assets(GitCometAssets)
            .run(move |cx: &mut App| {
                if let Err(err) = crate::bundled_fonts::register(cx) {
                    eprintln!("Failed to register bundled fonts: {err:#}");
                }
                let ui_session = session::load();
                let ui_scale = crate::ui_scale::current_or_initialize_from_session(&ui_session, cx);
                cx.on_window_closed(|cx| {
                    if cx.windows().is_empty() {
                        cx.quit();
                    }
                })
                .detach();

                bind_focused_diff_keys(cx);

                let exit_code_clone = exit_code_for_app.clone();
                let bounds = Bounds::centered(
                    None,
                    focused_diff_default_size_for_percent(ui_scale.percent),
                    cx,
                );
                let ui_scale_percent = ui_scale.percent;

                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        window_min_size: Some(focused_diff_min_size_for_percent(ui_scale_percent)),
                        titlebar: Some(TitlebarOptions {
                            title: Some("GitComet — Diff".into()),
                            appears_transparent: false,
                            traffic_light_position: Some(point(
                                crate::ui_scale::design_px_from_percent(9.0, ui_scale_percent),
                                crate::ui_scale::design_px_from_percent(9.0, ui_scale_percent),
                            )),
                        }),
                        app_id: Some("gitcomet-diff".to_string()),
                        window_decorations: Some(WindowDecorations::Server),
                        is_movable: true,
                        is_resizable: true,
                        ..Default::default()
                    },
                    move |window, cx| {
                        crate::ui_scale::apply_to_window(window, ui_scale_percent);
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
    use gpui::{
        Action, Context, FocusHandle, InteractiveElement, IntoElement, Render, Styled, Window, div,
    };
    use std::sync::{Arc, Mutex};

    struct FocusedDiffKeyProbe {
        focus_handle: FocusHandle,
        observed_actions: Arc<Mutex<Vec<String>>>,
    }

    impl FocusedDiffKeyProbe {
        fn new(observed_actions: Arc<Mutex<Vec<String>>>, cx: &mut Context<Self>) -> Self {
            Self {
                focus_handle: cx.focus_handle().tab_index(0).tab_stop(true),
                observed_actions,
            }
        }

        fn focus_handle(&self) -> FocusHandle {
            self.focus_handle.clone()
        }

        fn record_action(&self, action_name: &str) {
            self.observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(action_name.to_string());
        }
    }

    impl Render for FocusedDiffKeyProbe {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .key_context("FocusedDiff")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(|this, _: &Close, _window, _cx| {
                    this.record_action(Close.name());
                }))
        }
    }

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
        let lines = parse_diff_lines(diff, FocusedDiffWhitespaceMode::Show);

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
        let lines = parse_diff_lines("", FocusedDiffWhitespaceMode::Show);
        assert!(lines.is_empty());
    }

    #[test]
    fn parse_no_diff_only_context() {
        let lines = parse_diff_lines("hello\nworld\n", FocusedDiffWhitespaceMode::Show);
        assert!(lines.iter().all(|l| l.kind == DiffLineKind::Context));
    }

    #[test]
    fn parse_diff_lines_ignore_whitespace_handles_eof_newline_markers() {
        let diff = "\
@@ -1 +1 @@
-foo
\\ No newline at end of file
+foo
";
        let lines = parse_diff_lines(diff, FocusedDiffWhitespaceMode::Ignore);

        assert_eq!(lines[1].kind, DiffLineKind::Remove);
        assert_eq!(lines[2].kind, DiffLineKind::Context);
        assert_eq!(lines[3].kind, DiffLineKind::Add);
        assert_eq!(lines[1].visual_kind, DiffLineKind::Context);
        assert_eq!(lines[2].visual_kind, DiffLineKind::Context);
        assert_eq!(lines[3].visual_kind, DiffLineKind::Context);
    }

    #[test]
    fn parse_diff_lines_ignore_whitespace_neutralizes_visual_kinds() {
        let diff = "\
@@ -1,2 +1 @@
-foo
-bar
+foobar
";
        let lines = parse_diff_lines(diff, FocusedDiffWhitespaceMode::Ignore);

        assert_eq!(lines[1].kind, DiffLineKind::Remove);
        assert_eq!(lines[2].kind, DiffLineKind::Remove);
        assert_eq!(lines[3].kind, DiffLineKind::Add);
        assert_eq!(lines[1].visual_kind, DiffLineKind::Context);
        assert_eq!(lines[2].visual_kind, DiffLineKind::Context);
        assert_eq!(lines[3].visual_kind, DiffLineKind::Context);
    }

    #[gpui::test]
    fn focused_diff_keybindings_dispatch_close(cx: &mut gpui::TestAppContext) {
        let observed_actions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (view, cx) = cx.add_window_view(|_window, cx| {
            FocusedDiffKeyProbe::new(Arc::clone(&observed_actions), cx)
        });

        cx.update(|window, app| {
            app.clear_key_bindings();
            bind_focused_diff_keys(app);
            let focus = view.update(app, |view, _cx| view.focus_handle());
            window.focus(&focus, app);
            let _ = window.draw(app);
        });

        for keystroke in ["escape", "q", "ctrl-w", "cmd-w"] {
            observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            cx.simulate_keystrokes(keystroke);
            let actual_action = observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last()
                .cloned();
            assert_eq!(
                actual_action.as_deref(),
                Some(Close.name()),
                "expected `{keystroke}` to resolve to `{}`",
                Close.name(),
            );
        }
    }
}
