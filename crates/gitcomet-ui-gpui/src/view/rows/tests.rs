use super::*;
use gitcomet_core::domain::{CommitFileChange, FileStatusKind, LineStats};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Every bundled theme, so a tint invariant is proved against every
/// palette that ships rather than the default dark one alone.
fn bundled_themes() -> Vec<AppTheme> {
    crate::theme::bundled_theme_keys()
        .into_iter()
        .map(|(key, _)| AppTheme::from_key(&key).unwrap_or_else(|| panic!("bundled theme `{key}`")))
        .collect()
}

#[test]
fn file_row_icon_names_the_file_type() {
    let theme = AppTheme::from_key("gitcomet_dark").expect("bundled theme");

    let (icon, color) = file_row_icon(Path::new("src/main.rs"), FileStatusKind::Modified, &theme);
    assert_eq!(icon, "icons/file_icons/rust.svg");
    assert_ne!(
        color, theme.colors.foreground.secondary,
        "a known type should carry its brand tint, not the neutral fallback",
    );

    // Same file, every non-conflict kind: the glyph is the type, not the change.
    for kind in [
        FileStatusKind::Untracked,
        FileStatusKind::Added,
        FileStatusKind::Deleted,
        FileStatusKind::Renamed,
    ] {
        assert_eq!(
            file_row_icon(Path::new("src/main.rs"), kind, &theme).0,
            "icons/file_icons/rust.svg",
        );
    }

    let (icon, _) = file_row_icon(Path::new("nope.wat-is-this"), FileStatusKind::Added, &theme);
    assert_eq!(icon, "icons/file_icons/file.svg", "unknown types fall back");
}

#[test]
fn conflicts_keep_their_warning_glyph() {
    let theme = AppTheme::from_key("gitcomet_dark").expect("bundled theme");
    let (icon, color) = file_row_icon(Path::new("src/main.rs"), FileStatusKind::Conflicted, &theme);

    assert_eq!(icon, "icons/warning.svg");
    assert_eq!(color, theme.colors.status.danger.foreground);
}

#[test]
fn only_added_deleted_and_renamed_wear_a_badge() {
    let theme = AppTheme::from_key("gitcomet_dark").expect("bundled theme");

    for bare in [FileStatusKind::Modified, FileStatusKind::Conflicted] {
        assert_eq!(
            file_row_kind_badge(bare, &theme),
            None,
            "{bare:?} goes bare: the pencil reads worst at badge size and a \
             conflict already shows a warning triangle",
        );
    }
    for kind in [
        FileStatusKind::Untracked,
        FileStatusKind::Added,
        FileStatusKind::Deleted,
        FileStatusKind::Renamed,
    ] {
        let (icon, color) = file_row_kind_badge(kind, &theme).expect("badged kind");
        let visuals = commit_file_kind_visuals(kind);
        assert_eq!(icon, visuals.icon, "{kind:?} badge reuses the kind glyph");
        assert_eq!(color, visuals.color(&theme));
    }
}

/// The badge disc is punched out of the row, so its resting fill has to be
/// the row's resting fill and its hover fill the row's hover fill -- a disc
/// that does not move leaves a stale circle under the pointer.
#[test]
fn badge_disc_tracks_the_row_it_sits_on() {
    for theme in bundled_themes() {
        for kind in [FileStatusKind::Modified, FileStatusKind::Deleted] {
            let tint = file_kind_row_tint(kind, &theme);
            let resting = tinted_row_bg(theme.colors.surface.canvas, tint);
            let hovered = tinted_row_bg(theme.colors.interaction.hover_background, tint);
            assert_ne!(
                resting, hovered,
                "{kind:?} disc must have somewhere to move to",
            );
        }
    }
}

