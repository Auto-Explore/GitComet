use super::{RefBackend, backend, head_name};

/// gix loads `onbranch` includes using the files-backed placeholder HEAD.
/// Re-evaluate them with the native HEAD before exposing a reftable repository.
pub(crate) fn reload_conditional_includes(repo: &mut gix::Repository) -> gix::Result<()> {
    let options = repo.open_options();
    if !options.permissions.config.includes
        || !matches!(backend(repo), Ok(RefBackend::Reftable))
        || !repo.config_snapshot().plumbing().sections().any(|section| {
            section.header().name().eq_ignore_ascii_case(b"includeIf")
                && section
                    .header()
                    .subsection_name()
                    .is_some_and(|condition| condition.starts_with(b"onbranch:"))
        })
    {
        return Ok(());
    }

    let branch = head_name(repo).map_err(gix::Error::from_error)?;
    let git_dir = repo.git_dir().to_path_buf();
    let home =
        gix::path::env::home_dir().and_then(|home| options.permissions.env.home.check_opt(home));
    let executable = std::env::current_exe().ok();
    let include_options = gix::config::file::init::Options {
        includes: gix::config::file::includes::Options::follow(
            gix::config::path::interpolate::Context {
                home_dir: home.as_deref(),
                git_install_dir: executable.as_deref().and_then(std::path::Path::parent),
                home_for_user: Some(gix::config::path::interpolate::home_for_user),
            },
            gix::config::file::includes::conditional::Context {
                git_dir: Some(&git_dir),
                branch_name: branch.as_ref().map(|name| name.as_ref()),
            },
        ),
        // open_worktree_repo uses gix::open's lenient default.
        ignore_io_errors: true,
        ..Default::default()
    };
    let mut config = repo.config_snapshot_mut();
    // Retain top-level sources and their order, trust and overrides. Removing
    // all expanded sections avoids keeping a placeholder-matched include or
    // duplicating unconditional includes when resolving the graph again.
    let included: Vec<_> = config
        .sections_and_ids()
        .filter_map(|(section, id)| (section.meta().level > 0).then_some(id))
        .collect();
    for id in included {
        config.remove_section_by_id(id);
    }
    config.resolve_includes(include_options)?;
    config.commit()?;
    Ok(())
}
