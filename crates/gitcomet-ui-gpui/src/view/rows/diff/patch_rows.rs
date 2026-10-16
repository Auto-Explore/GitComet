use super::super::diff_canvas;
use super::super::diff_text::*;
use super::super::*;
use crate::kit::click::PointerClickExt as _;
use crate::kit::interaction as controls;
use crate::kit::interaction::ControlInteractionExt as _;
use crate::view::panes::diff::helpers::state::CollapsedDiffExpansionKind;
use crate::view::panes::diff::helpers::state::CollapsedDiffHunk;
use gitcomet_core::domain::DiffLineKind;
use gitcomet_core::file_diff::FileDiffRowKind;

use super::DiffRowsRenderPass;
use super::PatchSplitColumn;
use super::SplitDiffRowsRenderPass;
use super::blame_presentation::BlamePrev;
use super::blame_presentation::build_row_blame_paint_tracked;
use super::file_rows::build_file_diff_cached_styled_text;
use super::file_rows::file_diff_split_side_text;
use super::file_rows::file_diff_split_side_text_owned;
use super::file_rows::heuristic_streamed_diff_text_spec;

const COLLAPSED_DIFF_INLINE_HUNK_SHELL_DEBUG_SELECTOR: &str = "collapsed_diff_inline_hunk_shell";

const COLLAPSED_DIFF_INLINE_HUNK_GUTTER_DEBUG_SELECTOR: &str = "collapsed_diff_inline_hunk_gutter";

const COLLAPSED_DIFF_INLINE_HUNK_UP_DEBUG_SELECTOR: &str = "collapsed_diff_inline_hunk_up";

const COLLAPSED_DIFF_INLINE_HUNK_DOWN_DEBUG_SELECTOR: &str = "collapsed_diff_inline_hunk_down";

const COLLAPSED_DIFF_INLINE_HUNK_SHORT_DEBUG_SELECTOR: &str = "collapsed_diff_inline_hunk_short";

const COLLAPSED_DIFF_SPLIT_LEFT_HUNK_SHELL_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_left_hunk_shell";

const COLLAPSED_DIFF_SPLIT_LEFT_HUNK_GUTTER_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_left_hunk_gutter";

const COLLAPSED_DIFF_SPLIT_LEFT_HUNK_UP_DEBUG_SELECTOR: &str = "collapsed_diff_split_left_hunk_up";

const COLLAPSED_DIFF_SPLIT_LEFT_HUNK_DOWN_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_left_hunk_down";

const COLLAPSED_DIFF_SPLIT_LEFT_HUNK_SHORT_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_left_hunk_short";

const COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_SHELL_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_right_hunk_shell";

const COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_GUTTER_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_right_hunk_gutter";

const COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_UP_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_right_hunk_up";

const COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_DOWN_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_right_hunk_down";

const COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_SHORT_DEBUG_SELECTOR: &str =
    "collapsed_diff_split_right_hunk_short";

