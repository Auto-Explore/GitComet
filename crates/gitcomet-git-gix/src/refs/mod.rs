//! Storage-aware reference reads. Object access remains in gix; Git owns reftable writes.
mod capabilities;
mod config;
pub(crate) mod files;
mod reftable;
mod revision;
mod state;
mod write;
pub(crate) use config::reload_conditional_includes;
pub(crate) use revision::{resolve, resolve_required};
pub(crate) use state::{
    RootRef, diff_tree_to_tree, has_modules, head_tree_id_or_empty,
    index_or_load_from_head_or_empty, operation_state, root_ref, submodule_state, submodules,
};
pub(crate) use write::{create, delete, delete_batch, delete_named};

use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use gix::bstr::ByteSlice as _;
use itertools::Itertools as _;
use std::cell::{OnceCell, RefCell};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefBackend {
    Files,
    Reftable,
}

pub(crate) fn validate_open(
    repo: &gix::Repository,
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    if backend(repo)? == RefBackend::Reftable {
        capabilities::check(repo)?;
        view_cancellable(repo, cancellation.unwrap_or(&CancellationToken::new()))?.head_state()?;
    }
    Ok(())
}

pub(crate) fn backend(repo: &gix::Repository) -> Result<RefBackend> {
    let value = repo
        .config_snapshot()
        .plumbing()
        .string_filter("extensions.refStorage", |meta| {
            meta.source == gix::config::Source::Local && meta.level == 0
        });
    match value.as_deref().map(|v| v.as_slice()) {
        None | Some(b"files") => Ok(RefBackend::Files),
        Some(b"reftable") => Ok(RefBackend::Reftable),
        Some(other) => Err(failure(format!(
            "unsupported reference storage {}",
            other.as_bstr()
        ))),
    }
}

pub(crate) fn failure(error: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Backend(format!("references: {error}")))
}

#[derive(Clone, Debug)]
pub(crate) enum HeadState {
    Symbolic {
        name: gix::refs::FullName,
        id: Option<gix::ObjectId>,
    },
    Detached(gix::ObjectId),
}

/// A read-only adapter; the unmodified target remains available for fingerprints
/// and compare-and-swap writes even after symbolic resolution or peeling.
pub(crate) struct Reference<'r> {
    pub repo: &'r gix::Repository,
    inner: gix::Reference<'r>,
    raw: gix::refs::Target,
    // Only following a files-backed symref can replace the inner name.
    name: Option<gix::refs::FullName>,
}

impl<'r> Reference<'r> {
    fn from_gix(inner: gix::Reference<'r>) -> Self {
        Self {
            repo: inner.repo,
            raw: inner.inner.target.clone(),
            name: matches!(inner.inner.target, gix::refs::Target::Symbolic(_))
                .then(|| inner.name().to_owned()),
            inner,
        }
    }
    pub fn name(&self) -> &gix::refs::FullNameRef {
        self.name
            .as_ref()
            .map_or_else(|| self.inner.name(), |name| name.as_ref())
    }
    pub fn target(&self) -> gix::refs::TargetRef<'_> {
        self.raw.to_ref()
    }
    pub fn try_id(&self) -> Option<gix::Id<'r>> {
        self.inner.try_id()
    }
    pub fn id(&self) -> gix::Id<'r> {
        self.inner.id()
    }
    pub fn peel_to_id(&mut self) -> gix::Result<gix::Id<'r>> {
        self.inner.peel_to_id()
    }
    pub fn peel_to_commit(&mut self) -> gix::Result<gix::Commit<'r>> {
        self.inner.peel_to_commit()
    }
    pub fn follow_to_object(&mut self) -> gix::Result<gix::Id<'r>> {
        self.inner.follow_to_object()
    }
}

pub(crate) struct RefsView<'r> {
    repo: &'r gix::Repository,
    common: Option<Arc<reftable::Stack>>,
    private: Option<Arc<reftable::Stack>>,
    other_worktrees: RefCell<std::collections::BTreeMap<std::path::PathBuf, Arc<reftable::Stack>>>,
    files: OnceCell<gix::reference::iter::Platform<'r>>,
    cancellation: CancellationToken,
}

