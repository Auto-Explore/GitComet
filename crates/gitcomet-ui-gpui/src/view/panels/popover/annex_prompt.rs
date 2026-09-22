//! The one git-annex prompt: a single text field for adding or enabling a
//! special remote, describing a repository or setting numcopies, and plain
//! confirmations for dropping content, dropping unused content and starting
//! the webapp.

use super::*;
use gitcomet_core::large_files::{AnnexUnused, AnnexUnusedKind, LargeFileCommand};
use gitcomet_core::text_utils::human_readable_bytes;

/// Keys listed before the rest collapse into "and N more".
const MAX_UNUSED_ROWS: usize = 8;

pub(super) fn title(prompt: &AnnexPrompt) -> &'static str {
    match prompt {
        AnnexPrompt::AddSpecialRemote => "Add special remote",
        AnnexPrompt::EnableSpecialRemote { .. } => "Enable special remote",
        AnnexPrompt::Describe { .. } => "Describe repository",
        AnnexPrompt::Numcopies { .. } => "Number of copies",
        AnnexPrompt::ForceDrop { .. } => "Drop without verified copies?",
        AnnexPrompt::Unused => "Unused annexed content",
        AnnexPrompt::Webapp => "Open the git-annex webapp?",
    }
}

fn field_label(prompt: &AnnexPrompt) -> Option<&'static str> {
    match prompt {
        AnnexPrompt::AddSpecialRemote => {
            Some("name type key=value … (e.g. backup directory directory=/mnt/usb encryption=none)")
        }
        AnnexPrompt::EnableSpecialRemote { .. } => {
            Some("name key=value … (a directory remote needs directory=/its/path again)")
        }
        AnnexPrompt::Describe { .. } => Some("Description"),
        AnnexPrompt::Numcopies { .. } => Some("Copies git-annex keeps before allowing a drop"),
        AnnexPrompt::ForceDrop { .. } | AnnexPrompt::Unused | AnnexPrompt::Webapp => None,
    }
}

/// Text the field starts with.
pub(super) fn initial_text(prompt: &AnnexPrompt) -> String {
    match prompt {
        AnnexPrompt::EnableSpecialRemote { name } => name.clone(),
        AnnexPrompt::Describe { current, .. } => current.clone(),
        AnnexPrompt::Numcopies { current } => current.unwrap_or(1).to_string(),
        AnnexPrompt::AddSpecialRemote
        | AnnexPrompt::ForceDrop { .. }
        | AnnexPrompt::Unused
        | AnnexPrompt::Webapp => String::new(),
    }
}

/// The command a submitted prompt runs, or why the text is not usable yet.
pub(super) fn prompt_command(
    prompt: &AnnexPrompt,
    text: &str,
) -> Result<LargeFileCommand, &'static str> {
    let text = text.trim();
    match prompt {
        AnnexPrompt::AddSpecialRemote => {
            let mut parts = text.split_whitespace();
            let name = parts.next().ok_or("Enter a name and a type")?;
            let special_type = parts.next().ok_or("Enter the remote type after the name")?;
            let params: Vec<String> = parts.map(str::to_string).collect();
            if params.iter().any(|param| !param.contains('=')) {
                return Err("Parameters after the type are key=value pairs");
            }
            Ok(LargeFileCommand::AnnexInitRemote {
                name: name.to_string(),
                special_type: special_type.to_string(),
                params,
            })
        }
        AnnexPrompt::EnableSpecialRemote { .. } => {
            let mut parts = text.split_whitespace();
            let name = parts.next().ok_or("Enter the special remote name")?;
            let params: Vec<String> = parts.map(str::to_string).collect();
            if params.iter().any(|param| !param.contains('=')) {
                return Err("Settings after the name are key=value pairs");
            }
            Ok(LargeFileCommand::AnnexEnableRemote {
                name: name.to_string(),
                params,
            })
        }
        AnnexPrompt::Describe { repository, .. } => {
            if text.is_empty() {
                return Err("Enter a description");
            }
            Ok(LargeFileCommand::AnnexDescribe {
                repository: repository.clone(),
                description: text.to_string(),
            })
        }
        AnnexPrompt::Numcopies { .. } => match text.parse::<u32>() {
            Ok(copies) if copies > 0 => Ok(LargeFileCommand::AnnexNumcopies { copies }),
            _ => Err("Enter a whole number of at least 1"),
        },
        AnnexPrompt::ForceDrop { paths } => Ok(LargeFileCommand::AnnexDrop {
            paths: paths.clone(),
            from: None,
            force: true,
        }),
        AnnexPrompt::Unused => Ok(LargeFileCommand::AnnexDropUnused { force: false }),
        AnnexPrompt::Webapp => Ok(LargeFileCommand::AnnexWebapp),
    }
}