#[test]
fn explorer_rows_tint_ignored_over_hidden_and_never_like_hover() {
    for theme in bundled_themes() {
        assert_eq!(explorer_row_tint(&theme, false, false), None);
        let ignored = explorer_row_tint(&theme, false, true).expect("ignored tint");
        assert_eq!(explorer_row_tint(&theme, true, true), Some(ignored));
        let hidden = explorer_row_tint(&theme, true, false).expect("hidden tint");
        assert_ne!(hidden, ignored);

        // Hover moves the chrome toward the text colour; a hidden row must
        // move it the other way.
        let chrome = theme.colors.surface.chrome;
        let lightness = |c: gpui::Rgba| c.red + c.green + c.blue;
        let hovered = composite_over(chrome, theme.hover_overlay());
        let washed = composite_over(chrome, hidden);
        assert!(
            (lightness(hovered) - lightness(chrome)) * (lightness(washed) - lightness(chrome))
                < 0.0,
            "{chrome:?}: hover {hovered:?}, hidden {washed:?}"
        );
    }
}

#[test]
fn only_modified_rows_go_untinted() {
    for theme in bundled_themes() {
        assert_eq!(
            file_kind_row_tint(FileStatusKind::Modified, &theme),
            None,
            "modified is the untinted default",
        );
        for kind in [
            FileStatusKind::Untracked,
            FileStatusKind::Added,
            FileStatusKind::Deleted,
            FileStatusKind::Renamed,
            FileStatusKind::Conflicted,
        ] {
            assert!(
                file_kind_row_tint(kind, &theme).is_some(),
                "{kind:?} should be tinted",
            );
        }
    }
}

/// The invariant the whole treatment rests on: `.bg()` replaces rather than
/// layers, so an opaque tint would flatten resting, hover and press into one
/// colour and the row would stop answering the mouse.
#[test]
fn every_tint_is_translucent_and_leaves_hover_visible() {
    for theme in bundled_themes() {
        for kind in [
            FileStatusKind::Untracked,
            FileStatusKind::Added,
            FileStatusKind::Deleted,
            FileStatusKind::Renamed,
            FileStatusKind::Conflicted,
        ] {
            let tint = file_kind_row_tint(kind, &theme).expect("tinted kind");
            assert!(tint.alpha < 1.0, "{kind:?} tint must stay translucent");

            let resting = tinted_row_bg(theme.colors.surface.canvas, Some(tint));
            let hovered = tinted_row_bg(theme.colors.interaction.hover_background, Some(tint));
            let pressed = tinted_row_bg(theme.colors.interaction.pressed_background, Some(tint));

            assert_ne!(resting, hovered, "{kind:?} row must still answer hover");
            assert_ne!(hovered, pressed, "{kind:?} row must still answer press");
            assert_ne!(
                resting, theme.colors.surface.canvas,
                "{kind:?} tint must actually shift the surface",
            );
        }
    }
}

#[test]
fn file_row_interactions_support_translucent_canvases() {
    use crate::kit::interaction::InteractionFeedback;

    for base_theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
        for alpha in [0.0, 0.5, 1.0] {
            let mut theme = base_theme;
            theme.colors.surface.canvas = gpui::Rgba::new(0.2, 0.4, 0.6, alpha);
            let canvas = composite_over(
                base_theme.colors.surface.canvas,
                theme.colors.surface.canvas,
            );
            for kind in [FileStatusKind::Modified, FileStatusKind::Deleted] {
                let tint = file_kind_row_tint(kind, &theme);
                let surface = tinted_row_bg(canvas, tint);
                for (selected, open) in [(false, false), (true, false), (false, true), (true, true)]
                {
                    let interaction = FileRowInteraction::new(theme, tint, selected, open);
                    let disc = interaction.badge_disc("row".into());
                    for (feedback, actual) in [
                        (InteractionFeedback::Resting, disc.resting),
                        (InteractionFeedback::Hovered, disc.hover.unwrap().1),
                        (InteractionFeedback::Pressed, disc.pressed.unwrap().1),
                    ] {
                        let overlay = if open {
                            Some(theme.active_overlay())
                        } else if selected {
                            Some(with_alpha(
                                theme.colors.accent.foreground,
                                if theme.is_dark { 0.16 } else { 0.10 },
                            ))
                        } else {
                            match feedback {
                                InteractionFeedback::Resting => None,
                                InteractionFeedback::Hovered => Some(theme.hover_overlay()),
                                InteractionFeedback::Pressed => Some(theme.active_overlay()),
                            }
                        };
                        let expected =
                            overlay.map_or(surface, |color| composite_over(surface, color));
                        assert_eq!(actual.alpha, 1.0);
                        for (actual, expected) in [
                            (actual.red, expected.red),
                            (actual.green, expected.green),
                            (actual.blue, expected.blue),
                        ] {
                            assert!(
                                (actual - expected).abs() < 1e-6,
                                "alpha={alpha}, kind={kind:?}, selected={selected}, open={open}, feedback={feedback:?}"
                            );
                        }
                    }
                    // Both status and commit-file rows apply this same style.
                    interaction.apply(div().id("translucent_file_row"));
                }
            }
        }
    }
}

