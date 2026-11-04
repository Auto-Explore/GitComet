use super::*;
use gitcomet_core::domain::SubmoduleCheckoutUpdate;
use gitcomet_state::model::UnpushedPointerPrompt;

fn name(pointer: &SubmoduleCheckoutUpdate) -> String {
    gitcomet_state::model::submodule_display_name(&pointer.path)
}

fn short(pointer: &SubmoduleCheckoutUpdate) -> &str {
    gitcomet_core::domain::short_commit_id(pointer.commit.as_ref())
}

/// The words the prompt uses: one submodule is named, several are counted.
pub(super) struct UnpushedPointerCopy {
    pub(super) title: String,
    pub(super) body: String,
    pub(super) push_first_label: String,
}

pub(super) fn copy(prompt: &UnpushedPointerPrompt) -> UnpushedPointerCopy {
    match prompt.pointers.as_slice() {
        [pointer] => UnpushedPointerCopy {
            title: format!("Push {} first", name(pointer)),
            body: format!(
                "The commits you are pushing record {} at {}, which its remote does not have yet. Others would get a submodule commit they cannot fetch.",
                name(pointer),
                short(pointer)
            ),
            push_first_label: format!("Push {} first", name(pointer)),
        },
        pointers => UnpushedPointerCopy {
            title: format!("Push {} submodules first", pointers.len()),
            body: "The commits you are pushing record submodule commits their remotes do not have yet. Others would get submodule commits they cannot fetch.".to_string(),
            push_first_label: "Push submodules first".to_string(),
        },
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: UnpushedPointerPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let copy = copy(&prompt);
    let mut dialog = ConfirmDialog::new(copy.title, DIALOG_420_WIDTH).text(theme, copy.body);
    for pointer in &prompt.pointers {
        dialog = dialog.mono_value(
            theme,
            format!("{}  {}", pointer.path.display(), short(pointer)),
        );
    }

    let cancel_prompt = prompt.clone();
    let anyway_prompt = prompt.clone();
    let first_prompt = prompt;
    let cancel = cancel_button(
        "unpushed_pointer_cancel",
        "unpushed_pointer_cancel_hint",
        theme,
    )
    .on_click(theme, cx, move |this, _e, window, cx| {
        this.store.dispatch(Msg::DismissUnpushedPointerPrompt {
            prompt: cancel_prompt.clone(),
        });
        this.close_popover_and_restore_focus(window, cx);
    });
    let actions = div()
        .flex()
        .gap_1()
        .child(
            components::Button::new("unpushed_pointer_push_anyway", "Push anyway")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::ResolveUnpushedPointerPrompt {
                        prompt: anyway_prompt.clone(),
                        push_submodules_first: false,
                    });
                    this.close_resolved_popover(cx);
                }),
        )
        .child(
            components::Button::new("unpushed_pointer_push_first", copy.push_first_label)
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::ResolveUnpushedPointerPrompt {
                        prompt: first_prompt.clone(),
                        push_submodules_first: true,
                    });
                    this.close_resolved_popover(cx);
                }),
        );
    dialog.render(theme, cancel, actions, cx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::CommitId;
    use std::path::PathBuf;

    fn pointer(path: &str) -> SubmoduleCheckoutUpdate {
        SubmoduleCheckoutUpdate {
            path: PathBuf::from(path),
            commit: CommitId("c41f9e2aaaa".into()),
        }
    }

    #[test]
    fn one_unpushed_submodule_is_named() {
        let copy = copy(&UnpushedPointerPrompt {
            generation: 1,
            repo_id: RepoId(1),
            pointers: vec![pointer("fixtures/syntax_test")],
            push: gitcomet_state::model::BlockedPush::Push,
        });
        assert_eq!(copy.title, "Push syntax_test first");
        assert_eq!(copy.push_first_label, "Push syntax_test first");
        assert!(copy.body.contains("record syntax_test at c41f9e2"));
    }

    #[test]
    fn several_unpushed_submodules_are_counted() {
        let copy = copy(&UnpushedPointerPrompt {
            generation: 1,
            repo_id: RepoId(1),
            pointers: vec![pointer("a"), pointer("vendor/b")],
            push: gitcomet_state::model::BlockedPush::Push,
        });
        assert_eq!(copy.title, "Push 2 submodules first");
        assert_eq!(copy.push_first_label, "Push submodules first");
    }
}
