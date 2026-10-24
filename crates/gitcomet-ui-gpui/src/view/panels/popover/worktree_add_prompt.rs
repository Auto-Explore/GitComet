use super::*;
use crate::kit::interaction as controls;
use gitcomet_core::domain::{Remote, RemoteBranch, Upstream};
use gitcomet_core::services::WorktreeCheckout;
use gitcomet_state::model::RepoState;
use std::path::{Path, PathBuf};

/// Why Add is off: a field still needs input, git would refuse the request, or
/// the branch is checked out at the path already, which the dialog offers to open.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum WorktreeAddBlocked {
    Incomplete,
    Refused(String),
    CheckedOut(PathBuf),
}

pub(super) struct WorktreeAddFields<'a> {
    pub(super) folder: &'a str,
    pub(super) source: &'a str,
    pub(super) create_branch: bool,
    pub(super) new_branch: &'a str,
}

/// The ref the dialog starts from: the remote's default branch, preferring the
/// local branch that tracks it, then a local `main` or `master`, then HEAD's branch.
pub(super) fn default_worktree_source(repo: &RepoState) -> String {
    let branches = repo.branches.ready().map_or(&[][..], |b| b.as_slice());
    let remotes = repo.remotes.ready().map_or(&[][..], |r| r.as_slice());
    let remote_branches = repo.remote_branches.ready().map(|b| b.as_slice());
    let head = match &repo.head_branch {
        Loadable::Ready(head) if head != "HEAD" => Some(head.as_str()),
        _ => None,
    };
    let head_remote = head
        .and_then(|head| branches.iter().find(|branch| branch.name == head))
        .and_then(|branch| branch.upstream.as_ref())
        .map(|upstream| upstream.remote.as_str());
    if let Some((remote, default)) = preferred_remote_default(remotes, remote_branches, head_remote)
    {
        let tracking = branches.iter().find(|branch| {
            branch
                .upstream
                .as_ref()
                .is_some_and(|upstream| upstream.remote == remote && upstream.branch == default)
        });
        if let Some(tracking) = tracking {
            return tracking.name.clone();
        }
        if branches.iter().any(|branch| branch.name == default) {
            return default.to_string();
        }
        return format!("{remote}/{default}");
    }
    ["main", "master"]
        .into_iter()
        .find(|name| branches.iter().any(|branch| branch.name == *name))
        .or(head)
        .unwrap_or_default()
        .to_string()
}

/// The first remote with a known default branch, as `(remote, branch)`: the one
/// HEAD's branch tracks, then `origin`, then any other. A default whose branch is
/// no longer fetched (pruned since `refs/remotes/<remote>/HEAD` was set) is skipped.
fn preferred_remote_default<'a>(
    remotes: &'a [Remote],
    remote_branches: Option<&[RemoteBranch]>,
    head_remote: Option<&str>,
) -> Option<(&'a str, &'a str)> {
    let fetched = |remote: &str, branch: &str| {
        remote_branches.is_none_or(|fetched| {
            fetched
                .iter()
                .any(|candidate| candidate.remote == remote && candidate.name == branch)
        })
    };
    let default_of = |remote: &'a Remote| {
        remote
            .default_branch
            .as_deref()
            .filter(|branch| fetched(&remote.name, branch))
            .map(|branch| (remote.name.as_str(), branch))
    };
    let named = |name: &str| {
        remotes
            .iter()
            .filter(|remote| remote.name == name)
            .find_map(default_of)
    };
    head_remote
        .and_then(named)
        .or_else(|| named("origin"))
        .or_else(|| remotes.iter().find_map(default_of))
}

/// The `git worktree add` the fields describe. A refusal shows as soon as it is
/// known, even before the folder is filled in.
pub(super) fn worktree_add_request(
    repo: &RepoState,
    fields: &WorktreeAddFields<'_>,
) -> Result<(PathBuf, WorktreeCheckout), WorktreeAddBlocked> {
    let folder = fields.folder.trim();
    let path = PathBuf::from(folder);
    // The folder starts as the current worktree's path; it needs a new one.
    let new_folder = (!folder.is_empty() && !is_listed_worktree(repo, &path)).then_some(&path);
    let source = fields.source.trim();
    let checkout = if fields.create_branch {
        new_branch_checkout(repo, new_folder, source, fields.new_branch.trim())?
    } else {
        existing_checkout(repo, source)?
    };
    match new_folder {
        Some(path) => Ok((path.clone(), checkout)),
        None => Err(WorktreeAddBlocked::Incomplete),
    }
}