/// The listing when it has something to drop.
pub(super) fn droppable_unused(repo: Option<&RepoState>) -> Option<&AnnexUnused> {
    match &repo?.annex_unused {
        Loadable::Ready(unused) if !unused.entries.is_empty() => Some(unused),
        _ => None,
    }
}

/// "3 unused versions · 12 MB", or why there is no list.
pub(super) fn unused_summary(unused: &Loadable<std::sync::Arc<AnnexUnused>>) -> String {
    match unused {
        Loadable::NotLoaded | Loadable::Loading => "Looking for unused content…".to_string(),
        Loadable::Error(error) => format!("git annex unused failed: {error}"),
        Loadable::Ready(unused) if unused.entries.is_empty() => {
            "No unused content. Every annexed file here is still used by a branch or tag."
                .to_string()
        }
        Loadable::Ready(unused) => {
            let count = unused.entries.len();
            let noun = if count == 1 { "item" } else { "items" };
            match unused.known_bytes() {
                0 => format!("{count} {noun} no branch or tag uses"),
                bytes => format!(
                    "{count} {noun} no branch or tag uses · {}",
                    human_readable_bytes(bytes)
                ),
            }
        }
    }
}

fn unused_row(kind: AnnexUnusedKind, key: &str) -> String {
    let size = gitcomet_core::annex::parse_key(key)
        .and_then(|key| key.size)
        .map(human_readable_bytes);
    let kind = match kind {
        AnnexUnusedKind::Unused => "old version",
        AnnexUnusedKind::Bad => "corrupt copy",
        AnnexUnusedKind::Temporary => "partial download",
    };
    match size {
        Some(size) => format!("{kind} · {size} · {key}"),
        None => format!("{kind} · {key}"),
    }
}

