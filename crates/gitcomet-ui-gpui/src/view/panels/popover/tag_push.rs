use super::*;
use gitcomet_core::services::CancellationToken;
use gitcomet_core::tag_push::{TagPushMode, TagPushPreview, TagPushRequest};
use std::hash::{Hash, Hasher};

pub(super) fn request(repo: &RepoState, mode: TagPushMode) -> Option<TagPushRequest> {
    if repo.detached_head_commit.is_some() {
        return None;
    }
    let local = repo.head_branch.ready()?;
    let branch = repo
        .branches
        .ready()?
        .iter()
        .find(|branch| branch.name == *local)?;
    let (remote, target, set_upstream) = match &branch.upstream {
        Some(upstream) => (upstream.remote.clone(), upstream.branch.clone(), false),
        None => {
            let PushRequest::SetUpstream { remote } = push_request(repo) else {
                return None;
            };
            (remote, local.clone(), true)
        }
    };
    Some(TagPushRequest {
        mode,
        remote,
        branch: target,
        local_branch: local.clone(),
        head: branch.target.clone(),
        set_upstream,
    })
}

pub(super) fn preview<'a>(
    repo: &'a RepoState,
    request: &TagPushRequest,
) -> Option<&'a Loadable<Arc<TagPushPreview>>> {
    repo.tag_push_previews[request.mode.index()]
        .as_ref()
        .filter(|slot| slot.request == *request && !slot.cancellation.is_cancelled())
        .map(|slot| &slot.result)
}

pub(super) fn summary(result: Option<&Loadable<Arc<TagPushPreview>>>) -> String {
    match result {
        Some(Loadable::Ready(preview)) => {
            let count = preview.new_tags.len();
            let mut text = format!("{count} new {}", if count == 1 { "tag" } else { "tags" });
            if !preview.conflicting_tags.is_empty() {
                text.push_str(&format!(", {} conflicts", preview.conflicting_tags.len()));
            }
            if !preview.rejected_branches.is_empty() {
                text.push_str(" · branch rejected");
            }
            text
        }
        Some(Loadable::Error(_)) => "Preview unavailable".into(),
        _ => "Checking tags…".into(),
    }
}

pub(super) fn tooltip(
    request: &TagPushRequest,
    result: Option<&Loadable<Arc<TagPushPreview>>>,
) -> String {
    let mut text = match request.mode {
        TagPushMode::FollowAnnotated => "Push this branch and missing annotated tags reachable from it. Lightweight tags are excluded.".to_string(),
        TagPushMode::All => "Push this branch and all local tags, including lightweight tags and tags outside this branch.".to_string(),
    };
    text.push_str(&format!(
        "\nDestination: {}/{}\n{}",
        request.remote,
        request.branch,
        summary(result)
    ));
    if let Some(Loadable::Ready(preview)) = result {
        for name in preview.new_tags.iter().take(20) {
            text.push_str(&format!("\n{name}"));
        }
        if preview.new_tags.len() > 20 {
            text.push_str(&format!("\n… and {} more.", preview.new_tags.len() - 20));
        }
        for name in preview.conflicting_tags.iter().take(20) {
            text.push_str(&format!("\nConflict: {name}"));
        }
        text.push_str("\nPreview only. The remote may change before the push.");
    } else if matches!(result, Some(Loadable::Error(_))) {
        text.push_str("\nYou can still push; authentication will be requested if needed.");
    }
    text
}

impl PopoverHost {
    pub(super) fn cancel_tag_push_previews(&mut self) {
        for cancellation in self.tag_push_cancellations.drain(..) {
            cancellation.cancel();
        }
        self.tag_push_preview_key = None;
    }

    pub(super) fn tag_push_requests(&self, cx: &App) -> Option<(RepoId, Vec<TagPushRequest>)> {
        match self.popover.as_ref()? {
            PopoverKind::PushPicker => {
                let repo = self.active_repo()?;
                Some((
                    repo.id,
                    TagPushMode::ALL
                        .into_iter()
                        .filter_map(|mode| request(repo, mode))
                        .collect(),
                ))
            }
            PopoverKind::PushSetUpstreamPrompt {
                repo_id,
                configure_only_for: None,
                ..
            } => {
                let mode = self.push_upstream_tag_mode?;
                let repo = self.state.repos.iter().find(|repo| repo.id == *repo_id)?;
                let mut request = request(repo, mode)?;
                request.remote = self.selected_push_upstream_remote()?;
                request.branch = self
                    .push_upstream_branch_input
                    .read(cx)
                    .text()
                    .trim()
                    .to_string();
                request.set_upstream = true;
                Some((*repo_id, vec![request]))
            }
            _ => None,
        }
    }

    pub(super) fn sync_tag_push_previews(&mut self, cx: &mut gpui::Context<Self>) {
        let Some((repo_id, requests)) = self.tag_push_requests(cx) else {
            self.cancel_tag_push_previews();
            return;
        };
        let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
            return;
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (
            repo_id,
            &requests,
            repo.tags_rev,
            repo.remotes_rev,
            repo.branches_rev,
            repo.head_branch_rev,
        )
            .hash(&mut hasher);
        let key = hasher.finish();
        if self.tag_push_preview_key == Some(key) {
            return;
        }
        self.cancel_tag_push_previews();
        self.tag_push_preview_key = Some(key);
        for request in requests {
            let cancellation = CancellationToken::new();
            self.tag_push_cancellations.push(cancellation.clone());
            let store = self.store.clone();
            // Debounce typing in the upstream prompt; cancelling closes an
            // already-running probe as well as one waiting for this timer.
            cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(200))
                    .await;
                if !cancellation.is_cancelled() {
                    store.dispatch(Msg::PreviewTagPush {
                        repo_id,
                        request,
                        cancellation,
                    });
                }
            })
            .detach();
        }
    }
}

pub(super) fn prompt_summary(
    this: &mut PopoverHost,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Stateful<gpui::Div> {
    this.sync_tag_push_previews(cx);
    let theme = this.theme;
    let (text, tooltip_text) = this
        .tag_push_requests(cx)
        .and_then(|(repo_id, requests)| {
            let request = requests.first()?;
            let repo = this.state.repos.iter().find(|repo| repo.id == repo_id)?;
            let result = preview(repo, request);
            Some((
                format!("{} · {}", request.mode.label(), summary(result)),
                SharedString::from(tooltip(request, result)),
            ))
        })
        .unwrap_or_default();
    let tooltip_host_for_move = this.tooltip_host.clone();
    let tooltip_host_for_hover = this.tooltip_host.clone();
    let tooltip_text_for_move = tooltip_text.clone();
    div()
        .id("upstream_tag_push_summary")
        .px_2()
        .flex()
        .flex_col()
        .text_size(theme.ui_text(12.0))
        .child(text)
        .on_mouse_move(cx.listener(move |_, event: &MouseMoveEvent, _, cx| {
            let _ = tooltip_host_for_move.update(cx, |host, cx| {
                host.on_mouse_moved(event.position, cx);
                host.set_tooltip_text_if_changed(Some(tooltip_text_for_move.clone()), cx);
            });
        }))
        .on_hover(cx.listener(move |_, hovering: &bool, _, cx| {
            if !*hovering {
                let _ = tooltip_host_for_hover.update(cx, |host, cx| {
                    host.clear_tooltip_if_matches(&tooltip_text, cx);
                });
            }
        }))
}

impl Drop for PopoverHost {
    fn drop(&mut self) {
        for cancellation in &self.tag_push_cancellations {
            cancellation.cancel();
        }
    }
}
