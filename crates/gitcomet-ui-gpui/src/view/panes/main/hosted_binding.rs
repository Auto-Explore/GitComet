//! Hosted controls and list-boundary projection for the shared diff renderer.
use super::*;
use crate::view::hosted::projection::DisplayRow;
use gitcomet_extension_api::{
    DiffAnnotations, DiffInset, DiffLineRange, DiffLineSide, DiffPaneOptions,
};
use palette::IntoColor;

#[derive(Default)]
pub(in crate::view) struct HostedDiffDecor {
    pub view_id: u64,
    pub options: DiffPaneOptions,
    pub annotations: Arc<DiffAnnotations>,
    pub insets: Arc<[DiffInset]>,
    pub file_cache: Option<Arc<diff_cache::SharedFileDiffCache>>,
    key: Option<(u64, u64, u64, usize, DiffViewMode, usize)>,
    display: Vec<DisplayRow>,
    document: Vec<usize>,
    selection_anchor: Option<usize>,
    markers: Arc<Vec<(f32, gpui::Hsla)>>,
}

impl MainPaneView {
    pub(in crate::view) fn hosted_row_tint(
        &self,
        row: usize,
        side: Option<DiffLineSide>,
    ) -> Option<gpui::Rgba> {
        let decor = self.hosted_decor.as_ref()?;
        let provider = decor.options.decor.as_ref()?;
        let (old, new) = self.hosted_line_numbers(row);
        let (side, line) = match side {
            Some(DiffLineSide::Old) => (DiffLineSide::Old, old?),
            Some(DiffLineSide::New) => (DiffLineSide::New, new?),
            None => new
                .map(|line| (DiffLineSide::New, line))
                .or_else(|| old.map(|line| (DiffLineSide::Old, line)))?,
        };
        provider(side, line)?.tint.map(IntoColor::into_color)
    }

    pub(in crate::view) fn select_hosted_row(&mut self, row: usize, shift: bool) -> bool {
        let Some(decor) = self.hosted_decor.as_mut() else {
            return false;
        };
        let anchor = if shift {
            decor.selection_anchor.unwrap_or(row)
        } else {
            row
        };
        decor.selection_anchor = Some(anchor);
        self.clear_diff_text_selection_span();
        self.diff_selection_anchor = Some(anchor);
        self.diff_selection_range = Some((anchor.min(row), anchor.max(row)));
        true
    }
    pub(in crate::view) fn hosted_line_numbers(
        &self,
        visible: usize,
    ) -> (Option<u32>, Option<u32>) {
        if self
            .rendered_diff_target()
            .and_then(DiffTarget::file_path)
            .is_none()
        {
            return (None, None);
        }
        let Some(source) = self.diff_source_visible_ix_for_visible_ix(visible) else {
            return (None, None);
        };
        if self.is_collapsed_diff_projection_active() {
            return self
                .collapsed_visible_row(source)
                .and_then(|row| row.row_ix())
                .and_then(|row| self.collapsed_diff_file_row_line_numbers(row))
                .unwrap_or_default();
        }
        let Some(row) = self.diff_mapped_ix_for_visible_ix(visible) else {
            return (None, None);
        };
        if self.is_file_diff_view_active() {
            return self
                .collapsed_diff_file_row_line_numbers(row)
                .unwrap_or_default();
        }
        match self.diff_view {
            DiffViewMode::Inline => self
                .patch_diff_row(row)
                .map(|line| (line.old_line, line.new_line))
                .unwrap_or_default(),
            DiffViewMode::Split => match self.patch_diff_split_row(row) {
                Some(PatchSplitRow::Aligned { row, .. }) => (row.old_line, row.new_line),
                Some(PatchSplitRow::Raw { src_ix, .. }) => self
                    .patch_diff_row(src_ix)
                    .map(|line| (line.old_line, line.new_line))
                    .unwrap_or_default(),
                None => (None, None),
            },
        }
    }