const COLLAPSED_HUNK_BACKGROUND_OVERDRAW_PX: f32 = 1.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CollapsedHunkRevealAction {
    Up,
    Down,
    DownBefore,
    Short,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CollapsedHunkRevealClick {
    action: CollapsedHunkRevealAction,
    src_ix: usize,
}

fn diff_file_header_height(theme: AppTheme, ui_scale_percent: u32) -> Pixels {
    crate::view::panes::diff::helpers::presentation::diff_file_header_height_for_ui_scale(
        theme,
        ui_scale_percent,
    )
}

fn diff_hunk_header_height(theme: AppTheme, ui_scale_percent: u32) -> Pixels {
    crate::view::panes::diff::helpers::presentation::diff_hunk_header_height_for_ui_scale(
        theme,
        ui_scale_percent,
    )
}

fn collapsed_hunk_header_row_height(theme: AppTheme, ui_scale_percent: u32) -> Pixels {
    theme.editor_row_height(ui_scale_percent)
}

pub(super) fn collapsed_hunk_shell_width(
    handle: &gpui::UniformListScrollHandle,
    fallback_width: Pixels,
) -> Pixels {
    let width = handle
        .0
        .borrow()
        .base_handle
        .bounds()
        .size
        .width
        .max(px(0.0));
    if width > px(0.0) {
        width
    } else {
        fallback_width.max(px(0.0))
    }
}

fn scroll_pinned_hunk_shell(
    scroll_handle: gpui::UniformListScrollHandle,
    background: Option<gpui::Rgba>,
    child: AnyElement,
) -> ScrollPinnedHunkShell {
    ScrollPinnedHunkShell {
        child,
        scroll_handle,
        background,
    }
}

fn collapsed_hunk_bg_fill_bounds(bounds: gpui::Bounds<Pixels>) -> gpui::Bounds<Pixels> {
    gpui::Bounds::new(
        bounds.origin,
        gpui::size(
            bounds.size.width,
            bounds.size.height + px(COLLAPSED_HUNK_BACKGROUND_OVERDRAW_PX),
        ),
    )
}

fn collapsed_hunk_header_bg(theme: AppTheme) -> gpui::Rgba {
    with_alpha(
        theme.colors.foreground.secondary,
        if theme.is_dark { 0.14 } else { 0.10 },
    )
}

fn focused_diff_neutral_row_bg(theme: AppTheme) -> gpui::Rgba {
    with_alpha(
        theme.colors.foreground.secondary,
        if theme.is_dark { 0.26 } else { 0.16 },
    )
}

/// The focused row sits inside the diff body, so it takes the diff palette's own
/// focused token rather than a tint derived from the status palette: those are
/// two different greens and reds in every theme, and deriving it here made
/// focusing a row shift its hue and left `diff.*.focused_background` with no
/// effect at all.
///
/// A context/header/hunk row belongs to no diff kind and keeps the neutral wash.
fn focused_diff_line_bg(theme: AppTheme, kind: DiffLineKind) -> gpui::Rgba {
    let background = match kind {
        DiffLineKind::Add => theme.colors.diff.added.focused_background,
        DiffLineKind::Remove => theme.colors.diff.removed.focused_background,
        DiffLineKind::Context | DiffLineKind::Header | DiffLineKind::Hunk => {
            focused_diff_neutral_row_bg(theme)
        }
    };
    crate::theme::composite_over(theme.colors.editor.background, background)
}

fn focused_collapsed_hunk_bg(theme: AppTheme, _hunk: Option<CollapsedDiffHunk>) -> gpui::Rgba {
    with_alpha(
        theme.colors.accent.foreground,
        if theme.is_dark { 0.22 } else { 0.16 },
    )
}

fn collapsed_inline_hunk_bg(
    theme: AppTheme,
    _hunk: Option<CollapsedDiffHunk>,
    _expansion_kind: CollapsedDiffExpansionKind,
) -> gpui::Rgba {
    collapsed_hunk_header_bg(theme)
}

fn collapsed_inline_hunk_fg(theme: AppTheme, _hunk: Option<CollapsedDiffHunk>) -> gpui::Rgba {
    theme.colors.foreground.secondary
}

fn collapsed_split_hunk_bg(
    theme: AppTheme,
    _hunk: Option<CollapsedDiffHunk>,
    _column: PatchSplitColumn,
) -> gpui::Rgba {
    collapsed_hunk_header_bg(theme)
}

fn collapsed_split_hunk_fg(theme: AppTheme, _column: PatchSplitColumn) -> gpui::Rgba {
    theme.colors.foreground.secondary
}

/// Hit box for a collapsed-hunk reveal chevron. An editor row, not a chrome
/// row: follows the editor line height, not the density, and caps out.
fn collapsed_hunk_reveal_button_size(theme: AppTheme, ui_scale_percent: u32) -> Pixels {
    const ROW_FRACTION: f32 = 0.9;
    const MAX_PX: f32 = 24.0;
    let row: f32 = theme.editor_row_height(ui_scale_percent).into();
    px(row * ROW_FRACTION).min(crate::ui_scale::design_px_from_percent(
        MAX_PX,
        ui_scale_percent,
    ))
}

fn collapsed_hunk_reveal_button(
    id: impl Into<gpui::ElementId>,
    debug_selector: &'static str,
    theme: AppTheme,
    enabled: bool,
    icon: &'static str,
    tooltip: &'static str,
    icon_color: gpui::Rgba,
    click: CollapsedHunkRevealClick,
    cx: &mut gpui::Context<DiffRenderer>,
) -> AnyElement {
    let ui_scale_percent = crate::ui_scale::current(cx).percent;
    let size = collapsed_hunk_reveal_button_size(theme, ui_scale_percent);
    let button = components::inline_icon_button(
        id,
        theme,
        size,
        icon,
        crate::ui_scale::design_px_from_percent(10.0, ui_scale_percent),
        icon_color,
        enabled,
    )
    .debug_selector(move || debug_selector.to_string())
    .on_activate(
        !enabled,
        controls::ControlActivation::Nested,
        cx.listener(move |this, _e: &ClickEvent, _w, cx| match click.action {
            CollapsedHunkRevealAction::Up => {
                this.collapsed_diff_reveal_hunk_up(click.src_ix, cx);
            }
            CollapsedHunkRevealAction::Down => {
                this.collapsed_diff_reveal_hunk_down(click.src_ix, cx);
            }
            CollapsedHunkRevealAction::DownBefore => {
                this.collapsed_diff_reveal_hunk_down_before(click.src_ix, cx);
            }
            CollapsedHunkRevealAction::Short => {
                this.collapsed_diff_reveal_hunk_short(click.src_ix, cx);
            }
        }),
    );

    button
        .gitcomet_tooltip(theme, tooltip.into())
        .into_any_element()
}

struct ScrollPinnedHunkShell {
    child: AnyElement,
    scroll_handle: gpui::UniformListScrollHandle,
    background: Option<gpui::Rgba>,
}

impl gpui::IntoElement for ScrollPinnedHunkShell {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for ScrollPinnedHunkShell {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        let scroll_x = -self.scroll_handle.0.borrow().base_handle.offset().x;
        self.child.prepaint_at(
            gpui::point(bounds.origin.x + scroll_x, bounds.origin.y),
            window,
            cx,
        );
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint_state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        if let Some(background) = self.background {
            window.paint_quad(gpui::fill(
                collapsed_hunk_bg_fill_bounds(bounds),
                background,
            ));
        }
        self.child.paint(window, cx);
    }
}

/// Which diff palette a line's word highlights come from.
pub(super) fn diff_line_word_kind(kind: DiffLineKind) -> Option<crate::theme::DiffColorKind> {
    match kind {
        DiffLineKind::Add => Some(crate::theme::DiffColorKind::Added),
        DiffLineKind::Remove => Some(crate::theme::DiffColorKind::Removed),
        _ => None,
    }
}

pub(super) fn diff_placeholder_row(
    id: impl Into<gpui::ElementId>,
    theme: AppTheme,
    ui_scale_percent: u32,
) -> AnyElement {
    div()
        .id(id)
        .h(theme.editor_row_height(ui_scale_percent))
        .px_2()
        .text_size(theme.editor_font_size(ui_scale_percent))
        .text_color(theme.colors.foreground.secondary)
        .child("")
        .into_any_element()
}

pub(super) struct DiffRowStyle {
    pub(super) tab_width: usize,
    pub(super) theme: AppTheme,
    pub(super) ui_scale_percent: u32,
    pub(super) mode: DiffViewMode,
    pub(super) min_width: Pixels,
    pub(super) reveal_whitespace_chars: bool,
    pub(super) show_line_numbers: bool,
    pub(super) annotation_width: Pixels,
}

pub(super) struct DiffRowDecor<'a> {
    pub(super) tint: Option<gpui::Rgba>,
    pub(super) selected: bool,
    pub(super) visual_kind: DiffLineKind,
    pub(super) file_stat: Option<(usize, usize)>,
    pub(super) header_display: Option<SharedString>,
    pub(super) styled: Option<&'a CachedDiffStyledText>,
    pub(super) streamed_spec: Option<diff_canvas::StreamedDiffTextPaintSpec>,
    pub(super) raw_text: Option<diff_canvas::DiffRowText<'a>>,
    pub(super) context_menu_active: bool,
    pub(super) wrap: Option<diff_canvas::DiffTextWrapSlice>,
    pub(super) row_blame: Option<diff_canvas::RowBlamePaint>,
    pub(super) annot_hover: Option<(usize, AnnotArea)>,
    pub(super) stage_area: Option<DiffArea>,
    pub(super) stage_hover: Option<diff_canvas::DiffStageHover>,
}

