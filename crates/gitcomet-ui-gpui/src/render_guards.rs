//! Source guards over the host's rendering code; the kit guards its own.

#[cfg(test)]
mod tests {
    use gitcomet_ui_kit::test_support::source_guards::{
        DiscreteControlAllowlist, ELEMENT_OPACITY_HINT, direct_clipboard_access,
        discrete_control_violations, opacity_uses, production_text, unresolved_menu_icons,
        unscaled_icon_sizes,
    };

    fn src_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    /// Discrete controls style and activate through the kit's interaction APIs.
    #[test]
    fn discrete_control_styles_and_activation_are_owned_by_the_interaction_kit() {
        let violations = discrete_control_violations(
            &src_dir(),
            &DiscreteControlAllowlist {
                custom_styling: &[],
                // These releases end continuous gestures. Text links observe a
                // shared SubtargetClick because their input owns pointer selection.
                release_gestures: &[
                    "view/components/commit_link_menu.rs",
                    "view/gitcomet_view_render.rs",
                    "view/chrome.rs",
                    "view/settings_window/render.rs",
                    "view/terminal_panel.rs",
                    "view/panels/layout/status_view.rs",
                    "view/panels/repo_tabs_bar.rs",
                    "view/panes/history/history_panel.rs",
                    "view/rows/diff_text/build.rs",
                    "view/panels/main/conflict_resolver_view.rs",
                    "view/panels/main/diff.rs",
                    "view/panels/main/diff_view.rs",
                ],
                context_press: &[],
                // Selection, resizing, dragging, focus, and propagation only.
                // Discrete controls (including hosted group headers) use on_activate.
                primary_press: &[
                    "view/chrome.rs",
                    "view/diff_text_selection.rs",
                    "view/gitcomet_view_render.rs",
                    "view/panels/layout/status_view.rs",
                    "view/panels/main/conflict_resolver_view.rs",
                    "view/panels/main/diff.rs",
                    "view/panels/main/diff_view.rs",
                    "view/panels/popover/context_menu.rs",
                    "view/panels/popover/mod.rs",
                    "view/panels/repo_tabs_bar.rs",
                    "view/panes/details.rs",
                    "view/panes/file_history.rs",
                    "view/panes/history/history_panel.rs",
                    "view/rows/conflict_resolver.rs",
                    "view/rows/conflict_resolver/three_way_rows.rs",
                    "view/rows/diff_text/build.rs",
                    "view/rows/markdown_document.rs",
                    "view/settings_window/render.rs",
                    "view/terminal_panel.rs",
                ],
            },
        );
        assert!(
            violations.is_empty(),
            "Use the shared interaction APIs for discrete controls: {violations:?}"
        );
    }

    /// Clipboard access goes through the kit's clipboard module.
    #[test]
    fn production_clipboard_access_goes_through_the_kit() {
        let offenders = direct_clipboard_access(&src_dir(), &[]);
        assert!(
            offenders.is_empty(),
            "these access the GPUI clipboard directly; use crate::clipboard: {offenders:?}"
        );
    }

    /// A raw `px()` icon keeps its 100% size while the UI around it zooms. The
    /// window chrome holds one size at every UI scale, so only it may.
    #[test]
    fn icon_sizes_follow_the_ui_scale() {
        let unscaled = unscaled_icon_sizes(
            &src_dir(),
            &["view/chrome.rs", "view/panels/repo_tabs_bar.rs"],
        );
        assert!(
            unscaled.is_empty(),
            "icon sizes must go through the UI scale (`scaled_px`, `ui_scale.px`): {unscaled:#?}"
        );
    }

    #[test]
    fn every_menu_icon_literal_in_the_crate_resolves() {
        let (seen, unresolved) = unresolved_menu_icons(&src_dir());
        assert!(seen > 100, "the scan found only {seen} menu icons");
        assert!(
            unresolved.is_empty(),
            "menu icons must name an existing asset:\n{}",
            unresolved.join("\n")
        );
    }

