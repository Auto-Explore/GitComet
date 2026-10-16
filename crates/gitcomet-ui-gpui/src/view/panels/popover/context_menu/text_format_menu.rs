use super::*;
use crate::view::mod_helpers::TextFormatMenuSection;
use crate::view::panes::diff::TextEncodingMenuState;
use gitcomet_core::text_format::{LineEnding, TextEncoding};

pub(super) fn model(
    host: &PopoverHost,
    section: TextFormatMenuSection,
    cx: &gpui::App,
) -> ContextMenuModel {
    let pane = host.diff_renderer.read(cx);
    match section {
        TextFormatMenuSection::ImageSampling => {
            image_sampling_model(crate::view::image_preview::Preferences::current(cx))
        }
        TextFormatMenuSection::ImageZoom => {
            use crate::view::image_preview::Zoom;
            let mut items = vec![
                ContextMenuItem::Header("Image zoom".into()),
                ContextMenuItem::Entry {
                    label: "Fit".into(),
                    icon: None,
                    shortcut: Some("0".into()),
                    disabled: false,
                    action: Box::new(ContextMenuAction::ImageZoom { zoom: Zoom::Fit }),
                },
            ];
            for percent in [
                1, 2, 5, 10, 25, 50, 75, 100, 125, 150, 200, 300, 400, 800, 1600, 3200, 6400,
            ] {
                items.push(ContextMenuItem::Entry {
                    label: if percent == 100 {
                        "Actual size (100%)".into()
                    } else {
                        format!("{percent}%").into()
                    },
                    icon: None,
                    shortcut: (percent == 100).then(|| "1".into()),
                    disabled: false,
                    action: Box::new(ContextMenuAction::ImageZoom {
                        zoom: Zoom::Scale(f64::from(percent) / 100.0),
                    }),
                });
            }
            ContextMenuModel::new(items)
        }
        TextFormatMenuSection::Encoding => match pane.text_encoding_menu_state() {
            Some(state) => encoding_model(&state),
            None => {
                ContextMenuModel::new(vec![ContextMenuItem::Label("No file text is shown".into())])
            }
        },
        TextFormatMenuSection::TabSize => {
            let (current, _) = pane.effective_tab_size();
            let chosen = pane.active_repo().and_then(|repo| {
                let diff = pane.bound_diff_state(repo);
                let path = diff.diff_target.as_ref()?.file_path()?;
                diff.text_override_for(path)?.tab_size
            });
            tab_size_model(current, chosen, pane.default_tab_size)
        }
        TextFormatMenuSection::LineEnding => {
            let status = pane.text_format_status();
            line_ending_model(
                status.as_ref().is_some_and(|status| status.editable),
                status
                    .as_ref()
                    .map(|status| status.line_ending_tooltip.clone())
                    .unwrap_or_default(),
            )
        }
    }
}

fn image_sampling_model(prefs: crate::view::image_preview::Preferences) -> ContextMenuModel {
    use crate::view::image_preview::{ImageSamplingMode, ImageSmoothFilter};
    let mut items = vec![
        ContextMenuItem::Header("Image sampling".into()),
        ContextMenuItem::Description(
            "Auto uses nearest neighbor at 300% and above. 100% preserves original pixels.".into(),
        ),
    ];
    for mode in ImageSamplingMode::ALL {
        items.push(ContextMenuItem::Entry {
            label: mode.label().into(),
            icon: check(prefs.mode == *mode),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetImageSampling { mode: *mode }),
        });
    }
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Header("Smooth filter".into()));
    for filter in ImageSmoothFilter::ALL {
        items.push(ContextMenuItem::Entry {
            label: filter.label().into(),
            icon: check(prefs.filter == *filter),
            shortcut: None,
            disabled: prefs.mode == ImageSamplingMode::Pixels,
            action: Box::new(ContextMenuAction::SetImageSmoothFilter { filter: *filter }),
        });
    }
    ContextMenuModel::new(items)
}

fn check(enabled: bool) -> Option<SharedString> {
    enabled.then_some("icons/check.svg".into())
}