pub(super) fn diff_row(
    style: DiffRowStyle,
    visible_ix: usize,
    click_kind: DiffClickKind,
    line: &AnnotatedDiffLine,
    decor: DiffRowDecor<'_>,
    cx: &mut gpui::Context<DiffRenderer>,
) -> AnyElement {
    let DiffRowStyle {
        tab_width,
        theme,
        ui_scale_percent,
        mode,
        min_width,
        reveal_whitespace_chars,
        show_line_numbers,
        annotation_width,
    } = style;
    let DiffRowDecor {
        tint,
        selected,
        visual_kind,
        file_stat,
        header_display,
        styled,
        streamed_spec,
        raw_text,
        context_menu_active,
        wrap,
        row_blame,
        annot_hover,
        stage_area,
        stage_hover,
    } = decor;

    let on_click = cx.listener(move |this, e: &ClickEvent, _w, cx| {
        if this.consume_suppress_click_after_drag() {
            cx.notify();
            return;
        }
        this.handle_patch_row_click(visible_ix, click_kind, e.modifiers().shift);
        cx.notify();
    });

    if matches!(click_kind, DiffClickKind::FileHeader) {
        let file =
            header_display.unwrap_or_else(|| SharedString::from(line.text.as_ref().to_owned()));
        let mut row = div()
            .id(("diff_file_hdr", visible_ix))
            .h(diff_file_header_height(theme, ui_scale_percent))
            .w_full()
            .min_w(min_width)
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .bg(crate::theme::content_header_bg(theme))
            .border_b_1()
            .border_color(theme.colors.stroke.default)
            .text_size(theme.ui_text(14.0))
            .font_weight(FontWeight::BOLD)
            .child(selectable_cached_diff_text(
                visible_ix,
                DiffTextRegion::Inline,
                DiffClickKind::FileHeader,
                theme.colors.foreground.primary,
                None,
                file,
                cx,
            ))
            .when(file_stat.is_some_and(|(a, r)| a > 0 || r > 0), |this| {
                let (a, r) = file_stat.unwrap_or_default();
                this.child(components::diff_stat(theme, ui_scale_percent, a, r))
            })
            .on_activate(false, controls::ControlActivation::Composite, on_click);

        if selected {
            row = row.bg(focused_diff_neutral_row_bg(theme));
        }

        return row.into_any_element();
    }

    if matches!(click_kind, DiffClickKind::HunkHeader) {
        let display =
            header_display.unwrap_or_else(|| SharedString::from(line.text.as_ref().to_owned()));

        let mut row = div()
            .id(("diff_hunk_hdr", visible_ix))
            .h(diff_hunk_header_height(theme, ui_scale_percent))
            .w_full()
            .min_w(min_width)
            .flex()
            .items_center()
            .px_2()
            .bg(with_alpha(
                theme.colors.accent.foreground,
                if theme.is_dark { 0.10 } else { 0.07 },
            ))
            .border_b_1()
            .border_color(with_alpha(
                theme.colors.accent.foreground,
                if theme.is_dark { 0.28 } else { 0.22 },
            ))
            .text_size(theme.editor_font_size(ui_scale_percent))
            .text_color(theme.colors.foreground.secondary)
            .child(selectable_cached_diff_text(
                visible_ix,
                DiffTextRegion::Inline,
                DiffClickKind::HunkHeader,
                theme.colors.foreground.secondary,
                None,
                display,
                cx,
            ))
            .on_activate(false, controls::ControlActivation::Composite, on_click);
        let on_right_click = cx.listener(move |this, e: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            if this.is_inline_submodule_diff_active() || this.has_large_file_text_diff() {
                return;
            }
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let Some(src_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return;
            };
            let context_menu_invoker: SharedString =
                format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();

            this.open_popover_at(
                (PopoverKind::DiffHunkMenu { repo_id, src_ix }).invoked_by(context_menu_invoker),
                e.position,
                window,
                cx,
            );
        });
        row = row.on_pointer_click(MouseButton::Right, on_right_click);

        if selected {
            row = row.bg(focused_diff_neutral_row_bg(theme));
        }
        if context_menu_active {
            row = row.bg(theme.colors.interaction.pressed_background);
        }

        return row.into_any_element();
    }

    let (mut bg, fg, gutter_fg) = diff_line_colors(theme, visual_kind);
    bg = tint.unwrap_or(bg);
    if selected {
        bg = focused_diff_line_bg(theme, visual_kind);
    }

    let show_row_numbers = wrap.is_none_or(|wrap| wrap.wrap_ix == 0);
    // Continuation rows of a wrapped line share the line's gutter, so only the
    // first visual row carries the stage button.
    let stage_area = stage_area.filter(|_| show_row_numbers);
    let old = if show_row_numbers {
        line_number_string(line.old_line)
    } else {
        SharedString::default()
    };
    let new = if show_row_numbers {
        line_number_string(line.new_line)
    } else {
        SharedString::default()
    };

    match mode {
        DiffViewMode::Inline => {
            // `visual_kind`, not `line.kind`: in ignore-whitespace mode a
            // whitespace-only change renders as context, and a row painted as
            // context must not offer to stage itself. The split columns derive
            // their button from the same value.
            let stage = stage_area
                .and_then(|area| stage_gutter_spec(area, DiffStageSlot::Inline, visual_kind));
            diff_canvas::inline_diff_line_row_canvas(
                tab_width,
                theme,
                cx.entity(),
                ui_scale_percent,
                visible_ix,
                min_width,
                selected,
                old,
                new,
                bg,
                fg,
                gutter_fg,
                styled,
                streamed_spec,
                raw_text,
                reveal_whitespace_chars,
                show_line_numbers,
                wrap,
                annotation_width,
                row_blame,
                annot_hover,
                stage,
                stage_hover,
            )
        }
        DiffViewMode::Split => {
            let left_kind = if visual_kind == DiffLineKind::Remove {
                DiffLineKind::Remove
            } else {
                DiffLineKind::Context
            };
            let right_kind = if visual_kind == DiffLineKind::Add {
                DiffLineKind::Add
            } else {
                DiffLineKind::Context
            };

            let (mut left_bg, left_fg, left_gutter) = diff_line_colors(theme, left_kind);
            let (mut right_bg, right_fg, right_gutter) = diff_line_colors(theme, right_kind);
            if selected {
                left_bg = focused_diff_line_bg(theme, left_kind);
                right_bg = focused_diff_line_bg(theme, right_kind);
            }

            let (left_text, right_text) = match line.kind {
                DiffLineKind::Remove => (styled, None),
                DiffLineKind::Add => (None, styled),
                DiffLineKind::Context => (styled, styled),
                _ => (styled, None),
            };
            let left_streamed_spec = match line.kind {
                DiffLineKind::Remove | DiffLineKind::Context => streamed_spec.clone(),
                _ => None,
            };
            let right_streamed_spec = match line.kind {
                DiffLineKind::Add | DiffLineKind::Context => streamed_spec,
                _ => None,
            };
            let left_raw_text = match line.kind {
                DiffLineKind::Remove | DiffLineKind::Context => raw_text,
                _ => None,
            };
            let right_raw_text = match line.kind {
                DiffLineKind::Add | DiffLineKind::Context => raw_text,
                _ => None,
            };

            // A split row shows removals on the left and additions on the right,
            // so each side gets the button for its own kind of change.
            let stage_left = stage_area
                .and_then(|area| stage_gutter_spec(area, DiffStageSlot::SplitLeft, visual_kind))
                .filter(|spec| spec.kind == DiffLineKind::Remove);
            let stage_right = stage_area
                .and_then(|area| stage_gutter_spec(area, DiffStageSlot::SplitRight, visual_kind))
                .filter(|spec| spec.kind == DiffLineKind::Add);

            diff_canvas::split_diff_line_row_canvas(
                tab_width,
                theme,
                cx.entity(),
                ui_scale_percent,
                visible_ix,
                min_width,
                selected,
                old,
                new,
                left_bg,
                left_fg,
                left_gutter,
                right_bg,
                right_fg,
                right_gutter,
                left_text,
                right_text,
                left_streamed_spec,
                right_streamed_spec,
                left_raw_text,
                right_raw_text,
                reveal_whitespace_chars,
                show_line_numbers,
                wrap,
                annotation_width,
                row_blame,
                annot_hover,
                stage_left,
                stage_right,
                stage_hover,
            )
        }
    }
}