pub(crate) fn view(repo: &gix::Repository) -> Result<RefsView<'_>> {
    view_cancellable(repo, &CancellationToken::new())
}

pub(crate) fn view_cancellable<'r>(
    repo: &'r gix::Repository,
    cancellation: &CancellationToken,
) -> Result<RefsView<'r>> {
    cancellation.check_cancelled()?;
    let (common, private) = if backend(repo)? == RefBackend::Reftable {
        let common = reftable::Stack::load(
            &repo.common_dir().join("reftable"),
            repo.object_hash(),
            false,
            cancellation,
        )?;
        let private = if repo.git_dir() != repo.common_dir() {
            Some(reftable::Stack::load(
                &repo.git_dir().join("reftable"),
                repo.object_hash(),
                true,
                cancellation,
            )?)
        } else {
            None
        };
        (Some(common), private)
    } else {
        (None, None)
    };
    Ok(RefsView {
        repo,
        common,
        private,
        other_worktrees: RefCell::default(),
        files: OnceCell::new(),
        cancellation: cancellation.clone(),
    })
}

fn per_worktree(name: &[u8]) -> bool {
    (!name.is_empty()
        && name
            .iter()
            .all(|b| b.is_ascii_uppercase() || matches!(*b, b'_' | b'-')))
        || [
            b"refs/worktree/".as_slice(),
            b"refs/bisect/",
            b"refs/rewritten/",
        ]
        .iter()
        .any(|p| name.starts_with(p))
}