fn encoding_model(state: &TextEncodingMenuState) -> ContextMenuModel {
    if state.editor && state.unsaved {
        return unsaved_encoding_model(state);
    }
    let mut items = vec![ContextMenuItem::Header("Reopen with encoding".into())];
    if state.stored_as_utf8 {
        items.push(ContextMenuItem::Description(
            "Git stores this file as UTF-8 (working-tree-encoding); the choice applies to the working-tree file."
                .into(),
        ));
    }
    items.push(ContextMenuItem::Separator);
    let auto_label = match (state.chosen, state.current) {
        (None, Some(current)) => format!("Auto-detect ({current})"),
        _ => "Auto-detect".to_string(),
    };
    items.push(ContextMenuItem::Entry {
        label: auto_label.into(),
        icon: check(state.chosen.is_none()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::SetTextEncoding { encoding: None }),
    });
    let mut group = "";
    for encoding in TextEncoding::all() {
        if encoding.group() != group {
            group = encoding.group();
            items.push(ContextMenuItem::Header(group.into()));
        }
        items.push(ContextMenuItem::Entry {
            label: encoding.name().into(),
            icon: check(state.chosen == Some(encoding)),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetTextEncoding {
                encoding: Some(encoding),
            }),
        });
    }
    if state.editor {
        items.extend(save_with_items(state));
    }
    if let Some(chosen) = state.chosen {
        items.extend(remember_items(state, chosen));
    }
    ContextMenuModel::new(items)
}

/// Unsaved edits cannot be reopened, so every pick sets what Save writes.
fn unsaved_encoding_model(state: &TextEncodingMenuState) -> ContextMenuModel {
    use gitcomet_core::text_format::TextFormat;

    let mut items = vec![
        ContextMenuItem::Header("Save with encoding".into()),
        ContextMenuItem::Description(
            "Unsaved edits are written in the encoding you pick. Discard them to reopen the file in another encoding."
                .into(),
        ),
        ContextMenuItem::Separator,
    ];
    let legacy = TextEncoding::all()
        .filter(|encoding| !encoding.is_utf8() && !encoding.is_utf16())
        .map(|encoding| {
            (
                encoding.name().to_string(),
                TextFormat {
                    encoding,
                    bom: false,
                },
            )
        });
    let mut group = "";
    for (label, format) in unicode_save_formats().into_iter().chain(legacy) {
        if format.encoding.group() != group {
            group = format.encoding.group();
            items.push(ContextMenuItem::Header(group.into()));
        }
        items.push(save_with_entry(state, label, format));
    }
    if let Some(chosen) = state.chosen {
        items.extend(remember_items(state, chosen));
    }
    ContextMenuModel::new(items)
}

fn save_with_entry(
    state: &TextEncodingMenuState,
    label: String,
    format: gitcomet_core::text_format::TextFormat,
) -> ContextMenuItem {
    ContextMenuItem::Entry {
        label: label.into(),
        icon: check(state.save_format == Some(format)),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::SaveWithEncoding { format }),
    }
}

/// UTF-16 only with a byte-order mark, which is what lets it be read back.
fn unicode_save_formats() -> Vec<(String, gitcomet_core::text_format::TextFormat)> {
    use gitcomet_core::text_format::TextFormat;

    vec![
        ("UTF-8".to_string(), TextFormat::UTF_8),
        (
            "UTF-8 with BOM".to_string(),
            TextFormat {
                encoding: TextEncoding::UTF_8,
                bom: true,
            },
        ),
        (
            "UTF-16LE with BOM".to_string(),
            TextFormat {
                encoding: TextEncoding::UTF_16LE,
                bom: true,
            },
        ),
        (
            "UTF-16BE with BOM".to_string(),
            TextFormat {
                encoding: TextEncoding::UTF_16BE,
                bom: true,
            },
        ),
    ]
}

/// Converting the editor's file: the Unicode encodings, with and without a
/// byte-order mark, plus the one it was read in.
fn save_with_items(state: &TextEncodingMenuState) -> Vec<ContextMenuItem> {
    use gitcomet_core::text_format::TextFormat;

    let mut formats = unicode_save_formats();
    if let Some(current) = state
        .current
        .filter(|current| !current.is_utf8() && !current.is_utf16())
    {
        formats.push((
            current.name().to_string(),
            TextFormat {
                encoding: current,
                bom: false,
            },
        ));
    }
    let mut items = vec![
        ContextMenuItem::Separator,
        ContextMenuItem::Header("Save with encoding".into()),
        ContextMenuItem::Description("Converts the file the next time it is saved.".into()),
    ];
    items.extend(
        formats
            .into_iter()
            .map(|(label, format)| save_with_entry(state, label, format)),
    );
    items
}

