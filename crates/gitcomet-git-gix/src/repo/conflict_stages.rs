use gitcomet_core::domain::FileConflictKind;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::Result;
use std::path::Path;

#[derive(Default)]
pub(super) struct ConflictStageData {
    pub(super) conflict_kind: Option<FileConflictKind>,
    pub(super) base_bytes: Option<Vec<u8>>,
    pub(super) ours_bytes: Option<Vec<u8>>,
    pub(super) theirs_bytes: Option<Vec<u8>>,
}

fn gix_index_stage_from_u8(stage: u8) -> Option<gix::index::entry::Stage> {
    match stage {
        0 => Some(gix::index::entry::Stage::Unconflicted),
        1 => Some(gix::index::entry::Stage::Base),
        2 => Some(gix::index::entry::Stage::Ours),
        3 => Some(gix::index::entry::Stage::Theirs),
        _ => None,
    }
}

pub(super) fn gix_index_stage_object_id_optional(
    repo: &gix::Repository,
    path: &Path,
    stage: u8,
) -> Result<Option<gix::ObjectId>> {
    let Some(stage) = gix_index_stage_from_u8(stage) else {
        return Err(Error::new(ErrorKind::Backend(format!(
            "invalid conflict stage: {stage}"
        ))));
    };

    let index = repo
        .index_or_load_from_head_or_empty()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
    let path = gix::path::os_str_into_bstr(path.as_os_str())
        .map_err(|_| Error::new(ErrorKind::Unsupported("path is not valid UTF-8")))?;

    Ok(index
        .entry_by_path_and_stage(path, stage)
        .map(|entry| entry.id))
}

pub(super) fn conflict_kind_from_stage_mask(mask: u8) -> Option<FileConflictKind> {
    Some(match mask {
        0b001 => FileConflictKind::BothDeleted,
        0b010 => FileConflictKind::AddedByUs,
        0b011 => FileConflictKind::DeletedByThem,
        0b100 => FileConflictKind::AddedByThem,
        0b101 => FileConflictKind::DeletedByUs,
        0b110 => FileConflictKind::BothAdded,
        0b111 => FileConflictKind::BothModified,
        _ => return None,
    })
}

pub(super) fn gix_index_stage_exists(
    repo: &gix::Repository,
    path: &Path,
    stage: u8,
) -> Result<bool> {
    Ok(gix_index_stage_object_id_optional(repo, path, stage)?.is_some())
}

pub(super) fn gix_index_stage_blob_bytes_optional(
    repo: &gix::Repository,
    path: &Path,
    stage: u8,
) -> Result<Option<Vec<u8>>> {
    let Some(object_id) = gix_index_stage_object_id_optional(repo, path, stage)? else {
        return Ok(None);
    };

    let Some(object) = repo
        .try_find_object(object_id)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix try_find_object: {e}"))))?
    else {
        return Err(Error::new(ErrorKind::Backend(format!(
            "missing conflict stage object for :{stage}:{}",
            path.display()
        ))));
    };

    let mut blob = object.try_into_blob().map_err(|_| {
        Error::new(ErrorKind::Backend(format!(
            "conflict stage object for :{stage}:{} is not a blob",
            path.display()
        )))
    })?;
    Ok(Some(blob.take_data()))
}

fn gix_blob_bytes_from_object_id_optional(
    repo: &gix::Repository,
    path: &Path,
    stage: u8,
    object_id: Option<gix::ObjectId>,
) -> Result<Option<Vec<u8>>> {
    let Some(object_id) = object_id else {
        return Ok(None);
    };

    let Some(object) = repo
        .try_find_object(object_id)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix try_find_object: {e}"))))?
    else {
        return Err(Error::new(ErrorKind::Backend(format!(
            "missing conflict stage object for :{stage}:{}",
            path.display()
        ))));
    };

    let mut blob = object.try_into_blob().map_err(|_| {
        Error::new(ErrorKind::Backend(format!(
            "conflict stage object for :{stage}:{} is not a blob",
            path.display()
        )))
    })?;
    Ok(Some(blob.take_data()))
}

pub(super) fn gix_index_conflict_stage_data(
    repo: &gix::Repository,
    path: &Path,
) -> Result<ConflictStageData> {
    let index = repo
        .index_or_load_from_head_or_empty()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
    let path_key = gix::path::os_str_into_bstr(path.as_os_str())
        .map_err(|_| Error::new(ErrorKind::Unsupported("path is not valid UTF-8")))?;

    let base_id = index
        .entry_by_path_and_stage(path_key, gix::index::entry::Stage::Base)
        .map(|entry| entry.id);
    let ours_id = index
        .entry_by_path_and_stage(path_key, gix::index::entry::Stage::Ours)
        .map(|entry| entry.id);
    let theirs_id = index
        .entry_by_path_and_stage(path_key, gix::index::entry::Stage::Theirs)
        .map(|entry| entry.id);

    let mut stage_mask = 0u8;
    if base_id.is_some() {
        stage_mask |= 0b001;
    }
    if ours_id.is_some() {
        stage_mask |= 0b010;
    }
    if theirs_id.is_some() {
        stage_mask |= 0b100;
    }

    Ok(ConflictStageData {
        conflict_kind: conflict_kind_from_stage_mask(stage_mask),
        base_bytes: gix_blob_bytes_from_object_id_optional(repo, path, 1, base_id)?,
        ours_bytes: gix_blob_bytes_from_object_id_optional(repo, path, 2, ours_id)?,
        theirs_bytes: gix_blob_bytes_from_object_id_optional(repo, path, 3, theirs_id)?,
    })
}