/// Compared lexically: a filesystem check here would run on every render.
fn is_listed_worktree(repo: &RepoState, path: &Path) -> bool {
    let path = lexically_normalized(&repo.spec.workdir.join(path));
    path == repo.spec.workdir
        || repo
            .worktrees
            .ready()
            .is_some_and(|worktrees| worktrees.iter().any(|worktree| worktree.path == path))
}

fn lexically_normalized(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component),
        }
    }
    normalized
}

/// Unticked with a source that is no local branch, git starts the worktree on a
/// detached HEAD.
fn checks_out_detached(repo: &RepoState, create_branch: bool, source: &str) -> bool {
    let source = source.trim();
    !create_branch
        && !source.is_empty()
        && repo
            .branches
            .ready()
            .is_some_and(|branches| branches.iter().all(|branch| branch.name != source))
}

fn existing_checkout(
    repo: &RepoState,
    source: &str,
) -> Result<WorktreeCheckout, WorktreeAddBlocked> {
    if source.is_empty() {
        return Err(WorktreeAddBlocked::Incomplete);
    }
    if let Some(at) = crate::view::rows::branch_checked_out_at(repo, source) {
        return Err(WorktreeAddBlocked::CheckedOut(at));
    }
    Ok(WorktreeCheckout::Existing {
        reference: source.to_string(),
    })
}

