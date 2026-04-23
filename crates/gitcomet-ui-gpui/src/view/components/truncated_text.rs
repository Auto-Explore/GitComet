use super::super::text_truncation::{
    TextTruncationProfile, TruncatedLineLayout, path_alignment_style_key,
    shape_truncated_line_cached_with_path_anchor, truncated_line_ellipsis_x,
};
use crate::view::tooltip_host::TooltipHost;
use gpui::EntityId;
use gpui::prelude::*;
use gpui::{
    App, AvailableSpace, Bounds, Context, Element, ElementId, GlobalElementId, HighlightStyle,
    InspectorElementId, IntoElement, LayoutId, Pixels, SharedString, Stateful, TextAlign,
    WeakEntity, Window, div, point, px, size,
};
use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

#[cfg(test)]
use super::super::text_truncation::{
    clear_truncated_layout_cache_for_test, path_alignment_visible_signature,
};

pub(crate) use super::super::text_truncation::TruncatedTextPathAlignmentGroup;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TruncatedTextTooltipMode {
    #[default]
    None,
    FullTextIfTruncated,
}

pub struct TruncatedText {
    text: SharedString,
    profile: TextTruncationProfile,
    highlights: Arc<[(Range<usize>, HighlightStyle)]>,
    focus_range: Option<Range<usize>>,
    tooltip_mode: TruncatedTextTooltipMode,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    path_alignment_group: Option<TruncatedTextPathAlignmentGroup>,
}

impl TruncatedText {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            profile: TextTruncationProfile::End,
            highlights: Arc::from([]),
            focus_range: None,
            tooltip_mode: TruncatedTextTooltipMode::None,
            tooltip_host: None,
            path_alignment_group: None,
        }
    }

    pub fn profile(mut self, profile: TextTruncationProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn highlights(
        mut self,
        highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
    ) -> Self {
        self.highlights = Arc::from(highlights.into_iter().collect::<Vec<_>>());
        self
    }

    pub fn focus_range(mut self, focus_range: Option<Range<usize>>) -> Self {
        self.focus_range = focus_range;
        self
    }

    pub fn tooltip_mode(mut self, mode: TruncatedTextTooltipMode) -> Self {
        self.tooltip_mode = mode;
        self
    }

    pub fn tooltip_host(mut self, tooltip_host: WeakEntity<TooltipHost>) -> Self {
        self.tooltip_host = Some(tooltip_host);
        self
    }

    pub(crate) fn path_alignment_group(
        mut self,
        path_alignment_group: TruncatedTextPathAlignmentGroup,
    ) -> Self {
        self.path_alignment_group = Some(path_alignment_group);
        self
    }

    pub fn render<V: 'static>(self, cx: &Context<V>) -> impl IntoElement {
        let tooltip_text = self.text.clone();
        let tooltip_mode = self.tooltip_mode;
        let tooltip_host = self.tooltip_host.clone();
        let owner_view_id = cx.entity_id();
        let truncated = Rc::new(Cell::new(false));
        let element = TruncatedTextElement {
            text: self.text,
            profile: self.profile,
            highlights: self.highlights,
            focus_range: self.focus_range,
            layout: TruncatedTextLayoutState::default(),
            truncated: Rc::clone(&truncated),
            owner_view_id,
            path_alignment_group: self.path_alignment_group,
        };

        let mut root: Stateful<_> = div()
            .id(("truncated_text", Rc::as_ptr(&truncated) as usize))
            .min_w(px(0.0))
            .overflow_hidden()
            .whitespace_nowrap()
            .child(element);

        if matches!(tooltip_mode, TruncatedTextTooltipMode::FullTextIfTruncated)
            && let Some(tooltip_host) = tooltip_host
        {
            root = root.on_hover(cx.listener(move |_this, hovering: &bool, _window, cx| {
                if *hovering {
                    if truncated.get() {
                        let _ = tooltip_host.update(cx, |host, cx| {
                            host.set_tooltip_text_if_changed(Some(tooltip_text.clone()), cx);
                        });
                    }
                } else {
                    let _ = tooltip_host.update(cx, |host, cx| {
                        host.clear_tooltip_if_matches(&tooltip_text, cx);
                    });
                }
            }));
        }

        root
    }
}

#[derive(Default, Clone)]
struct TruncatedTextLayoutState(Rc<RefCell<Option<TruncatedTextLayoutInner>>>);

struct TruncatedTextLayoutInner {
    line: Arc<TruncatedLineLayout>,
}

struct TruncatedTextElement {
    text: SharedString,
    profile: TextTruncationProfile,
    highlights: Arc<[(Range<usize>, HighlightStyle)]>,
    focus_range: Option<Range<usize>>,
    layout: TruncatedTextLayoutState,
    truncated: Rc<Cell<bool>>,
    owner_view_id: EntityId,
    path_alignment_group: Option<TruncatedTextPathAlignmentGroup>,
}