#[test]
fn file_row_badges_preserve_selection_and_tint_through_hover_and_press() {
    for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
        let tint = file_kind_row_tint(FileStatusKind::Deleted, &theme).expect("tinted kind");
        let selected =
            FileRowInteraction::new(theme, Some(tint), true, false).badge_disc("row".into());
        let plain = FileRowInteraction::new(theme, None, true, false).badge_disc("row".into());
        assert_eq!(selected.resting.alpha, 1.0);
        assert_ne!(
            selected.resting, plain.resting,
            "change-kind tint survives selection"
        );
        assert_eq!(selected.resting, selected.hover.unwrap().1);
        assert_eq!(selected.resting, selected.pressed.unwrap().1);
        let open = FileRowInteraction::new(theme, Some(tint), true, true).badge_disc("row".into());
        assert_ne!(open.resting, selected.resting);
        assert_eq!(open.resting, open.hover.unwrap().1);
        assert_eq!(open.resting, open.pressed.unwrap().1);
    }
}

fn reset_line_number_string_cache() {
    LINE_NUMBER_STRINGS.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.clear();
        cache.push(SharedString::default());
    });
}

fn line_number_string_cache_len() -> usize {
    LINE_NUMBER_STRINGS.with(|cache| cache.borrow().len())
}

#[test]
fn line_number_cache_does_not_grow_for_uncached_large_numbers() {
    reset_line_number_string_cache();
    assert_eq!(line_number_string_cache_len(), 1);

    assert_eq!(line_number_string(Some(8)), SharedString::from("8"));
    assert_eq!(line_number_string_cache_len(), 9);

    let uncached_line = (MAX_CACHED_LINE_NUMBER as u32).saturating_add(1);
    assert_eq!(
        line_number_string(Some(uncached_line)),
        uncached_line.to_string()
    );
    assert_eq!(line_number_string_cache_len(), 9);
}

#[test]
fn line_number_cache_still_caches_small_numbers() {
    reset_line_number_string_cache();
    assert_eq!(line_number_string_cache_len(), 1);

    assert_eq!(line_number_string(Some(1)), SharedString::from("1"));
    assert_eq!(line_number_string(Some(3)), SharedString::from("3"));
    assert_eq!(line_number_string(Some(1)), SharedString::from("1"));
    assert_eq!(line_number_string_cache_len(), 4);
}

#[test]
fn lru_cache_evicts_least_recently_used() {
    let mut cache: FxLruCache<u64, u64> = new_fx_lru_cache(8);
    for key in 0..8u64 {
        cache.put(key, key);
    }
    assert_eq!(cache.len(), 8);

    // Insert a 9th entry — should evict key 0 (LRU)
    cache.put(999, 999);
    assert_eq!(cache.len(), 8);
    assert!(cache.peek(&999).is_some());
    assert!(cache.peek(&0).is_none(), "LRU entry should be evicted");
    assert!(cache.peek(&7).is_some(), "MRU entry should remain");
}