/// "Save to .gitattributes" for a chosen encoding: the display-only
/// `encoding` attribute, or git's storage conversion behind its warning.
fn remember_items(state: &TextEncodingMenuState, chosen: TextEncoding) -> Vec<ContextMenuItem> {
    use gitcomet_core::gitattributes::{pattern_for_extension, pattern_for_path};

    let entry = |label: String, rule: String| ContextMenuItem::Entry {
        label: label.into(),
        icon: None,
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::AddGitattributesRule { rule }),
    };
    let file_pattern = pattern_for_path(&state.path);
    let extension_pattern = pattern_for_extension(&state.path);
    // git-gui reads `encoding` with its own names; ours round-trip through
    // `TextEncoding::from_label`, as does iconv's spelling below.
    let display_label = chosen.git_label();
    let mut items = vec![
        ContextMenuItem::Separator,
        ContextMenuItem::Header("Save to .gitattributes".into()),
        ContextMenuItem::Description(
            format!(
                "encoding= only changes how {} and git-gui show the file.",
                crate::view::product_name()
            )
            .into(),
        ),
        entry(
            format!("This file: encoding={display_label}"),
            format!("{file_pattern} encoding={display_label}"),
        ),
    ];
    if let Some(extension_pattern) = &extension_pattern {
        items.push(entry(
            format!("All {extension_pattern} files: encoding={display_label}"),
            format!("{extension_pattern} encoding={display_label}"),
        ));
    }
    if !chosen.is_utf8() {
        let Some(label) = working_tree_encoding_label(chosen, state.had_bom) else {
            items.push(ContextMenuItem::Description(
                "Git conversion cannot preserve a UTF-16BE byte-order mark. Save as UTF-16LE with BOM to enable it."
                    .into(),
            ));
            return items;
        };
        items.push(ContextMenuItem::Description(
            "working-tree-encoding= makes Git store the file as UTF-8 from the next add: run git add --renormalize, old commits keep their bytes, and everyone's Git must support the encoding."
                .into(),
        ));
        items.push(entry(
            format!("This file: working-tree-encoding={label}"),
            format!("{file_pattern} working-tree-encoding={label}"),
        ));
    }
    items
}

/// Only offer labels that Git can use without changing the byte order.
/// `UTF-16BE-BOM` is not an iconv label, and generic `UTF-16` checks out in
/// native byte order, so neither can preserve BOM-marked big-endian files.
fn working_tree_encoding_label(encoding: TextEncoding, had_bom: bool) -> Option<&'static str> {
    if had_bom && encoding == TextEncoding::UTF_16BE {
        None
    } else if had_bom && encoding == TextEncoding::UTF_16LE {
        Some("UTF-16LE-BOM")
    } else {
        Some(encoding.git_label())
    }
}

fn tab_size_model(current: u8, chosen: Option<u8>, default: u8) -> ContextMenuModel {
    let mut items = vec![
        ContextMenuItem::Header("Tab size".into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: format!("Default ({default})").into(),
            icon: check(chosen.is_none()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetTabSize { size: None }),
        },
    ];
    for size in [2u8, 3, 4, 6, 8] {
        items.push(ContextMenuItem::Entry {
            label: format!("{size} spaces").into(),
            icon: check(chosen == Some(size)),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetTabSize { size: Some(size) }),
        });
    }
    if chosen.is_none() && current != default {
        items.insert(
            1,
            ContextMenuItem::Description(
                format!("This file uses {current} from its attributes.").into(),
            ),
        );
    }
    ContextMenuModel::new(items)
}

