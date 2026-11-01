use super::*;

#[gpui::test]
fn yaml_file_diff_keeps_consistent_highlighting_for_added_paths_and_keys(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;

    fn split_right_row_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn split_right_cached_styled_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn inline_row_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn force_file_diff_fallback_mode(pane: &mut DiffRenderer) {
        pane.file_diff_syntax_generation = pane.file_diff_syntax_generation.wrapping_add(1);
        for view_mode in [
            PreparedSyntaxViewMode::FileDiffSplitLeft,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        ] {
            if let Some(key) = pane.file_diff_prepared_syntax_key(view_mode) {
                pane.prepared_syntax_documents.remove(&key);
            }
        }
        pane.clear_diff_text_style_caches();
    }

    fn quoted_scalar_style(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<(std::ops::Range<usize>, gpui::Hsla)> {
        let quote_start = text.find('"')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none()
                && range.start == quote_start
                && range.end == text.len())
            .then_some((range.clone(), color))
        })
    }

    fn list_item_dash_color(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let dash_ix = text.find('-')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none()
                && range.start <= dash_ix
                && range.end >= dash_ix.saturating_add(1))
            .then_some(color)
        })
    }

    fn mapping_key_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let key_start = text.find(|ch: char| !ch.is_ascii_whitespace())?;
        let key_end = text[key_start..].find(':')?.saturating_add(key_start);
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none() && range.start <= key_start && range.end >= key_end)
                .then_some(color)
        })
    }

    fn line_debug(
        line: Option<(&str, &super::CachedDiffStyledText)>,
    ) -> Option<(
        String,
        Vec<(
            std::ops::Range<usize>,
            Option<gpui::Hsla>,
            Option<gpui::Hsla>,
        )>,
    )> {
        let (text, styled) = line?;
        Some((
            text.to_string(),
            styled
                .highlights
                .iter()
                .map(|(range, style)| (range.clone(), style.color, style.background_color))
                .collect(),
        ))
    }

    fn split_debug(
        pane: &DiffRenderer,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    line_debug(split_right_cached_styled_by_new_line(pane, line)),
                )
            })
            .collect()
    }

    fn inline_debug(
        pane: &DiffRenderer,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    line_debug(inline_cached_styled_by_new_line(pane, line)),
                )
            })
            .collect()
    }

    fn split_kind_debug(pane: &DiffRenderer, lines: &[u32]) -> Vec<(u32, Option<FileDiffRowKind>)> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    split_right_row_by_new_line(pane, line).map(|row| row.kind),
                )
            })
            .collect()
    }

    fn inline_kind_debug(pane: &DiffRenderer, lines: &[u32]) -> Vec<(u32, Option<DiffLineKind>)> {
        lines
            .iter()
            .copied()
            .map(|line| (line, inline_row_by_new_line(pane, line).map(|row| row.kind)))
            .collect()
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    #[derive(Clone, Copy, Debug)]
    struct ExpectedPaintRow {
        line_no: u32,
        visible_ix: usize,
        expects_add_bg: bool,
    }

    fn split_visible_ix_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn split_draw_rows_for_lines(pane: &DiffRenderer, lines: &[u32]) -> Vec<ExpectedPaintRow> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let visible_ix = split_visible_ix_by_new_line(pane, line_no)
                    .unwrap_or_else(|| panic!("expected split visible row for line {line_no}"));
                let expects_add_bg = split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Add);
                ExpectedPaintRow {
                    line_no,
                    visible_ix,
                    expects_add_bg,
                }
            })
            .collect()
    }

    fn inline_draw_rows_for_lines(pane: &DiffRenderer, lines: &[u32]) -> Vec<ExpectedPaintRow> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let visible_ix = inline_visible_ix_by_new_line(pane, line_no)
                    .unwrap_or_else(|| panic!("expected inline visible row for line {line_no}"));
                let expects_add_bg = inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Add);
                ExpectedPaintRow {
                    line_no,
                    visible_ix,
                    expects_add_bg,
                }
            })
            .collect()
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.diff_selection_anchor = None;
                    pane.diff_selection_range = None;
                    pane.diff_autoscroll_pending = false;
                    pane.clear_diff_text_selection();
                    pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                    cx.notify();
                });
            });
        });
        cx.run_until_parked();

        cx.update(|window, app| {
            rows::clear_diff_paint_log_for_tests();
            let _ = window.draw(app);
            rows::diff_paint_log_for_tests()
                .into_iter()
                .find(|record| record.visible_ix == visible_ix && record.region == region)
                .unwrap_or_else(|| {
                    panic!("expected paint record for visible_ix={visible_ix} region={region:?}")
                })
        })
    }

    fn assert_split_rows_match_render_cache(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
        expected_rows: Vec<ExpectedPaintRow>,
    ) {
        let mut add_bg = None;
        let mut context_bg = None;

        for expected in expected_rows {
            let record = draw_paint_record_for_visible_ix(
                cx,
                view,
                expected.visible_ix,
                DiffTextRegion::SplitRight,
            );
            let (text, highlights) = cx.update(|_window, app| {
                let pane = view.read(app).diff_renderer.read(app);
                let (text, styled) = split_right_cached_styled_by_new_line(pane, expected.line_no)
                    .unwrap_or_else(|| {
                        panic!(
                            "expected cached split-right styled text for line {}",
                            expected.line_no
                        )
                    });
                (
                    text.to_string(),
                    highlight_snapshot(styled.highlights.as_ref()),
                )
            });
            assert_eq!(
                record.text.as_ref(),
                text.as_str(),
                "{label} render text mismatch for line {}",
                expected.line_no,
            );
            assert_eq!(
                record.highlights, highlights,
                "{label} render highlights mismatch for line {}",
                expected.line_no,
            );

            if expected.expects_add_bg {
                match add_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} add-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => add_bg = record.row_bg,
                }
            } else {
                match context_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} context-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => context_bg = record.row_bg,
                }
            }
        }

        if let (Some(add_bg), Some(context_bg)) = (add_bg, context_bg) {
            assert_ne!(
                add_bg, context_bg,
                "{label} should paint add rows with a different background than context rows",
            );
        }
    }

    fn assert_inline_rows_match_render_cache(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
        expected_rows: Vec<ExpectedPaintRow>,
    ) {
        let mut add_bg = None;
        let mut context_bg = None;

        for expected in expected_rows {
            let record = draw_paint_record_for_visible_ix(
                cx,
                view,
                expected.visible_ix,
                DiffTextRegion::Inline,
            );
            let (text, highlights) = cx.update(|_window, app| {
                let pane = view.read(app).diff_renderer.read(app);
                let (text, styled) = inline_cached_styled_by_new_line(pane, expected.line_no)
                    .unwrap_or_else(|| {
                        panic!(
                            "expected cached inline styled text for line {}",
                            expected.line_no
                        )
                    });
                (
                    text.to_string(),
                    highlight_snapshot(styled.highlights.as_ref()),
                )
            });
            assert_eq!(
                record.text.as_ref(),
                text.as_str(),
                "{label} render text mismatch for line {}",
                expected.line_no,
            );
            assert_eq!(
                record.highlights, highlights,
                "{label} render highlights mismatch for line {}",
                expected.line_no,
            );

            if expected.expects_add_bg {
                match add_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} add-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => add_bg = record.row_bg,
                }
            } else {
                match context_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} context-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => context_bg = record.row_bg,
                }
            }
        }

        if let (Some(add_bg), Some(context_bg)) = (add_bg, context_bg) {
            assert_ne!(
                add_bg, context_bg,
                "{label} should paint add rows with a different background than context rows",
            );
        }
    }

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(80);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_file_diff",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/deployment-ci.yml");
    let old_text = DEPLOYMENT_CI.old_text.to_owned();
    let new_text = DEPLOYMENT_CI.new_text.to_owned();

    let baseline_path_line = 17u32;
    let affected_path_lines = [18u32, 22, 24, 26, 27, 28, 29, 30, 31, 32, 33];
    let baseline_nested_key_line = 4u32;
    let affected_nested_key_lines = [19u32, 34u32];
    let baseline_top_key_line = 3u32;
    let affected_top_key_lines = [36u32];
    let affected_add_lines = [18u32, 33u32];
    let affected_context_lines = [19u32, 22, 24, 26, 27, 28, 29, 30, 31, 32, 34, 36];
    let render_lines = [
        17u32, 18, 19, 21, 22, 24, 26, 27, 28, 29, 30, 31, 32, 33, 34, 36,
    ];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 0, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff cache build before fallback highlighting checks",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 0
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?} rows={} inline_rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                // Other YAML tests can warm the shared prepared-syntax cache before this
                // test runs. Clear the local prepared documents and invalidate any in-flight
                // background parse so the next draw deterministically exercises fallback mode.
                force_file_diff_fallback_mode(pane);
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff fallback mode forced for highlight checks",
        |pane| {
            pane.file_diff_cache_rev == 0
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_none()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_none()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?} rows={} inline_rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        let (baseline_path_text, baseline_path_styled) =
            split_right_cached_styled_by_new_line(pane, baseline_path_line)
                .expect("fallback split draw should cache the baseline YAML path row");
        let baseline_dash_color = list_item_dash_color(baseline_path_styled, baseline_path_text)
            .expect("fallback split draw should syntax-highlight the YAML list dash");
        let (_, baseline_path_color) = quoted_scalar_style(baseline_path_styled, baseline_path_text)
            .expect("fallback split draw should syntax-highlight the YAML quoted path");
        for line_no in affected_path_lines {
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback split draw should cache YAML row {line_no}"));
            assert_eq!(
                list_item_dash_color(styled, text),
                Some(baseline_dash_color),
                "fallback split draw should keep YAML list punctuation highlighting on line {line_no}",
            );
            assert_eq!(
                quoted_scalar_style(styled, text).map(|(_, color)| color),
                Some(baseline_path_color),
                "fallback split draw should keep YAML quoted-string highlighting on line {line_no}",
            );
        }

        let (baseline_nested_key_text, baseline_nested_key_styled) =
            split_right_cached_styled_by_new_line(pane, baseline_nested_key_line)
                .expect("fallback split draw should cache the baseline YAML nested key row");
        let baseline_nested_key_color = mapping_key_color(
            baseline_nested_key_styled,
            baseline_nested_key_text,
        )
        .expect("fallback split draw should syntax-highlight the YAML nested key");
        for line_no in affected_nested_key_lines {
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback split draw should cache YAML key row {line_no}"));
            assert_eq!(
                mapping_key_color(styled, text),
                Some(baseline_nested_key_color),
                "fallback split draw should keep YAML key highlighting on line {line_no}",
            );
        }

        let (baseline_top_key_text, baseline_top_key_styled) =
            split_right_cached_styled_by_new_line(pane, baseline_top_key_line)
                .expect("fallback split draw should cache the baseline YAML top-level key row");
        let baseline_top_key_color =
            mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
                .expect("fallback split draw should syntax-highlight the YAML top-level key");
        for line_no in affected_top_key_lines {
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback split draw should cache YAML top-level key row {line_no}"));
            assert_eq!(
                mapping_key_color(styled, text),
                Some(baseline_top_key_color),
                "fallback split draw should keep YAML top-level key highlighting on line {line_no}",
            );
        }
    });

    let fallback_split_draw_rows = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        split_draw_rows_for_lines(pane, &render_lines)
    });
    assert_split_rows_match_render_cache(cx, &view, "fallback split", fallback_split_draw_rows);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        let (baseline_path_text, baseline_path_styled) =
            inline_cached_styled_by_new_line(pane, baseline_path_line)
                .expect("fallback inline draw should cache the baseline YAML path row");
        let baseline_dash_color = list_item_dash_color(baseline_path_styled, baseline_path_text)
            .expect("fallback inline draw should syntax-highlight the YAML list dash");
        let (_, baseline_path_color) = quoted_scalar_style(baseline_path_styled, baseline_path_text)
            .expect("fallback inline draw should syntax-highlight the YAML quoted path");
        for line_no in affected_path_lines {
            let (text, styled) = inline_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback inline draw should cache YAML row {line_no}"));
            assert_eq!(
                list_item_dash_color(styled, text),
                Some(baseline_dash_color),
                "fallback inline draw should keep YAML list punctuation highlighting on line {line_no}",
            );
            assert_eq!(
                quoted_scalar_style(styled, text).map(|(_, color)| color),
                Some(baseline_path_color),
                "fallback inline draw should keep YAML quoted-string highlighting on line {line_no}",
            );
        }

        let (baseline_nested_key_text, baseline_nested_key_styled) =
            inline_cached_styled_by_new_line(pane, baseline_nested_key_line)
                .expect("fallback inline draw should cache the baseline YAML nested key row");
        let baseline_nested_key_color = mapping_key_color(
            baseline_nested_key_styled,
            baseline_nested_key_text,
        )
        .expect("fallback inline draw should syntax-highlight the YAML nested key");
        for line_no in affected_nested_key_lines {
            let (text, styled) = inline_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback inline draw should cache YAML key row {line_no}"));
            assert_eq!(
                mapping_key_color(styled, text),
                Some(baseline_nested_key_color),
                "fallback inline draw should keep YAML key highlighting on line {line_no}",
            );
        }

        let (baseline_top_key_text, baseline_top_key_styled) =
            inline_cached_styled_by_new_line(pane, baseline_top_key_line)
                .expect("fallback inline draw should cache the baseline YAML top-level key row");
        let baseline_top_key_color =
            mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
                .expect("fallback inline draw should syntax-highlight the YAML top-level key");
        for line_no in affected_top_key_lines {
            let (text, styled) = inline_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback inline draw should cache YAML top-level key row {line_no}"));
            assert_eq!(
                mapping_key_color(styled, text),
                Some(baseline_top_key_color),
                "fallback inline draw should keep YAML top-level key highlighting on line {line_no}",
            );
        }
    });

    let fallback_inline_draw_rows = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        inline_draw_rows_for_lines(pane, &render_lines)
    });
    assert_inline_rows_match_render_cache(cx, &view, "fallback inline", fallback_inline_draw_rows);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_millis(50),
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 1, &old_text, &old_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff baseline revision prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 2, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 2
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} rows={} inline_rows={} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff split syntax stays consistent for repeated paths and keys",
        |pane| {
            let Some((baseline_path_text, baseline_path_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_path_line)
            else {
                return false;
            };
            let Some(baseline_dash_color) =
                list_item_dash_color(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            let Some((_, baseline_path_color)) =
                quoted_scalar_style(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            if affected_add_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Add)
            }) {
                return false;
            }
            if affected_context_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Context)
            }) {
                return false;
            }
            if affected_path_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                list_item_dash_color(styled, text) != Some(baseline_dash_color)
                    || quoted_scalar_style(styled, text).map(|(_, color)| color)
                        != Some(baseline_path_color)
            }) {
                return false;
            }

            let Some((baseline_nested_key_text, baseline_nested_key_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_nested_key_line)
            else {
                return false;
            };
            let Some(baseline_nested_key_color) =
                mapping_key_color(baseline_nested_key_styled, baseline_nested_key_text)
            else {
                return false;
            };
            if affected_nested_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_nested_key_color)
            }) {
                return false;
            }

            let Some((baseline_top_key_text, baseline_top_key_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_top_key_line)
            else {
                return false;
            };
            let Some(baseline_top_key_color) =
                mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
            else {
                return false;
            };
            !affected_top_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_top_key_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_path_line);
            lines.extend(affected_path_lines);
            lines.push(baseline_nested_key_line);
            lines.extend(affected_nested_key_lines);
            lines.push(baseline_top_key_line);
            lines.extend(affected_top_key_lines);
            format!(
                "diff_view={:?} split_kinds={:?} split_debug={:?}",
                pane.diff_view,
                split_kind_debug(pane, &lines),
                split_debug(pane, &lines),
            )
        },
    );

    let prepared_split_draw_rows = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        split_draw_rows_for_lines(pane, &render_lines)
    });
    assert_split_rows_match_render_cache(cx, &view, "prepared split", prepared_split_draw_rows);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff inline syntax stays consistent for repeated paths and keys",
        |pane| {
            let Some((baseline_path_text, baseline_path_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_path_line)
            else {
                return false;
            };
            let Some(baseline_dash_color) =
                list_item_dash_color(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            let Some((_, baseline_path_color)) =
                quoted_scalar_style(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            if affected_add_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Add)
            }) {
                return false;
            }
            if affected_context_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Context)
            }) {
                return false;
            }
            if affected_path_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                list_item_dash_color(styled, text) != Some(baseline_dash_color)
                    || quoted_scalar_style(styled, text).map(|(_, color)| color)
                        != Some(baseline_path_color)
            }) {
                return false;
            }

            let Some((baseline_nested_key_text, baseline_nested_key_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_nested_key_line)
            else {
                return false;
            };
            let Some(baseline_nested_key_color) =
                mapping_key_color(baseline_nested_key_styled, baseline_nested_key_text)
            else {
                return false;
            };
            if affected_nested_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_nested_key_color)
            }) {
                return false;
            }

            let Some((baseline_top_key_text, baseline_top_key_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_top_key_line)
            else {
                return false;
            };
            let Some(baseline_top_key_color) =
                mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
            else {
                return false;
            };
            !affected_top_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_top_key_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_path_line);
            lines.extend(affected_path_lines);
            lines.push(baseline_nested_key_line);
            lines.extend(affected_nested_key_lines);
            lines.push(baseline_top_key_line);
            lines.extend(affected_top_key_lines);
            format!(
                "diff_view={:?} inline_kinds={:?} inline_debug={:?}",
                pane.diff_view,
                inline_kind_debug(pane, &lines),
                inline_debug(pane, &lines),
            )
        },
    );

    let prepared_inline_draw_rows = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        inline_draw_rows_for_lines(pane, &render_lines)
    });
    assert_inline_rows_match_render_cache(cx, &view, "prepared inline", prepared_inline_draw_rows);
}