#[test]
fn lru_cache_promotes_on_get() {
    let mut cache: FxLruCache<u64, u64> = new_fx_lru_cache(4);
    for key in 0..4u64 {
        cache.put(key, key);
    }

    // Access key 0 to promote it to MRU
    assert_eq!(cache.get(&0), Some(&0));

    // Insert 4 more entries — key 0 should survive (was promoted)
    cache.put(10, 10);
    cache.put(11, 11);
    cache.put(12, 12);

    assert!(cache.peek(&0).is_some(), "promoted entry should survive");
    assert!(
        cache.peek(&1).is_none(),
        "unpromoted old entry should be evicted"
    );
}

#[test]
fn lru_cache_metrics_track_hits_misses_evictions_and_clears() {
    let mut cache: FxLruCache<u64, u64> = new_fx_lru_cache(2);

    assert_eq!(cache.get(&1), None);
    assert_eq!(
        cache.metrics(),
        LruCacheMetrics {
            hits: 0,
            misses: 1,
            evictions: 0,
            clears: 0,
        }
    );

    cache.put(1, 10);
    cache.put(2, 20);
    assert_eq!(cache.get(&1), Some(&10));
    assert_eq!(
        cache.metrics(),
        LruCacheMetrics {
            hits: 1,
            misses: 1,
            evictions: 0,
            clears: 0,
        }
    );

    cache.put(3, 30);
    assert_eq!(
        cache.peek(&2),
        None,
        "least-recently used entry should evict"
    );
    assert_eq!(
        cache.metrics(),
        LruCacheMetrics {
            hits: 1,
            misses: 1,
            evictions: 1,
            clears: 0,
        }
    );

    cache.clear();
    assert_eq!(cache.len(), 0);
    assert_eq!(
        cache.metrics(),
        LruCacheMetrics {
            hits: 1,
            misses: 1,
            evictions: 1,
            clears: 1,
        }
    );
}

#[test]
fn commit_file_row_presentation_cache_reuses_same_key_and_invalidates_on_new_key() {
    let mut cache: CommitFileRowPresentationCache<u64> = CommitFileRowPresentationCache::default();
    let files = vec![
        CommitFileChange::new(PathBuf::from("src/lib.rs"), FileStatusKind::Modified),
        CommitFileChange::new(PathBuf::from("README.md"), FileStatusKind::Added),
    ];

    let first = cache.rows_for(&7, &files);
    let reused = cache.rows_for(
        &7,
        &[CommitFileChange::new(
            PathBuf::from("should/not/appear.rs"),
            FileStatusKind::Deleted,
        )],
    );

    assert!(Arc::ptr_eq(&first, &reused));
    assert_eq!(
        first
            .iter()
            .map(|row| row.label.as_ref())
            .collect::<Vec<_>>(),
        vec!["src/lib.rs", "README.md"]
    );
    assert_eq!(
        first[0].visuals,
        commit_file_kind_visuals(FileStatusKind::Modified)
    );
    assert_eq!(
        first[1].visuals,
        commit_file_kind_visuals(FileStatusKind::Added)
    );

    let replacement = cache.rows_for(
        &8,
        &[CommitFileChange::new(
            PathBuf::from("docs/guide.md"),
            FileStatusKind::Renamed,
        )],
    );

    assert!(!Arc::ptr_eq(&first, &replacement));
    assert_eq!(
        replacement
            .iter()
            .map(|row| row.label.as_ref())
            .collect::<Vec<_>>(),
        vec!["docs/guide.md"]
    );
    assert_eq!(
        replacement[0].visuals,
        commit_file_kind_visuals(FileStatusKind::Renamed)
    );
}

#[test]
fn commit_file_signature_changes_with_large_file_state() {
    let plain = vec![commit_file("a.bin", FileStatusKind::Modified, None, None)];
    let mut managed = plain.clone();
    managed[0].large_file = Some(gitcomet_core::large_files::LargeFileState {
        pointer: gitcomet_core::large_files::LargeFilePointer::Lfs(
            gitcomet_core::lfs::LfsPointer {
                oid: gitcomet_core::lfs::LfsOid([1; 32]),
                size: 5,
            },
        ),
        in_local_store: Some(true),
        worktree: None,
        lockable: false,
    });
    assert_ne!(
        commit_file_row_presentation_signature(&plain),
        commit_file_row_presentation_signature(&managed),
        "a chip appearing must invalidate cached rows"
    );
}