fn line_ending_model(editable: bool, policy: SharedString) -> ContextMenuModel {
    let mut items = vec![
        ContextMenuItem::Header("Line endings".into()),
        ContextMenuItem::Description(policy.to_string().into()),
    ];
    if editable {
        items.push(ContextMenuItem::Separator);
        for ending in [LineEnding::Lf, LineEnding::CrLf] {
            items.push(ContextMenuItem::Entry {
                label: format!("Convert to {}", ending.label()).into(),
                icon: None,
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::ConvertLineEndings { ending }),
            });
        }
    }
    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_mode_disables_smooth_filters_and_keeps_the_choice() {
        use crate::view::image_preview::{ImageSamplingMode, ImageSmoothFilter, Preferences};
        let model = image_sampling_model(Preferences {
            mode: ImageSamplingMode::Pixels,
            filter: ImageSmoothFilter::Hann,
        });
        let mut filters = 0;
        for item in model.items {
            if let ContextMenuItem::Entry {
                disabled,
                icon,
                action,
                ..
            } = item
            {
                match *action {
                    ContextMenuAction::SetImageSmoothFilter { filter } => {
                        filters += 1;
                        assert!(disabled);
                        assert_eq!(icon.is_some(), filter == ImageSmoothFilter::Hann);
                    }
                    ContextMenuAction::SetImageSampling { .. } => assert!(!disabled),
                    _ => panic!("unexpected image menu action"),
                }
            }
        }
        assert_eq!(filters, 3);
    }

    fn state(chosen: Option<TextEncoding>) -> TextEncodingMenuState {
        TextEncodingMenuState {
            path: "docs/read me.txt".into(),
            chosen,
            current: Some(TextEncoding::WINDOWS_1252),
            stored_as_utf8: false,
            had_bom: false,
            editor: false,
            unsaved: false,
            save_format: None,
        }
    }

    #[test]
    fn unsaved_edits_turn_the_menu_into_save_with() {
        use gitcomet_core::text_format::TextFormat;

        let mut state = state(None);
        state.editor = true;
        state.unsaved = true;
        state.save_format = Some(TextFormat {
            encoding: TextEncoding::WINDOWS_1252,
            bom: false,
        });
        let model = encoding_model(&state);
        assert!(matches!(
            model.items.first(),
            Some(ContextMenuItem::Header(header)) if header.as_ref() == "Save with encoding"
        ));
        let mut checked = Vec::new();
        let mut saves = Vec::new();
        for item in &model.items {
            if let ContextMenuItem::Entry {
                label,
                icon,
                action,
                ..
            } = item
            {
                match action.as_ref() {
                    ContextMenuAction::SetTextEncoding { .. } => {
                        panic!("reopening would drop the unsaved edits")
                    }
                    ContextMenuAction::SaveWithEncoding { format } => saves.push(*format),
                    _ => {}
                }
                if icon.is_some() {
                    checked.push(label.to_string());
                }
            }
        }
        assert_eq!(checked, vec!["Windows-1252".to_string()]);
        assert_eq!(saves.first(), Some(&TextFormat::UTF_8));
        assert!(saves.contains(&TextFormat {
            encoding: TextEncoding::from_label("koi8-r").unwrap(),
            bom: false,
        }));
    }

    fn rules(model: &ContextMenuModel) -> Vec<String> {
        model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { action, .. } => match action.as_ref() {
                    ContextMenuAction::AddGitattributesRule { rule } => Some(rule.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn remembering_is_offered_only_after_a_choice() {
        assert!(rules(&encoding_model(&state(None))).is_empty());
        let koi8 = TextEncoding::from_label("koi8-r");
        assert_eq!(
            rules(&encoding_model(&state(koi8))),
            vec![
                "\"/docs/read me.txt\" encoding=KOI8-R".to_string(),
                "*.txt encoding=KOI8-R".to_string(),
                "\"/docs/read me.txt\" working-tree-encoding=KOI8-R".to_string(),
            ]
        );
    }

    #[test]
    fn utf16_rules_use_supported_labels_and_preserve_byte_order() {
        for (encoding, had_bom, label) in [
            (TextEncoding::UTF_16LE, true, Some("UTF-16LE-BOM")),
            (TextEncoding::UTF_16LE, false, Some("UTF-16LE")),
            (TextEncoding::UTF_16BE, true, None),
            (TextEncoding::UTF_16BE, false, Some("UTF-16BE")),
        ] {
            let mut state = state(Some(encoding));
            state.had_bom = had_bom;
            let rules = rules(&encoding_model(&state));
            let conversion = rules
                .iter()
                .find(|rule| rule.contains("working-tree-encoding="));
            assert_eq!(
                conversion.map(String::as_str),
                label
                    .map(|label| format!("\"/docs/read me.txt\" working-tree-encoding={label}"))
                    .as_deref()
            );
            assert!(
                rules
                    .iter()
                    .any(|rule| rule.ends_with(&format!(" encoding={}", encoding.git_label())))
            );
        }
    }

    #[test]
    fn line_endings_convert_only_where_the_view_writes() {
        let entries = |model: ContextMenuModel| {
            model
                .items
                .iter()
                .filter(|item| matches!(item, ContextMenuItem::Entry { .. }))
                .count()
        };
        assert_eq!(entries(line_ending_model(false, "".into())), 0);
        assert_eq!(entries(line_ending_model(true, "".into())), 2);
    }
}
