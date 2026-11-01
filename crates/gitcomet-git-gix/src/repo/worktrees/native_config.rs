//! Conservative configuration eligibility shared by native worktree listings.
use super::GixRepo;
use std::path::Path;

pub(super) fn reopen(owner: &GixRepo) -> Option<gix::Repository> {
    open(&owner.spec.workdir, owner.repo().open_options().permissions)
}

pub(super) fn open(workdir: &Path, permissions: gix::open::Permissions) -> Option<gix::Repository> {
    // Preserve the original source permissions, but recalculate trust and read
    // fresh config. Strictness is local to native-list eligibility: on failure
    // the existing Git runner determines the authoritative result or error.
    let repo = crate::open::open_worktree_repo_with_options(
        workdir,
        gix::open::Options::default()
            .permissions(permissions)
            .strict_config(true),
    )
    .ok()?;
    let config = repo.config_snapshot();
    for section in config.plumbing().sections() {
        let header = section.header();
        if !permissions.config.includes
            && (header.name().eq_ignore_ascii_case(b"include")
                || header.name().eq_ignore_ascii_case(b"includeIf"))
        {
            // Preserve the owner's permission boundary. Git can read an
            // include we cannot inspect, so it must remain authoritative.
            return None;
        }
        if header.name().eq_ignore_ascii_case(b"includeIf")
            && header.subsection_name().is_some_and(|condition| {
                condition.starts_with(b"worktree:") || condition.starts_with(b"worktree/i:")
            })
        {
            // Git supports these conditions, but pinned gix ignores them. The
            // expanded graph retains their sections, including nested includes.
            return None;
        }
        if header.name().eq_ignore_ascii_case(b"core") && header.subsection_name().is_none() {
            // Git validates each assignment, even one shadowed by a later
            // value. Numeric parsing also differs from gix; unusual spellings
            // stay with Git instead of inventing another boolean parser here.
            for value in section.values("ignorecase") {
                if !canonical_boolean(&value) {
                    return None;
                }
            }
        }
    }
    // Unlike the convenience accessor, this never defaults an invalid value.
    let _ = config.try_boolean("core.ignorecase").ok()?;
    Some(repo)
}

fn canonical_boolean(value: &[u8]) -> bool {
    [
        b"".as_slice(),
        b"0",
        b"1",
        b"true",
        b"false",
        b"yes",
        b"no",
        b"on",
        b"off",
    ]
    .iter()
    .any(|known| value.eq_ignore_ascii_case(known))
}
