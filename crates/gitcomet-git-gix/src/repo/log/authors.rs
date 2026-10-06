use super::*;
use gitcomet_core::services::HistorySnapshot;

pub(in super::super) struct HistoryAuthorsCache {
    snapshot: HistorySnapshot,
    names: Arc<[Arc<str>]>,
}

impl GixRepo {
    pub(in super::super) fn history_authors_impl(
        &self,
        mode: HistoryMode,
        cancellation: &CancellationToken,
    ) -> Result<Arc<[Arc<str>]>> {
        cancellation.check_cancelled()?;
        let (store, shared) = self.fresh_history_store()?;
        let generation = shared.id;
        let repo = store.to_thread_local();
        let query = self.resolve_history_query(
            &repo,
            shared.id,
            shared.common.id,
            mode,
            None,
            cancellation,
        )?;
        let snapshot = query.snapshot();
        let tips = query.tips;
        let shallow = query.shallow;
        if let Some(cached) = self
            .history_authors_cache
            .lock()
            .expect("history authors cache")
            .as_ref()
            .filter(|cached| cached.snapshot == snapshot)
        {
            return Ok(cached.names.clone());
        }
        let mut walk = new_log_paged_walk(
            &store,
            tips.iter().copied(),
            mode,
            &shallow,
            Some(cancellation),
            None,
            Some((&shared.topology, generation)),
        )?;
        let mut buffer = Vec::new();
        let mut seen = FxHashSet::default();
        let mut names = Vec::new();
        for info in &mut walk.walk {
            cancellation.check_cancelled()?;
            let info = info.map_err(|error| {
                crate::repo::object_store::gix_error("gix history authors", &*error)
            })?;
            if !mode_includes(mode, info.parent_ids.len()) {
                continue;
            }
            let commit = repo
                .objects
                .find_commit(info.id.as_ref(), &mut buffer)
                .map_err(|error| {
                    crate::repo::object_store::gix_error(
                        "gix history author",
                        &gix::Error::from(error),
                    )
                })?;
            let name = commit
                .author()
                .ok()
                .map_or(b"unknown".as_slice(), |author| author.name.as_ref());
            // Repeated names borrow the decode buffer; allocate once per name.
            if !seen.contains(name) {
                seen.insert(name.to_vec());
                names.push(bstr_to_arc_str(name));
            }
        }
        cancellation.check_cancelled()?;
        let names: Arc<[Arc<str>]> = names.into();
        *self
            .history_authors_cache
            .lock()
            .expect("history authors cache") = Some(HistoryAuthorsCache {
            snapshot,
            names: names.clone(),
        });
        Ok(names)
    }
}