/// Build the stage-gutter spec for a change line, or `None` for anything that
/// cannot be staged line-by-line (context lines and headers).
fn stage_gutter_spec(
    area: DiffArea,
    slot: DiffStageSlot,
    kind: DiffLineKind,
) -> Option<diff_canvas::StageGutterSpec> {
    matches!(kind, DiffLineKind::Add | DiffLineKind::Remove)
        .then_some(diff_canvas::StageGutterSpec { area, slot, kind })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn collapsed_inline_header_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    visible_ix: usize,
    click_kind: DiffClickKind,
    selected: bool,
    min_width: Pixels,
    pinned_hunk_shell_width: Pixels,
    pinned_hunk_shell_scroll: gpui::UniformListScrollHandle,
    collapsed_hunk: Option<CollapsedDiffHunk>,
    file_stat: Option<(usize, usize)>,
    display: SharedString,
    styled: Option<&CachedDiffStyledText>,
    context_menu_active: bool,
    src_ix: usize,
    expansion_kind: CollapsedDiffExpansionKind,
    hidden_rows: usize,
    cx: &mut gpui::Context<DiffRenderer>,
) -> AnyElement {
    match click_kind {
        DiffClickKind::FileHeader => {
            let header_bg = if selected {
                focused_diff_neutral_row_bg(theme)
            } else {
                crate::theme::content_header_bg(theme)
            };
            // Pin the header content to the viewport while the background
            // band spans the full scrollable width, so horizontal scrolling
            // moves neither the band nor the file name.
            let inner = div()
                .id(("collapsed_diff_file_hdr", visible_ix))
                .h(diff_file_header_height(theme, ui_scale_percent))
                .w(pinned_hunk_shell_width)
                .min_w(px(0.0))
                .relative()
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_between()
                .px_2()
                .text_size(theme.ui_text(14.0))
                .font_weight(FontWeight::BOLD)
                .child(selectable_cached_diff_text(
                    visible_ix,
                    DiffTextRegion::Inline,
                    DiffClickKind::FileHeader,
                    theme.colors.foreground.primary,
                    styled,
                    display,
                    cx,
                ))
                .when(file_stat.is_some_and(|(a, r)| a > 0 || r > 0), |this| {
                    let (a, r) = file_stat.unwrap_or_default();
                    this.child(components::diff_stat(theme, ui_scale_percent, a, r))
                });

            div()
                .h(diff_file_header_height(theme, ui_scale_percent))
                .w_full()
                .min_w(min_width)
                .bg(header_bg)
                .border_b_1()
                .border_color(theme.colors.stroke.subtle)
                .child(scroll_pinned_hunk_shell(
                    pinned_hunk_shell_scroll,
                    None,
                    inner.into_any_element(),
                ))
                .into_any_element()
        }
        DiffClickKind::HunkHeader => {
            let gutter_w = diff_canvas::diff_inline_text_start(
                ui_scale::UiScale::from_percent(ui_scale_percent).with_appearance(theme.metrics),
            );
            let trailing_pad = diff_canvas::diff_row_horizontal_padding(ui_scale_percent);
            let text_color = collapsed_inline_hunk_fg(theme, collapsed_hunk);
            let on_right_click = cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if this.is_inline_submodule_diff_active() || this.has_large_file_text_diff() {
                    return;
                }
                let Some(repo_id) = this.active_repo_id() else {
                    return;
                };
                let context_menu_invoker: SharedString =
                    format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();

                this.open_popover_at(
                    (PopoverKind::DiffHunkMenu { repo_id, src_ix })
                        .invoked_by(context_menu_invoker),
                    e.position,
                    window,
                    cx,
                );
            });
            let button_color = if hidden_rows > 0 {
                text_color
            } else {
                with_alpha(text_color, 0.45)
            };
            let controls = match expansion_kind {
                CollapsedDiffExpansionKind::Up => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        ("collapsed_diff_hunk_up", visible_ix),
                        COLLAPSED_DIFF_INLINE_HUNK_UP_DEBUG_SELECTOR,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_up.svg",
                        "Show hidden lines above",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Up,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::Down => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        ("collapsed_diff_hunk_down", visible_ix),
                        COLLAPSED_DIFF_INLINE_HUNK_DOWN_DEBUG_SELECTOR,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_down.svg",
                        "Show hidden lines below",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Down,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::Both => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        ("collapsed_diff_hunk_down", visible_ix),
                        COLLAPSED_DIFF_INLINE_HUNK_DOWN_DEBUG_SELECTOR,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_down.svg",
                        "Show hidden lines below",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::DownBefore,
                            src_ix,
                        },
                        cx,
                    ))
                    .child(collapsed_hunk_reveal_button(
                        ("collapsed_diff_hunk_up", visible_ix),
                        COLLAPSED_DIFF_INLINE_HUNK_UP_DEBUG_SELECTOR,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_up.svg",
                        "Show hidden lines above",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Up,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::Short => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        ("collapsed_diff_hunk_short", visible_ix),
                        COLLAPSED_DIFF_INLINE_HUNK_SHORT_DEBUG_SELECTOR,
                        theme,
                        hidden_rows > 0,
                        "icons/plus.svg",
                        "Show hidden lines",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Short,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::None => div().into_any_element(),
            };

            let row_bg = collapsed_inline_hunk_bg(theme, collapsed_hunk, expansion_kind);
            let painted_row_bg = if selected {
                focused_collapsed_hunk_bg(theme, collapsed_hunk)
            } else {
                row_bg
            };
            let mut row = div()
                .id(("collapsed_diff_hunk_hdr", visible_ix))
                .debug_selector(|| COLLAPSED_DIFF_INLINE_HUNK_SHELL_DEBUG_SELECTOR.to_string())
                .h(collapsed_hunk_header_row_height(theme, ui_scale_percent))
                .w(pinned_hunk_shell_width)
                .min_w(px(0.0))
                .relative()
                .overflow_hidden()
                .flex()
                .items_center()
                .bg(painted_row_bg)
                .text_size(theme.editor_font_size(ui_scale_percent))
                .text_color(text_color);
            row = row
                .child(
                    div()
                        .debug_selector(|| {
                            COLLAPSED_DIFF_INLINE_HUNK_GUTTER_DEBUG_SELECTOR.to_string()
                        })
                        .w(gutter_w)
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(controls),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .pr(trailing_pad)
                        .overflow_hidden()
                        .child(selectable_cached_diff_text(
                            visible_ix,
                            DiffTextRegion::Inline,
                            DiffClickKind::HunkHeader,
                            text_color,
                            styled,
                            display,
                            cx,
                        )),
                )
                .on_pointer_click(MouseButton::Right, on_right_click);

            if selected {
                row = row.bg(painted_row_bg);
            }
            if context_menu_active {
                row = row.bg(theme.colors.interaction.pressed_background);
            }

            div()
                .h(collapsed_hunk_header_row_height(theme, ui_scale_percent))
                .min_w(min_width)
                .bg(painted_row_bg)
                .child(scroll_pinned_hunk_shell(
                    pinned_hunk_shell_scroll,
                    Some(painted_row_bg),
                    row.into_any_element(),
                ))
                .into_any_element()
        }
        DiffClickKind::Line => diff_placeholder_row(
            ("collapsed_diff_invalid", visible_ix),
            theme,
            ui_scale_percent,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn patch_split_column_row(
    tab_width: usize,
    theme: AppTheme,
    ui_scale_percent: u32,
    column: PatchSplitColumn,
    visible_ix: usize,
    selected: bool,
    min_width: Pixels,
    row: &gitcomet_core::file_diff::FileDiffRow,
    visual_kind: FileDiffRowKind,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<diff_canvas::StreamedDiffTextPaintSpec>,
    reveal_whitespace_chars: bool,
    show_line_numbers: bool,
    wrap: Option<diff_canvas::DiffTextWrapSlice>,
    annotation_width: Pixels,
    row_blame: Option<diff_canvas::RowBlamePaint>,
    annot_hover: Option<(usize, AnnotArea)>,
    stage_area: Option<DiffArea>,
    stage_hover: Option<diff_canvas::DiffStageHover>,
    tint: Option<gpui::Rgba>,
    cx: &mut gpui::Context<DiffRenderer>,
) -> AnyElement {
    let line_kind = match (column, visual_kind) {
        (PatchSplitColumn::Left, FileDiffRowKind::Remove | FileDiffRowKind::Modify) => {
            DiffLineKind::Remove
        }
        (PatchSplitColumn::Right, FileDiffRowKind::Add | FileDiffRowKind::Modify) => {
            DiffLineKind::Add
        }
        _ => DiffLineKind::Context,
    };
    let (mut bg, fg, gutter_fg) = diff_line_colors(theme, line_kind);
    bg = tint.unwrap_or(bg);
    if selected {
        bg = focused_diff_line_bg(theme, line_kind);
    }

    let show_row_number = wrap.is_none_or(|wrap| wrap.wrap_ix == 0);
    let line_no = if show_row_number {
        match column {
            PatchSplitColumn::Left => line_number_string(row.old_line),
            PatchSplitColumn::Right => line_number_string(row.new_line),
        }
    } else {
        SharedString::default()
    };

    // `line_kind` is already resolved per column, so each side offers the button
    // only for the change it actually shows.
    let stage = stage_area.filter(|_| show_row_number).and_then(|area| {
        stage_gutter_spec(
            area,
            match column {
                PatchSplitColumn::Left => DiffStageSlot::SplitLeft,
                PatchSplitColumn::Right => DiffStageSlot::SplitRight,
            },
            line_kind,
        )
    });

    diff_canvas::patch_split_column_row_canvas(
        tab_width,
        theme,
        cx.entity(),
        ui_scale_percent,
        column,
        visible_ix,
        min_width,
        selected,
        bg,
        fg,
        gutter_fg,
        line_no,
        styled,
        streamed_spec,
        match column {
            PatchSplitColumn::Left => row.old.as_ref(),
            PatchSplitColumn::Right => row.new.as_ref(),
        }
        .map(diff_canvas::DiffRowText::Line),
        reveal_whitespace_chars,
        show_line_numbers,
        wrap,
        annotation_width,
        row_blame,
        annot_hover,
        stage,
        stage_hover,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn patch_split_header_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    column: PatchSplitColumn,
    visible_ix: usize,
    click_kind: DiffClickKind,
    selected: bool,
    min_width: Pixels,
    line: &AnnotatedDiffLine,
    file_stat: Option<(usize, usize)>,
    header_display: Option<SharedString>,
    styled: Option<&CachedDiffStyledText>,
    context_menu_active: bool,
    cx: &mut gpui::Context<DiffRenderer>,
) -> AnyElement {
    let on_click = cx.listener(move |this, e: &ClickEvent, _w, cx| {
        if this.consume_suppress_click_after_drag() {
            cx.notify();
            return;
        }
        this.handle_patch_row_click(visible_ix, click_kind, e.modifiers().shift);
        cx.notify();
    });
    let region = match column {
        PatchSplitColumn::Left => DiffTextRegion::SplitLeft,
        PatchSplitColumn::Right => DiffTextRegion::SplitRight,
    };

    match click_kind {
        DiffClickKind::FileHeader => {
            let display =
                header_display.unwrap_or_else(|| SharedString::from(line.text.as_ref().to_owned()));
            let mut row = div()
                .id((
                    match column {
                        PatchSplitColumn::Left => "diff_split_left_file_hdr",
                        PatchSplitColumn::Right => "diff_split_right_file_hdr",
                    },
                    visible_ix,
                ))
                .h(diff_file_header_height(theme, ui_scale_percent))
                .w_full()
                .min_w(min_width)
                .flex()
                .items_center()
                .justify_between()
                .px_2()
                .bg(crate::theme::content_header_bg(theme))
                .border_b_1()
                .border_color(theme.colors.stroke.default)
                .text_size(theme.ui_text(14.0))
                .font_weight(FontWeight::BOLD)
                .child(selectable_cached_diff_text(
                    visible_ix,
                    region,
                    DiffClickKind::FileHeader,
                    theme.colors.foreground.primary,
                    styled,
                    display,
                    cx,
                ))
                .when(file_stat.is_some_and(|(a, r)| a > 0 || r > 0), |this| {
                    let (a, r) = file_stat.unwrap_or_default();
                    this.child(components::diff_stat(theme, ui_scale_percent, a, r))
                })
                .on_activate(false, controls::ControlActivation::Composite, on_click);

            if selected {
                row = row.bg(focused_diff_neutral_row_bg(theme));
            }

            row.into_any_element()
        }
        DiffClickKind::HunkHeader => {
            let display =
                header_display.unwrap_or_else(|| SharedString::from(line.text.as_ref().to_owned()));

            let mut row = div()
                .id((
                    match column {
                        PatchSplitColumn::Left => "diff_split_left_hunk_hdr",
                        PatchSplitColumn::Right => "diff_split_right_hunk_hdr",
                    },
                    visible_ix,
                ))
                .h(diff_hunk_header_height(theme, ui_scale_percent))
                .w_full()
                .min_w(min_width)
                .flex()
                .items_center()
                .px_2()
                .bg(with_alpha(
                    theme.colors.accent.foreground,
                    if theme.is_dark { 0.10 } else { 0.07 },
                ))
                .border_b_1()
                .border_color(with_alpha(
                    theme.colors.accent.foreground,
                    if theme.is_dark { 0.28 } else { 0.22 },
                ))
                .text_size(theme.editor_font_size(ui_scale_percent))
                .text_color(theme.colors.foreground.secondary)
                .child(selectable_cached_diff_text(
                    visible_ix,
                    region,
                    DiffClickKind::HunkHeader,
                    theme.colors.foreground.secondary,
                    styled,
                    display,
                    cx,
                ))
                .on_activate(false, controls::ControlActivation::Composite, on_click);
            let on_right_click = cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if this.is_inline_submodule_diff_active() || this.has_large_file_text_diff() {
                    return;
                }
                let Some(repo_id) = this.active_repo_id() else {
                    return;
                };
                let Some(row_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                    return;
                };
                let Some(PatchSplitRow::Raw {
                    src_ix,
                    click_kind: DiffClickKind::HunkHeader,
                }) = this.patch_diff_split_row(row_ix)
                else {
                    return;
                };
                let context_menu_invoker: SharedString =
                    format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();

                this.open_popover_at(
                    (PopoverKind::DiffHunkMenu { repo_id, src_ix })
                        .invoked_by(context_menu_invoker),
                    e.position,
                    window,
                    cx,
                );
            });
            row = row.on_pointer_click(MouseButton::Right, on_right_click);

            if selected {
                row = row.bg(focused_diff_neutral_row_bg(theme));
            }
            if context_menu_active {
                row = row.bg(theme.colors.interaction.pressed_background);
            }

            row.into_any_element()
        }
        DiffClickKind::Line => patch_split_meta_row(
            theme,
            ui_scale_percent,
            column,
            visible_ix,
            selected,
            line,
            cx,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn collapsed_split_header_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    column: PatchSplitColumn,
    visible_ix: usize,
    click_kind: DiffClickKind,
    selected: bool,
    min_width: Pixels,
    pinned_hunk_shell_width: Pixels,
    pinned_hunk_shell_scroll: gpui::UniformListScrollHandle,
    collapsed_hunk: Option<CollapsedDiffHunk>,
    file_stat: Option<(usize, usize)>,
    display: SharedString,
    styled: Option<&CachedDiffStyledText>,
    context_menu_active: bool,
    src_ix: usize,
    expansion_kind: CollapsedDiffExpansionKind,
    hidden_rows: usize,
    cx: &mut gpui::Context<DiffRenderer>,
) -> AnyElement {
    let region = match column {
        PatchSplitColumn::Left => DiffTextRegion::SplitLeft,
        PatchSplitColumn::Right => DiffTextRegion::SplitRight,
    };

    match click_kind {
        DiffClickKind::FileHeader => {
            let header_bg = if selected {
                focused_diff_neutral_row_bg(theme)
            } else {
                crate::theme::content_header_bg(theme)
            };
            // Pin the header content to the viewport while the background
            // band spans the full scrollable width, so horizontal scrolling
            // moves neither the band nor the file name.
            let inner = div()
                .id((
                    match column {
                        PatchSplitColumn::Left => "collapsed_diff_split_left_file_hdr",
                        PatchSplitColumn::Right => "collapsed_diff_split_right_file_hdr",
                    },
                    visible_ix,
                ))
                .h(diff_file_header_height(theme, ui_scale_percent))
                .w(pinned_hunk_shell_width)
                .min_w(px(0.0))
                .relative()
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_between()
                .px_2()
                .text_size(theme.ui_text(14.0))
                .font_weight(FontWeight::BOLD)
                .child(selectable_cached_diff_text(
                    visible_ix,
                    region,
                    DiffClickKind::FileHeader,
                    theme.colors.foreground.primary,
                    styled,
                    display,
                    cx,
                ))
                .when(file_stat.is_some_and(|(a, r)| a > 0 || r > 0), |this| {
                    let (a, r) = file_stat.unwrap_or_default();
                    this.child(components::diff_stat(theme, ui_scale_percent, a, r))
                });

            div()
                .h(diff_file_header_height(theme, ui_scale_percent))
                .w_full()
                .min_w(min_width)
                .bg(header_bg)
                .border_b_1()
                .border_color(theme.colors.stroke.subtle)
                .child(scroll_pinned_hunk_shell(
                    pinned_hunk_shell_scroll,
                    None,
                    inner.into_any_element(),
                ))
                .into_any_element()
        }
        DiffClickKind::HunkHeader => {
            let gutter_w = diff_canvas::diff_single_column_text_start(
                ui_scale::UiScale::from_percent(ui_scale_percent).with_appearance(theme.metrics),
            );
            let trailing_pad = diff_canvas::diff_row_horizontal_padding(ui_scale_percent);
            let text_color = collapsed_split_hunk_fg(theme, column);
            let (
                row_id,
                shell_debug_selector,
                gutter_debug_selector,
                up_id,
                up_debug_selector,
                down_id,
                down_debug_selector,
                short_id,
                short_debug_selector,
            ) = match column {
                PatchSplitColumn::Left => (
                    "collapsed_diff_split_left_hunk_hdr",
                    COLLAPSED_DIFF_SPLIT_LEFT_HUNK_SHELL_DEBUG_SELECTOR,
                    COLLAPSED_DIFF_SPLIT_LEFT_HUNK_GUTTER_DEBUG_SELECTOR,
                    "collapsed_diff_split_left_hunk_up",
                    COLLAPSED_DIFF_SPLIT_LEFT_HUNK_UP_DEBUG_SELECTOR,
                    "collapsed_diff_split_left_hunk_down",
                    COLLAPSED_DIFF_SPLIT_LEFT_HUNK_DOWN_DEBUG_SELECTOR,
                    "collapsed_diff_split_left_hunk_short",
                    COLLAPSED_DIFF_SPLIT_LEFT_HUNK_SHORT_DEBUG_SELECTOR,
                ),
                PatchSplitColumn::Right => (
                    "collapsed_diff_split_right_hunk_hdr",
                    COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_SHELL_DEBUG_SELECTOR,
                    COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_GUTTER_DEBUG_SELECTOR,
                    "collapsed_diff_split_right_hunk_up",
                    COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_UP_DEBUG_SELECTOR,
                    "collapsed_diff_split_right_hunk_down",
                    COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_DOWN_DEBUG_SELECTOR,
                    "collapsed_diff_split_right_hunk_short",
                    COLLAPSED_DIFF_SPLIT_RIGHT_HUNK_SHORT_DEBUG_SELECTOR,
                ),
            };
            let on_right_click = cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if this.is_inline_submodule_diff_active() || this.has_large_file_text_diff() {
                    return;
                }
                let Some(repo_id) = this.active_repo_id() else {
                    return;
                };
                let context_menu_invoker: SharedString =
                    format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();

                this.open_popover_at(
                    (PopoverKind::DiffHunkMenu { repo_id, src_ix })
                        .invoked_by(context_menu_invoker),
                    e.position,
                    window,
                    cx,
                );
            });
            let button_color = if hidden_rows > 0 {
                text_color
            } else {
                with_alpha(text_color, 0.45)
            };
            let controls = match expansion_kind {
                CollapsedDiffExpansionKind::Up => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        (up_id, visible_ix),
                        up_debug_selector,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_up.svg",
                        "Show hidden lines above",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Up,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::Down => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        (down_id, visible_ix),
                        down_debug_selector,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_down.svg",
                        "Show hidden lines below",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Down,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::Both => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        (down_id, visible_ix),
                        down_debug_selector,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_down.svg",
                        "Show hidden lines below",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::DownBefore,
                            src_ix,
                        },
                        cx,
                    ))
                    .child(collapsed_hunk_reveal_button(
                        (up_id, visible_ix),
                        up_debug_selector,
                        theme,
                        hidden_rows > 0,
                        "icons/arrow_up.svg",
                        "Show hidden lines above",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Up,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::Short => div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(collapsed_hunk_reveal_button(
                        (short_id, visible_ix),
                        short_debug_selector,
                        theme,
                        hidden_rows > 0,
                        "icons/plus.svg",
                        "Show hidden lines",
                        button_color,
                        CollapsedHunkRevealClick {
                            action: CollapsedHunkRevealAction::Short,
                            src_ix,
                        },
                        cx,
                    ))
                    .into_any_element(),
                CollapsedDiffExpansionKind::None => div().into_any_element(),
            };

            let row_bg = collapsed_split_hunk_bg(theme, collapsed_hunk, column);
            let painted_row_bg = if selected {
                focused_collapsed_hunk_bg(theme, collapsed_hunk)
            } else {
                row_bg
            };
            let mut row = div()
                .id((row_id, visible_ix))
                .debug_selector(move || shell_debug_selector.to_string())
                .h(collapsed_hunk_header_row_height(theme, ui_scale_percent))
                .w(pinned_hunk_shell_width)
                .min_w(px(0.0))
                .relative()
                .overflow_hidden()
                .flex()
                .items_center()
                .bg(painted_row_bg)
                .text_size(theme.editor_font_size(ui_scale_percent))
                .text_color(text_color)
                .child(
                    div()
                        .debug_selector(move || gutter_debug_selector.to_string())
                        .w(gutter_w)
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(controls),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .pr(trailing_pad)
                        .overflow_hidden()
                        .child(selectable_cached_diff_text(
                            visible_ix,
                            region,
                            DiffClickKind::HunkHeader,
                            text_color,
                            styled,
                            display,
                            cx,
                        )),
                )
                .on_pointer_click(MouseButton::Right, on_right_click);

            if selected {
                row = row.bg(painted_row_bg);
            }
            if context_menu_active {
                row = row.bg(theme.colors.interaction.pressed_background);
            }

            div()
                .h(collapsed_hunk_header_row_height(theme, ui_scale_percent))
                .min_w(min_width)
                .bg(painted_row_bg)
                .child(scroll_pinned_hunk_shell(
                    pinned_hunk_shell_scroll,
                    Some(painted_row_bg),
                    row.into_any_element(),
                ))
                .into_any_element()
        }
        DiffClickKind::Line => diff_placeholder_row(
            (
                match column {
                    PatchSplitColumn::Left => "collapsed_diff_split_left_invalid",
                    PatchSplitColumn::Right => "collapsed_diff_split_right_invalid",
                },
                visible_ix,
            ),
            theme,
            ui_scale_percent,
        ),
    }
}