    /// Visits every non-test line under `dir` as `(path, 1-based line, text)`,
    /// skipping test files, test modules and benchmarks.
    fn production_lines(
        dir: &std::path::Path,
        visit: &mut dyn FnMut(&std::path::Path, usize, &str),
    ) {
        for entry in std::fs::read_dir(dir).expect("read src") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| {
                    name == "tests" || name == "smoke_tests" || name == "benchmarks"
                }) {
                    continue;
                }
                production_lines(&path, visit);
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if !name.ends_with(".rs") || name.ends_with("tests.rs") || name == "smoke_tests.rs" {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("read source");
            for (ix, line) in production_text(&source).lines().enumerate() {
                visit(&path, ix + 1, line);
            }
        }
    }

    #[test]
    fn production_line_scan_skips_smoke_modules_but_keeps_similarly_named_sources() {
        let sources = tempfile::tempdir().unwrap();
        for directory in ["smoke_tests", "smoke_tests_extra", "production"] {
            let directory = sources.path().join(directory);
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(directory.join("fixture.rs"), "AppTheme::gitcomet_dark()\n").unwrap();
        }
        let mut visited = Vec::new();
        production_lines(sources.path(), &mut |path, _, _| {
            visited.push(path.strip_prefix(sources.path()).unwrap().to_path_buf());
        });
        visited.sort();
        assert_eq!(
            visited,
            [
                std::path::PathBuf::from("production/fixture.rs"),
                std::path::PathBuf::from("smoke_tests_extra/fixture.rs"),
            ]
        );
    }

    /// An `icons/…` path the assets lack draws nothing (`svg()` of a missing
    /// asset is blank), so every such literal in production code must load.
    #[test]
    fn every_icon_path_literal_in_the_crate_loads() {
        use gpui::AssetSource as _;
        let mut sources = Vec::new();
        gitcomet_ui_kit::test_support::rust_sources_under(&src_dir(), &mut sources);
        let assets = crate::assets::GitCometAssets::default();
        let mut seen = 0usize;
        let mut missing = Vec::new();
        for path in sources {
            let relative = path
                .strip_prefix(src_dir())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if relative.contains("tests/") || relative.ends_with("tests.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            let production = source.split("#[cfg(test)]\nmod tests").next().unwrap_or("");
            for (start, _) in production.match_indices("\"icons/") {
                let Some(literal) = production[start + 1..].split('"').next() else {
                    continue;
                };
                // Format strings and directory prefixes are assembled elsewhere.
                if !literal.ends_with(".svg") || literal.contains('{') {
                    continue;
                }
                seen += 1;
                if assets.load(literal).ok().flatten().is_none() {
                    missing.push(format!("{relative}: {literal}"));
                }
            }
        }
        assert!(seen > 100, "the scan found only {seen} icon paths");
        assert!(missing.is_empty(), "missing icons:\n{}", missing.join("\n"));
    }

    /// A theme built here carries the default `Appearance`, so any render path
    /// that constructs one silently sizes itself for a 13px editor font and a
    /// Compact density. Rendering code must take the caller's theme.
    #[test]
    fn render_code_never_builds_its_own_theme() {
        let mut offenders = Vec::new();
        production_lines(std::path::Path::new("src"), &mut |path, line_no, line| {
            if path.ends_with("theme.rs") {
                return;
            }
            if line.contains("AppTheme::gitcomet_") {
                offenders.push(format!("{}:{line_no}", path.display()));
            }
        });

        assert!(
            offenders.is_empty(),
            "these must take the theme they are handed: {offenders:?}"
        );
    }

    /// gpui snaps a quad's edges to device pixels but keeps its corner radius,
    /// so a round history shape on fractional bounds paints as a lens.
    #[test]
    fn history_rows_paint_round_shapes_through_history_shapes() {
        let rows = src_dir().join("view").join("rows");
        let mut offenders = Vec::new();
        production_lines(&rows, &mut |path, line_no, line| {
            let history = path.strip_prefix(&rows).is_ok_and(|relative| {
                relative.starts_with("history")
                    || relative.to_string_lossy().starts_with("history_")
            });
            if history && !path.ends_with("history_shapes.rs") && line.contains("corner_radii(") {
                offenders.push(format!("{}:{line_no}", path.display()));
            }
        });

        assert!(
            offenders.is_empty(),
            "use history_shapes::paint_rounded_rect: {offenders:?}"
        );
    }

    /// `window_bounds()` reports the *restore* size while the window is
    /// maximized or fullscreen, and includes the client-side decoration inset, so
    /// anything sized or placed from it misfits the visible window. Views use
    /// `crate::kit::window_chrome::window_surface_bounds`.
    #[test]
    fn view_code_sizes_against_the_visible_surface_not_restore_bounds() {
        let mut offenders = Vec::new();
        production_lines(&src_dir().join("view"), &mut |path, line_no, line| {
            if line.contains(".window_bounds()") {
                offenders.push(format!("{}:{line_no}", path.display()));
            }
        });

        assert!(
            offenders.is_empty(),
            "use crate::kit::window_chrome::window_surface_bounds instead: {offenders:#?}"
        );
    }

    /// The remaining element opacity uses, so a new one, or an entry nothing
    /// needs any more, fails here.
    #[test]
    fn element_opacity_stays_on_its_allowlist() {
        let mut allowed: Vec<(String, usize)> = [
            // A panel, popover or toast fading while it mounts or leaves.
            ("view/panels/main/diff_view.rs", 1),
            ("view/panes/history/find.rs", 1),
            ("view/splash.rs", 1),
            ("view/toast_host.rs", 1),
            // Rebase rows: the dropped row's dim and the drag source's fade both
            // stay on; the reordered pair fades in.
            ("view/panes/main/interactive_rebase.rs", 3),
            // History rows dimmed by a find miss or an extension annotation.
            ("view/rows/history/mod.rs", 3),
        ]
        .into_iter()
        .map(|(path, count): (&str, usize)| (path.to_string(), count))
        .collect();
        allowed.sort();
        assert_eq!(
            opacity_uses(&src_dir()),
            allowed,
            "{ELEMENT_OPACITY_HINT}, or update this list when a use goes away"
        );
    }
}
