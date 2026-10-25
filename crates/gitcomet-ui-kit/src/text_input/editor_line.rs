use super::*;

#[derive(Clone, Debug)]
struct TabExpansion {
    source: usize,
    display: Range<usize>,
}

/// A shaped source line. Unvisited wrapped rows contain an empty value rather
/// than allocating a backend layout. Tab expansion belongs to the display;
/// all editor offsets continue to address the original source bytes.
#[derive(Clone, Debug, Default)]
pub(super) struct EditorLine {
    shaped: Option<Arc<gpui::ShapedText>>,
    tabs: Option<Arc<[TabExpansion]>>,
    source_len: usize,
    #[cfg(test)]
    pub(super) text: SharedString,
    pub(super) width: Pixels,
}

impl EditorLine {
    pub(super) fn shape(
        source: SharedString,
        font_size: Pixels,
        runs: &[TextRun],
        wrap_width: Option<Pixels>,
        tab_size: usize,
        window: &mut Window,
    ) -> Self {
        let mut line = Self {
            source_len: source.len(),
            #[cfg(test)]
            text: source.clone(),
            ..Self::default()
        };
        let display = if source.contains('\t') {
            let mut display = String::with_capacity(source.len());
            let mut tabs = Vec::new();
            let mut column = 0usize;
            for (offset, ch) in source.char_indices() {
                if ch == '\t' {
                    let columns = tab_size.max(1) - column % tab_size.max(1);
                    let start = display.len();
                    display.extend(std::iter::repeat_n(' ', columns));
                    tabs.push(TabExpansion {
                        source: offset,
                        display: start..display.len(),
                    });
                    column += columns;
                } else {
                    display.push(ch);
                    column += 1;
                }
            }
            line.tabs = Some(tabs.into());
            SharedString::from(display)
        } else {
            source
        };
        let projected_runs;
        let runs = if line.tabs.is_some() {
            let mut offset = 0;
            projected_runs = runs
                .iter()
                .map(|run| {
                    let mut projected = run.clone();
                    let end = (offset + run.len).min(line.source_len);
                    projected.len = line.display_offset(end) - line.display_offset(offset);
                    offset = end;
                    projected
                })
                .collect::<Vec<_>>();
            projected_runs.as_slice()
        } else {
            runs
        };
        let shaped = window
            .text_system()
            .shape_text(display, font_size, runs, wrap_width, None)
            .expect("editor runs cover the shaped source line");
        line.width = shaped.width();
        line.shaped = Some(Arc::new(shaped));
        line
    }

    fn display_offset(&self, source: usize) -> usize {
        let source = source.min(self.source_len);
        let Some(tabs) = &self.tabs else {
            return source;
        };
        let count = tabs.partition_point(|tab| tab.source <= source);
        let Some(tab) = count.checked_sub(1).map(|index| &tabs[index]) else {
            return source;
        };
        if source == tab.source {
            tab.display.start
        } else {
            source + tab.display.end - (tab.source + 1)
        }
    }

    fn source_offset(&self, display: usize, nearest: bool) -> usize {
        let Some(tabs) = &self.tabs else {
            return display.min(self.source_len);
        };
        let count = tabs.partition_point(|tab| tab.display.start <= display);
        let Some(tab) = count.checked_sub(1).map(|index| &tabs[index]) else {
            return display.min(self.source_len);
        };
        if display < tab.display.end {
            tab.source
                + usize::from(nearest && (display - tab.display.start) * 2 >= tab.display.len())
        } else {
            (display - (tab.display.end - tab.source - 1)).min(self.source_len)
        }
    }

    pub(super) fn len(&self) -> usize {
        self.source_len
    }

    pub(super) fn line_count(&self) -> usize {
        self.shaped.as_ref().map_or(1, |line| line.line_count())
    }

    pub(super) fn row_end_indices(&self) -> Vec<usize> {
        self.shaped.as_ref().map_or_else(
            || vec![0],
            |line| {
                line.visual_lines()
                    .iter()
                    .map(|row| self.source_offset(row.text_range.end, false))
                    .collect()
            },
        )
    }

    pub(super) fn x_for_index(&self, index: usize) -> Pixels {
        self.position_for_index(index, px(1.0))
            .map_or(Pixels::ZERO, |position| position.x)
    }

    pub(super) fn index_for_x(&self, x: Pixels) -> Option<usize> {
        self.shaped
            .as_ref()?
            .byte_index_for_pixel_point(point(x, px(0.5)), px(1.0))
            .ok()
            .map(|index| self.source_offset(index, false))
    }

    pub(super) fn closest_index_for_x(&self, x: Pixels) -> usize {
        self.closest_index_for_position(point(x, px(0.5)), px(1.0))
            .unwrap_or_else(|index| index)
    }

    pub(super) fn position_for_index(
        &self,
        index: usize,
        line_height: Pixels,
    ) -> Option<Point<Pixels>> {
        if index > self.source_len {
            return None;
        }
        self.shaped
            .as_ref()?
            .visual_position_for_byte_index(self.display_offset(index), line_height)
    }

