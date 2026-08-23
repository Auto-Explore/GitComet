use super::*;

pub(super) fn merge_commit_destination_label(repo: Option<&RepoState>) -> SharedString {
    repo.and_then(|repo| match &repo.head_branch {
        Loadable::Ready(head) if !head.is_empty() && head != "HEAD" => Some(head.as_str().into()),
        _ => None,
    })
    .unwrap_or_else(|| "HEAD".into())
}

fn merge_commit_repo_is_ready(repo: Option<&RepoState>) -> bool {
    repo.is_some_and(|repo| !repo.history_rewrite_busy())
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    commit_id: CommitId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let sha = commit_id.as_ref();
    let short: SharedString = sha.get(0..7).unwrap_or(sha).into();
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let summary = repo
        .and_then(|repo| match &repo.log {
            Loadable::Ready(page) => page
                .commits
                .iter()
                .find(|commit| commit.id == commit_id)
                .map(|commit| commit.summary.to_string()),
            _ => None,
        })
        .unwrap_or_default();
    let destination = merge_commit_destination_label(repo);
    let merge_ready = merge_commit_repo_is_ready(repo);

    let dispatch = move |this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>| {
        // The dialog can stay open while another command starts. Recheck the
        // live repository instead of relying only on the render-time button.
        let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
        if !merge_commit_repo_is_ready(repo) {
            cx.notify();
            return;
        }
        this.store.dispatch(Msg::MergeRef {
            repo_id,
            reference: commit_id.as_ref().to_string(),
        });
        this.close_popover(cx);
    };

    let mut dialog = ConfirmDialog::new("Merge commit?", DIALOG_380_WIDTH)
        .text(theme, format!("Merge {short} into {destination}?"))
        .note(
            theme,
            "Git resolves the merge with a merge commit when history diverges.",
        );
    if !summary.is_empty() {
        dialog = dialog.note(theme, summary);
    }

    dialog.render(
        theme,
        dialog_cancel_button("merge_commit_cancel", "merge_commit_cancel_hint", theme, cx),
        div().flex().items_center().gap_1().child(
            components::Button::new("merge_commit_confirm", "Merge")
                .disabled(!merge_ready)
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, move |this, _e, _w, cx| dispatch(this, cx)),
        ),
        cx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::RepoSpec;
    use std::path::PathBuf;

    fn repo_state() -> RepoState {
        RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        )
    }

    #[test]
    fn detached_head_destination_uses_head_label() {
        let mut repo = repo_state();
        repo.head_branch = Loadable::Ready("HEAD".to_string());
        repo.detached_head_commit = Some(CommitId("cafebabecafebabe".into()));

        assert_eq!(merge_commit_destination_label(Some(&repo)).as_ref(), "HEAD");
    }

    #[test]
    fn merge_commit_readiness_rechecks_busy_repository_state() {
        let mut repo = repo_state();
        assert!(merge_commit_repo_is_ready(Some(&repo)));

        repo.local_actions_in_flight = 1;
        assert!(!merge_commit_repo_is_ready(Some(&repo)));
        assert!(!merge_commit_repo_is_ready(None));
    }
}