impl<'r> RefsView<'r> {
    fn worktree_alias<'a>(&self, name: &'a [u8]) -> Result<Option<(std::path::PathBuf, &'a [u8])>> {
        if let Some(inner) = name.strip_prefix(b"main-worktree/")
            && per_worktree(inner)
        {
            return Ok(Some((self.repo.common_dir().to_path_buf(), inner)));
        }
        if let Some(other) = name.strip_prefix(b"worktrees/")
            && let Some(slash) = other.iter().position(|b| *b == b'/')
        {
            let (id, inner) = (&other[..slash], &other[slash + 1..]);
            if per_worktree(inner) {
                if id.is_empty()
                    || id == b"."
                    || id == b".."
                    || id.contains(&b'\\')
                    || id.contains(&b':')
                {
                    return Err(failure("invalid worktree reference"));
                }
                let id = crate::util::path_buf_from_git_bytes(id, "worktree reference")?;
                return Ok(Some((
                    self.repo.common_dir().join("worktrees").join(id),
                    inner,
                )));
            }
        }
        Ok(None)
    }

    fn stack_for<'a>(&self, name: &'a [u8]) -> Result<(Arc<reftable::Stack>, &'a [u8])> {
        let common = self
            .common
            .as_ref()
            .ok_or_else(|| failure("not a reftable view"))?;
        if let Some((admin_dir, inner)) = self.worktree_alias(name)? {
            if admin_dir == self.repo.common_dir() {
                return Ok((Arc::clone(common), inner));
            }
            if admin_dir == self.repo.git_dir() {
                return Ok((Arc::clone(self.private.as_ref().unwrap_or(common)), inner));
            }
            let dir = admin_dir.join("reftable");
            if let Some(stack) = self.other_worktrees.borrow().get(&dir) {
                return Ok((Arc::clone(stack), inner));
            }
            let stack =
                reftable::Stack::load(&dir, self.repo.object_hash(), true, &self.cancellation)?;
            self.other_worktrees
                .borrow_mut()
                .insert(dir, Arc::clone(&stack));
            return Ok((stack, inner));
        }
        let stack = if per_worktree(name) {
            self.private.as_ref().unwrap_or(common)
        } else {
            common
        };
        Ok((Arc::clone(stack), name))
    }

    pub fn find_exact(&self, name: impl AsRef<[u8]>) -> Result<Option<reftable::Record>> {
        let name = name.as_ref();
        if self.common.is_none() {
            return self
                .repo
                .try_find_reference(name.as_bstr())
                .map(|r| {
                    // gix's lookup expands short names; root refs must not
                    // resolve to ordinary branches such as refs/heads/REVERT_HEAD.
                    r.filter(|r| r.name().as_bstr() == name).map(|r| {
                        let r = r.detach();
                        reftable::Record {
                            target: r.target,
                            peeled: r.peeled,
                        }
                    })
                })
                .map_err(failure);
        }
        let alias = self.worktree_alias(name)?;
        let file_name = alias.as_ref().map_or(name, |(_, inner)| *inner);
        if matches!(file_name, b"FETCH_HEAD" | b"MERGE_HEAD") {
            // Git exposes these file-backed pseudorefs only in the current
            // worktree; worktrees/<id>/ aliases address the ref store instead.
            if alias.is_some() {
                return Ok(None);
            }
            let path = self
                .repo
                .git_dir()
                .join(std::str::from_utf8(file_name).map_err(failure)?);
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(failure(e)),
            };
            let hex = bytes
                .split(|b| b.is_ascii_whitespace())
                .next()
                .unwrap_or_default();
            let id = gix::ObjectId::from_hex(hex).map_err(failure)?;
            if id.kind() != self.repo.object_hash() {
                return Err(failure("pseudoref object format mismatch"));
            }
            return Ok(Some(reftable::Record {
                target: gix::refs::Target::Object(id),
                peeled: None,
            }));
        }
        let (stack, name) = self.stack_for(name)?;
        Ok(stack.refs.get(name).cloned())
    }

    fn resolve_exact(&self, name: &[u8]) -> Result<Option<(gix::ObjectId, Option<gix::ObjectId>)>> {
        let mut seen = smallvec::SmallVec::<[gix::refs::FullName; 5]>::new();
        for _ in 0..5 {
            let current = seen.last().map_or(name, |target| target.as_bstr().as_ref());
            match self.find_exact(current)? {
                None => return Ok(None),
                Some(reftable::Record {
                    target: gix::refs::Target::Object(id),
                    peeled,
                }) => return Ok(Some((id, peeled))),
                Some(reftable::Record {
                    target: gix::refs::Target::Symbolic(target),
                    ..
                }) => {
                    // Like Git's ref_resolve_ref_unsafe, follow the stored name
                    // in the caller's ref context, even after a worktree alias.
                    if target.as_bstr() == name || seen.contains(&target) {
                        return Err(failure("symbolic reference cycle"));
                    }
                    seen.push(target);
                }
            }
        }
        Err(failure("symbolic reference chain exceeds Git's limit"))
    }

    fn attached_exact(&self, name: &[u8]) -> Result<Option<Reference<'r>>> {
        if self.common.is_none() {
            return self
                .repo
                .try_find_reference(name.as_bstr())
                .map(|r| r.map(Reference::from_gix))
                .map_err(failure);
        }
        let Some(record) = self.find_exact(name)? else {
            return Ok(None);
        };
        self.attach_record(name, &record)
    }

    fn attach_record(
        &self,
        name: &[u8],
        record: &reftable::Record,
    ) -> Result<Option<Reference<'r>>> {
        let resolved = match &record.target {
            gix::refs::Target::Object(id) => Some((*id, record.peeled)),
            gix::refs::Target::Symbolic(_) => self.resolve_exact(name)?,
        };
        let Some((id, peeled)) = resolved else {
            return Ok(None);
        };
        use gix::prelude::ReferenceExt as _;
        let inner = gix::refs::Reference {
            name: gix::refs::FullName::try_from(name.as_bstr()).map_err(failure)?,
            target: gix::refs::Target::Object(id),
            peeled,
        }
        .attach(self.repo);
        Ok(Some(Reference {
            repo: self.repo,
            name: None,
            inner,
            raw: record.target.clone(),
        }))
    }

    pub fn find(&self, name: impl AsRef<[u8]>) -> Result<Option<Reference<'r>>> {
        let name = name.as_ref();
        if self.common.is_none() {
            return self.attached_exact(name);
        }
        for (prefix, suffix) in [
            (b"".as_slice(), b"".as_slice()),
            (b"refs/", b""),
            (b"refs/tags/", b""),
            (b"refs/heads/", b""),
            (b"refs/remotes/", b""),
            (b"refs/remotes/", b"/HEAD"),
        ] {
            let candidate = [prefix, name, suffix].concat();
            if let Some(reference) = self.attached_exact(&candidate)? {
                return Ok(Some(reference));
            }
        }
        Ok(None)
    }

    pub fn head_state(&self) -> Result<HeadState> {
        if self.common.is_none() {
            let mut head = self.repo.head().map_err(failure)?;
            let name = head.referent_name().map(ToOwned::to_owned);
            // Classified so a pack I/O failure triggers the history store retry.
            let id = head
                .try_peel_to_id()
                .map_err(|e| crate::repo::object_store::gix_error("gix head peel", &e))?
                .map(|id| id.detach());
            return match (name, id) {
                (Some(name), id) => Ok(HeadState::Symbolic { name, id }),
                (None, Some(id)) => Ok(HeadState::Detached(id)),
                (None, None) => Err(failure("HEAD is missing")),
            };
        }
        match self.find_exact(b"HEAD")? {
            Some(reftable::Record {
                target: gix::refs::Target::Object(id),
                ..
            }) => Ok(HeadState::Detached(id)),
            Some(reftable::Record {
                target: gix::refs::Target::Symbolic(name),
                ..
            }) => {
                let id = self.resolve_exact(b"HEAD")?.map(|(id, _)| id);
                Ok(HeadState::Symbolic { name, id })
            }
            None => Err(failure("HEAD is missing from the reftable stack")),
        }
    }
    pub fn head_oid(&self) -> Result<Option<gix::ObjectId>> {
        Ok(match self.head_state()? {
            HeadState::Symbolic { id, .. } => id,
            HeadState::Detached(id) => Some(id),
        })
    }
    pub fn head_name(&self) -> Result<Option<gix::refs::FullName>> {
        if self.common.is_none() {
            return self.repo.head_name().map_err(failure);
        }
        Ok(match self.head_state()? {
            HeadState::Symbolic { name, .. } => Some(name),
            HeadState::Detached(_) => None,
        })
    }
    pub fn head_branch(&self) -> Result<Option<Reference<'r>>> {
        match self.head_name()? {
            Some(name) => self.attached_exact(name.as_bstr()),
            None => Ok(None),
        }
    }

    fn files(&self) -> Result<&gix::reference::iter::Platform<'r>> {
        if self.files.get().is_none() {
            let _ = self.files.set(self.repo.references().map_err(failure)?);
        }
        self.files
            .get()
            .ok_or_else(|| failure("reference iterator unavailable"))
    }
    fn iter<'v>(&'v self, prefix: &'v [u8]) -> Result<RefIter<'v, 'r>> {
        if self.common.is_none() {
            let iter = if prefix == b"refs/" {
                self.files()?.all()
            } else {
                self.files()?.prefixed(prefix.as_bstr())
            }
            .map_err(failure)?;
            return Ok(RefIter::Files(Box::new(iter)));
        }
        self.cancellation.check_cancelled()?;
        let common = records_with_prefix(self.common.as_deref(), prefix)
            .filter(|(name, _)| self.private.is_none() || !per_worktree(name));
        let private = records_with_prefix(self.private.as_deref(), prefix)
            .filter(|(name, _)| per_worktree(name));
        Ok(RefIter::Reftable {
            // Each stack is sorted and worktree routing makes them disjoint.
            // Borrow and merge records instead of copying every name and ref.
            iter: Box::new(common.merge_by(private, |(a, _), (b, _)| a <= b)),
            view: self,
            peel: false,
        })
    }
    pub fn all(&self) -> Result<RefIter<'_, 'r>> {
        self.iter(b"refs/")
    }
    pub fn prefixed<'v>(&'v self, prefix: &'v [u8]) -> Result<RefIter<'v, 'r>> {
        self.iter(prefix)
    }
    pub fn local_branches(&self) -> Result<RefIter<'_, 'r>> {
        self.iter(b"refs/heads/")
    }
    pub fn remote_branches(&self) -> Result<RefIter<'_, 'r>> {
        self.iter(b"refs/remotes/")
    }
    pub fn tags(&self) -> Result<RefIter<'_, 'r>> {
        self.iter(b"refs/tags/")
    }

    pub fn reflog(&self, name: &str, limit: Option<usize>) -> Result<Vec<gix::refs::log::Line>> {
        if self.common.is_some() {
            let (stack, name) = self.stack_for(name.as_bytes())?;
            return stack.reflog(name, limit, &self.cancellation);
        }
        let Some(reference) = self.repo.try_find_reference(name).map_err(failure)? else {
            return Ok(Vec::new());
        };
        let mut platform = reference.log_iter();
        files::reflog_lines_rev(&mut platform, name, limit)
    }
}