    pub(in crate::view) fn hosted_selection(&self) -> Option<DiffLineRange> {
        let (a, b, side) = if let (Some(a), Some(b)) = (self.diff_text_anchor, self.diff_text_head)
        {
            let side = match a.region {
                DiffTextRegion::SplitLeft => DiffLineSide::Old,
                DiffTextRegion::Inline
                    if self
                        .hosted_line_numbers(
                            self.diff_visual_ix_for_source_visible_ix(a.source_visible_ix),
                        )
                        .1
                        .is_none() =>
                {
                    DiffLineSide::Old
                }
                _ => DiffLineSide::New,
            };
            (
                self.diff_visual_ix_for_source_visible_ix(a.source_visible_ix),
                self.diff_visual_ix_for_source_visible_ix(b.source_visible_ix),
                side,
            )
        } else {
            let (a, b) = self
                .diff_selection_range
                .or_else(|| self.diff_selection_anchor.map(|i| (i, i)))?;
            let (old, new) = self.hosted_line_numbers(a);
            let side = if new.is_none() && old.is_some() {
                DiffLineSide::Old
            } else {
                DiffLineSide::New
            };
            (a, b, side)
        };
        let mut lines = (a.min(b)..=a.max(b)).filter_map(|row| {
            let (old, new) = self.hosted_line_numbers(row);
            match side {
                DiffLineSide::Old => old,
                DiffLineSide::New => new,
            }
        });
        let start = lines.next()?;
        let end = lines.last().unwrap_or(start);
        Some(DiffLineRange {
            side,
            start: start.min(end),
            end: start.max(end),
        })
    }

    pub(in crate::view) fn hosted_scroll_anchor(
        &self,
    ) -> Option<gitcomet_extension_api::DiffScrollAnchor> {
        let display = self
            .diff_scroll
            .0
            .borrow()
            .base_handle
            .logical_scroll_top()
            .0;
        let document = self
            .hosted_decor
            .as_ref()
            .filter(|d| !d.insets.is_empty())
            .and_then(|d| d.display.get(display..))
            .and_then(|rows| {
                rows.iter().find_map(|row| match row {
                    DisplayRow::Document(ix) => Some(*ix),
                    _ => None,
                })
            })
            .unwrap_or(display);
        let (old, new) = self.hosted_line_numbers(document);
        new.map(|line| gitcomet_extension_api::DiffScrollAnchor {
            side: DiffLineSide::New,
            line,
        })
        .or_else(|| {
            old.map(|line| gitcomet_extension_api::DiffScrollAnchor {
                side: DiffLineSide::Old,
                line,
            })
        })
    }

    pub(in crate::view) fn hosted_reveal_at(
        &mut self,
        side: DiffLineSide,
        line: u32,
        top: bool,
    ) -> bool {
        let found = (0..self.diff_visible_len()).find(|&row| {
            let (old, new) = self.hosted_line_numbers(row);
            (match side {
                DiffLineSide::Old => old,
                DiffLineSide::New => new,
            }) == Some(line)
        });
        if let Some(row) = found {
            self.scroll_diff_to_item_strict(
                row,
                if top {
                    gpui::ScrollStrategy::Top
                } else {
                    gpui::ScrollStrategy::Center
                },
            );
            true
        } else {
            false
        }
    }

    pub(in crate::view) fn set_hosted_decor(
        &mut self,
        view_id: u64,
        options: DiffPaneOptions,
        annotations: Arc<DiffAnnotations>,
        insets: Arc<[DiffInset]>,
    ) {
        self.store.set_policy(options.policy);
        self.diff_show_line_numbers = options.policy.line_numbers;
        let reset = self
            .hosted_decor
            .as_ref()
            .is_none_or(|decor| !Arc::ptr_eq(&decor.insets, &insets));
        let decor = self.hosted_decor.get_or_insert_with(Default::default);
        decor.view_id = view_id;
        decor.options = options;
        decor.annotations = annotations;
        decor.insets = insets;
        if reset {
            decor.key = None;
        }
    }

    fn prepare_hosted_projection(&mut self) {
        let Some(decor) = &self.hosted_decor else {
            return;
        };
        if decor.insets.is_empty() && decor.annotations.is_empty() {
            return;
        }
        let key = (
            self.diff_visible_projection_rev,
            self.rendered_patch_diff_rev(),
            self.rendered_file_diff_rev(),
            self.diff_visible_len(),
            self.diff_view,
            Arc::as_ptr(&decor.annotations) as usize,
        );
        if decor.key == Some(key) {
            return;
        }
        let mut after = FxHashMap::<usize, Vec<usize>>::default();
        let mut anchors = FxHashMap::default();
        for row in 0..self.diff_visible_len() {
            let (old, new) = self.hosted_line_numbers(row);
            for (side, line) in [(DiffLineSide::Old, old), (DiffLineSide::New, new)] {
                if let Some(line) = line {
                    anchors.insert((side, line), row);
                }
            }
        }
        for (ix, inset) in decor.insets.iter().enumerate() {
            if let Some(row) = anchors.get(&(inset.side, inset.line)) {
                after.entry(*row).or_default().push(ix);
            }
        }
        let len = self.diff_visible_len();
        let decor = self.hosted_decor.as_mut().unwrap();
        decor.display.clear();
        decor.document.clear();
        for row in 0..len {
            decor.document.push(decor.display.len());
            decor.display.push(DisplayRow::Document(row));
            for inset in after.remove(&row).unwrap_or_default() {
                decor.display.extend(
                    (0..decor.insets[inset].lines.len())
                        .map(|line| DisplayRow::Inset { inset, line }),
                );
            }
        }
        decor.markers = Arc::new(
            decor
                .annotations
                .iter()
                .filter_map(|(side, line, mark)| {
                    let document = *anchors.get(&(side, line))?;
                    let display = *decor.document.get(document)?;
                    Some((
                        display as f32 / decor.display.len().max(1) as f32,
                        mark.color,
                    ))
                })
                .collect(),
        );
        decor.key = Some(key);
    }

