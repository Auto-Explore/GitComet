use super::{RefBackend, backend, failure, view};
use gitcomet_core::services::Result;

#[derive(Clone, Copy)]
pub(crate) enum RootRef {
    CherryPick,
    Revert,
}
impl RootRef {
    fn name(self) -> &'static str {
        match self {
            Self::CherryPick => "CHERRY_PICK_HEAD",
            Self::Revert => "REVERT_HEAD",
        }
    }
}
pub(crate) fn root_ref(repo: &gix::Repository, root: RootRef) -> Result<Option<gix::ObjectId>> {
    Ok(view(repo)?
        .resolve_exact(root.name().as_bytes())?
        .map(|(id, _)| id))
}

pub(crate) fn operation_state(repo: &gix::Repository) -> Result<Option<gix::state::InProgress>> {
    use gix::state::InProgress as S;
    let files = repo.state();
    if backend(repo)? == RefBackend::Files
        || matches!(
            files,
            Some(S::ApplyMailbox | S::ApplyMailboxRebase | S::Rebase | S::RebaseInteractive)
        )
    {
        return Ok(files);
    }
    let refs = view(repo)?;
    let sequence = repo.git_dir().join("sequencer/todo").is_file();
    if refs.find_exact(b"CHERRY_PICK_HEAD")?.is_some() {
        return Ok(Some(if sequence {
            S::CherryPickSequence
        } else {
            S::CherryPick
        }));
    }
    if matches!(files, Some(S::Merge | S::Bisect)) {
        return Ok(files);
    }
    if refs.find_exact(b"REVERT_HEAD")?.is_some() {
        return Ok(Some(if sequence {
            S::RevertSequence
        } else {
            S::Revert
        }));
    }
    Ok(files)
}

pub(crate) fn head_tree_id_or_empty(repo: &gix::Repository) -> Result<gix::ObjectId> {
    match super::head_oid(repo)? {
        Some(id) => repo
            .find_object(id)
            .and_then(|o| o.peel_to_tree())
            .map(|t| t.id)
            .map_err(failure),
        None => Ok(gix::ObjectId::empty_tree(repo.object_hash())),
    }
}

pub(crate) fn index_or_load_from_head_or_empty(
    repo: &gix::Repository,
) -> Result<gix::worktree::IndexPersistedOrInMemory> {
    use gix::worktree::IndexPersistedOrInMemory as Index;
    if backend(repo)? == RefBackend::Files {
        return repo.index_or_load_from_head_or_empty().map_err(failure);
    }
    Ok(match repo.try_index().map_err(failure)? {
        Some(index) => Index::Persisted(index),
        None => match super::head_oid(repo)? {
            Some(id) => {
                let tree = repo
                    .find_object(id)
                    .and_then(|o| o.peel_to_tree())
                    .map_err(failure)?;
                Index::InMemory(repo.index_from_tree(&tree.id).map_err(failure)?)
            }
            None => Index::InMemory(gix::index::File::from_state(
                gix::index::State::new(repo.object_hash()),
                repo.index_path(),
            )),
        },
    })
}

/// Avoid gix's implicit HEAD lookup when no modules file exists. In reftable
/// repositories modules present only in the HEAD tree are intentionally omitted.
pub(crate) fn has_modules(repo: &gix::Repository) -> Result<bool> {
    if backend(repo)? == RefBackend::Files {
        return Ok(true);
    }
    if let Some(path) = repo.modules_path() {
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_file() => return Ok(true),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(failure(e)),
        }
    }
    Ok(repo
        .try_index()
        .map_err(failure)?
        .is_some_and(|i| i.entry_by_path(".gitmodules".into()).is_some()))
}

pub(crate) fn submodules(
    repo: &gix::Repository,
) -> Result<Option<impl Iterator<Item = gix::Submodule<'_>>>> {
    if !has_modules(repo)? {
        return Ok(None);
    }
    repo.submodules().map_err(failure)
}

pub(crate) fn submodule_state(submodule: &gix::Submodule<'_>) -> Result<gix::submodule::State> {
    submodule.state().map_err(failure)
}

/// gix's convenience tree diff implicitly loads HEAD when the index is absent.
/// Supply the reference-aware index explicitly for that case (e.g. --no-checkout).
pub(crate) fn diff_tree_to_tree<'r>(
    repo: &'r gix::Repository,
    old: Option<&gix::Tree<'r>>,
    new: &gix::Tree<'r>,
) -> Result<Vec<gix::object::tree::diff::ChangeDetached>> {
    if backend(repo)? == RefBackend::Files || repo.try_index().map_err(failure)?.is_some() {
        return repo.diff_tree_to_tree(old, new, None).map_err(failure);
    }
    let index = index_or_load_from_head_or_empty(repo)?;
    let attributes = repo
        .attributes_only(
            &index,
            gix::worktree::stack::state::attributes::Source::IdMapping,
        )
        .map_err(failure)?
        .detach();
    let mut cache = gix::diff::resource_cache(
        repo,
        gix::diff::blob::pipeline::Mode::ToGit,
        attributes,
        Default::default(),
    )
    .map_err(failure)?;
    let (rewrites, configured) =
        gix::diff::new_rewrites(repo.config_snapshot().plumbing(), true).map_err(failure)?;
    let options = gix::diff::Options::default().with_rewrites(if configured {
        rewrites
    } else {
        Some(Default::default())
    });
    let empty = repo.empty_tree();
    let old = old.unwrap_or(&empty);
    let mut changes = Vec::new();
    gix::diff::tree_with_rewrites(
        gix::objs::TreeRefIter::from_bytes(&old.data, repo.object_hash()),
        gix::objs::TreeRefIter::from_bytes(&new.data, repo.object_hash()),
        &mut cache,
        &mut Default::default(),
        &repo.objects,
        |change| -> gix::ExnResult<_> {
            changes.push(change.into_owned());
            Ok(std::ops::ControlFlow::Continue(()))
        },
        options.into(),
    )
    .map_err(failure)?;
    Ok(changes)
}
