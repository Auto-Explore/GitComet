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

    /// A theme built here carries the default `Appearance`, so any render path
    /// that constructs one silently sizes itself for a 13px editor font and a
    /// Compact density. Rendering code must take the caller's theme.
    #[test]
    fn render_code_never_builds_its_own_theme() {
        fn walk(dir: &std::path::Path, offenders: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("read src") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    if path
                        .file_name()
                        .is_some_and(|name| name == "tests" || name == "benchmarks")
                    {
                        continue;
                    }
                    walk(&path, offenders);
                    continue;
                }
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if !name.ends_with(".rs")
                    || name.ends_with("tests.rs")
                    || name == "theme.rs"
                    || name == "smoke_tests.rs"
                    // TextInput's constructor supplies a default until set_theme is called.
                    // Match path components before formatting platform-specific diagnostics.
                    || path.ends_with(std::path::Path::new("kit/text_input/editing.rs"))
                {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("read source");
                let production = match source.find("#[cfg(test)]") {
                    Some(cut) => &source[..cut],
                    None => &source[..],
                };
                for (ix, line) in production.lines().enumerate() {
                    if line.contains("AppTheme::gitcomet_") {
                        offenders.push(format!("{}:{}", path.display(), ix + 1));
                    }
                }
            }
        }

        let mut offenders = Vec::new();
        walk(std::path::Path::new("src"), &mut offenders);

        assert!(
            offenders.is_empty(),
            "these must take the theme they are handed: {offenders:?}"
        );
    }
}
