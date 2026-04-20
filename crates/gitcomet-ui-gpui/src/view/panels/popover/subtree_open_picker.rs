use super::*;

fn openable_subtree_entries(repo: &RepoState) -> Vec<(SharedString, std::path::PathBuf)> {
    let Loadable::Ready(subtrees) = &repo.subtrees else {
        return Vec::new();
    };

    subtrees
        .iter()
        .filter_map(|subtree| {
            let open_path = local_subtree_source_repo_path(repo, &subtree.path)?;
            Some((subtree.path.display().to_string().into(), open_path))
        })
        .collect()
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    if let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) {
        match &repo.subtrees {
            Loadable::Loading => components::context_menu_label(theme, ui_scale_percent, "Loading"),
            Loadable::NotLoaded => {
                components::context_menu_label(theme, ui_scale_percent, "Not loaded")
            }
            Loadable::Error(e) => {
                components::context_menu_label(theme, ui_scale_percent, e.clone())
            }
            Loadable::Ready(_) => {
                let entries = openable_subtree_entries(repo);
                let items = entries
                    .iter()
                    .map(|(label, _)| label.clone())
                    .collect::<Vec<SharedString>>();
                let paths = entries
                    .iter()
                    .map(|(_, path)| path.clone())
                    .collect::<Vec<_>>();

                if let Some(search) = this.subtree_picker_search_input.clone() {
                    components::context_menu(
                        theme,
                        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                            .items(items)
                            .empty_text("No local subtree source repositories")
                            .max_height(scaled_px(260.0))
                            .render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                                let Some(path) = paths.get(ix).cloned() else {
                                    return;
                                };
                                this.store.dispatch(Msg::OpenRepo(path));
                                this.popover = None;
                                this.popover_anchor = None;
                                cx.notify();
                            }),
                    )
                    .w(scaled_px(520.0))
                    .max_w(scaled_px(820.0))
                } else {
                    components::context_menu_label(
                        theme,
                        ui_scale_percent,
                        "Search input not initialized",
                    )
                }
            }
        }
    } else {
        components::context_menu_label(theme, ui_scale_percent, "No repository")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{RepoSpec, Subtree, SubtreeSourceConfig};
    use gitcomet_core::path_utils::canonicalize_or_original;

    #[test]
    fn openable_subtree_entries_only_include_local_sources() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let workdir = dir.path().join("parent");
        let source = dir.path().join("source");
        std::fs::create_dir_all(&workdir).expect("create workdir");
        std::fs::create_dir_all(&source).expect("create source repo");

        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: workdir.clone(),
            },
        );
        repo.subtrees = Loadable::Ready(std::sync::Arc::new(vec![
            Subtree {
                path: std::path::PathBuf::from("vendor/local"),
                source: Some(SubtreeSourceConfig {
                    local_repository: None,
                    repository: "../source".to_string(),
                    reference: "main".to_string(),
                    push_refspec: None,
                    squash: true,
                }),
            },
            Subtree {
                path: std::path::PathBuf::from("vendor/remote"),
                source: Some(SubtreeSourceConfig {
                    local_repository: None,
                    repository: "https://example.com/repo.git".to_string(),
                    reference: "main".to_string(),
                    push_refspec: None,
                    squash: true,
                }),
            },
            Subtree {
                path: std::path::PathBuf::from("vendor/missing"),
                source: None,
            },
        ]));

        let entries = openable_subtree_entries(&repo);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0.as_ref(), "vendor/local");
        assert_eq!(entries[0].1, canonicalize_or_original(source));
    }
}
