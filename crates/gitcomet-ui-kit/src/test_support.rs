//! Test helpers for the kit and for crates that test against it
//! (`test-support` feature). The locks are process-wide, so a host that
//! re-exports them serializes its tests with the kit's.

/// Replace the default single-line test shaper before creating windows. Bundled
/// fonts keep wrapping and cluster tests independent of the machine font catalog.
pub fn use_real_text_backend(cx: &mut gpui::TestAppContext) {
    use_text_backend(cx, gpui_parley::SystemFonts::Skip);
}

/// Include the OS font catalog for regressions involving the backend's default
/// generic font (for example, synthetic direction markers without source runs).
pub fn use_real_text_backend_with_system_fonts(cx: &mut gpui::TestAppContext) {
    use_text_backend(cx, gpui_parley::SystemFonts::Load);
}

fn use_text_backend(cx: &mut gpui::TestAppContext, system_fonts: gpui_parley::SystemFonts) {
    use gpui::PlatformTextSystem as _;
    let system = std::sync::Arc::new(
        gpui_parley::ParleyTextSystem::new_with_system_font(system_fonts, "IBM Plex Sans")
            .with_fallback_families(["IBM Plex Sans", "Lilex"]),
    );
    system
        .add_fonts(vec![
            std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/fonts/ibm_plex_sans/IBMPlexSans-Regular.ttf"
            )),
            std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/lilex/Lilex-Regular.ttf")),
        ])
        .expect("bundled test fonts");
    *cx = gpui::TestAppContext::build_with_text_system(cx.dispatcher.clone(), None, system);
}

/// Force layout and paint even when only a scroll handle or test fixture changed.
pub fn refresh_and_draw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
}

/// Draw one frame without forcing a refresh.
pub fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

/// Soft-wrapped text-input lines this thread shaped since the last call.
pub fn take_wrapped_lines_shaped() -> usize {
    crate::text_input::take_wrapped_lines_shaped_for_tests()
}

pub fn lock_clipboard_test() -> std::sync::MutexGuard<'static, ()> {
    static CLIPBOARD_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    match CLIPBOARD_TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub fn lock_visual_test() -> std::sync::MutexGuard<'static, ()> {
    static VISUAL_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    match VISUAL_TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Compare the real fill and border quad, rather than inspecting which style
/// methods the renderer called. Text and independently painted inset rings are
/// intentionally outside the background assertion.
pub fn painted_control_quads(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
) -> Vec<(gpui::Background, gpui::Background)> {
    let bounds = cx.debug_bounds(selector).expect("control must be drawn");
    cx.update(|window, _| {
        let scale = window.scale_factor();
        window
            .painted_quads()
            .into_iter()
            .filter(|quad| {
                let rect = quad.bounds;
                (rect.origin.x.0 - f32::from(bounds.origin.x) * scale).abs() < 1.0
                    && (rect.origin.y.0 - f32::from(bounds.origin.y) * scale).abs() < 1.0
                    && (rect.size.width.0 - f32::from(bounds.size.width) * scale).abs() < 1.0
                    && (rect.size.height.0 - f32::from(bounds.size.height) * scale).abs() < 1.0
            })
            .map(|quad| (quad.background, quad.border_color))
            .collect()
    })
}