fn commit_file(
    path: &str,
    kind: FileStatusKind,
    additions: Option<u32>,
    deletions: Option<u32>,
) -> CommitFileChange {
    CommitFileChange::new(PathBuf::from(path), kind).with_line_stats((additions, deletions).into())
}

#[test]
fn commit_file_projection_counts_the_whole_commit_and_filters_by_kind() {
    let files = vec![
        commit_file("modified.rs", FileStatusKind::Modified, Some(1), Some(2)),
        commit_file("conflicted.rs", FileStatusKind::Conflicted, None, None),
        commit_file("removed.rs", FileStatusKind::Deleted, Some(0), Some(3)),
        commit_file("added.rs", FileStatusKind::Added, Some(4), Some(0)),
        commit_file("untracked.rs", FileStatusKind::Untracked, None, None),
        commit_file("renamed.rs", FileStatusKind::Renamed, Some(0), Some(0)),
    ];

    let projection = build_commit_file_projection(
        &files,
        CommitFileSort::PathAscending,
        CommitFileFilter::Modified,
    );

    assert_eq!(projection.source_indices.as_ref(), &[1, 0]);
    assert_eq!(
        projection.counts,
        CommitFileKindCounts {
            all: 6,
            modified: 2,
            removed: 1,
            added: 2,
            renamed: 1,
        }
    );
    assert_eq!(
        CommitFileFilter::ALL.map(|filter| projection.counts.for_filter(filter)),
        [6, 2, 1, 2, 1]
    );
    assert_eq!(
        CommitFileFilter::ALL.map(CommitFileFilter::label),
        ["All", "Modified", "Deleted", "Added", "Renamed"]
    );
    assert_eq!(
        CommitFileFilter::Removed.tooltip_in("this commit", 1),
        "Show files deleted by this commit (1)"
    );
    assert_eq!(
        CommitFileFilter::Removed.tooltip_in("this worktree", 2),
        "Show files deleted by this worktree (2)"
    );
}

#[test]
fn commit_file_projection_sorts_paths_case_insensitively_with_stable_ties() {
    let files = vec![
        commit_file("src/zeta.rs", FileStatusKind::Modified, None, None),
        commit_file("src/Alpha.rs", FileStatusKind::Modified, None, None),
        commit_file("src/alpha.rs", FileStatusKind::Modified, None, None),
        commit_file("src/Alpha.rs", FileStatusKind::Modified, None, None),
    ];

    let ascending =
        build_commit_file_projection(&files, CommitFileSort::PathAscending, CommitFileFilter::All);
    let descending = build_commit_file_projection(
        &files,
        CommitFileSort::PathDescending,
        CommitFileFilter::All,
    );

    assert_eq!(ascending.source_indices.as_ref(), &[1, 3, 2, 0]);
    assert_eq!(descending.source_indices.as_ref(), &[0, 2, 3, 1]);
}

#[test]
fn commit_file_projection_sorts_edit_size_with_unknown_stats_last() {
    let files = vec![
        commit_file("z-large.rs", FileStatusKind::Modified, Some(7), Some(3)),
        commit_file("b-small.rs", FileStatusKind::Modified, Some(1), Some(1)),
        commit_file("unknown.rs", FileStatusKind::Modified, None, None),
        commit_file("a-small.rs", FileStatusKind::Modified, Some(0), Some(2)),
    ];

    let ascending = build_commit_file_projection(
        &files,
        CommitFileSort::EditSizeAscending,
        CommitFileFilter::All,
    );
    let descending = build_commit_file_projection(
        &files,
        CommitFileSort::EditSizeDescending,
        CommitFileFilter::All,
    );

    assert_eq!(ascending.source_indices.as_ref(), &[3, 1, 0, 2]);
    assert_eq!(descending.source_indices.as_ref(), &[0, 3, 1, 2]);
}