#[gpui::test]
fn yaml_file_diff_fallback_matches_prepared_document_for_deployment_ci(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;
    use std::collections::BTreeMap;

    fn split_right_cached_styled_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn split_right_row_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn inline_row_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn split_visible_ix_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn draw_rows_for_visible_indices(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_indices: &[usize],
    ) {
        for &visible_ix in visible_indices {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.diff_renderer.update(cx, |pane, cx| {
                        pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                        cx.notify();
                    });
                });
            });
            cx.run_until_parked();
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
        }
    }

    fn cached_snapshot(line: (&str, &super::CachedDiffStyledText)) -> LineSyntaxSnapshot {
        let (text, styled) = line;
        LineSyntaxSnapshot {
            text: text.to_string(),
            syntax: styled
                .highlights
                .iter()
                .filter(|(_, style)| style.background_color.is_none())
                .map(|(range, style)| (range.clone(), style.color))
                .collect(),
        }
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.diff_selection_anchor = None;
                    pane.diff_selection_range = None;
                    pane.diff_autoscroll_pending = false;
                    pane.clear_diff_text_selection();
                    pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                    cx.notify();
                });
            });
        });
        cx.run_until_parked();

        cx.update(|window, app| {
            rows::clear_diff_paint_log_for_tests();
            let _ = window.draw(app);
            rows::diff_paint_log_for_tests()
                .into_iter()
                .find(|record| record.visible_ix == visible_ix && record.region == region)
                .unwrap_or_else(|| {
                    panic!("expected paint record for visible_ix={visible_ix} region={region:?}")
                })
        })
    }

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let theme = cx.update(|_window, app| view.read(app).diff_renderer.read(app).theme);

    let repo_id = gitcomet_state::model::RepoId(180);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_fallback_prepared_baseline",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/deployment-ci.yml");
    let old_text = DEPLOYMENT_CI.old_text.to_owned();
    let new_text = DEPLOYMENT_CI.new_text.to_owned();
    let (old_shared_text, old_line_starts) = shared_text_and_line_starts(old_text.as_str());
    let (new_shared_text, new_line_starts) = shared_text_and_line_starts(new_text.as_str());
    let old_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        old_shared_text,
        Arc::clone(&old_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared old YAML baseline document, got {other:?}"),
    };
    let new_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        new_shared_text,
        Arc::clone(&new_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared new YAML baseline document, got {other:?}"),
    };

    let old_lines = [3u32, 4];
    let new_lines = [
        3u32, 4, 17, 18, 19, 22, 24, 26, 27, 28, 29, 30, 31, 32, 33, 34, 36,
    ];
    let baseline_old_by_line = old_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                old_text.as_str(),
                old_line_starts.as_ref(),
                old_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for old line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();
    let baseline_new_by_line = new_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                new_text.as_str(),
                new_line_starts.as_ref(),
                new_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for new line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 1, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "deployment-ci YAML rows ready for prepared-baseline comparison",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?} rows={} inline_rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        for line_no in new_lines {
            let actual = split_right_cached_styled_by_new_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected fallback split-right styled text for deployment-ci line {line_no}")
                });
            let expected = baseline_new_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for deployment-ci line {line_no}"));
            assert_eq!(
                actual, expected,
                "fallback split-right YAML highlighting should match prepared baseline for deployment-ci line {line_no}"
            );
        }
    });

    let split_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        new_lines
            .iter()
            .copied()
            .map(|line_no| {
                split_visible_ix_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected split visible row for deployment-ci line {line_no}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, split_visible_indices.as_slice());

    for (&line_no, &visible_ix) in new_lines.iter().zip(split_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::SplitRight);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!(
                        "expected cached split-right styled text for deployment-ci line {line_no}"
                    )
                });
            let kind = split_right_row_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected split-right row for deployment-ci line {line_no}")
                })
                .kind;
            (
                text.to_string(),
                highlight_snapshot(styled.highlights.as_ref()),
                kind,
            )
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "deployment-ci split render text should match cache for line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "deployment-ci split render highlights should match cache for line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, FileDiffRowKind::Add | FileDiffRowKind::Modify),
            "deployment-ci split render should preserve diff background for line {line_no}"
        );
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let inline_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        new_lines
            .iter()
            .copied()
            .map(|line_no| {
                inline_visible_ix_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected inline visible row for deployment-ci line {line_no}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, inline_visible_indices.as_slice());

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        for line_no in new_lines {
            let actual = inline_cached_styled_by_new_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected fallback inline styled text for deployment-ci line {line_no}")
                });
            let expected = baseline_new_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for deployment-ci line {line_no}"));
            assert_eq!(
                actual, expected,
                "fallback inline YAML highlighting should match prepared baseline for deployment-ci line {line_no}"
            );
        }
    });

    for (&line_no, &visible_ix) in new_lines.iter().zip(inline_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::Inline);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            let (text, styled) =
                inline_cached_styled_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected cached inline styled text for deployment-ci line {line_no}")
                });
            let kind = inline_row_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("expected inline row for deployment-ci line {line_no}"))
                .kind;
            (
                text.to_string(),
                highlight_snapshot(styled.highlights.as_ref()),
                kind,
            )
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "deployment-ci inline render text should match cache for line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "deployment-ci inline render highlights should match cache for line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, DiffLineKind::Add | DiffLineKind::Remove),
            "deployment-ci inline render should preserve diff background for line {line_no}"
        );
    }

    assert_eq!(
        baseline_old_by_line.len(),
        old_lines.len(),
        "old-side YAML baselines should be materialized for the deployment-ci fixture"
    );
}