/// Every `.rs` file below `dir`, for guards that scan a crate's own source.
pub fn rust_sources_under(dir: &std::path::Path, sources: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read source directory") {
        let path = entry.expect("read source entry").path();
        if path.is_dir() {
            rust_sources_under(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

/// Source scans shared by the kit and its hosts: each crate runs them over its
/// own `src` directory.
pub mod source_guards {
    use std::path::Path;

    /// Production sources below `src_dir` as `(relative path, text before the
    /// first inline test module)`, skipping test files and directories.
    fn production_sources(src_dir: &Path) -> Vec<(String, String)> {
        let mut paths = Vec::new();
        super::rust_sources_under(src_dir, &mut paths);
        let mut out = Vec::new();
        for path in paths {
            let relative = path.strip_prefix(src_dir).expect("source below src");
            let relative_str = relative.to_string_lossy().replace('\\', "/");
            let is_test_source = relative
                .components()
                .any(|component| component.as_os_str() == "tests")
                || relative_str.ends_with("tests.rs")
                || relative_str.ends_with("test_support.rs");
            if is_test_source {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("read Rust source");
            let production = source
                .split("#[cfg(test)]\nmod tests")
                .next()
                .unwrap_or("")
                .to_string();
            out.push((relative_str, production));
        }
        out
    }

    /// The size argument of every `svg_icon(`/`svg_spinner(` call in `source`.
    pub fn icon_size_args(source: &str) -> Vec<(usize, String)> {
        let mut sizes = Vec::new();
        for call in ["svg_icon(", "svg_spinner("] {
            for (start, _) in source.match_indices(call) {
                let preceded_by_ident = source[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_');
                if preceded_by_ident || source[..start].ends_with("fn ") {
                    continue;
                }
                let args_start = start + call.len();
                let (mut depth, mut arg_start, mut last_arg) = (0usize, args_start, "");
                for (offset, c) in source[args_start..].char_indices() {
                    let at = args_start + offset;
                    match c {
                        '(' | '[' | '{' => depth += 1,
                        ')' | ']' | '}' if depth > 0 => depth -= 1,
                        ',' | ')' if depth == 0 => {
                            // Skipping empty slots tolerates a trailing comma.
                            let arg = source[arg_start..at].trim();
                            if !arg.is_empty() {
                                last_arg = arg;
                            }
                            arg_start = at + 1;
                            if c == ')' {
                                let line = source[..start].lines().count();
                                sizes.push((line, last_arg.to_string()));
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        sizes
    }

    /// `svg_icon`/`svg_spinner` calls with a raw `px()` size, outside
    /// `fixed_scale` sources (window chrome that holds one size).
    pub fn unscaled_icon_sizes(src_dir: &Path, fixed_scale: &[&str]) -> Vec<String> {
        let mut unscaled = Vec::new();
        for (relative, production) in production_sources(src_dir) {
            if fixed_scale.contains(&relative.as_str()) {
                continue;
            }
            for (line, arg) in icon_size_args(&production) {
                if arg.starts_with("px(") {
                    unscaled.push(format!("{relative}:{line}: {arg}"));
                }
            }
        }
        unscaled
    }

    /// Menu icon literals (`icon: Some("icons/..")`,
    /// `ContextMenuIconSlot::Icon("icons/..")`) and those the context menu
    /// would draw as an empty slot.
    pub fn unresolved_menu_icons(src_dir: &Path) -> (usize, Vec<String>) {
        let mut unresolved = Vec::new();
        let mut seen = 0usize;
        for (relative, production) in production_sources(src_dir) {
            for marker in ["icon: Some(", "ContextMenuIconSlot::Icon("] {
                for (start, _) in production.match_indices(marker) {
                    // `leading_icon: Some(..)` and friends are not menu icons.
                    if production[..start]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_')
                    {
                        continue;
                    }
                    let rest = production[start + marker.len()..].trim_start();
                    let Some(literal) = rest
                        .strip_prefix("\"icons/")
                        .and_then(|rest| rest.split('"').next())
                        .map(|name| format!("icons/{name}"))
                    else {
                        continue;
                    };
                    seen += 1;
                    if crate::components::context_menu_icon_path(&literal, "").map(str::to_owned)
                        != Some(literal.clone())
                    {
                        unresolved.push(format!("{relative}: {literal}"));
                    }
                }
            }
        }
        (seen, unresolved)
    }

    /// Files that call GPUI's clipboard directly instead of the kit's
    /// clipboard module; `allowed` lists relative paths that may.
    pub fn direct_clipboard_access(src_dir: &Path, allowed: &[&str]) -> Vec<String> {
        let mut paths = Vec::new();
        super::rust_sources_under(src_dir, &mut paths);
        let mut offenders = Vec::new();
        for path in paths {
            let relative = path.strip_prefix(src_dir).expect("source below src");
            let relative_str = relative.to_string_lossy().replace('\\', "/");
            let is_test_source = relative
                .components()
                .any(|component| component.as_os_str() == "tests")
                || relative.file_name().is_some_and(|name| {
                    name == "tests.rs" || name == "smoke_tests.rs" || name == "test_support.rs"
                });
            if allowed.contains(&relative_str.as_str()) || is_test_source {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("read Rust source");
            if [".write_to_clipboard(", ".read_from_clipboard("]
                .iter()
                .any(|forbidden| source.contains(forbidden))
            {
                offenders.push(relative_str);
            }
        }
        offenders
    }

    /// Files a crate deliberately exempts from [`discrete_control_violations`].
    pub struct DiscreteControlAllowlist<'a> {
        /// Custom hover/press styling (editing surfaces, the interaction kit).
        pub custom_styling: &'a [&'a str],
        /// Mouse-ups that end a continuous gesture rather than click.
        pub release_gestures: &'a [&'a str],
        /// Left presses that start gestures or manage focus/propagation.
        pub primary_press: &'a [&'a str],
        /// Context actions opened on press.
        pub context_press: &'a [&'a str],
    }

    /// Discrete controls must style and activate through the shared
    /// interaction APIs: no ad-hoc `.hover(|..`/`.active(|..`, raw
    /// `.on_click`, bare `.on_mouse_up`, or raw button presses.
    pub fn discrete_control_violations(
        src_dir: &Path,
        allow: &DiscreteControlAllowlist<'_>,
    ) -> Vec<String> {
        let pattern = regex::Regex::new(r"\.(?:hover|active)\(\s*(?:move\s+)?\|").unwrap();
        let raw_click =
            regex::Regex::new(r"\.on_click\(\s*(?:cx\.listener|on_click\b|(?:move\s+)?\|)")
                .unwrap();
        let raw_release = regex::Regex::new(r"\.on_mouse_up\(").unwrap();
        let primary_press =
            regex::Regex::new(r"\.on_mouse_down\(\s*(?:gpui::)?MouseButton::Left").unwrap();
        let context_press =
            regex::Regex::new(r"\.on_mouse_down\(\s*(?:gpui::)?MouseButton::Right").unwrap();
        let listed = |relative: &Path, list: &[&str]| {
            list.iter().any(|allowed| relative == Path::new(allowed))
        };
        let mut pending = vec![src_dir.to_path_buf()];
        let mut violations = Vec::new();
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy();
                if name == "tests"
                    || name == "benchmarks"
                    || name.contains("tests")
                    || name == "test_support.rs"
                {
                    continue;
                }
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().is_none_or(|extension| extension != "rs") {
                    continue;
                }
                let relative = path.strip_prefix(src_dir).unwrap();
                if listed(relative, allow.custom_styling) {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                if pattern.is_match(&source) {
                    violations.push(format!(
                        "{}: custom hover/press styling",
                        relative.display()
                    ));
                }
                if raw_click.is_match(&source) {
                    violations.push(format!("{}: raw click activation", relative.display()));
                }
                // Compare paths as paths so Windows separators match the lists.
                if raw_release.is_match(&source) && !listed(relative, allow.release_gestures) {
                    violations.push(format!(
                        "{}: release without the shared click gate",
                        relative.display()
                    ));
                }
                if context_press.is_match(&source) && !listed(relative, allow.context_press) {
                    violations.push(format!("{}: context action on press", relative.display()));
                }
                if primary_press.is_match(&source) && !listed(relative, allow.primary_press) {
                    violations.push(format!("{}: primary action on press", relative.display()));
                }
            }
        }
        violations
    }
}