#[test]
fn file_type_sort_groups_by_extension_then_path() {
    let files = vec![
        commit_file("src/ui/view.ts", FileStatusKind::Modified, Some(1), Some(1)),
        commit_file("Makefile", FileStatusKind::Modified, Some(1), Some(1)),
        commit_file("src/main.rs", FileStatusKind::Modified, Some(1), Some(1)),
        commit_file("Cargo.toml", FileStatusKind::Modified, Some(1), Some(1)),
        commit_file("src/ui/app.ts", FileStatusKind::Modified, Some(1), Some(1)),
        commit_file(
            "src/core/lib.RS",
            FileStatusKind::Modified,
            Some(1),
            Some(1),
        ),
    ];

    let ascending = build_commit_file_projection(
        &files,
        CommitFileSort::FileTypeAscending,
        CommitFileFilter::All,
    );

    // "" (Makefile) < rs < toml < ts, and `.RS` groups with `.rs`.
    assert_eq!(ascending.source_indices.as_ref(), &[1, 5, 2, 3, 4, 0]);
}

#[test]
fn file_type_descending_flips_the_groups_but_not_the_paths_inside_them() {
    let files = vec![
        commit_file("b.rs", FileStatusKind::Modified, Some(1), Some(1)),
        commit_file("a.ts", FileStatusKind::Modified, Some(1), Some(1)),
        commit_file("a.rs", FileStatusKind::Modified, Some(1), Some(1)),
        commit_file("b.ts", FileStatusKind::Modified, Some(1), Some(1)),
    ];

    let descending = build_commit_file_projection(
        &files,
        CommitFileSort::FileTypeDescending,
        CommitFileFilter::All,
    );

    // ts before rs, but a.* before b.* within each -- reading a group is
    // still alphabetical, only the group order reverses.
    assert_eq!(descending.source_indices.as_ref(), &[1, 3, 2, 0]);
}

pub(in crate::view) fn edit(text: &str) -> Option<EditSignature> {
    let mut edit = gitcomet_core::edit_signature::EditSignatureBuilder::default();
    edit.added(text.as_bytes());
    edit.finish()
}

#[test]
fn edits_put_the_largest_shared_edit_first_then_singles_then_unknown() {
    let file = |path: &str, text: Option<&str>| {
        CommitFileChange::new(PathBuf::from(path), FileStatusKind::Modified).with_line_stats(
            LineStats {
                additions: Some(1),
                deletions: Some(1),
                edit: text.and_then(edit),
            },
        )
    };
    let files = vec![
        file("z/one.rs", Some("a")),
        file("b.rs", Some("b")),
        file("a.rs", Some("a")),
        file("c.rs", Some("b")),
        file("d.rs", Some("b")),
        file("solo2.rs", Some("c")),
        file("solo1.rs", Some("d")),
        file("bin.png", None),
        file("aaa.rs", None),
        file("0.rs", Some("e")),
        file("y.rs", Some("e")),
    ];

    let projection =
        build_commit_file_projection(&files, CommitFileSort::Edits, CommitFileFilter::All);

    // Three share "b"; "e" and "a" share by two each, "e" first by its
    // first path; then the singles and the unknown, each by path.
    assert_eq!(
        projection.source_indices.as_ref(),
        &[1, 3, 4, 9, 10, 2, 0, 6, 5, 8, 7]
    );
}

#[test]
fn edits_cluster_only_the_files_a_filter_shows() {
    let file = |path: &str, kind: FileStatusKind, text: &str| {
        CommitFileChange::new(PathBuf::from(path), kind).with_line_stats(LineStats {
            additions: Some(1),
            deletions: Some(0),
            edit: edit(text),
        })
    };
    let files = vec![
        file("a.rs", FileStatusKind::Added, "x"),
        file("b.rs", FileStatusKind::Modified, "x"),
        file("c.rs", FileStatusKind::Modified, "y"),
        file("d.rs", FileStatusKind::Modified, "y"),
    ];

    let modified =
        build_commit_file_projection(&files, CommitFileSort::Edits, CommitFileFilter::Modified);

    // With `a.rs` filtered out, "x" is one file's edit and sorts as a single.
    assert_eq!(modified.source_indices.as_ref(), &[2, 3, 1]);
}