#[gpui::test]
fn yaml_file_diff_keeps_consistent_highlighting_for_build_release_artifacts(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;

    fn split_right_row_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn split_left_row_by_old_line(
        pane: &DiffRenderer,
        old_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.old_line == Some(old_line))
    }

    fn split_right_cached_styled_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn split_left_cached_styled_by_old_line(
        pane: &DiffRenderer,
        old_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.old_line == Some(old_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.old.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitLeft)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn inline_row_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn mapping_key_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let key_start = text.find(|ch: char| !ch.is_ascii_whitespace())?;
        let key_end = text[key_start..].find(':')?.saturating_add(key_start);
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none() && range.start <= key_start && range.end >= key_end)
                .then_some(color)
        })
    }

    fn scalar_color_after_colon(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let value_start = text.find(':')?.checked_add(1).and_then(|start| {
            text[start..]
                .find(|ch: char| !ch.is_ascii_whitespace())
                .map(|offset| start.saturating_add(offset))
        })?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none()
                && range.start <= value_start
                && range.end > value_start)
                .then_some(color)
        })
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    fn expected_yaml_snapshot(
        theme: AppTheme,
        text: &str,
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlight_snapshot(
            rows::syntax_highlights_for_line(
                theme,
                text,
                rows::DiffSyntaxLanguage::Yaml,
                rows::DiffSyntaxMode::Auto,
            )
            .as_slice(),
        )
    }

    fn line_debug(
        line: Option<(&str, &super::CachedDiffStyledText)>,
    ) -> Option<(
        String,
        Vec<(
            std::ops::Range<usize>,
            Option<gpui::Hsla>,
            Option<gpui::Hsla>,
        )>,
    )> {
        let (text, styled) = line?;
        Some((
            text.to_string(),
            styled
                .highlights
                .iter()
                .map(|(range, style)| (range.clone(), style.color, style.background_color))
                .collect(),
        ))
    }

    fn split_debug(
        pane: &DiffRenderer,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    line_debug(split_right_cached_styled_by_new_line(pane, line)),
                )
            })
            .collect()
    }

    fn inline_debug(
        pane: &DiffRenderer,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    line_debug(inline_cached_styled_by_new_line(pane, line)),
                )
            })
            .collect()
    }

    fn split_kind_debug(pane: &DiffRenderer, lines: &[u32]) -> Vec<(u32, Option<FileDiffRowKind>)> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    split_right_row_by_new_line(pane, line).map(|row| row.kind),
                )
            })
            .collect()
    }

    fn inline_kind_debug(pane: &DiffRenderer, lines: &[u32]) -> Vec<(u32, Option<DiffLineKind>)> {
        lines
            .iter()
            .copied()
            .map(|line| (line, inline_row_by_new_line(pane, line).map(|row| row.kind)))
            .collect()
    }

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let theme = cx.update(|_window, app| view.read(app).diff_renderer.read(app).theme);

    let repo_id = gitcomet_state::model::RepoId(84);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_build_release_file_diff",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/build-release-artifacts.yml");
    let old_text = BUILD_RELEASE_ARTIFACTS.old_text.to_owned();
    let new_text = BUILD_RELEASE_ARTIFACTS.new_text.to_owned();

    let baseline_secret_key_line = 20u32;
    let affected_secret_key_lines = [22u32, 24, 26, 28, 30, 32];
    let baseline_required_line = 21u32;
    let affected_required_lines = [23u32, 25, 27, 29, 31, 33];
    let add_lines = [20u32, 21u32];
    let context_lines = [22u32, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33];
    let old_baseline_secret_key_line = 20u32;
    let old_affected_secret_key_lines = [22u32, 24, 26, 28, 30];
    let old_baseline_required_line = 21u32;
    let old_affected_required_lines = [23u32, 25, 27, 29, 31];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_millis(50),
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 0, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release YAML file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 0
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(33))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(33))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} rows={} inline_rows={} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release YAML file-diff split syntax keeps repeated secret keys and booleans consistent",
        |pane| {
            let Some((baseline_secret_key_text, baseline_secret_key_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_secret_key_line)
            else {
                return false;
            };
            let Some(baseline_secret_key_color) =
                mapping_key_color(baseline_secret_key_styled, baseline_secret_key_text)
            else {
                return false;
            };
            if add_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Add)
            }) {
                return false;
            }
            if context_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Context)
            }) {
                return false;
            }
            if affected_secret_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_secret_key_color)
            }) {
                return false;
            }

            let Some((baseline_required_text, baseline_required_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_required_line)
            else {
                return false;
            };
            let Some(baseline_required_color) =
                scalar_color_after_colon(baseline_required_styled, baseline_required_text)
            else {
                return false;
            };
            !affected_required_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                scalar_color_after_colon(styled, text) != Some(baseline_required_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_secret_key_line);
            lines.extend(affected_secret_key_lines);
            lines.push(baseline_required_line);
            lines.extend(affected_required_lines);
            format!(
                "diff_view={:?} split_kinds={:?} split_debug={:?}",
                pane.diff_view,
                split_kind_debug(pane, &lines),
                split_debug(pane, &lines),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        let mut old_lines = Vec::new();
        old_lines.push(old_baseline_secret_key_line);
        old_lines.extend(old_affected_secret_key_lines);
        old_lines.push(old_baseline_required_line);
        old_lines.extend(old_affected_required_lines);

        for old_line in old_lines {
            let Some(row) = split_left_row_by_old_line(pane, old_line) else {
                panic!("expected split-left row for old line {old_line}");
            };
            assert_eq!(
                row.kind,
                FileDiffRowKind::Context,
                "expected build-release old line {old_line} to remain a context row on the left side"
            );
            let Some((text, styled)) = split_left_cached_styled_by_old_line(pane, old_line) else {
                panic!("expected cached split-left styled text for old line {old_line}");
            };
            let expected = expected_yaml_snapshot(theme, text);
            let actual = highlight_snapshot(styled.highlights.as_ref());
            assert_eq!(
                actual, expected,
                "split-left YAML highlighting should match direct single-line YAML highlights for build-release old line {old_line}: text={text:?}"
            );
        }
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release YAML file-diff inline syntax keeps repeated secret keys and booleans consistent",
        |pane| {
            let Some((baseline_secret_key_text, baseline_secret_key_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_secret_key_line)
            else {
                return false;
            };
            let Some(baseline_secret_key_color) =
                mapping_key_color(baseline_secret_key_styled, baseline_secret_key_text)
            else {
                return false;
            };
            if add_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Add)
            }) {
                return false;
            }
            if context_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Context)
            }) {
                return false;
            }
            if affected_secret_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_secret_key_color)
            }) {
                return false;
            }

            let Some((baseline_required_text, baseline_required_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_required_line)
            else {
                return false;
            };
            let Some(baseline_required_color) =
                scalar_color_after_colon(baseline_required_styled, baseline_required_text)
            else {
                return false;
            };
            !affected_required_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                scalar_color_after_colon(styled, text) != Some(baseline_required_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_secret_key_line);
            lines.extend(affected_secret_key_lines);
            lines.push(baseline_required_line);
            lines.extend(affected_required_lines);
            format!(
                "diff_view={:?} inline_kinds={:?} inline_debug={:?}",
                pane.diff_view,
                inline_kind_debug(pane, &lines),
                inline_debug(pane, &lines),
            )
        },
    );
}

