use super::*;
use gitcomet_extension_api::{HistoryAnnotator, HistoryRowAnnotation, RepositoryViewContext};

pub(in crate::view) struct HistoryAnnotations {
    providers: Vec<HistoryAnnotator>,
    revisions: Vec<u64>,
    cache: std::cell::RefCell<FxHashMap<(RepoId, u64, CommitId), HistoryRowAnnotation>>,
    headers: FxHashMap<(RepoId, u64), Vec<gpui::AnyView>>,
}
impl HistoryAnnotations {
    pub(super) fn new(
        providers: &[(gitcomet_extension_api::ContributionId, HistoryAnnotator)],
    ) -> Self {
        Self {
            providers: providers
                .iter()
                .map(|(_, provider)| provider.clone())
                .collect(),
            revisions: Vec::new(),
            cache: Default::default(),
            headers: Default::default(),
        }
    }
}

impl HistoryView {
    pub(in crate::view) fn open_history_find(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.history_find_input.is_none() {
            let input = cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Find commits…".into(),
                        ..Default::default()
                    },
                    window,
                    cx,
                )
            });
            self._history_find_subscription = Some(cx.observe(&input, |this, input, cx| {
                if this.history_find.is_some() {
                    this.history_find = Some(input.read(cx).text().to_lowercase().into());
                    cx.notify();
                }
            }));
            self.history_find_input = Some(input);
        }
        let input = self.history_find_input.as_ref().unwrap();
        self.history_find = Some(input.read(cx).text().to_lowercase().into());
        window.focus(&input.read(cx).focus_handle(), cx);
        cx.notify();
    }

    pub(in crate::view) fn history_row_annotation(
        &self,
        repo: &RepoState,
        commit: &gitcomet_core::domain::Commit,
        cx: &App,
    ) -> HistoryRowAnnotation {
        let mut annotation = HistoryRowAnnotation::default();
        if let Some(providers) = &self.history_annotations {
            let key = (repo.id, repo.lifetime(), commit.id.clone());
            let cached = providers.cache.borrow().get(&key).cloned();
            annotation = cached.unwrap_or_else(|| {
                let mut result = HistoryRowAnnotation::default();
                if let Ok(Some(context)) = self.root_view.read_with(cx, |root, _cx| {
                    let host = root.extension_window.as_ref()?.host();
                    let repository = gitcomet_extension_api::RepositoryHandle::new(
                        host.id(),
                        repo.id,
                        repo.lifetime(),
                        repo.spec.workdir.clone(),
                    );
                    Some(RepositoryViewContext {
                        window: host,
                        repository,
                    })
                }) {
                    for provider in &providers.providers {
                        crate::view::perf::extension_dispatch();
                        let next = (provider.annotate)(&context, commit, cx);
                        result.opacity *= if next.opacity.is_finite() {
                            next.opacity.clamp(0.0, 1.0)
                        } else {
                            1.0
                        };
                        result.leading = next.leading.or(result.leading);
                        result.trailing = next.trailing.or(result.trailing);
                        result.range = next.range.or(result.range);
                    }
                }
                let mut cache = providers.cache.borrow_mut();
                if cache.len() >= 2048 {
                    cache.clear();
                }
                cache.insert(key, result.clone());
                result
            });
        }
        if let Some(query) = self.history_find.as_ref().filter(|query| !query.is_empty())
            && !commit.summary.to_lowercase().contains(query.as_ref())
            && !commit.author.to_lowercase().contains(query.as_ref())
            && !commit.id.as_ref().contains(query.as_ref())
        {
            annotation.opacity *= 0.35;
        }
        annotation
    }

    pub(in crate::view) fn sync_history_annotations(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<gpui::AnyView> {
        let repo = self.active_repo().map(|repo| (repo.id, repo.lifetime()));
        let Some(annotations) = &mut self.history_annotations else {
            return Vec::new();
        };
        if annotations.revisions.len() != annotations.providers.len()
            || annotations
                .providers
                .iter()
                .zip(&annotations.revisions)
                .any(|(p, r)| p.signal.revision() != *r)
        {
            annotations.revisions = annotations
                .providers
                .iter()
                .map(|p| p.signal.revision())
                .collect();
            annotations.cache.borrow_mut().clear();
        }
        annotations.headers.retain(|(id, lifetime), _| {
            self.state
                .repos
                .iter()
                .any(|r| r.id == *id && r.lifetime() == *lifetime)
        });
        let Some(key) = repo else { return Vec::new() };
        if !annotations.headers.contains_key(&key)
            && let Ok(Some(context)) = self.root_view.read_with(cx, |root, _cx| {
                let host = root.extension_window.as_ref()?.host();
                let repo = root
                    .state
                    .repos
                    .iter()
                    .find(|repo| repo.id == key.0 && repo.lifetime() == key.1)?;
                let repository = gitcomet_extension_api::RepositoryHandle::new(
                    host.id(),
                    repo.id,
                    repo.lifetime(),
                    repo.spec.workdir.clone(),
                );
                Some(RepositoryViewContext {
                    window: host,
                    repository,
                })
            })
        {
            let headers = annotations
                .providers
                .iter()
                .filter_map(|provider| provider.scope_header.as_ref())
                .map(|build| build(context.clone(), window, cx))
                .collect();
            annotations.headers.insert(key, headers);
        }
        annotations.headers.get(&key).cloned().unwrap_or_default()
    }
}