impl Element for TruncatedTextElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_state = self.layout.clone();
        let text = self.text.clone();
        let profile = self.profile;
        let highlights = Arc::clone(&self.highlights);
        let focus_range = self.focus_range.clone();
        let truncated = Rc::clone(&self.truncated);
        let owner_view_id = self.owner_view_id;
        let path_alignment_group = self.path_alignment_group.clone();

        let layout_id = window.request_measured_layout(
            Default::default(),
            move |known_dimensions, available_space, window, cx| {
                let max_width = known_dimensions.width.or(match available_space.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                let base_style = window.text_style();
                let alignment_style_key = (profile == TextTruncationProfile::Path)
                    .then(|| path_alignment_style_key(&base_style, window.rem_size()));
                let path_ellipsis_anchor = path_alignment_group.as_ref().and_then(|group| {
                    alignment_style_key
                        .map(|style_key| group.path_anchor_for_layout(max_width, style_key))
                        .flatten()
                });
                let line = shape_truncated_line_cached_with_path_anchor(
                    window,
                    cx,
                    &base_style,
                    &text,
                    max_width,
                    profile,
                    highlights.as_ref(),
                    focus_range.clone(),
                    path_ellipsis_anchor,
                );
                if profile == TextTruncationProfile::Path
                    && path_ellipsis_anchor.is_none()
                    && let Some(group) = path_alignment_group.as_ref()
                    && let Some(style_key) = alignment_style_key
                    && let Some(ellipsis_x) = truncated_line_ellipsis_x(&line)
                    && group.report_natural_ellipsis(max_width, style_key, ellipsis_x)
                {
                    cx.notify(owner_view_id);
                }
                truncated.set(line.truncated);
                let width = max_width
                    .map(|width| width.max(px(0.0)))
                    .unwrap_or(line.shaped_line.width);
                let size = size(width, line.line_height);
                layout_state
                    .0
                    .replace(Some(TruncatedTextLayoutInner { line }));
                size
            },
        );

        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let binding = self.layout.0.borrow();
        let Some(inner) = binding.as_ref() else {
            return;
        };

        if inner.line.has_background_runs {
            let _ = inner.line.shaped_line.paint_background(
                point(bounds.left(), bounds.top()),
                inner.line.line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }

        let _ = inner.line.shaped_line.paint(
            point(bounds.left(), bounds.top()),
            inner.line.line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}

impl IntoElement for TruncatedTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PATH_A: &str = "src/components/really_long_directory_name/rows/file_name_alpha.rs";
    const PATH_B: &str = "src/components/dir/another_super_long_directory_name/file_name_beta.rs";

    struct TruncatedTextPathAlignmentTestView {
        group: TruncatedTextPathAlignmentGroup,
        width: Pixels,
        font_size: Pixels,
        line_height: Pixels,
    }

    impl TruncatedTextPathAlignmentTestView {
        fn new() -> Self {
            Self {
                group: TruncatedTextPathAlignmentGroup::default(),
                width: px(190.0),
                font_size: px(14.0),
                line_height: px(18.0),
            }
        }
    }

    impl Render for TruncatedTextPathAlignmentTestView {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.group
                .begin_visible_rows(path_alignment_visible_signature(&(PATH_A, PATH_B)));

            div()
                .flex_col()
                .child(
                    div()
                        .w(self.width)
                        .text_size(self.font_size)
                        .line_height(self.line_height)
                        .child(
                            TruncatedText::new(PATH_A)
                                .profile(TextTruncationProfile::Path)
                                .path_alignment_group(self.group.clone())
                                .render(cx),
                        ),
                )
                .child(
                    div()
                        .w(self.width)
                        .text_size(self.font_size)
                        .line_height(self.line_height)
                        .child(
                            TruncatedText::new(PATH_B)
                                .profile(TextTruncationProfile::Path)
                                .path_alignment_group(self.group.clone())
                                .render(cx),
                        ),
                )
        }
    }

    #[gpui::test]
    fn truncated_text_path_alignment_converges_in_layout_passes(cx: &mut gpui::TestAppContext) {
        clear_truncated_layout_cache_for_test();
        let (view, cx) =
            cx.add_window_view(|_window, _cx| TruncatedTextPathAlignmentTestView::new());

        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });

        let after_first_draw = cx.update(|_window, app| view.read(app).group.snapshot_for_test());
        assert!(after_first_draw.layout_key.is_some());

        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });

        let after_second_draw = cx.update(|_window, app| view.read(app).group.snapshot_for_test());
        assert!(after_second_draw.layout_key.is_some());
        assert!(after_second_draw.resolved_anchor.is_some());
        assert_eq!(after_second_draw.pending_anchor, None);
    }
}