    pub(super) fn closest_index_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        let Some(line) = &self.shaped else {
            return Err(0);
        };
        line.closest_byte_index_for_pixel_point(position, line_height)
            .map(|index| self.source_offset(index, true))
            .map_err(|index| self.source_offset(index, true))
    }

    pub(super) fn selection_bounds(
        &self,
        range: Range<usize>,
        line_height: Pixels,
    ) -> Vec<Bounds<Pixels>> {
        self.shaped.as_ref().map_or_else(Vec::new, |line| {
            line.selection_bounds(
                self.display_offset(range.start)..self.display_offset(range.end),
                line_height,
            )
            .into_vec()
        })
    }

    pub(super) fn paint(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        bounds: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) -> gpui::Result<()> {
        match &self.shaped {
            Some(line) => line.paint(origin, line_height, align, bounds, window, cx),
            None => Ok(()),
        }
    }

    pub(super) fn paint_background(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        bounds: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) -> gpui::Result<()> {
        match &self.shaped {
            Some(line) => line.paint_background(origin, line_height, align, bounds, window, cx),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(
        source: &str,
        tab_size: usize,
        wrap: Option<Pixels>,
        window: &mut Window,
    ) -> EditorLine {
        let run = super::super::highlight::text_run_for_style(
            &gpui::font("Monospace"),
            gpui::black(),
            source.len(),
            None,
        );
        EditorLine::shape(
            source.to_string().into(),
            px(12.0),
            &[run],
            wrap,
            tab_size,
            window,
        )
    }

    #[gpui::test]
    fn tabs_advance_to_configured_stops_and_keep_unicode_source_offsets(
        cx: &mut gpui::TestAppContext,
    ) {
        crate::test_support::use_real_text_backend(cx);
        let cx = cx.add_empty_window();
        cx.update(|window, _| {
            for (source, columns, display) in [
                ("a\tbc\td", 4, "a   bc  d"),
                ("abcd\tx", 4, "abcd    x"),
                ("\tx", 8, "        x"),
                ("日本\tx", 4, "日本  x"),
                ("é\t😀x", 4, "é   😀x"),
                ("a\tb", 0, "a b"),
            ] {
                let line = shape(source, columns, None, window);
                let native = line.shaped.as_ref().expect("shaped line");
                assert_eq!(native.text.as_ref(), display);
                assert_eq!(line.len(), source.len());
                for offset in source
                    .char_indices()
                    .map(|(offset, _)| offset)
                    .chain([source.len()])
                {
                    let display = line.display_offset(offset);
                    assert_eq!(line.source_offset(display, false), offset);
                    assert_eq!(
                        line.position_for_index(offset, px(18.0)),
                        native.visual_position_for_byte_index(display, px(18.0))
                    );
                }
                for display in 0..=display.len() {
                    if !native.text.is_char_boundary(display) {
                        continue;
                    }
                    assert!(source.is_char_boundary(line.source_offset(display, true)));
                }
            }
        });
    }

    #[gpui::test]
    fn tab_highlights_and_selection_use_the_expanded_backend_layout(cx: &mut gpui::TestAppContext) {
        crate::test_support::use_real_text_backend(cx);
        let cx = cx.add_empty_window();
        cx.update(|window, _| {
            let source = "a\t日本x";
            let font = gpui::font("Monospace");
            let first = super::super::highlight::text_run_for_style(&font, gpui::red(), 2, None);
            let rest = super::super::highlight::text_run_for_style(&font, gpui::black(), source.len() - 2, None);
            let line = EditorLine::shape(source.into(), px(12.0), &[first, rest], None, 4, window);
            let native = line.shaped.as_ref().unwrap();
            assert_eq!(native.text.as_ref(), "a   日本x");
            assert!(native.paint_fragments.iter().any(|fragment| fragment.source_run == 0 && fragment.style.color == gpui::red()));
            assert_eq!(line.selection_bounds(1..2, px(18.0)), native.selection_bounds(1..4, px(18.0)).into_vec());
            let left = line.x_for_index(1);
            let right = line.x_for_index(2);
            assert_eq!(line.closest_index_for_x(left + (right - left) * 0.2), 1);
            assert_eq!(line.closest_index_for_x(left + (right - left) * 0.8), 2);
        });
    }

    #[gpui::test]
    fn wrapped_tabs_share_paint_selection_and_hit_test_geometry(cx: &mut gpui::TestAppContext) {
        crate::test_support::use_real_text_backend(cx);
        let cx = cx.add_empty_window();
        cx.update(|window, _| {
            let source = "\t\t\tabc def 日本 e\u{301}";
            let line = shape(source, 4, Some(px(50.0)), window);
            assert!(line.line_count() > 1);
            assert!(line.position_for_index(source.len(), px(18.0)).is_some());
            assert!(!line.selection_bounds(0..source.len(), px(18.0)).is_empty());
            for row in 0..line.line_count() {
                for x in [0.0, 12.0, 49.0] {
                    let offset = line
                        .closest_index_for_position(
                            point(px(x), px(row as f32 * 18.0 + 9.0)),
                            px(18.0),
                        )
                        .unwrap_or_else(|index| index);
                    assert!(source.is_char_boundary(offset));
                }
            }
        });
    }

    #[gpui::test]
    fn unexpanded_lines_reuse_native_cluster_geometry(cx: &mut gpui::TestAppContext) {
        crate::test_support::use_real_text_backend(cx);
        let cx = cx.add_empty_window();
        cx.update(|window, _| {
            for source in ["", "abc", "e\u{301} 日本", "abc אבג def"] {
                let line = shape(source, 4, None, window);
                assert!(line.tabs.is_none());
                let native = line.shaped.as_ref().unwrap();
                for x in [0.0, 12.0, 50.0] {
                    assert_eq!(
                        line.closest_index_for_x(px(x)),
                        native
                            .closest_byte_index_for_pixel_point(point(px(x), px(0.5)), px(1.0))
                            .unwrap_or_else(|index| index)
                    );
                }
            }
        });
    }
}