#[gpui::test]
fn yaml_file_diff_matches_prepared_document_for_build_release_artifacts(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;
    use std::collections::BTreeMap;

    fn split_right_cached_styled_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn split_left_cached_styled_by_old_line(
        pane: &DiffRenderer,
        old_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.old_line == Some(old_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.old.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitLeft)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn split_right_row_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn split_left_row_by_old_line(
        pane: &DiffRenderer,
        old_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.old_line == Some(old_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn inline_row_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn split_visible_ix_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn draw_rows_for_visible_indices(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_indices: &[usize],
    ) {
        for &visible_ix in visible_indices {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.diff_renderer.update(cx, |pane, cx| {
                        pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                        cx.notify();
                    });
                });
            });
            cx.run_until_parked();
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
        }
    }

    fn cached_snapshot(line: (&str, &super::CachedDiffStyledText)) -> LineSyntaxSnapshot {
        let (text, styled) = line;
        LineSyntaxSnapshot {
            text: text.to_string(),
            syntax: styled
                .highlights
                .iter()
                .filter(|(_, style)| style.background_color.is_none())
                .map(|(range, style)| (range.clone(), style.color))
                .collect(),
        }
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.diff_selection_anchor = None;
                    pane.diff_selection_range = None;
                    pane.diff_autoscroll_pending = false;
                    pane.clear_diff_text_selection();
                    pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                    cx.notify();
                });
            });
        });
        cx.run_until_parked();

        cx.update(|window, app| {
            rows::clear_diff_paint_log_for_tests();
            let _ = window.draw(app);
            rows::diff_paint_log_for_tests()
                .into_iter()
                .find(|record| record.visible_ix == visible_ix && record.region == region)
                .unwrap_or_else(|| {
                    panic!("expected paint record for visible_ix={visible_ix} region={region:?}")
                })
        })
    }

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let theme = cx.update(|_window, app| view.read(app).diff_renderer.read(app).theme);

    let repo_id = gitcomet_state::model::RepoId(184);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_build_release_prepared_baseline",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/build-release-artifacts.yml");
    let old_text = BUILD_RELEASE_ARTIFACTS.old_text.to_owned();
    let new_text = BUILD_RELEASE_ARTIFACTS.new_text.to_owned();
    let (old_shared_text, old_line_starts) = shared_text_and_line_starts(old_text.as_str());
    let (new_shared_text, new_line_starts) = shared_text_and_line_starts(new_text.as_str());
    let old_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        old_shared_text,
        Arc::clone(&old_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared old YAML baseline document, got {other:?}"),
    };
    let new_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        new_shared_text,
        Arc::clone(&new_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared new YAML baseline document, got {other:?}"),
    };

    let old_lines = [20u32, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31];
    let new_lines = [20u32, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33];
    let baseline_old_by_line = old_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                old_text.as_str(),
                old_line_starts.as_ref(),
                old_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for old line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();
    let baseline_new_by_line = new_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                new_text.as_str(),
                new_line_starts.as_ref(),
                new_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for new line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_secs(1),
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 1, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release YAML rows ready for prepared-baseline comparison",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(33))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(33))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?} rows={} inline_rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        for line_no in old_lines {
            let actual = split_left_cached_styled_by_old_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected split-left styled text for build-release old line {line_no}")
                });
            let expected = baseline_old_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for build-release old line {line_no}"));
            assert_eq!(
                actual, expected,
                "split-left YAML highlighting should match prepared baseline for build-release old line {line_no}"
            );
        }

        for line_no in new_lines {
            let actual = split_right_cached_styled_by_new_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected split-right styled text for build-release new line {line_no}")
                });
            let expected = baseline_new_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for build-release new line {line_no}"));
            assert_eq!(
                actual, expected,
                "split-right YAML highlighting should match prepared baseline for build-release new line {line_no}"
            );
        }
    });

    let split_left_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        old_lines
            .iter()
            .copied()
            .map(|line_no| {
                (0..pane.diff_visible_len())
                    .find(|&visible_ix| {
                        let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                            return false;
                        };
                        pane.file_diff_split_row(row_ix)
                            .is_some_and(|row| row.old_line == Some(line_no))
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "expected split-left visible row for build-release old line {line_no}"
                        )
                    })
            })
            .collect::<Vec<_>>()
    });
    let split_right_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        new_lines
            .iter()
            .copied()
            .map(|line_no| {
                split_visible_ix_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected split-right visible row for build-release new line {line_no}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, split_left_visible_indices.as_slice());
    draw_rows_for_visible_indices(cx, &view, split_right_visible_indices.as_slice());

    for (&line_no, &visible_ix) in old_lines.iter().zip(split_left_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::SplitLeft);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            let (text, styled) = split_left_cached_styled_by_old_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected cached split-left styled text for build-release old line {line_no}")
                });
            let kind = split_left_row_by_old_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected split-left row for build-release old line {line_no}")
                })
                .kind;
            (text.to_string(), highlight_snapshot(styled.highlights.as_ref()), kind)
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "build-release split-left render text should match cache for old line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "build-release split-left render highlights should match cache for old line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, FileDiffRowKind::Remove | FileDiffRowKind::Modify),
            "build-release split-left render should preserve diff background for old line {line_no}"
        );
    }

    for (&line_no, &visible_ix) in new_lines.iter().zip(split_right_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::SplitRight);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected cached split-right styled text for build-release new line {line_no}")
                });
            let kind = split_right_row_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected split-right row for build-release new line {line_no}")
                })
                .kind;
            (text.to_string(), highlight_snapshot(styled.highlights.as_ref()), kind)
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "build-release split-right render text should match cache for new line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "build-release split-right render highlights should match cache for new line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, FileDiffRowKind::Add | FileDiffRowKind::Modify),
            "build-release split-right render should preserve diff background for new line {line_no}"
        );
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let inline_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        new_lines
            .iter()
            .copied()
            .map(|line_no| {
                inline_visible_ix_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected inline visible row for build-release new line {line_no}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, inline_visible_indices.as_slice());

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        for line_no in new_lines {
            let actual = inline_cached_styled_by_new_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected inline styled text for build-release new line {line_no}")
                });
            let expected = baseline_new_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for build-release new line {line_no}"));
            assert_eq!(
                actual, expected,
                "inline YAML highlighting should match prepared baseline for build-release new line {line_no}"
            );
        }
    });

    for (&line_no, &visible_ix) in new_lines.iter().zip(inline_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::Inline);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            let (text, styled) =
                inline_cached_styled_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!(
                        "expected cached inline styled text for build-release new line {line_no}"
                    )
                });
            let kind = inline_row_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected inline row for build-release new line {line_no}")
                })
                .kind;
            (
                text.to_string(),
                highlight_snapshot(styled.highlights.as_ref()),
                kind,
            )
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "build-release inline render text should match cache for new line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "build-release inline render highlights should match cache for new line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, DiffLineKind::Add | DiffLineKind::Remove),
            "build-release inline render should preserve diff background for new line {line_no}"
        );
    }
}