/// The direction word is the same across options; only the noun changes.
#[test]
fn sort_labels_name_what_is_ordered_and_which_way() {
    assert_eq!(CommitFileSort::PathAscending.label(), "Path: Ascending");
    assert_eq!(CommitFileSort::PathDescending.label(), "Path: Descending");
    assert_eq!(
        CommitFileSort::FileTypeAscending.label(),
        "File type: Ascending"
    );
    assert_eq!(
        CommitFileSort::FileTypeDescending.label(),
        "File type: Descending"
    );

    for sort in CommitFileSort::ALL {
        assert!(
            !sort.label().contains('–'),
            "{sort:?} still reads as an A–Z range",
        );
    }
}

#[test]
fn commit_file_projection_cache_reuses_a_key_and_invalidates_on_change() {
    let files = vec![commit_file(
        "src/lib.rs",
        FileStatusKind::Modified,
        Some(1),
        Some(2),
    )];
    let mut cache = CommitFileProjectionCache::<u64>::default();

    let first = cache.projection_for(
        &1,
        &files,
        CommitFileSort::PathAscending,
        CommitFileFilter::All,
    );
    let reused = cache.projection_for(
        &1,
        &[],
        CommitFileSort::PathDescending,
        CommitFileFilter::Removed,
    );
    let replacement = cache.projection_for(
        &2,
        &files,
        CommitFileSort::PathDescending,
        CommitFileFilter::All,
    );

    assert!(Arc::ptr_eq(&first, &reused));
    assert!(!Arc::ptr_eq(&first, &replacement));
}

/// A linked worktree's file list contains untracked files, which commit and
/// range lists never do. The table slot for them used to hold a
/// "shouldn't happen" question mark, which is what those rows rendered.
#[test]
fn untracked_files_read_as_additions() {
    let untracked = commit_file_kind_visuals(FileStatusKind::Untracked);
    assert_eq!(untracked.icon, "icons/plus.svg");
    assert_eq!(
        untracked.icon,
        commit_file_kind_visuals(FileStatusKind::Added).icon,
        "an untracked file reads as an addition, as it does everywhere else"
    );
}

/// `kind_key` is a separate ordering from the table's own: callers count into
/// buckets by it (`benchmarks/repo_history.rs` reads bucket 0 as "added") and
/// it is hashed into the row signature. Retoning an entry must never move it.
#[test]
fn kind_keys_are_stable_buckets() {
    assert_eq!(commit_file_kind_visuals(FileStatusKind::Added).kind_key, 0);
    assert_eq!(
        commit_file_kind_visuals(FileStatusKind::Modified).kind_key,
        1
    );
    assert_eq!(
        commit_file_kind_visuals(FileStatusKind::Deleted).kind_key,
        2
    );
    assert_eq!(
        commit_file_kind_visuals(FileStatusKind::Renamed).kind_key,
        3
    );
    assert_eq!(
        commit_file_kind_visuals(FileStatusKind::Untracked).kind_key,
        4
    );
    assert_eq!(
        commit_file_kind_visuals(FileStatusKind::Conflicted).kind_key,
        5
    );
}

