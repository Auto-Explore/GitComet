use super::*;

pub const MAX_RECENT_DOCUMENTS: usize = 50;

/// Enqueue foreground history changes in their original order. A shared writer
/// also prevents an older background task from persisting after a newer one.
enum RecentDocumentWrite {
    Update(PathBuf, bool),
    Flush(std::sync::mpsc::Sender<()>),
}
static WRITER: std::sync::OnceLock<std::sync::mpsc::Sender<RecentDocumentWrite>> =
    std::sync::OnceLock::new();

pub fn enqueue_recent_document(path: PathBuf, remove: bool) {
    let writer = WRITER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("recent-documents".into())
            .spawn(move || {
                for command in rx {
                    match command {
                        RecentDocumentWrite::Update(path, remove) => {
                            let result = if remove {
                                remove_recent_document(&path)
                            } else {
                                persist_recent_document(&path)
                            };
                            if let Err(error) = result {
                                eprintln!("Could not save recent documents: {error}");
                            }
                        }
                        RecentDocumentWrite::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })
            .expect("start recent document writer");
        tx
    });
    let _ = writer.send(RecentDocumentWrite::Update(path, remove));
}

/// Call from the shutdown worker after foreground history changes have stopped.
pub fn flush_recent_documents() {
    if let Some(writer) = WRITER.get() {
        let (done, completion) = std::sync::mpsc::channel();
        if writer.send(RecentDocumentWrite::Flush(done)).is_ok() {
            let _ = completion.recv();
        }
    }
}

pub fn persist_recent_document(path: &Path) -> io::Result<()> {
    let Some(session) = default_session_file_path() else {
        return Ok(());
    };
    update_recent_document_at(&session, path, false)
}

pub fn remove_recent_document(path: &Path) -> io::Result<()> {
    let Some(session) = default_session_file_path() else {
        return Ok(());
    };
    update_recent_document_at(&session, path, true)
}

pub fn update_recent_document_at(session: &Path, path: &Path, remove: bool) -> io::Result<()> {
    let identity = gitcomet_core::filesystem::absolute_identity(path)
        .or_else(|_| std::path::absolute(path))?;
    with_session_file_persist_lock(|| {
        let mut file = load_file(session).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;
        let recents = file.recent_documents.get_or_insert_with(Vec::new);
        recents.retain(|key| {
            let stored = path_from_storage_key(key);
            stored != path && stored != identity
        });
        if !remove {
            recents.insert(0, path_storage_key(&identity));
        }
        recents.truncate(MAX_RECENT_DOCUMENTS);
        persist_to_path(session, &file)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn documents_deduplicate_cap_and_keep_missing_entries_removable() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("session.json");
        for i in 0..55 {
            update_recent_document_at(&session, &dir.path().join(format!("missing-{i}")), false)
                .unwrap();
        }
        let first = dir.path().join("missing-54");
        update_recent_document_at(&session, &first, false).unwrap();
        let loaded = load_from_path(&session);
        assert_eq!(loaded.recent_documents.len(), 50);
        assert_eq!(loaded.recent_documents[0], first);
        update_recent_document_at(&session, &first, true).unwrap();
        assert!(!load_from_path(&session).recent_documents.contains(&first));
    }
}