/// A new branch named like the remote branch it starts from tracks that branch.
/// An empty name takes the new folder's name, once there is a new folder.
fn new_branch_checkout(
    repo: &RepoState,
    new_folder: Option<&PathBuf>,
    source: &str,
    typed_name: &str,
) -> Result<WorktreeCheckout, WorktreeAddBlocked> {
    let name = match typed_name {
        "" => new_folder
            .and_then(|folder| folder.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        typed => typed.to_string(),
    };
    if name.is_empty() {
        return Err(WorktreeAddBlocked::Incomplete);
    }
    if !gitcomet_core::branch_name::is_valid_branch_name(&name) {
        return Err(WorktreeAddBlocked::Refused(format!(
            "'{name}' is not a valid branch name"
        )));
    }
    if repo
        .branches
        .ready()
        .is_some_and(|branches| branches.iter().any(|branch| branch.name == name))
    {
        return Err(WorktreeAddBlocked::Refused(format!(
            "Branch '{name}' already exists"
        )));
    }
    let tracked = repo.remote_branches.ready().and_then(|branches| {
        branches.iter().find(|branch| {
            branch.name == name
                && source
                    .strip_prefix(branch.remote.as_str())
                    .and_then(|rest| rest.strip_prefix('/'))
                    == Some(branch.name.as_str())
        })
    });
    Ok(match tracked {
        Some(branch) => WorktreeCheckout::TrackRemote {
            name,
            upstream: Upstream {
                remote: branch.remote.clone(),
                branch: branch.name.clone(),
            },
        },
        None => WorktreeCheckout::NewBranch {
            name,
            start: (!source.is_empty()).then(|| source.to_string()),
        },
    })
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let blocked = this.worktree_add_request(repo_id, cx).err();
    let can_submit = blocked.is_none();
    let (refused, checked_out_at) = match blocked {
        Some(WorktreeAddBlocked::Refused(reason)) => (Some(reason), None),
        Some(WorktreeAddBlocked::CheckedOut(at)) => (None, Some(at)),
        _ => (None, None),
    };
    let checked_out = checked_out_at.map(|at| {
        format!(
            "'{}' is already checked out in {}.",
            this.worktree_ref_source_target.trim(),
            at.display()
        )
    });
    // Unticked, a branch that is checked out already opens where it is.
    let submit = match checked_out {
        Some(_) => components::Button::new("worktree_add_open", "Open worktree")
            .separated_end_slot(super::hotkey_hint(theme, "worktree_add_open_hint", "Enter")),
        None => components::Button::new("worktree_add_go", "Add")
            .disabled(!can_submit)
            .separated_end_slot(super::hotkey_hint(theme, "worktree_add_go_hint", "Enter")),
    };
    let create_branch = this.worktree_add_create_branch;
    let detached = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .is_some_and(|repo| {
            checks_out_detached(repo, create_branch, &this.worktree_ref_source_target)
        });
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = crate::ui_scale::scaler(ui_scale_percent);

    let ref_row = if let Some(search) = this.branch_picker_search_input.clone() {
        let is_focused = search
            .read_with(cx, |input, _| input.focus_handle())
            .is_focused(window);
        search.update(cx, |input, cx| {
            input.set_chromeless(is_focused, cx);
            input.set_leading_icon(is_focused.then_some("icons/git_branch.svg"), cx);
        });

        if is_focused {
            let query = search.read(cx).text().trim().to_string();
            let built = branch_picker::ref_rows_cached(
                this,
                branch_picker::RefRowsSpec::source_ref(),
                &query,
            );
            let names = std::rc::Rc::clone(&built.payloads);

            div().px_2().pb_1().w_full().min_w(px(0.0)).child(
                branch_picker::ref_picker_prompt(
                    search,
                    this.picker_prompt_scroll.clone(),
                    &built,
                    cx,
                )
                .tooltip_host(this.tooltip_host.clone())
                .empty_text("No matches")
                .max_height(scaled_px(branch_picker::REF_PICKER_LIST_MAX_HEIGHT_PX))
                .selected_index(this.branch_picker_selected_index)
                .render(
                    theme,
                    ui_scale_percent,
                    cx,
                    move |this, ix, _e, window, cx| {
                        let Some(name) = names.get(ix).cloned() else {
                            return;
                        };
                        let repo_id = this.active_repo_id().unwrap_or(RepoId(0));
                        this.handle_inline_branch_picker_select(name, repo_id, window, cx);
                    },
                ),
            )
        } else {
            div().px_2().pb_1().w_full().min_w(px(0.0)).child(search)
        }
    } else {
        div()
            .px_2()
            .pb_1()
            .w_full()
            .min_w(px(0.0))
            .child(this.worktree_ref_input.clone())
    };

    div()
        .flex()
        .flex_col()
        .w(scaled_px(640.0))
        .child(popover_title(theme, "Add worktree"))
        .child(super::popover_rule(theme))
        .child(input_label(theme, "Worktree folder"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(this.worktree_path_input.clone()),
                )
                .child(
                    components::Button::new("worktree_browse", "Browse")
                        .focus_handle(this.worktree_browse_focus_handle.clone())
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, move |this, _e, window, cx| {
                            cx.stop_propagation();
                            let view = cx.weak_entity();
                            // A relative folder counts from the worktree, as git reads it.
                            let workdir = this
                                .state
                                .repos
                                .iter()
                                .find(|repo| repo.id == repo_id)
                                .map(|repo| repo.spec.workdir.clone());
                            let directory = crate::view::path_prompt::existing_folder(
                                this.worktree_path_input.read(cx).text(),
                                workdir.as_deref(),
                            );
                            let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                                files: false,
                                directories: true,
                                multiple: false,
                                prompt: Some("Select worktree folder".into()),
                                directory,
                            });

                            window
                                .spawn(cx, async move |cx| {
                                    let result = rx.await;
                                    let paths = match result {
                                        Ok(Ok(Some(paths))) => paths,
                                        Ok(Ok(None)) => return,
                                        Ok(Err(_)) | Err(_) => return,
                                    };
                                    let Some(path) = paths.into_iter().next() else {
                                        return;
                                    };
                                    let _ = view.update(cx, |this, cx| {
                                        this.worktree_path_input.update(cx, |input, cx| {
                                            input.set_text(path.display().to_string(), cx);
                                        });
                                        cx.notify();
                                    });
                                })
                                .detach();
                        }),
                ),
        )
        .child(
            div().px_1().py_1().child(
                checkbox_row(
                    "worktree_add_create_branch",
                    "worktree_add_create_branch",
                    "Create new branch",
                    create_branch,
                    &this.worktree_add_create_branch_focus_handle,
                    theme,
                    cx,
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _w, cx| {
                        this.worktree_add_create_branch = !this.worktree_add_create_branch;
                        cx.notify();
                    }),
                ),
            ),
        )
        .child(input_label(
            theme,
            if create_branch {
                "Start from"
            } else {
                "Checkout branch"
            },
        ))
        .child(ref_row)
        .when(detached, |panel| {
            panel.child(
                div()
                    .debug_selector(|| "worktree_add_detached".to_string())
                    .px_2()
                    .pb_1()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child("Not a local branch: the worktree starts on a detached HEAD."),
            )
        })
        .when(create_branch, |panel| {
            panel.child(input_label(theme, "New branch name")).child(
                div()
                    .debug_selector(|| "worktree_add_new_branch".to_string())
                    .px_2()
                    .pb_1()
                    .w_full()
                    .min_w(px(0.0))
                    .child(this.worktree_new_branch_input.clone()),
            )
        })
        .when_some(checked_out, |panel, notice| {
            panel.child(
                div()
                    .debug_selector(|| "worktree_add_checked_out".to_string())
                    .px_2()
                    .pb_1()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(notice),
            )
        })
        .when_some(refused, |panel, reason| {
            panel.child(
                div()
                    .debug_selector(|| "worktree_add_refused".to_string())
                    .px_2()
                    .pb_1()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.status.danger.foreground)
                    .child(reason),
            )
        })
        .child(super::popover_rule(theme))
        .child(
            super::prompt_footer_row()
                .child(
                    cancel_button("worktree_add_cancel", "worktree_add_cancel_hint", theme)
                        .focus_handle(this.worktree_focus.cancel.clone())
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    submit
                        .focus_handle(this.worktree_focus.submit.clone())
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.submit_worktree_add(cx);
                        }),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Branch, CommitId, Remote, RemoteBranch, RepoSpec, Worktree};
    use std::sync::Arc;

    fn branch(name: &str, upstream: Option<(&str, &str)>) -> Branch {
        Branch {
            name: name.to_string(),
            target: CommitId(Arc::from("1111111111111111111111111111111111111111")),
            upstream: upstream.map(|(remote, branch)| Upstream {
                remote: remote.to_string(),
                branch: branch.to_string(),
            }),
            divergence: None,
        }
    }

    fn remote(name: &str, default_branch: Option<&str>) -> Remote {
        Remote {
            name: name.to_string(),
            url: None,
            default_branch: default_branch.map(str::to_string),
        }
    }

    fn repo(head: &str, branches: Vec<Branch>, remotes: Vec<Remote>) -> RepoState {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/ws/main"),
            },
        );
        repo.head_branch = Loadable::Ready(head.to_string());
        repo.branches = Loadable::Ready(Arc::new(branches));
        repo.remotes = Loadable::Ready(Arc::new(remotes));
        repo
    }

    #[test]
    fn default_source_is_the_local_branch_tracking_the_remote_default() {
        let repo = repo(
            "feature",
            vec![
                branch("feature", None),
                branch("trunk", Some(("origin", "main"))),
            ],
            vec![remote("origin", Some("main"))],
        );
        assert_eq!(default_worktree_source(&repo), "trunk");
    }

    #[test]
    fn default_source_is_the_remote_default_when_no_local_branch_has_it() {
        let repo = repo(
            "feature",
            vec![branch("feature", None)],
            vec![remote("origin", Some("main"))],
        );
        assert_eq!(default_worktree_source(&repo), "origin/main");
    }

    #[test]
    fn default_source_prefers_the_remote_the_current_branch_tracks() {
        let repo = repo(
            "feature",
            vec![branch("feature", Some(("upstream", "feature")))],
            vec![
                remote("origin", Some("main")),
                remote("upstream", Some("develop")),
            ],
        );
        assert_eq!(default_worktree_source(&repo), "upstream/develop");
    }

    #[test]
    fn default_source_falls_back_to_main_master_then_head() {
        let with_master = repo(
            "feature",
            vec![branch("feature", None), branch("master", None)],
            vec![remote("origin", None)],
        );
        assert_eq!(default_worktree_source(&with_master), "master");
        let bare = repo("feature", vec![branch("feature", None)], Vec::new());
        assert_eq!(default_worktree_source(&bare), "feature");
    }

    /// A repo on `main` with `origin/x` fetched and `main` also listed as a worktree.
    fn request_repo() -> RepoState {
        let mut repo = repo(
            "main",
            vec![
                branch("main", Some(("origin", "main"))),
                branch("free", None),
            ],
            vec![remote("origin", Some("main"))],
        );
        repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
            remote: "origin".to_string(),
            name: "x".to_string(),
            target: CommitId(Arc::from("2222222222222222222222222222222222222222")),
        }]));
        repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
            path: PathBuf::from("/tmp/ws/main"),
            head: None,
            branch: Some("main".to_string()),
            detached: false,
        }]));
        repo
    }

    fn request(
        folder: &str,
        source: &str,
        create_branch: bool,
        new_branch: &str,
    ) -> Result<(PathBuf, WorktreeCheckout), WorktreeAddBlocked> {
        worktree_add_request(
            &request_repo(),
            &WorktreeAddFields {
                folder,
                source,
                create_branch,
                new_branch,
            },
        )
    }

    #[test]
    fn checkout_branch_names_the_existing_branch() {
        assert_eq!(
            request("/tmp/ws/free", "free", false, "ignored"),
            Ok((
                PathBuf::from("/tmp/ws/free"),
                WorktreeCheckout::Existing {
                    reference: "free".to_string()
                }
            ))
        );
    }

    #[test]
    fn checkout_branch_offers_the_worktree_that_has_the_branch() {
        assert_eq!(
            request("/tmp/ws/again", "main", false, ""),
            Err(WorktreeAddBlocked::CheckedOut(PathBuf::from(
                "/tmp/ws/main"
            )))
        );
    }

    #[test]
    fn a_checked_out_branch_can_start_a_new_branch() {
        assert_eq!(
            request("/tmp/ws/next", "main", true, ""),
            Ok((
                PathBuf::from("/tmp/ws/next"),
                WorktreeCheckout::NewBranch {
                    name: "next".to_string(),
                    start: Some("main".to_string()),
                }
            ))
        );
    }

    #[test]
    fn a_new_branch_named_like_its_remote_start_tracks_it() {
        assert_eq!(
            request("/tmp/ws/x", "origin/x", true, "x"),
            Ok((
                PathBuf::from("/tmp/ws/x"),
                WorktreeCheckout::TrackRemote {
                    name: "x".to_string(),
                    upstream: Upstream {
                        remote: "origin".to_string(),
                        branch: "x".to_string(),
                    },
                }
            ))
        );
    }

    #[test]
    fn a_differently_named_new_branch_only_starts_from_the_remote() {
        assert_eq!(
            request("/tmp/ws/y", "origin/x", true, "y"),
            Ok((
                PathBuf::from("/tmp/ws/y"),
                WorktreeCheckout::NewBranch {
                    name: "y".to_string(),
                    start: Some("origin/x".to_string()),
                }
            ))
        );
    }

    #[test]
    fn an_empty_branch_name_takes_the_folder_name() {
        assert_eq!(
            request("/tmp/ws/shiny", "", true, "  "),
            Ok((
                PathBuf::from("/tmp/ws/shiny"),
                WorktreeCheckout::NewBranch {
                    name: "shiny".to_string(),
                    start: None,
                }
            ))
        );
    }

    #[test]
    fn a_new_branch_that_exists_is_refused() {
        assert_eq!(
            request("/tmp/ws/free", "main", true, "free"),
            Err(WorktreeAddBlocked::Refused(
                "Branch 'free' already exists".to_string()
            ))
        );
        assert_eq!(
            request("/tmp/ws/free", "main", true, ""),
            Err(WorktreeAddBlocked::Refused(
                "Branch 'free' already exists".to_string()
            )),
            "the folder-name fallback is checked too"
        );
    }

    #[test]
    fn checkout_branch_offers_another_worktree_that_has_the_branch() {
        let mut repo = request_repo();
        repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
            path: PathBuf::from("/tmp/ws/held"),
            head: None,
            branch: Some("free".to_string()),
            detached: false,
        }]));
        assert_eq!(
            worktree_add_request(
                &repo,
                &WorktreeAddFields {
                    folder: "/tmp/ws/again",
                    source: "free",
                    create_branch: false,
                    new_branch: "",
                },
            ),
            Err(WorktreeAddBlocked::CheckedOut(PathBuf::from(
                "/tmp/ws/held"
            )))
        );
    }

    #[test]
    fn an_invalid_branch_name_is_refused() {
        assert_eq!(
            request("/tmp/ws/my feature", "main", true, ""),
            Err(WorktreeAddBlocked::Refused(
                "'my feature' is not a valid branch name".to_string()
            )),
            "the folder-name fallback is checked too"
        );
        assert_eq!(
            request("/tmp/ws/x", "main", true, "a..b"),
            Err(WorktreeAddBlocked::Refused(
                "'a..b' is not a valid branch name".to_string()
            ))
        );
    }

    #[test]
    fn a_detached_head_can_be_checked_out_again() {
        let mut repo = request_repo();
        repo.head_branch = Loadable::Ready("HEAD".to_string());
        repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
            path: PathBuf::from("/tmp/ws/main"),
            head: None,
            branch: None,
            detached: true,
        }]));
        assert_eq!(
            worktree_add_request(
                &repo,
                &WorktreeAddFields {
                    folder: "/tmp/ws/detached",
                    source: "HEAD",
                    create_branch: false,
                    new_branch: "",
                },
            ),
            Ok((
                PathBuf::from("/tmp/ws/detached"),
                WorktreeCheckout::Existing {
                    reference: "HEAD".to_string()
                }
            ))
        );
    }

    #[test]
    fn relative_spellings_of_the_current_worktree_still_need_a_new_folder() {
        for folder in [".", "../main", "./sub/.."] {
            assert_eq!(
                request(folder, "free", false, ""),
                Err(WorktreeAddBlocked::Incomplete),
                "{folder} names the current worktree"
            );
        }
        assert!(request("../sibling", "free", false, "").is_ok());
    }

    #[test]
    fn a_dangling_remote_default_is_not_preselected() {
        let mut repo = repo(
            "feature",
            vec![branch("feature", None), branch("master", None)],
            vec![remote("origin", Some("gone"))],
        );
        repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
        assert_eq!(default_worktree_source(&repo), "master");
    }

    #[test]
    fn a_refusal_shows_before_the_folder_is_filled_in() {
        assert!(matches!(
            request("", "main", true, "a..b"),
            Err(WorktreeAddBlocked::Refused(_))
        ));
        assert_eq!(
            request("", "main", false, ""),
            Err(WorktreeAddBlocked::CheckedOut(PathBuf::from(
                "/tmp/ws/main"
            ))),
            "opening the worktree that has the branch needs no folder"
        );
    }

    #[test]
    fn an_existing_worktree_folder_blocks_add_without_a_reason() {
        assert_eq!(
            request("/tmp/ws/main", "free", false, ""),
            Err(WorktreeAddBlocked::Incomplete),
            "the prefilled current worktree path still needs a new folder"
        );
        assert_eq!(
            request("/tmp/ws/main/", "main", true, "fresh"),
            Err(WorktreeAddBlocked::Incomplete)
        );
        assert_eq!(
            request("/tmp/ws/main", "main", true, ""),
            Err(WorktreeAddBlocked::Incomplete),
            "the current folder's name is no branch name to refuse yet"
        );
    }

    #[test]
    fn missing_fields_block_without_a_reason() {
        assert_eq!(
            request("  ", "main", true, "x"),
            Err(WorktreeAddBlocked::Incomplete)
        );
        assert_eq!(
            request("/tmp/ws/z", "", false, ""),
            Err(WorktreeAddBlocked::Incomplete)
        );
    }
}
