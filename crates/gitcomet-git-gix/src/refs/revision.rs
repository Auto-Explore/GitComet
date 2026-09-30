use super::{RefBackend, RefsView, backend, failure, view};
use gitcomet_core::services::Result;
use gix::bstr::ByteSlice as _;
use gix::prelude::ObjectIdExt as _;

pub(crate) fn resolve(repo: &gix::Repository, spec: &str) -> Result<Option<gix::ObjectId>> {
    let leading = spec.split(['~', '^', '@', ':']).next().unwrap_or_default();
    if leading.len() > repo.object_hash().len_in_hex()
        && leading.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(failure(
            "revision object id is longer than the repository's object format",
        ));
    }
    if backend(repo)? == RefBackend::Files && repo.object_hash() == gix::hash::Kind::Sha1 {
        return match repo.rev_parse_single(spec) {
            Ok(id) => Ok(Some(id.detach())),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(failure(e)),
        };
    }
    view(repo)?.resolve(spec)
}

pub(crate) fn resolve_required<'r>(repo: &'r gix::Repository, spec: &str) -> Result<gix::Id<'r>> {
    resolve(repo, spec)?
        .map(|id| id.attach(repo))
        .ok_or_else(|| failure(format!("revision {spec:?} not found")))
}

impl RefsView<'_> {
    pub fn resolve(&self, spec: &str) -> Result<Option<gix::ObjectId>> {
        if self.common.is_none() && self.repo.object_hash() == gix::hash::Kind::Sha1 {
            return resolve(self.repo, spec);
        }
        // Translate only syntax whose reference semantics are fully known here.
        // Git handles date selectors, upstream/push, checkout history and searches.
        if spec.is_empty()
            || spec.starts_with(':')
            || spec.contains("^{/")
            || spec.contains("..")
            || spec.starts_with('-')
        {
            return self.resolve_cli(spec);
        }
        let (revision, path) = spec
            .split_once(':')
            .map_or((spec, None), |(r, p)| (r, Some(p)));
        let split = revision.find(['~', '^', '@']).unwrap_or(revision.len());
        let mut base = &revision[..split];
        let mut suffix = &revision[split..];
        let omitted_base = base.is_empty();
        if base.is_empty() {
            base = "HEAD";
        }
        if spec == "@" {
            suffix = "";
        }
        let mut id = if let Some(rest) = suffix.strip_prefix("@{") {
            let Some((selector, remaining)) = rest.split_once('}') else {
                return self.resolve_cli(spec);
            };
            let Ok(n) = selector.parse::<usize>() else {
                return self.resolve_cli(spec);
            };
            suffix = remaining;
            let name = if omitted_base {
                match self.head_name()? {
                    Some(name) => match name.as_bstr().to_str() {
                        Ok(name) => name.to_owned(),
                        Err(_) => return self.resolve_cli(spec),
                    },
                    None => "HEAD".to_owned(),
                }
            } else if base == "HEAD" {
                "HEAD".to_string()
            } else {
                let Some(reference) = self.find(base)? else {
                    return Ok(None);
                };
                // Revision input is UTF-8, but DWIM can still resolve a byte name.
                reference
                    .name()
                    .as_bstr()
                    .to_str()
                    .map_err(failure)?
                    .to_owned()
            };
            self.reflog(&name, n.checked_add(1))?
                .get(n)
                .map(|line| line.new_oid)
        } else if base.len() == self.repo.object_hash().len_in_hex()
            && base.bytes().all(|b| b.is_ascii_hexdigit())
        {
            Some(gix::ObjectId::from_hex(base.as_bytes()).map_err(failure)?)
        } else if base == "HEAD" {
            self.head_oid()?
        } else if let Some(reference) = self.find(base)? {
            reference.try_id().map(|id| id.detach())
        } else if base.len() >= 4
            && base.len() < self.repo.object_hash().len_in_hex()
            && base.bytes().all(|b| b.is_ascii_hexdigit())
        {
            let padded = format!(
                "{base:0<width$}",
                width = self.repo.object_hash().len_in_hex()
            );
            let full = gix::ObjectId::from_hex(padded.as_bytes()).map_err(failure)?;
            let prefix = gix::hash::Prefix::new(&full, base.len()).map_err(failure)?;
            match self
                .repo
                .objects
                .lookup_prefix(prefix, None)
                .map_err(failure)?
            {
                Some(Ok(id)) => Some(id),
                Some(Err(())) => return self.resolve_cli(spec),
                None => None,
            }
        } else {
            None
        };
        if suffix.contains('@') {
            return self.resolve_cli(spec);
        }
        if let Some(oid) = id
            && (!suffix.is_empty() || path.is_some())
        {
            let translated = format!(
                "{oid}{suffix}{}",
                path.map_or(String::new(), |p| format!(":{p}"))
            );
            id = Some(
                self.repo
                    .rev_parse_single(translated.as_str())
                    .map_err(failure)?
                    .detach(),
            );
        }
        Ok(id)
    }

    fn resolve_cli(&self, spec: &str) -> Result<Option<gix::ObjectId>> {
        let mut cmd =
            crate::util::git_workdir_cmd_for(self.repo.workdir().unwrap_or(self.repo.git_dir()));
        cmd.args(["rev-parse", "--verify", "--quiet", "--end-of-options", spec]);
        let label = "git rev-parse --verify --quiet --end-of-options";
        let output = crate::util::run_git_raw_output(cmd, label)?;
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        if !output.status.success() {
            return Err(crate::util::git_command_failed_error(label, output));
        }
        let id = gix::ObjectId::from_hex(output.stdout.trim_ascii()).map_err(failure)?;
        if id.kind() != self.repo.object_hash() {
            return Err(failure("revision object format mismatch"));
        }
        Ok(Some(id))
    }
}