fn unused_body(theme: AppTheme, repo: Option<&RepoState>, mut body: gpui::Div) -> gpui::Div {
    let Some(repo) = repo else {
        return body;
    };
    body = body.child(super::popover_detail(
        theme,
        unused_summary(&repo.annex_unused),
    ));
    if let Some(unused) = droppable_unused(Some(repo)) {
        let mut list = div()
            .px_2()
            .pb_1()
            .flex()
            .flex_col()
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.secondary);
        for entry in unused.entries.iter().take(MAX_UNUSED_ROWS) {
            list = list.child(
                div()
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .child(unused_row(entry.kind, &entry.key)),
            );
        }
        if unused.entries.len() > MAX_UNUSED_ROWS {
            list = list.child(format!(
                "and {} more",
                unused.entries.len() - MAX_UNUSED_ROWS
            ));
        }
        body = body.child(list).child(super::popover_detail(
            theme,
            "Drop keeps anything git-annex cannot verify in another repository. \
             Dropping without verified copies deletes it anyway.",
        ));
    }
    body
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    prompt: &AnnexPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let scaled_px = super::popover_scaled_px_fn(cx);
    let text = this
        .submodule_ref_input
        .read_with(cx, |input, _| input.text().to_string());
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let verdict = prompt_command(prompt, &text);
    let force_drop = matches!(prompt, AnnexPrompt::ForceDrop { .. });
    let is_unused = matches!(prompt, AnnexPrompt::Unused);
    let can_drop_unused = droppable_unused(repo).is_some();
    let mut body = div()
        .flex()
        .flex_col()
        .w(scaled_px(460.0))
        .child(popover_title(theme, title(prompt)))
        .child(super::popover_rule(theme));
    if let AnnexPrompt::ForceDrop { paths } = prompt {
        body = body.child(super::popover_detail(
            theme,
            format!(
                "git-annex could not verify enough other copies of {}. Dropping anyway can lose the only copy.",
                match paths.as_slice() {
                    [one] => one.display().to_string(),
                    many => format!("{} files", many.len()),
                }
            ),
        ));
    }
    match prompt {
        AnnexPrompt::Unused => body = unused_body(theme, repo, body),
        AnnexPrompt::Webapp => {
            body = body.child(super::popover_detail(
                theme,
                "git-annex's own interface opens in your browser. It runs the \
                 git-annex assistant, which adds and commits changes in this \
                 repository by itself and syncs them, until you stop it from the \
                 webapp or from the git-annex menu.",
            ));
        }
        _ => {}
    }
    if let Some(field) = field_label(prompt) {
        body = body.child(input_label(theme, field)).child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.submodule_ref_input.clone()),
        );
        if let Err(reason) = &verdict
            && !text.trim().is_empty()
        {
            body = body.child(super::popover_detail(theme, reason.to_string()));
        }
    }
    let submit_label = match prompt {
        AnnexPrompt::ForceDrop { .. } => "Drop anyway",
        AnnexPrompt::Unused => "Drop",
        AnnexPrompt::Webapp => "Open webapp",
        _ => "Apply",
    };
    let force_unused = is_unused.then(|| {
        components::Button::new("annex_prompt_force_unused", "Drop without verified copies")
            .style(components::ButtonStyle::Danger)
            .disabled(!can_drop_unused)
            .on_click(theme, cx, move |this, _e, window, cx| {
                this.store.dispatch(Msg::RunLargeFileCommand {
                    repo_id,
                    command: LargeFileCommand::AnnexDropUnused { force: true },
                });
                this.dismiss_inline_popover(window, cx);
            })
    });
    body.child(super::popover_rule(theme)).child(
        super::prompt_footer_row()
            .child(
                cancel_button("annex_prompt_cancel", "annex_prompt_cancel_hint", theme).on_click(
                    theme,
                    cx,
                    |this, _e, window, cx| this.dismiss_inline_popover(window, cx),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .children(force_unused)
                    .child(
                        components::Button::new("annex_prompt_go", submit_label)
                            .separated_end_slot(super::hotkey_hint(
                                theme,
                                "annex_prompt_go_hint",
                                "Enter",
                            ))
                            .style(if force_drop {
                                components::ButtonStyle::Solid
                            } else {
                                components::ButtonStyle::Filled
                            })
                            .disabled(verdict.is_err() || (is_unused && !can_drop_unused))
                            .on_click(theme, cx, |this, _e, window, cx| {
                                this.submit_annex_prompt(window, cx);
                            }),
                    ),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_build_commands_or_explain_what_is_missing() {
        let add = AnnexPrompt::AddSpecialRemote;
        assert!(matches!(
            prompt_command(&add, "backup directory directory=/mnt/usb encryption=none"),
            Ok(LargeFileCommand::AnnexInitRemote { name, special_type, params })
                if name == "backup" && special_type == "directory" && params.len() == 2
        ));
        assert_eq!(
            prompt_command(&add, "backup"),
            Err("Enter the remote type after the name")
        );
        assert_eq!(
            prompt_command(&add, "backup directory oops"),
            Err("Parameters after the type are key=value pairs")
        );
        let enable = AnnexPrompt::EnableSpecialRemote { name: "usb".into() };
        assert_eq!(initial_text(&enable), "usb");
        assert!(matches!(
            prompt_command(&enable, "usb directory=/mnt/usb"),
            Ok(LargeFileCommand::AnnexEnableRemote { name, params })
                if name == "usb" && params == ["directory=/mnt/usb"]
        ));
        assert!(prompt_command(&enable, "usb /mnt/usb").is_err());
        let copies = AnnexPrompt::Numcopies { current: Some(2) };
        assert_eq!(initial_text(&copies), "2");
        assert!(prompt_command(&copies, "0").is_err());
        assert!(matches!(
            prompt_command(&copies, " 3 "),
            Ok(LargeFileCommand::AnnexNumcopies { copies: 3 })
        ));
        assert!(matches!(
            prompt_command(&AnnexPrompt::Unused, ""),
            Ok(LargeFileCommand::AnnexDropUnused { force: false })
        ));
        assert!(matches!(
            prompt_command(&AnnexPrompt::Webapp, ""),
            Ok(LargeFileCommand::AnnexWebapp)
        ));
        let drop = AnnexPrompt::ForceDrop {
            paths: vec!["a.bin".into()],
        };
        assert!(matches!(
            prompt_command(&drop, ""),
            Ok(LargeFileCommand::AnnexDrop { force: true, .. })
        ));
    }

    #[test]
    fn unused_summary_counts_items_and_known_sizes() {
        use gitcomet_core::large_files::{AnnexUnusedEntry, AnnexUnusedKind};
        let entry = |key: &str| AnnexUnusedEntry {
            key: key.into(),
            kind: AnnexUnusedKind::Unused,
        };
        let listed = AnnexUnused {
            entries: vec![entry("SHA256E-s1500--a.bin"), entry("URL--https&c%%x")],
        };
        assert_eq!(
            unused_summary(&Loadable::Ready(std::sync::Arc::new(listed))),
            "2 items no branch or tag uses · 1.5 KB"
        );
        assert!(
            unused_summary(&Loadable::Ready(Default::default())).starts_with("No unused content")
        );
        assert_eq!(
            unused_row(AnnexUnusedKind::Temporary, "URL--x"),
            "partial download · URL--x"
        );
    }
}
