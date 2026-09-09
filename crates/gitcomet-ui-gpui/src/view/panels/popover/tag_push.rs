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

    pub(super) fn open_tag_push_list(
        &mut self,
        mode: TagPushMode,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.tag_push_search.is_none() {
            let theme = self.theme;
            let input = cx.new(|cx| {
                let mut input = components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Search tags…".into(),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                input.set_theme(theme, cx);
                input
            });
            self._tag_push_search_subscription =
                Some(cx.observe_in(&input, window, |this, input, window, cx| {
                    let escape = input.update(cx, |input, _| input.take_escape_pressed());
                    if escape {
                        this.close_popover_and_restore_focus(window, cx);
                    }
                    cx.notify();
                }));
            self.tag_push_search = Some(input);
        }
        self.tag_push_list = Some(mode);
        let input = self.tag_push_search.as_ref().unwrap();
        input.update(cx, |input, cx| input.set_text("", cx));
        window.focus(&input.read(cx).focus_handle(), cx);
        cx.notify();
    }
}

/// Full searchable list. This uses a virtual list so a large release-tag set
/// doesn't create an element for every tag on every keystroke.
pub(super) fn list_panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let scale = ui_scale::UiScale::current(cx);
    let mode = this.tag_push_list.unwrap_or(TagPushMode::FollowAnnotated);
    let query = this
        .tag_push_search
        .as_ref()
        .map(|input| input.read(cx).text().to_lowercase())
        .unwrap_or_default();
    let requests = this.tag_push_requests(cx);
    let request = requests
        .as_ref()
        .and_then(|(_, requests)| requests.iter().find(|request| request.mode == mode));
    let repo = requests
        .as_ref()
        .and_then(|(repo_id, _)| this.state.repos.iter().find(|repo| repo.id == *repo_id));
    let result = repo
        .zip(request)
        .and_then(|(repo, request)| preview(repo, request));
    let ready = matches!(result, Some(Loadable::Ready(_)));
    let status = summary(result);
    let mut rows: Vec<SharedString> = Vec::new();
    if let Some(Loadable::Ready(preview)) = result {
        rows.extend(
            preview
                .new_tags
                .iter()
                .filter(|name| name.to_lowercase().contains(&query))
                .map(|name| SharedString::from(name.clone())),
        );
        rows.extend(
            preview
                .conflicting_tags
                .iter()
                .filter(|name| name.to_lowercase().contains(&query))
                .map(|name| SharedString::from(format!("Conflict: {name}"))),
        );
    }
    let tooltip_host = this.tooltip_host.clone();
    let rows = Arc::new(rows);
    let count = rows.len();
    div().w(scale.px(380.0)).flex().flex_col().gap_2().p_2()
        .child(components::Button::new("tag_push_list_back", "Back to Push")
            .on_click(theme, cx, |this, _, window, cx| {
                this.tag_push_list = None;
                let focus = if matches!(this.popover, Some(PopoverKind::PushSetUpstreamPrompt { .. })) {
                    this.push_upstream_branch_input.read(cx).focus_handle()
                } else { this.context_menu_focus_handle.clone() };
                window.focus(&focus, cx);
                cx.notify();
            }))
        .child(div().text_size(theme.ui_text(14.0)).child(mode.label()))
        .child(div().text_size(theme.ui_text(12.0)).child(status))
        .children(this.tag_push_search.clone())
        .when(ready && count == 0, |el| el.child(div().text_size(theme.ui_text(12.0)).child("No matching tags to show")))
        .when(count > 0, |el| el.child(gpui::uniform_list("tags_to_push", count, cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
            range.map(|ix| div().h(scale.row_height(24.0, 32.0)).flex().items_center().text_size(theme.ui_text(13.0)).min_w(px(0.0)).child(components::TruncatedText::new(rows[ix].clone(), theme.ui_text(13.0)).full_text_tooltip(tooltip_host.clone()).render(cx))).collect::<Vec<_>>()
        })).h(scale.px(280.0))))
        .child(div().text_size(theme.ui_text(12.0)).text_color(theme.colors.foreground.secondary)
            .child("The preview may change before the push. Conflicting tags will not be overwritten."))
}

pub(super) fn prompt_summary(
    this: &mut PopoverHost,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    this.sync_tag_push_previews(cx);
    let theme = this.theme;
    let text = this
        .tag_push_requests(cx)
        .and_then(|(repo_id, requests)| {
            let request = requests.first()?;
            let repo = this.state.repos.iter().find(|repo| repo.id == repo_id)?;
            Some(format!(
                "{} · {}",
                request.mode.label(),
                summary(preview(repo, request))
            ))
        })
        .unwrap_or_default();
    div()
        .px_2()
        .flex()
        .flex_col()
        .text_size(theme.ui_text(12.0))
        .child(text)
        .when_some(this.push_upstream_tag_mode, |row, mode| {
            row.child(
                components::Button::new("upstream_tags_to_push", "View tags to push…").on_click(
                    theme,
                    cx,
                    move |this, _, window, cx| this.open_tag_push_list(mode, window, cx),
                ),
            )
        })
}

impl Drop for PopoverHost {
    fn drop(&mut self) {
        for cancellation in &self.tag_push_cancellations {
            cancellation.cancel();
        }
    }
}
