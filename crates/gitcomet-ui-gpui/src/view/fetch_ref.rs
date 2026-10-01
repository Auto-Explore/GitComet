//! Fetch an individual remote ref through the regular authenticated pipeline.
use super::*;

struct FetchRefPrompt {
    root: WeakEntity<GitCometView>,
    repo_id: RepoId,
    lifetime: u64,
    dialog_id: u64,
    theme: AppTheme,
    remote: Entity<components::TextInput>,
    refspec: Entity<components::TextInput>,
    error: Option<SharedString>,
}

impl FetchRefPrompt {
    fn submit(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let remote = self.remote.read(cx).text().trim().to_string();
        let refspec = self.refspec.read(cx).text().trim().to_string();
        if remote.is_empty() || refspec.is_empty() {
            self.error = Some("Enter a remote and a ref to fetch.".into());
            cx.notify();
            return;
        }
        let (repo_id, lifetime, dialog_id) = (self.repo_id, self.lifetime, self.dialog_id);
        let result = self.root.update(cx, |root, cx| {
            if !root
                .state
                .repos
                .iter()
                .any(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
            {
                return false;
            }
            root.store
                .dispatch(Msg::Fetch(gitcomet_state::msg::FetchMsg::Refspecs {
                    repo_id,
                    remote,
                    refspecs: vec![refspec],
                }));
            root.close_extension_dialog(dialog_id, window, cx);
            true
        });
        if !matches!(result, Ok(true)) {
            self.error = Some("This repository has closed.".into());
            cx.notify();
        }
    }
}

impl Render for FetchRefPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().flex().flex_col().gap_2()
            .child("Remote")
            .child(self.remote.clone())
            .child("Ref or refspec")
            .child(self.refspec.clone())
            .child(div().text_color(self.theme.colors.foreground.secondary)
                .child("For example, refs/heads/main or refs/tags/v1.0. The fetched commit is recorded in FETCH_HEAD."))
            .children(self.error.clone())
            .child(components::Button::new("fetch_ref_submit", "Fetch")
                .style(components::ButtonStyle::Filled)
                .on_click(self.theme, cx, |this, _, window, cx| this.submit(window, cx)))
    }
}

impl GitCometView {
    pub(super) fn open_fetch_ref(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        let repo_id = repo.id;
        let lifetime = repo.lifetime();
        let default_remote = if let Loadable::Ready(remotes) = &repo.remotes {
            remotes
                .iter()
                .find(|remote| remote.name == "origin")
                .or_else(|| remotes.first())
                .map(|remote| remote.name.clone())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let root = cx.entity().downgrade();
        let theme = self.theme;
        let dialog_id = super::extension_host::next_dialog_id();
        self.open_extension_dialog(
            dialog_id,
            "Fetch ref…".into(),
            Box::new(move |window, cx| {
                let remote = cx.new(|cx| {
                    components::TextInput::new(
                        components::TextInputOptions {
                            placeholder: "origin".into(),
                            ..Default::default()
                        },
                        window,
                        cx,
                    )
                });
                remote.update(cx, |input, cx| {
                    input.set_theme(theme, cx);
                    input.set_text(default_remote, cx);
                });
                let refspec = cx.new(|cx| {
                    components::TextInput::new(
                        components::TextInputOptions {
                            placeholder: "refs/heads/main".into(),
                            ..Default::default()
                        },
                        window,
                        cx,
                    )
                });
                refspec.update(cx, |input, cx| input.set_theme(theme, cx));
                window.focus(&refspec.read(cx).focus_handle(), cx);
                cx.new(|_| FetchRefPrompt {
                    root,
                    repo_id,
                    lifetime,
                    dialog_id,
                    theme,
                    remote,
                    refspec,
                    error: None,
                })
                .into()
            }),
            window,
            cx,
        );
    }
}