fn patch_split_meta_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    column: PatchSplitColumn,
    visible_ix: usize,
    selected: bool,
    line: &AnnotatedDiffLine,
    cx: &mut gpui::Context<DiffRenderer>,
) -> AnyElement {
    let on_click = cx.listener(move |this, e: &ClickEvent, _w, cx| {
        if this.consume_suppress_click_after_drag() {
            cx.notify();
            return;
        }
        this.handle_patch_row_click(visible_ix, DiffClickKind::Line, e.modifiers().shift);
        cx.notify();
    });
    let region = match column {
        PatchSplitColumn::Left => DiffTextRegion::SplitLeft,
        PatchSplitColumn::Right => DiffTextRegion::SplitRight,
    };

    let (bg, fg, _) = diff_line_colors(theme, line.kind);
    let mut row = div()
        .id((
            match column {
                PatchSplitColumn::Left => "diff_split_left_meta",
                PatchSplitColumn::Right => "diff_split_right_meta",
            },
            visible_ix,
        ))
        .h(theme.editor_row_height(ui_scale_percent))
        .flex()
        .items_center()
        .px_2()
        .text_size(theme.editor_font_size(ui_scale_percent))
        .bg(bg)
        .text_color(fg)
        .whitespace_nowrap()
        .child(selectable_cached_diff_text(
            visible_ix,
            region,
            DiffClickKind::Line,
            fg,
            None,
            SharedString::from(line.text.as_ref().to_owned()),
            cx,
        ))
        .on_activate(false, controls::ControlActivation::Composite, on_click);

    if selected {
        row = row.bg(focused_diff_line_bg(theme, line.kind));
    }

    row.into_any_element()
}

