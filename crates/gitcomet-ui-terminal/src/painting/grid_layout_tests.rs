use super::*;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::term::cell::{Cell, Flags};

#[gpui::test]
fn ascii_with_style_changes_keeps_each_glyph_on_its_cell(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let cx = cx.add_empty_window();
    cx.update(|window, _| {
        for family in ["Lilex", "IBM Plex Sans"] {
            for text in [
                "GitComet-terminal-output",
                "AV To 1234 /src/main.rs",
                "          ",
            ] {
                let cells: Vec<_> = text
                    .chars()
                    .enumerate()
                    .map(|(col, c)| {
                        let mut cell = Cell {
                            c,
                            ..Cell::default()
                        };
                        if col % 3 == 0 {
                            cell.flags.insert(Flags::BOLD);
                        }
                        IndexedCell {
                            point: AlacPoint::new(Line(0), Column(col)),
                            cell,
                        }
                    })
                    .collect();
                let style = gpui::TextStyle {
                    font_family: family.into(),
                    ..Default::default()
                };
                let (text, runs, _) =
                    build_alacritty_row(&cells, 0, cells.len(), &style, AppTheme::gitcomet_dark());
                let shaped = shape_terminal_grid_line(
                    text,
                    &runs,
                    &cells,
                    0,
                    cells.len(),
                    px(12.),
                    px(16.),
                    window,
                );
                let mut positions: Vec<_> = shaped
                    .paint_fragments
                    .iter()
                    .flat_map(|f| f.glyphs.iter())
                    .map(|g| g.position.x)
                    .collect();
                positions.sort_by(|a, b| f32::from(*a).total_cmp(&f32::from(*b)));
                assert_eq!(
                    positions,
                    (0..cells.len())
                        .map(|i| px(i as f32 * 16.))
                        .collect::<Vec<_>>(),
                    "{family}"
                );
            }
        }
    });
}

#[gpui::test]
fn wide_and_combining_cells_keep_the_authoritative_grid(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let cx = cx.add_empty_window();
    cx.update(|window, _| {
        let mut wide = Cell {
            c: '日',
            ..Cell::default()
        };
        wide.flags.insert(Flags::WIDE_CHAR);
        let mut spacer = Cell::default();
        spacer.flags.insert(Flags::WIDE_CHAR_SPACER);
        let cells = vec![
            IndexedCell {
                point: AlacPoint::new(Line(0), Column(0)),
                cell: Cell {
                    c: 'a',
                    ..Cell::default()
                },
            },
            IndexedCell {
                point: AlacPoint::new(Line(0), Column(1)),
                cell: wide,
            },
            IndexedCell {
                point: AlacPoint::new(Line(0), Column(2)),
                cell: spacer,
            },
            IndexedCell {
                point: AlacPoint::new(Line(0), Column(3)),
                cell: Cell {
                    c: 'b',
                    ..Cell::default()
                },
            },
            IndexedCell {
                point: AlacPoint::new(Line(0), Column(4)),
                cell: Cell {
                    c: 'é',
                    ..Cell::default()
                },
            },
        ];
        let style = gpui::TextStyle {
            font_family: "Lilex".into(),
            ..gpui::TextStyle::default()
        };
        let (text, runs, _) = build_alacritty_row(&cells, 0, 5, &style, AppTheme::gitcomet_dark());
        let line = shape_terminal_grid_line(text, &runs, &cells, 0, 5, px(12.0), px(16.0), window);
        let positions: Vec<_> = line
            .paint_fragments
            .iter()
            .flat_map(|fragment| fragment.glyphs.iter())
            .map(|glyph| glyph.position.x)
            .collect();
        assert_eq!(positions, [px(0.0), px(16.0), px(48.0), px(64.0)]);
        assert_eq!(line.width, px(80.0));

        let mut accent = Cell {
            c: 'e',
            ..Cell::default()
        };
        accent.push_zerowidth('\u{301}');
        let cells = vec![
            IndexedCell {
                point: AlacPoint::new(Line(0), Column(0)),
                cell: accent,
            },
            IndexedCell {
                point: AlacPoint::new(Line(0), Column(1)),
                cell: Cell {
                    c: 'b',
                    ..Cell::default()
                },
            },
        ];
        let (text, runs, _) = build_alacritty_row(&cells, 0, 2, &style, AppTheme::gitcomet_dark());
        let line = shape_terminal_grid_line(text, &runs, &cells, 0, 2, px(12.0), px(16.0), window);
        let positions: Vec<_> = line
            .paint_fragments
            .iter()
            .flat_map(|fragment| fragment.glyphs.iter())
            .map(|glyph| glyph.position.x)
            .collect();
        assert_eq!(
            positions.last(),
            Some(&px(16.0)),
            "combining marks do not consume a cell"
        );
        assert_eq!(line.width, px(32.0));
    });
}