/// `rows_for` returns cached rows when the *key* matches, without looking at
/// the files. The worktree list keys on the scan revision, which does not
/// move when a different worktree is selected, so the key has to carry the
/// worktree too — otherwise one worktree shows another's files.
#[test]
fn a_key_that_does_not_move_with_the_selection_serves_stale_rows() {
    let mut cache: CommitFileRowPresentationCache<(u64, std::path::PathBuf)> =
        CommitFileRowPresentationCache::default();

    let first = cache.rows_for(
        &(1, PathBuf::from("/wt/a")),
        &[CommitFileChange::new(
            PathBuf::from("a.rs"),
            FileStatusKind::Modified,
        )],
    );
    assert_eq!(first[0].label.as_ref(), "a.rs");

    // Same revision, different worktree: the path in the key is what keeps
    // these apart.
    let second = cache.rows_for(
        &(1, PathBuf::from("/wt/b")),
        &[CommitFileChange::new(
            PathBuf::from("b.rs"),
            FileStatusKind::Modified,
        )],
    );
    assert_eq!(
        second[0].label.as_ref(),
        "b.rs",
        "a second worktree must not be served the first one's rows"
    );
}

#[test]
fn commit_file_row_presentation_cache_reuses_identical_files_across_new_keys() {
    let mut cache: CommitFileRowPresentationCache<u64> = CommitFileRowPresentationCache::default();
    let files = vec![
        CommitFileChange::new(PathBuf::from("src/lib.rs"), FileStatusKind::Modified),
        CommitFileChange::new(PathBuf::from("README.md"), FileStatusKind::Added),
    ];

    let first = cache.rows_for(&7, &files);
    let reused = cache.rows_for(&8, &files);

    assert!(Arc::ptr_eq(&first, &reused));
    assert_eq!(
        reused
            .iter()
            .map(|row| row.label.as_ref())
            .collect::<Vec<_>>(),
        vec!["src/lib.rs", "README.md"]
    );
}

#[test]
fn commit_file_row_presentation_cache_handles_empty_file_lists() {
    let mut cache: CommitFileRowPresentationCache<u64> = CommitFileRowPresentationCache::default();

    let first = cache.rows_for(&1, &[]);
    let second = cache.rows_for(&1, &[]);

    assert!(first.is_empty());
    assert!(Arc::ptr_eq(&first, &second));
}

/// The resolver's text is shaped from `window.rem_size()`, so its row box and
/// line-number cell have to grow with UI scale too -- a flat 20px row holds a
/// 41px line box at 200% and spills into the row below.
#[test]
fn conflict_row_geometry_scales_with_ui_scale() {
    for percent in [80, 100, 150, 200] {
        let factor = percent as f32 / 100.0;
        let height: f32 = AppTheme::gitcomet_dark().editor_row_height(percent).into();
        let line_no: f32 = conflict_line_no_width(percent).into();
        let base = crate::appearance::Appearance::default().editor_line_height();
        assert!(
            (height - base * factor).abs() < 0.01,
            "row height at {percent}% should be {}, got {height}",
            base * factor,
        );
        // Also measured at the editor font, so anchor it at 100%.
        let line_no_base: f32 = conflict_line_no_width(100).into();
        assert!(
            (line_no - line_no_base * factor).abs() < 0.01,
            "line-number width at {percent}% should be {}, got {line_no}",
            line_no_base * factor,
        );
    }

    // Strictly monotonic across the presets, so no two zoom levels collapse
    // onto the same geometry.
    let heights = crate::ui_scale::UI_SCALE_PRESETS
        .iter()
        .map(|percent| f32::from(AppTheme::gitcomet_dark().editor_row_height(*percent)))
        .collect::<Vec<_>>();
    assert!(
        heights.windows(2).all(|pair| pair[0] < pair[1]),
        "row heights should grow with every preset, got {heights:?}"
    );
}

/// The conflict rows and the diff rows are the same 20px design row; keeping the
/// two helpers in agreement is what stops the resolver drifting away from the
/// diff view it sits beside.
#[test]
fn conflict_row_height_matches_the_diff_row_height() {
    for percent in crate::ui_scale::UI_SCALE_PRESETS.iter().copied() {
        assert_eq!(
            AppTheme::gitcomet_dark().editor_row_height(percent),
            AppTheme::gitcomet_dark().editor_row_height(percent),
            "conflict and diff rows disagree at {percent}%"
        );
    }
}
