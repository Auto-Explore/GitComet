use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn model(
    repo_id: RepoId,
    area: DiffArea,
    path: &Option<std::path::PathBuf>,
    hunk_patch: &Option<String>,
    hunks_count: usize,
    lines_patch: &Option<String>,
    discard_lines_patch: &Option<String>,
    lines_count: usize,
    copy_text: &Option<String>,
    copy_target: Option<(usize, DiffTextRegion)>,
) -> ContextMenuModel {
    let title = path
        .as_ref()
        .and_then(|p| {
            p.file_name()
                .and_then(|name| name.to_str().map(ToOwned::to_owned))
                .map(Into::into)
        })
        .unwrap_or_else(|| "Diff".into());

    let mut items = vec![ContextMenuItem::Header(title)];
    if let Some(path) = path {
        items.push(ContextMenuItem::Label(path.display().to_string().into()));
    }
    items.push(ContextMenuItem::Separator);

    let (line_label, line_icon, line_shortcut, line_reverse) = match area {
        DiffArea::Unstaged => ("Stage line", "icons/plus.svg", Some("S"), false),
        DiffArea::Staged => ("Unstage line", "icons/minus.svg", Some("U"), true),
    };
    items.push(ContextMenuItem::Entry {
        label: if lines_count > 1 {
            format!("{line_label}s ({lines_count})").into()
        } else {
            line_label.into()
        },
        icon: Some(line_icon.into()),
        shortcut: line_shortcut.map(Into::into),
        disabled: lines_patch.is_none(),
        action: Box::new(ContextMenuAction::ApplyIndexPatch {
            repo_id,
            patch: lines_patch.clone().unwrap_or_default(),
            reverse: line_reverse,
        }),
    });

    if area == DiffArea::Unstaged {
        items.push(ContextMenuItem::Entry {
            label: if lines_count > 1 {
                format!("Discard lines ({lines_count})").into()
            } else {
                "Discard line".into()
            },
            icon: Some("icons/refresh.svg".into()),
            shortcut: Some("D".into()),
            disabled: discard_lines_patch.is_none(),
            action: Box::new(ContextMenuAction::ApplyWorktreePatch {
                repo_id,
                patch: discard_lines_patch.clone().unwrap_or_default(),
                reverse: true,
            }),
        });
    }

    items.push(ContextMenuItem::Separator);

    let (hunk_label, hunk_icon, hunk_reverse) = match area {
        DiffArea::Unstaged => ("Stage hunk", "icons/plus.svg", false),
        DiffArea::Staged => ("Unstage hunk", "icons/minus.svg", true),
    };
    items.push(ContextMenuItem::Entry {
        label: if hunks_count > 1 {
            format!("{}s ({hunks_count})", hunk_label).into()
        } else {
            hunk_label.into()
        },
        icon: Some(hunk_icon.into()),
        shortcut: None,
        disabled: hunk_patch.is_none(),
        action: Box::new(ContextMenuAction::ApplyIndexPatch {
            repo_id,
            patch: hunk_patch.clone().unwrap_or_default(),
            reverse: hunk_reverse,
        }),
    });

    if area == DiffArea::Unstaged {
        items.push(ContextMenuItem::Entry {
            label: if hunks_count > 1 {
                format!("Discard hunks ({hunks_count})").into()
            } else {
                "Discard hunk".into()
            },
            icon: Some("icons/refresh.svg".into()),
            shortcut: None,
            disabled: hunk_patch.is_none(),
            action: Box::new(ContextMenuAction::ApplyWorktreePatch {
                repo_id,
                patch: hunk_patch.clone().unwrap_or_default(),
                reverse: true,
            }),
        });
    }

    items.push(ContextMenuItem::Separator);
    if let Some(path) = path {
        items.push(ContextMenuItem::Entry {
            label: "Open file".into(),
            icon: Some("icons/file.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenFile {
                repo_id,
                path: path.clone(),
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: "Open file location".into(),
            icon: Some("icons/folder.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenFileLocation {
                repo_id,
                path: path.clone(),
            }),
        });
        items.push(ContextMenuItem::Separator);
    }
    items.push(ContextMenuItem::Entry {
        label: "Copy".into(),
        icon: Some("icons/copy.svg".into()),
        shortcut: Some("C".into()),
        disabled: copy_text
            .as_ref()
            .map(|text| text.trim().is_empty())
            .unwrap_or(copy_target.is_none()),
        action: Box::new(match copy_text {
            Some(text) => ContextMenuAction::CopyText { text: text.clone() },
            None => {
                let (visible_ix, region) = copy_target.unwrap_or((0, DiffTextRegion::Inline));
                ContextMenuAction::CopyDiffText { visible_ix, region }
            }
        }),
    });

    ContextMenuModel::new(items)
}
