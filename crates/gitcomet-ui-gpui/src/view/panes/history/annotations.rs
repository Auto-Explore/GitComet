use super::*;
use gitcomet_extension_api::{
    HistoryAnnotator, HistoryRowAnnotation, HostedMenuItem, MarkEdge, RepositoryViewContext,
};

/// A row's merged annotation, and for each of its marks (leading,
/// trailing) the annotator that drew it, when that annotator offers a menu.
#[derive(Clone, Default)]
pub(in crate::view) struct RowAnnotation {
    pub(in crate::view) annotation: HistoryRowAnnotation,
    pub(in crate::view) mark_menus: [Option<usize>; 2],
}

pub(in crate::view) struct HistoryAnnotations {
    providers: Vec<HistoryAnnotator>,
    revisions: Vec<u64>,
    cache: std::cell::RefCell<FxHashMap<(RepoId, u64, CommitId), RowAnnotation>>,
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
    /// What an annotator is handed about `repo`: the window's host and the
    /// repository's handle.
    fn annotator_context(&self, repo: &RepoState, cx: &App) -> Option<RepositoryViewContext> {
        self.root_view
            .read_with(cx, |root, _cx| {
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
            })
            .ok()
            .flatten()
    }

    /// The menu for `commit`'s `edge` mark, from annotator `owner`, the one
    /// [`Self::history_row`] found drew it. Asked only on the right-click.
    pub(in crate::view) fn history_mark_menu(
        &self,
        repo_id: RepoId,
        commit: &gitcomet_core::domain::Commit,
        edge: MarkEdge,
        owner: usize,
        cx: &App,
    ) -> Vec<HostedMenuItem> {
        let Some(menu) = self
            .history_annotations
            .as_ref()
            .and_then(|providers| providers.providers.get(owner))
            .and_then(|provider| provider.mark_menu.as_ref())
        else {
            return Vec::new();
        };
        let Some(context) = self
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| self.annotator_context(repo, cx))
        else {
            return Vec::new();
        };
        crate::view::perf::extension_dispatch();
        menu(&context, commit, edge, cx)
    }

    /// The row's annotation, merged from every annotator, with who drew each
    /// mark: a later annotator's mark is drawn over an earlier one's.
    pub(in crate::view) fn history_row(
        &self,
        repo: &RepoState,
        commit: &gitcomet_core::domain::Commit,
        cx: &App,
    ) -> RowAnnotation {
        let Some(providers) = &self.history_annotations else {
            return RowAnnotation::default();
        };
        let key = (repo.id, repo.lifetime(), commit.id.clone());
        let cached = providers.cache.borrow().get(&key).cloned();
        cached.unwrap_or_else(|| {
            let mut row = RowAnnotation::default();
            if let Some(context) = self.annotator_context(repo, cx) {
                for (ix, provider) in providers.providers.iter().enumerate() {
                    crate::view::perf::extension_dispatch();
                    let next = (provider.annotate)(&context, commit, cx);
                    let menu = provider.mark_menu.is_some().then_some(ix);
                    if next.leading.is_some() {
                        row.mark_menus[0] = menu;
                    }
                    if next.trailing.is_some() {
                        row.mark_menus[1] = menu;
                    }
                    let result = &mut row.annotation;
                    result.opacity *= if next.opacity.is_finite() {
                        next.opacity.clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    result.leading = next.leading.or(result.leading.take());
                    result.trailing = next.trailing.or(result.trailing.take());
                    result.range = next.range.or(result.range);
                }
            }
            let mut cache = providers.cache.borrow_mut();
            if cache.len() >= 2048 {
                cache.clear();
            }
            cache.insert(key, row.clone());
            row
        })
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