fn records_with_prefix<'v>(
    stack: Option<&'v reftable::Stack>,
    prefix: &'v [u8],
) -> impl Iterator<Item = (&'v Vec<u8>, &'v reftable::Record)> {
    use std::ops::Bound::{Included, Unbounded};
    stack
        .into_iter()
        .flat_map(move |stack| stack.refs.range::<[u8], _>((Included(prefix), Unbounded)))
        .take_while(move |(name, _)| name.starts_with(prefix))
}

type ReftableRecords<'v> = Box<dyn Iterator<Item = (&'v Vec<u8>, &'v reftable::Record)> + 'v>;

pub(crate) enum RefIter<'v, 'r> {
    Files(Box<gix::reference::iter::Iter<'v, 'r>>),
    Reftable {
        iter: ReftableRecords<'v>,
        view: &'v RefsView<'r>,
        peel: bool,
    },
}
impl<'v, 'r> RefIter<'v, 'r> {
    pub fn peeled(self) -> Result<Self> {
        Ok(match self {
            Self::Files(iter) => Self::Files(Box::new(iter.peeled().map_err(failure)?)),
            Self::Reftable { iter, view, .. } => Self::Reftable {
                iter,
                view,
                peel: true,
            },
        })
    }
}
impl<'r> Iterator for RefIter<'_, 'r> {
    type Item = Result<Reference<'r>>;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Files(iter) => iter
                .next()
                .map(|r| r.map(Reference::from_gix).map_err(failure)),
            Self::Reftable { iter, view, peel } => loop {
                let (name, record) = iter.next()?;
                if let Err(error) = view.cancellation.check_cancelled() {
                    return Some(Err(error));
                }
                let mut reference = match view.attach_record(name, record) {
                    Ok(Some(reference)) => reference,
                    Ok(None) => continue,
                    Err(error) => return Some(Err(error)),
                };
                if *peel && let Err(error) = reference.peel_to_id() {
                    return Some(Err(failure(error)));
                }
                return Some(Ok(reference));
            },
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Files(i) => i.size_hint(),
            Self::Reftable { iter, .. } => (0, iter.size_hint().1),
        }
    }
}

pub(crate) fn head_oid(repo: &gix::Repository) -> Result<Option<gix::ObjectId>> {
    view(repo)?.head_oid()
}
pub(crate) fn head_name(repo: &gix::Repository) -> Result<Option<gix::refs::FullName>> {
    view(repo)?.head_name()
}
pub(crate) fn find<'r>(
    repo: &'r gix::Repository,
    name: impl AsRef<[u8]>,
) -> Result<Option<Reference<'r>>> {
    view(repo)?.find(name)
}
#[cfg(test)]
pub(crate) fn find_required<'r>(
    repo: &'r gix::Repository,
    name: impl AsRef<[u8]>,
) -> Result<Reference<'r>> {
    find(repo, name)?.ok_or_else(|| failure("reference not found"))
}