#[cfg(test)]
mod tests;

pub(super) fn render_patch_diff_rows(
    this: &mut DiffRenderer,
    range: Range<usize>,
    pass: DiffRowsRenderPass,
    cx: &mut gpui::Context<DiffRenderer>,
) -> Vec<AnyElement> {
    let DiffRowsRenderPass {
        tab_width,
        annot_hover,
        stage_area,
        stage_hover,
        min_width,
        query,
        query_options,
        query_matcher,
        reveal_whitespace_chars,
        ui_scale_percent,
        annotation_width,
        blame_ctx,
    } = pass;

    let theme = this.theme;
    let cache_epoch = 0u64;
    let repo_id_for_context_menu = this.active_repo_id();
    let active_context_menu_invoker = this.active_context_menu_invoker.clone();
    let syntax_mode = this.patch_diff_syntax_mode();
    let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
    range
        .map(|visible_ix| {
            let selected = this
                .diff_selection_range
                .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
            let show_line_numbers = this.diff_show_line_numbers;
            let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);

            let Some(src_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return diff_placeholder_row(("diff_missing", visible_ix), theme, ui_scale_percent);
            };
            let click_kind = this
                .diff_click_kinds
                .get(src_ix)
                .copied()
                .unwrap_or(DiffClickKind::Line);

            this.ensure_patch_diff_word_highlight_for_src_ix(src_ix);
            let word_ranges: &[Range<usize>] = this
                .diff_word_highlights
                .get(src_ix)
                .and_then(|r| r.as_ref().map(Vec::as_slice))
                .unwrap_or(&[]);

            let file_stat = this.diff_file_stats.get(src_ix).and_then(|s| *s);

            let language = this.diff_language_for_src_ix.get(src_ix).copied().flatten();
            let Some(line) = this.patch_diff_row(src_ix) else {
                return diff_placeholder_row(("diff_oob", visible_ix), theme, ui_scale_percent);
            };
            let visual_kind = this.patch_visual_line_kind(src_ix);
            let streamed_spec = matches!(click_kind, DiffClickKind::Line)
                .then(|| {
                    heuristic_streamed_diff_text_spec(
                        crate::view::diff_utils::diff_content_line_text(&line),
                        &query,
                        query_options,
                        query_matcher.clone(),
                        Arc::from(word_ranges.to_vec()),
                        diff_line_word_kind(visual_kind),
                        language,
                        syntax_mode,
                    )
                })
                .flatten();

            let should_style = matches!(click_kind, DiffClickKind::Line) || !query.is_empty();
            if should_style
                && streamed_spec.is_none()
                && this
                    .diff_text_segments_cache_get(src_ix, cache_epoch)
                    .is_none()
            {
                let computed = if matches!(click_kind, DiffClickKind::Line) {
                    let word_kind = diff_line_word_kind(visual_kind);
                    let content_text = diff_content_text(&line);

                    build_cached_diff_styled_text_with_source_identity(
                        tab_width,
                        theme,
                        content_text,
                        Some(DiffTextSourceIdentity::from_str(content_text)),
                        word_ranges,
                        "",
                        language,
                        syntax_mode,
                        word_kind,
                    )
                } else {
                    let display =
                        this.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                    build_cached_diff_styled_text(
                        tab_width,
                        theme,
                        display.as_ref(),
                        &[] as &[Range<usize>],
                        "",
                        None,
                        syntax_mode,
                        None,
                    )
                };
                this.diff_text_segments_cache_set(src_ix, cache_epoch, computed);
            }

            let header_display = matches!(
                click_kind,
                DiffClickKind::FileHeader | DiffClickKind::HunkHeader
            )
            .then(|| this.diff_header_display_cache.get(&src_ix).cloned())
            .flatten();
            let context_menu_active = click_kind == DiffClickKind::HunkHeader
                && repo_id_for_context_menu.is_some_and(|repo_id| {
                    let invoker: SharedString =
                        format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                    active_context_menu_invoker.as_ref() == Some(&invoker)
                });
            let row_tint = this.hosted_row_tint(visible_ix, None);
            let styled = if should_style && streamed_spec.is_none() {
                this.diff_text_segments_cache_get_for_query(
                    src_ix,
                    query.as_ref(),
                    query_options,
                    cache_epoch,
                )
            } else {
                None
            };
            diff_row(
                DiffRowStyle {
                    tab_width,
                    theme,
                    ui_scale_percent,
                    mode: DiffViewMode::Inline,
                    min_width,
                    reveal_whitespace_chars,
                    show_line_numbers,
                    annotation_width,
                },
                visible_ix,
                click_kind,
                &line,
                DiffRowDecor {
                    tint: row_tint,
                    selected,
                    visual_kind,
                    file_stat,
                    header_display,
                    styled,
                    streamed_spec,
                    raw_text: Some(diff_canvas::DiffRowText::Str(
                        if matches!(click_kind, DiffClickKind::Line) {
                            diff_content_text(&line)
                        } else {
                            line.text.as_ref()
                        },
                    )),
                    context_menu_active,
                    wrap,
                    row_blame: blame_ctx.as_ref().and_then(|ctx| {
                        build_row_blame_paint_tracked(
                            ctx,
                            matches!(visual_kind, DiffLineKind::Context),
                            line.old_line,
                            line.new_line,
                            &blame_prev_nl,
                            wrap,
                            theme,
                        )
                    }),
                    annot_hover,
                    stage_area,
                    stage_hover,
                },
                cx,
            )
        })
        .collect()
}