#[allow(dead_code)]
fn yaml_same_content_rev_refresh_invalidates_cached_heuristic_file_diff_rows(
    cx: &mut gpui::TestAppContext,
) {
    use std::collections::BTreeMap;

    fn split_right_cached_styled_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn inline_cached_styled_by_new_line(
        pane: &DiffRenderer,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn split_visible_ix_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn draw_rows_for_visible_indices(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_indices: &[usize],
    ) {
        for &visible_ix in visible_indices {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.diff_renderer.update(cx, |pane, cx| {
                        pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                        cx.notify();
                    });
                });
            });
            cx.run_until_parked();
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
        }
    }

    fn cached_snapshot(line: (&str, &super::CachedDiffStyledText)) -> LineSyntaxSnapshot {
        let (text, styled) = line;
        LineSyntaxSnapshot {
            text: text.to_string(),
            syntax: styled
                .highlights
                .iter()
                .filter(|(_, style)| style.background_color.is_none())
                .map(|(range, style)| (range.clone(), style.color))
                .collect(),
        }
    }

    fn paint_snapshot(record: &rows::DiffPaintRecord) -> LineSyntaxSnapshot {
        LineSyntaxSnapshot {
            text: record.text.to_string(),
            syntax: record
                .highlights
                .iter()
                .filter(|(_, _, bg)| bg.is_none())
                .map(|(range, color, _)| (range.clone(), *color))
                .collect(),
        }
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.diff_selection_anchor = None;
                    pane.diff_selection_range = None;
                    pane.diff_autoscroll_pending = false;
                    pane.clear_diff_text_selection();
                    pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                    cx.notify();
                });
            });
        });
        cx.run_until_parked();

        cx.update(|window, app| {
            rows::clear_diff_paint_log_for_tests();
            let _ = window.draw(app);
            rows::diff_paint_log_for_tests()
                .into_iter()
                .find(|record| record.visible_ix == visible_ix && record.region == region)
                .unwrap_or_else(|| {
                    panic!("expected paint record for visible_ix={visible_ix} region={region:?}")
                })
        })
    }

    fn split_mismatch_lines(
        pane: &DiffRenderer,
        baselines: &BTreeMap<u32, LineSyntaxSnapshot>,
        lines: &[u32],
    ) -> Vec<u32> {
        lines
            .iter()
            .copied()
            .filter(|line| {
                let Some(actual) =
                    split_right_cached_styled_by_new_line(pane, *line).map(cached_snapshot)
                else {
                    return false;
                };
                baselines
                    .get(line)
                    .is_some_and(|expected| actual != *expected)
            })
            .collect()
    }

    fn inline_mismatch_lines(
        pane: &DiffRenderer,
        baselines: &BTreeMap<u32, LineSyntaxSnapshot>,
        lines: &[u32],
    ) -> Vec<u32> {
        lines
            .iter()
            .copied()
            .filter(|line| {
                let Some(actual) =
                    inline_cached_styled_by_new_line(pane, *line).map(cached_snapshot)
                else {
                    return false;
                };
                baselines
                    .get(line)
                    .is_some_and(|expected| actual != *expected)
            })
            .collect()
    }

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let theme = cx.update(|_window, app| view.read(app).diff_renderer.read(app).theme);
    let repo_id = gitcomet_state::model::RepoId(87);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_same_content_rev_refresh",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/build-release-artifacts.yml");
    fn append_yaml_padding(text: &str) -> String {
        use std::fmt::Write as _;

        const PADDING_LINES: usize = 65_536;
        let mut out = String::with_capacity(text.len().saturating_add(PADDING_LINES * 64));
        out.push_str(text);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        for ix in 0..PADDING_LINES {
            let _ = writeln!(
                out,
                "# syntax-padding-{ix:05}-abcdefghijklmnopqrstuvwxyz0123456789"
            );
        }
        out
    }

    let old_text = append_yaml_padding(BUILD_RELEASE_ARTIFACTS.old_text);
    let new_text = append_yaml_padding(BUILD_RELEASE_ARTIFACTS.new_text);
    let affected_lines = [173u32, 175, 176, 183, 190, 193, 206, 212, 218, 221];
    let (new_shared_text, new_line_starts) = shared_text_and_line_starts(new_text.as_str());
    let new_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        new_shared_text,
        Arc::clone(&new_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(5),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!(
            "expected prepared YAML baseline document for same-content refresh, got {other:?}"
        ),
    };
    let baseline_new_by_line = affected_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                new_text.as_str(),
                new_line_starts.as_ref(),
                new_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| {
                panic!("expected prepared YAML baseline for build-release line {line_no}")
            });
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 1, &old_text, &new_text);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release file-diff rows ready before same-content refresh",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 1
                && affected_lines
                    .iter()
                    .copied()
                    .all(|line| split_visible_ix_by_new_line(pane, line).is_some())
        },
        |pane| {
            let split_mismatches =
                split_mismatch_lines(pane, &baseline_new_by_line, &affected_lines);
            let first_mismatch = split_mismatches.first().copied();
            let cache_row_ix = first_mismatch.and_then(|line_no| {
                pane.file_diff_cache_rows
                    .iter()
                    .position(|row| row.new_line == Some(line_no))
            });
            let provider_row_ix = first_mismatch.and_then(|line_no| {
                (0..pane.file_diff_split_row_len()).find(|&row_ix| {
                    pane.file_diff_split_row(row_ix)
                        .is_some_and(|row| row.new_line == Some(line_no))
                })
            });
            let actual = first_mismatch.and_then(|line_no| {
                split_right_cached_styled_by_new_line(pane, line_no).map(cached_snapshot)
            });
            let cached_text = cache_row_ix.and_then(|row_ix| {
                let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
                let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
                pane.diff_text_segments_cache_get(key, epoch)
                    .map(|styled| styled.text.to_string())
            });
            let expected =
                first_mismatch.and_then(|line_no| baseline_new_by_line.get(&line_no).cloned());
            let doc_actual = pane
                .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .and_then(|document| {
                    first_mismatch.and_then(|line_no| {
                        prepared_document_snapshot_for_line(
                            theme,
                            new_text.as_str(),
                            new_line_starts.as_ref(),
                            document,
                            rows::DiffSyntaxLanguage::Yaml,
                            line_no,
                        )
                    })
                });
            format!(
                "rev={} inflight={:?} right_doc={:?} split_epoch={} split_mismatches={split_mismatches:?} first_mismatch={first_mismatch:?} cache_row_ix={cache_row_ix:?} provider_row_ix={provider_row_ix:?} cached_text={cached_text:?} actual={actual:?} doc_actual={doc_actual:?} expected={expected:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            )
        },
    );

    let split_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        affected_lines
            .iter()
            .copied()
            .map(|line| {
                split_visible_ix_by_new_line(pane, line).unwrap_or_else(|| {
                    panic!("expected split visible row for build-release line {line}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, split_visible_indices.as_slice());

    let (epoch_before, right_doc_ready_before, heuristic_mismatches) = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        (
            pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_some(),
            split_mismatch_lines(pane, &baseline_new_by_line, &affected_lines),
        )
    });
    if !right_doc_ready_before {
        assert!(
            !heuristic_mismatches.is_empty(),
            "expected at least one build-release YAML block-scalar line to differ while only heuristic styling is cached"
        );
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_millis(500),
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 2, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release file-diff rows ready after same-content refresh",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 2
                && affected_lines
                    .iter()
                    .copied()
                    .all(|line| split_visible_ix_by_new_line(pane, line).is_some())
        },
        |pane| {
            let split_mismatches =
                split_mismatch_lines(pane, &baseline_new_by_line, &affected_lines);
            let first_mismatch = split_mismatches.first().copied();
            let actual = first_mismatch.and_then(|line_no| {
                split_right_cached_styled_by_new_line(pane, line_no).map(cached_snapshot)
            });
            let expected =
                first_mismatch.and_then(|line_no| baseline_new_by_line.get(&line_no).cloned());
            let doc_actual = pane
                .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .and_then(|document| {
                    first_mismatch.and_then(|line_no| {
                        prepared_document_snapshot_for_line(
                            theme,
                            new_text.as_str(),
                            new_line_starts.as_ref(),
                            document,
                            rows::DiffSyntaxLanguage::Yaml,
                            line_no,
                        )
                    })
                });
            format!(
                "rev={} inflight={:?} right_doc={:?} split_epoch={} split_mismatches={split_mismatches:?} first_mismatch={first_mismatch:?} actual={actual:?} doc_actual={doc_actual:?} expected={expected:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            )
        },
    );
    draw_rows_for_visible_indices(cx, &view, split_visible_indices.as_slice());

    wait_for_main_pane_condition(
        cx,
        &view,
        "same-content file-diff rev refresh should expose the build-release right document",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 2
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && (right_doc_ready_before
                    || pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight)
                        > epoch_before)
        },
        |pane| {
            format!(
                "rev={} inflight={:?} right_doc={:?} split_epoch={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            )
        },
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "same-content file-diff rev refresh should finish build-release right-doc chunk requests",
        |pane| {
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
        },
        |pane| {
            let right_doc =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
            format!(
                "rev={} right_doc={right_doc:?} right_pending={:?} split_mismatches={:?}",
                pane.file_diff_cache_rev,
                right_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                split_mismatch_lines(pane, &baseline_new_by_line, &affected_lines),
            )
        },
    );
    draw_rows_for_visible_indices(cx, &view, split_visible_indices.as_slice());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.run_until_parked();

    for (&line_no, &visible_ix) in affected_lines.iter().zip(split_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::SplitRight);
        let cached = cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            split_right_cached_styled_by_new_line(pane, line_no).map(cached_snapshot)
        });
        let expected = baseline_new_by_line
            .get(&line_no)
            .unwrap_or_else(|| panic!("missing build-release baseline for line {line_no}"));
        assert_eq!(
            cached,
            Some(expected.clone()),
            "diagnostic: split-right cache should match the prepared baseline after painting line {line_no}"
        );
        let actual = paint_snapshot(&record);
        assert_eq!(
            actual, *expected,
            "same-content refresh should repaint split-right build-release YAML highlighting for line {line_no}"
        );

        let expects_row_bg = cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            (0..pane.file_diff_split_row_len()).any(|row_ix| {
                pane.file_diff_split_row(row_ix).is_some_and(|row| {
                    row.new_line == Some(line_no)
                        && matches!(
                            row.kind,
                            gitcomet_core::file_diff::FileDiffRowKind::Add
                                | gitcomet_core::file_diff::FileDiffRowKind::Modify
                        )
                })
            })
        });
        assert_eq!(
            record.row_bg.is_some(),
            expects_row_bg,
            "same-content refresh should preserve split-right diff background for line {line_no}"
        );
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    let inline_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        affected_lines
            .iter()
            .copied()
            .map(|line| {
                inline_visible_ix_by_new_line(pane, line).unwrap_or_else(|| {
                    panic!("expected inline visible row for build-release line {line}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, inline_visible_indices.as_slice());

    wait_for_main_pane_condition(
        cx,
        &view,
        "same-content file-diff rev refresh should expose inline build-release rows",
        |pane| {
            pane.file_diff_cache_rev == 2
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "rev={} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );
    draw_rows_for_visible_indices(cx, &view, inline_visible_indices.as_slice());

    for (&line_no, &visible_ix) in affected_lines.iter().zip(inline_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::Inline);
        let expected = baseline_new_by_line
            .get(&line_no)
            .unwrap_or_else(|| panic!("missing build-release baseline for line {line_no}"));
        let actual = paint_snapshot(&record);
        assert_eq!(
            actual, *expected,
            "same-content refresh should repaint inline build-release YAML highlighting for line {line_no}"
        );

        let expects_row_bg = cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            (0..pane.file_diff_inline_row_len()).any(|inline_ix| {
                pane.file_diff_inline_row(inline_ix).is_some_and(|line| {
                    line.new_line == Some(line_no)
                        && line.kind == gitcomet_core::domain::DiffLineKind::Add
                })
            })
        });
        assert_eq!(
            record.row_bg.is_some(),
            expects_row_bg,
            "same-content refresh should preserve inline diff background for line {line_no}"
        );
    }
}
