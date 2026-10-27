use gitcomet_core::domain::{RepositoryMode, RepositoryRequirement};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::GitRepository;

pub(super) fn check(
    repository: &dyn GitRepository,
    requirement: RepositoryRequirement,
) -> Result<(), Error> {
    if repository.repository_mode()? == RepositoryMode::Bare
        && requirement == RepositoryRequirement::WorkingTree
    {
        return Err(Error::new(ErrorKind::BareUnsupported));
    }
    Ok(())
}