pub(super) fn render_patch_diff_split_rows(
    this: &mut DiffRenderer,
    range: Range<usize>,
    pass: SplitDiffRowsRenderPass,
    cx: &mut gpui::Context<DiffRenderer>,
) -> Vec<AnyElement> {
    let SplitDiffRowsRenderPass {
        common,
        column,
        is_left,
        region,
        id_missing,
        id_oob,
        id_src_oob,
        id_hidden,
    } = pass;
    let DiffRowsRenderPass {
        tab_width,
        annot_hover,
        stage_area,
        stage_hover,
        min_width,
        query,
        query_options,
        query_matcher,
        reveal_whitespace_chars,
        ui_scale_percent,
        annotation_width,
        blame_ctx,
    } = common;

    let theme = this.theme;
    let cache_epoch = 0u64;
    let syntax_mode = this.patch_diff_syntax_mode();
    let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
    range
        .map(|visible_ix| {
            let selected = this
                .diff_selection_range
                .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
            let show_line_numbers = this.diff_show_line_numbers;
            let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);

            let Some(row_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return diff_placeholder_row((id_missing, visible_ix), theme, ui_scale_percent);
            };
            let Some(row) = this.patch_diff_split_row(row_ix) else {
                return diff_placeholder_row((id_oob, visible_ix), theme, ui_scale_percent);
            };

            match row {
                PatchSplitRow::Aligned {
                    row,
                    old_src_ix,
                    new_src_ix,
                } => {
                    let src_ix = if is_left { old_src_ix } else { new_src_ix };
                    let old_changed = old_src_ix.is_some_and(|src_ix| {
                        matches!(this.patch_visual_line_kind(src_ix), DiffLineKind::Remove)
                    });
                    let new_changed = new_src_ix.is_some_and(|src_ix| {
                        matches!(this.patch_visual_line_kind(src_ix), DiffLineKind::Add)
                    });
                    let visual_kind = match (old_changed, new_changed) {
                        (true, true) => FileDiffRowKind::Modify,
                        (true, false) => FileDiffRowKind::Remove,
                        (false, true) => FileDiffRowKind::Add,
                        (false, false) => FileDiffRowKind::Context,
                    };
                    let row_tint = this.hosted_row_tint(
                        visible_ix,
                        Some(if is_left {
                            gitcomet_extension_api::DiffLineSide::Old
                        } else {
                            gitcomet_extension_api::DiffLineSide::New
                        }),
                    );
                    let (streamed_spec, styled) = if let Some(src_ix) = src_ix {
                        let language = this.diff_language_for_src_ix.get(src_ix).copied().flatten();
                        this.ensure_patch_diff_word_highlight_for_src_ix(src_ix);
                        let word_ranges = this
                            .diff_word_highlights
                            .get(src_ix)
                            .and_then(|r| r.as_ref().cloned())
                            .unwrap_or_default();
                        let word_kind = diff_line_word_kind(this.patch_visual_line_kind(src_ix));
                        let streamed_spec = file_diff_split_side_text_owned(&row, is_left)
                            .and_then(|raw_text| {
                                heuristic_streamed_diff_text_spec(
                                    raw_text,
                                    &query,
                                    query_options,
                                    query_matcher.clone(),
                                    Arc::from(word_ranges.clone()),
                                    word_kind,
                                    language,
                                    syntax_mode,
                                )
                            });
                        if streamed_spec.is_none()
                            && this
                                .diff_text_segments_cache_get(src_ix, cache_epoch)
                                .is_none()
                        {
                            let computed =
                                if let Some(raw_text) = file_diff_split_side_text(&row, is_left) {
                                    build_file_diff_cached_styled_text(
                                        tab_width,
                                        theme,
                                        raw_text,
                                        word_ranges.as_slice(),
                                        "",
                                        language,
                                        syntax_mode,
                                        word_kind,
                                    )
                                } else {
                                    build_cached_diff_styled_text(
                                        tab_width,
                                        theme,
                                        "",
                                        word_ranges.as_slice(),
                                        "",
                                        language,
                                        syntax_mode,
                                        word_kind,
                                    )
                                };
                            this.diff_text_segments_cache_set(src_ix, cache_epoch, computed);
                        }

                        let styled = if streamed_spec.is_none() {
                            this.diff_text_segments_cache_get_for_query(
                                src_ix,
                                query.as_ref(),
                                query_options,
                                cache_epoch,
                            )
                        } else {
                            None
                        };
                        (streamed_spec, styled)
                    } else {
                        (None, None)
                    };

                    patch_split_column_row(
                        tab_width,
                        theme,
                        ui_scale_percent,
                        column,
                        visible_ix,
                        selected,
                        min_width,
                        &row,
                        visual_kind,
                        styled,
                        streamed_spec,
                        reveal_whitespace_chars,
                        show_line_numbers,
                        wrap,
                        annotation_width,
                        if is_left {
                            blame_ctx.as_ref().and_then(|ctx| {
                                build_row_blame_paint_tracked(
                                    ctx,
                                    matches!(visual_kind, FileDiffRowKind::Context),
                                    row.old_line,
                                    row.new_line,
                                    &blame_prev_nl,
                                    wrap,
                                    theme,
                                )
                            })
                        } else {
                            None
                        },
                        annot_hover,
                        stage_area,
                        stage_hover,
                        row_tint,
                        cx,
                    )
                }
                PatchSplitRow::Raw { src_ix, click_kind } => {
                    if this.patch_diff_row(src_ix).is_none() {
                        return diff_placeholder_row(
                            (id_src_oob, visible_ix),
                            theme,
                            ui_scale_percent,
                        );
                    };
                    let file_stat = this.diff_file_stats.get(src_ix).and_then(|s| *s);
                    let should_style = !query.is_empty();
                    if should_style
                        && this
                            .diff_text_segments_cache_get(src_ix, cache_epoch)
                            .is_none()
                    {
                        let display = this.diff_text_line_for_region(visible_ix, region);
                        let computed = build_cached_diff_styled_text(
                            tab_width,
                            theme,
                            display.as_ref(),
                            &[],
                            "",
                            None,
                            syntax_mode,
                            None,
                        );
                        this.diff_text_segments_cache_set(src_ix, cache_epoch, computed);
                    }
                    let Some(line) = this.patch_diff_row(src_ix) else {
                        return diff_placeholder_row(
                            (id_src_oob, visible_ix),
                            theme,
                            ui_scale_percent,
                        );
                    };
                    if should_hide_unified_diff_header_line(&line) {
                        return div()
                            .id((id_hidden, visible_ix))
                            .h(px(0.0))
                            .into_any_element();
                    }
                    let context_menu_active = click_kind == DiffClickKind::HunkHeader
                        && this.active_repo_id().is_some_and(|repo_id| {
                            let invoker: SharedString =
                                format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                            this.active_context_menu_invoker.as_ref() == Some(&invoker)
                        });
                    let header_display = this.diff_header_display_cache.get(&src_ix).cloned();
                    let styled = if should_style {
                        this.diff_text_segments_cache_get_for_query(
                            src_ix,
                            query.as_ref(),
                            query_options,
                            cache_epoch,
                        )
                    } else {
                        None
                    };
                    patch_split_header_row(
                        theme,
                        ui_scale_percent,
                        column,
                        visible_ix,
                        click_kind,
                        selected,
                        min_width,
                        &line,
                        file_stat,
                        header_display,
                        styled,
                        context_menu_active,
                        cx,
                    )
                }
            }
        })
        .collect()
}
