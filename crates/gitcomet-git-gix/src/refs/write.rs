use super::{RefBackend, Reference, backend, failure};
use gitcomet_core::services::Result;
use gix::bstr::ByteSlice as _;

pub(super) fn transaction(
    repo: &gix::Repository,
    commands: Vec<u8>,
    message: Option<&str>,
) -> Result<()> {
    let mut cmd = crate::util::git_workdir_cmd_for(repo.workdir().unwrap_or(repo.git_dir()));
    // Ref edits, unlike commits, must also work before identity is configured.
    // Command-local fallback preserves configured identity and never writes config.
    let identity = gitcomet_core::identity::current();
    cmd.arg("-c")
        .arg(format!("user.name={}", identity.display_name()))
        .arg("-c")
        .arg(format!(
            "user.email={}@localhost",
            identity.executable_name()
        ));
    if let Some(Ok(signature)) = repo.committer() {
        cmd.env(
            "GIT_COMMITTER_NAME",
            signature.name.to_os_str().map_err(failure)?,
        )
        .env(
            "GIT_COMMITTER_EMAIL",
            signature.email.to_os_str().map_err(failure)?,
        );
    }
    cmd.arg("update-ref");
    if let Some(message) = message {
        cmd.args(["-m", message]);
    }
    cmd.args(["--stdin", "-z"]);
    let mut input = b"start\0".to_vec();
    input.extend(commands);
    input.extend_from_slice(b"prepare\0commit\0");
    crate::util::run_git_with_stdin_capture(
        cmd,
        input,
        "git update-ref --stdin -z",
        crate::util::git_command_timeout(),
        None,
    )?;
    Ok(())
}

pub(crate) fn create(
    repo: &gix::Repository,
    name: &str,
    id: gix::ObjectId,
    message: &str,
) -> Result<()> {
    if backend(repo)? == RefBackend::Files {
        repo.reference(
            name,
            id,
            gix::refs::transaction::PreviousValue::MustNotExist,
            message,
        )
        .map_err(failure)?;
        return Ok(());
    }
    gix::refs::FullName::try_from(name).map_err(failure)?;
    if id.kind() != repo.object_hash() {
        return Err(failure("new reference object format mismatch"));
    }
    let command = format!("option no-deref\0create {name}\0{id}\0").into_bytes();
    transaction(repo, command, Some(message))
}

pub(crate) fn delete(reference: Reference<'_>) -> Result<()> {
    if backend(reference.repo)? == RefBackend::Files {
        return reference.inner.delete().map_err(failure);
    }
    let mut commands = Vec::new();
    encode_delete(
        &mut commands,
        reference.name().as_bstr(),
        reference.target(),
    );
    transaction(reference.repo, commands, None)
}

pub(super) fn encode_delete(commands: &mut Vec<u8>, name: &[u8], target: gix::refs::TargetRef<'_>) {
    commands.extend_from_slice(b"option no-deref\0");
    match target {
        gix::refs::TargetRef::Object(id) => {
            commands.extend_from_slice(b"delete ");
            commands.extend_from_slice(name);
            commands.push(0);
            commands.extend_from_slice(id.to_string().as_bytes());
            commands.push(0);
        }
        gix::refs::TargetRef::Symbolic(target) => {
            commands.extend_from_slice(b"symref-delete ");
            commands.extend_from_slice(name);
            commands.push(0);
            commands.extend_from_slice(target.as_bstr());
            commands.push(0);
        }
    }
}

/// One atomic native-format transaction. A stale expectation aborts the whole
/// batch; callers performing opportunistic cleanup may leave it for a refresh.
pub(crate) fn delete_batch(repo: &gix::Repository, names: &[&str]) -> Result<()> {
    if backend(repo)? == RefBackend::Files {
        for name in names {
            if let Some(reference) = super::find(repo, name)? {
                delete(reference)?;
            }
        }
        return Ok(());
    }
    let refs = super::view(repo)?;
    let mut commands = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for name in names {
        if !seen.insert(*name) {
            continue;
        }
        if let Some(entry) = refs.find_exact(name)? {
            encode_delete(&mut commands, name.as_bytes(), entry.target.to_ref());
        }
    }
    if commands.is_empty() {
        return Ok(());
    }
    transaction(repo, commands, None)
}

pub(crate) fn delete_named(repo: &gix::Repository, name: &str) -> Result<bool> {
    if backend(repo)? == RefBackend::Files {
        let Some(reference) = super::find(repo, name)? else {
            return Ok(false);
        };
        delete(reference)?;
    } else {
        let Some(entry) = super::view(repo)?.find_exact(name)? else {
            return Ok(false);
        };
        let mut commands = Vec::new();
        encode_delete(&mut commands, name.as_bytes(), entry.target.to_ref());
        transaction(repo, commands, None)?;
    }
    Ok(true)
}
