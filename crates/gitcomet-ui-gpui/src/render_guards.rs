//! Source guards over the host's rendering code; the kit guards its own.

#[cfg(test)]
mod tests {
    use gitcomet_ui_kit::test_support::source_guards::{
        DiscreteControlAllowlist, direct_clipboard_access, discrete_control_violations,
        unresolved_menu_icons, unscaled_icon_sizes,
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
                    "view/gitcomet_view.rs",
                    "view/chrome.rs",
                    "view/settings_window/render.rs",
                    "view/terminal_panel.rs",
                    "view/terminal_panel/viewport.rs",
                    "view/panels/layout/status_view.rs",
                    "view/panels/repo_tabs_bar.rs",
                    "view/panes/history/history_panel.rs",
                    "view/rows/diff_text/build.rs",
                    "view/panels/main/conflict_resolver_view.rs",
                    "view/panels/main/diff.rs",
                    "view/panels/main/diff_view.rs",
                ],
                context_press: &["view/terminal_panel/viewport.rs"],
                // Selection, resizing, dragging, focus, and propagation only.
                // Discrete controls (including hosted group headers) use on_activate.
                primary_press: &[
                    "view/chrome.rs",
                    "view/diff_text_selection.rs",
                    "view/gitcomet_view.rs",
                    "view/gitcomet_view_render.rs",
                    "view/hosted/diff_pane.rs",
                    "view/panels/layout/status_view.rs",
                    "view/panels/main/conflict_resolver_view.rs",
                    "view/panels/main/diff.rs",
                    "view/panels/main/diff_view.rs",
                    "view/panels/popover/context_menu.rs",
                    "view/panels/popover/mod.rs",
                    "view/panels/repo_tabs_bar.rs",
                    "view/panes/details.rs",
                    "view/panes/history/history_panel.rs",
                    "view/rows/conflict_resolver.rs",
                    "view/rows/diff_text/build.rs",
                    "view/rows/markdown_document.rs",
                    "view/settings_window/render.rs",
                    "view/terminal_panel.rs",
                    "view/terminal_panel/viewport.rs",
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

    /// An icon path the context menu does not list renders an empty, still
    /// indented slot, so every menu icon literal must resolve to itself.
    #[test]
    fn every_menu_icon_literal_in_the_crate_resolves() {
        let (seen, unresolved) = unresolved_menu_icons(&src_dir());
        assert!(seen > 100, "the scan found only {seen} menu icons");
        assert!(
            unresolved.is_empty(),
            "add these to context_menu_icon_path:\n{}",
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
                if path
                    .file_name()
                    .is_some_and(|name| name == "tests" || name == "benchmarks")
                {
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
            // Stop at the inline test module. Any other `#[cfg(test)]` marks a
            // single item (a test-only field, helper or `mod tests;`), and
            // production code continues after it.
            let lines: Vec<&str> = source.lines().collect();
            for (ix, line) in lines.iter().enumerate() {
                let opens_test_module = line.starts_with("#[cfg(test)]")
                    && lines.get(ix + 1).is_some_and(|next| {
                        next.starts_with("mod ") && next.trim_end().ends_with('{')
                    });
                if opens_test_module {
                    break;
                }
                visit(&path, ix + 1, line);
            }
        }
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
            if path.ends_with("theme.rs")
                // TextInput's constructor supplies a default until set_theme is called.
                // Match path components before formatting platform-specific diagnostics.
                || path.ends_with(std::path::Path::new("kit/text_input/editing.rs"))
            {
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

    /// `window_bounds()` reports the *restore* size while the window is
    /// maximized or fullscreen, and includes the client-side shadow band, so
    /// anything sized or placed from it misfits the visible window. Views use
    /// `chrome::window_surface_bounds`.
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
            "use chrome::window_surface_bounds instead: {offenders:#?}"
        );
    }
}