    pub(in crate::view) fn hosted_markers(&mut self) -> Arc<Vec<(f32, gpui::Hsla)>> {
        self.prepare_hosted_projection();
        self.hosted_decor
            .as_ref()
            .filter(|decor| !decor.annotations.is_empty())
            .map(|decor| decor.markers.clone())
            .unwrap_or_default()
    }

    pub(in crate::view) fn diff_list_len(&mut self) -> usize {
        self.prepare_hosted_projection();
        self.hosted_decor
            .as_ref()
            .filter(|d| !d.insets.is_empty())
            .map_or_else(|| self.diff_visible_len(), |d| d.display.len())
    }

    pub(in crate::view) fn diff_list_index(&self, document: usize) -> usize {
        self.hosted_decor
            .as_ref()
            .filter(|d| !d.insets.is_empty())
            .and_then(|d| d.document.get(document))
            .copied()
            .unwrap_or(document)
    }

    pub(in crate::view) fn render_projected_inline(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        Self::render_projected(this, range, None, window, cx)
    }
    pub(in crate::view) fn render_projected_left(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        Self::render_projected(this, range, Some(DiffLineSide::Old), window, cx)
    }
    pub(in crate::view) fn render_projected_right(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        Self::render_projected(this, range, Some(DiffLineSide::New), window, cx)
    }
    fn render_projected(
        this: &mut Self,
        range: Range<usize>,
        side: Option<DiffLineSide>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let render = match side {
            None => Self::render_diff_rows,
            Some(DiffLineSide::Old) => Self::render_diff_split_left_rows,
            Some(DiffLineSide::New) => Self::render_diff_split_right_rows,
        };
        if this.hosted_decor.is_none() {
            return render(this, range, window, cx);
        }
        let pane_id = this.hosted_decor.as_ref().unwrap().view_id;
        let original_theme = this.theme;
        let style = this.hosted_decor.as_ref().unwrap().options.style;
        if let Some(color) = style.added_background {
            this.theme.colors.diff.added.background = color.into_color();
        }
        if let Some(color) = style.removed_background {
            this.theme.colors.diff.removed.background = color.into_color();
        }
        if let Some(color) = style.context_background {
            this.theme.colors.editor.background = color.into_color();
        }
        let document_rows: Vec<_> = range
            .clone()
            .filter_map(|display| {
                match this
                    .hosted_decor
                    .as_ref()
                    .filter(|d| !d.insets.is_empty())
                    .and_then(|d| d.display.get(display))
                    .copied()
                    .unwrap_or(DisplayRow::Document(display))
                {
                    DisplayRow::Document(row) => Some(row),
                    _ => None,
                }
            })
            .collect();
        let mut rendered_rows = std::collections::VecDeque::new();
        let mut start = 0;
        while start < document_rows.len() {
            let mut end = start + 1;
            while end < document_rows.len() && document_rows[end] == document_rows[end - 1] + 1 {
                end += 1;
            }
            rendered_rows.extend(render(
                this,
                document_rows[start]..document_rows[end - 1] + 1,
                window,
                cx,
            ));
            start = end;
        }
        this.theme = original_theme;
        range
            .map(|display| {
                let row = this
                    .hosted_decor
                    .as_ref()
                    .filter(|d| !d.insets.is_empty())
                    .and_then(|d| d.display.get(display))
                    .copied()
                    .unwrap_or(DisplayRow::Document(display));
                match row {
                    DisplayRow::Inset { inset, line } => {
                        use crate::kit::interaction::ControlInteractionExt as _;
                        let inset = &this.hosted_decor.as_ref().unwrap().insets[inset];
                        let own_side = side.is_none_or(|side| side == inset.side);
                        let action = inset.on_click.clone().filter(|_| own_side);
                        div()
                            .id(("hosted_inset", display))
                            .debug_selector(move || format!("hosted_diff_{pane_id}_row_{display}"))
                            .h(this.theme.editor_row_height(ui_scale::current(cx).percent))
                            .w_full()
                            .overflow_hidden()
                            .bg(this.theme.colors.surface.raised)
                            .text_color(inset.color.unwrap_or_else(|| {
                                this.theme.colors.foreground.secondary.into_color()
                            }))
                            .child(if own_side {
                                inset.lines[line].clone()
                            } else {
                                "".into()
                            })
                            .when_some(action, |row, action| {
                                row.cursor_pointer().on_activate(
                                    false,
                                    crate::kit::interaction::ControlActivation::Action,
                                    move |_, _, cx| {
                                        cx.stop_propagation();
                                        action.invoke(cx);
                                    },
                                )
                            })
                            .into_any_element()
                    }
                    DisplayRow::Document(row) => {
                        use crate::kit::interaction::ControlInteractionExt as _;
                        let element = div()
                            .id(("hosted_row", display))
                            .debug_selector(move || format!("hosted_diff_{pane_id}_row_{display}"))
                            .child(
                                rendered_rows
                                    .pop_front()
                                    .unwrap_or_else(|| div().into_any_element()),
                            )
                            .on_activate(
                                false,
                                crate::kit::interaction::ControlActivation::Composite,
                                cx.listener(move |pane, event: &gpui::ClickEvent, _, cx| {
                                    if !pane.store.policy.select_lines {
                                        return;
                                    }
                                    let shift = event.modifiers().shift;
                                    if !shift && pane.diff_text_has_selection() {
                                        return;
                                    }
                                    let decor = pane.hosted_decor.as_mut().unwrap();
                                    let anchor = if shift {
                                        decor.selection_anchor.unwrap_or(row)
                                    } else {
                                        row
                                    };
                                    decor.selection_anchor = Some(anchor);
                                    pane.clear_diff_text_selection_span();
                                    pane.diff_selection_anchor = Some(anchor);
                                    pane.diff_selection_range =
                                        Some((anchor.min(row), anchor.max(row)));
                                    cx.notify();
                                }),
                            )
                            .into_any_element();
                        let (old, new) = this.hosted_line_numbers(row);
                        let anchor = match side {
                            Some(DiffLineSide::Old) => old.map(|n| (DiffLineSide::Old, n)),
                            Some(DiffLineSide::New) => new.map(|n| (DiffLineSide::New, n)),
                            None => new
                                .map(|n| (DiffLineSide::New, n))
                                .or_else(|| old.map(|n| (DiffLineSide::Old, n))),
                        };
                        let Some((side, line)) = anchor else {
                            return element;
                        };
                        let decor = this.hosted_decor.as_ref().unwrap();
                        let mark = decor
                            .options
                            .decor
                            .as_ref()
                            .and_then(|provider| provider(side, line));
                        let annotation = decor.annotations.get(side, line);
                        if mark.is_none()
                            && annotation.is_none()
                            && decor.options.on_gutter_click.is_none()
                        {
                            return element;
                        }
                        // The lane draws the annotation, so a click there is on it.
                        let on_annotation =
                            annotation.and(decor.options.on_annotation_click.clone());
                        let color = annotation
                            .map(|a| a.color)
                            .or_else(|| mark.as_ref().and_then(|m| m.tint));
                        let label = mark
                            .as_ref()
                            .and_then(|m| m.gutter.clone())
                            .or_else(|| annotation.and_then(|a| a.label.clone()));
                        let action = on_annotation
                            .or_else(|| decor.options.on_gutter_click.clone())
                            .filter(|_| decor.options.policy.line_action);

                        div()
                            .relative()
                            .child(element)
                            .child(
                                div()
                                    .id(("hosted_gutter", display))
                                    .debug_selector(move || {
                                        format!("hosted_diff_{pane_id}_gutter_{display}")
                                    })
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(px(12.0))
                                    .when_some(color, |lane, color| lane.bg(color))
                                    .when_some(label, |lane, label| lane.child(label))
                                    .when_some(action, |lane, action| {
                                        lane.cursor_pointer().on_activate(
                                            false,
                                            crate::kit::interaction::ControlActivation::Action,
                                            move |_, _, cx| {
                                                cx.stop_propagation();
                                                let action = action.clone();
                                                cx.defer(move |cx| action(side, line, cx));
                                            },
                                        )
                                    }),
                            )
                            .into_any_element()
                    }
                }
            })
            .collect()
    }
}
