use super::bytes::invalid;
use super::table::{Record, Table};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

const CACHE_BYTES: usize = 128 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DirectoryIdentity {
    device: u64,
    inode: u64,
    // Tiebreaker only: ReFS file indexes are not guaranteed unique, and a
    // recreated directory can report the old one's creation time.
    #[cfg(windows)]
    created: Option<u64>,
}

#[cfg(unix)]
fn directory_identity(dir: &Path) -> Option<DirectoryIdentity> {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::metadata(dir).ok()?;
    Some(DirectoryIdentity {
        device: meta.dev(),
        inode: meta.ino(),
    })
}

#[cfg(windows)]
fn directory_identity(dir: &Path) -> Option<DirectoryIdentity> {
    // std keeps the volume serial and file index unstable; read them by handle.
    let handle = winapi_util::Handle::from_path_any(dir).ok()?;
    let info = winapi_util::file::information(&handle).ok()?;
    Some(DirectoryIdentity {
        device: info.volume_serial_number(),
        inode: info.file_index(),
        created: info.creation_time(),
    })
}
type CacheKey = (PathBuf, Option<DirectoryIdentity>, gix::hash::Kind);
static CACHE: LazyLock<Mutex<lru::LruCache<CacheKey, Arc<Stack>>>> = LazyLock::new(|| {
    Mutex::new(lru::LruCache::new(
        NonZeroUsize::new(32).expect("nonzero cache capacity"),
    ))
});

pub(crate) struct Stack {
    pub refs: BTreeMap<Vec<u8>, Record>,
    list: Vec<u8>,
    tables: Vec<Arc<Table>>,
    resident: usize,
}

impl Stack {
    pub fn load(
        dir: &Path,
        hash: gix::hash::Kind,
        optional: bool,
        cancel: &CancellationToken,
    ) -> Result<Arc<Self>> {
        let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        let dir = dir.as_path();
        let key = (dir.to_path_buf(), directory_identity(dir), hash);
        for _ in 0..5 {
            cancel.check_cancelled()?;
            let list = match std::fs::read(dir.join("tables.list")) {
                Ok(list) => list,
                Err(e) if optional && e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(invalid(&dir.join("tables.list"), 0, e)),
            };
            let old = CACHE
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&key)
                .cloned();
            if let Some(old) = &old
                && old.list == list
            {
                return Ok(Arc::clone(old));
            }
            let mut tables = Vec::new();
            let mut missing = false;
            for name in list.split(|b| *b == b'\n').filter(|n| !n.is_empty()) {
                cancel.check_cancelled()?;
                let name = std::str::from_utf8(name).map_err(|e| invalid(dir, 0, e))?;
                if !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
                    || !name.ends_with(".ref")
                    || !matches!(
                        Path::new(name).components().next(),
                        Some(Component::Normal(_))
                    )
                {
                    return Err(invalid(dir, 0, "invalid table filename"));
                }
                let path = dir.join(name);
                if tables.iter().any(|t: &Arc<Table>| t.path == path) {
                    return Err(invalid(dir, 0, "duplicate table name"));
                }
                if let Some(table) = old
                    .as_ref()
                    .and_then(|s| s.tables.iter().find(|t| t.path == path))
                {
                    tables.push(Arc::clone(table));
                    continue;
                }
                let bytes = match std::fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        missing = true;
                        break;
                    }
                    Err(e) => return Err(invalid(&path, 0, e)),
                };
                tables.push(Arc::new(Table::parse(path, bytes.into(), hash, cancel)?));
            }
            if missing {
                if std::fs::read(dir.join("tables.list")).ok().as_deref() == Some(list.as_slice()) {
                    return Err(invalid(dir, 0, "unchanged stack refers to a missing table"));
                }
                continue;
            }
            if tables.windows(2).any(|pair| pair[0].max >= pair[1].min) {
                return Err(invalid(dir, 0, "overlapping or out-of-order tables"));
            }
            let mut refs = BTreeMap::new();
            for table in &tables {
                cancel.check_cancelled()?;
                for (name, record) in &table.refs {
                    cancel.check_cancelled()?;
                    if let Some(record) = record {
                        refs.insert(name.clone(), record.clone());
                    } else {
                        refs.remove(name);
                    }
                }
            }
            let resident = tables
                .iter()
                .map(|t| t.resident_bytes())
                .sum::<usize>()
                .saturating_add(
                    refs.keys()
                        .map(|n| n.len() + std::mem::size_of::<Record>() + 96)
                        .sum::<usize>(),
                );
            let out = Arc::new(Self {
                refs,
                list,
                tables,
                resident,
            });
            if resident <= CACHE_BYTES {
                let mut cache = CACHE.lock().unwrap_or_else(|p| p.into_inner());
                // Do not overwrite a newer generation published while we loaded.
                let unchanged = match (cache.peek(&key), &old) {
                    (None, _) => true,
                    (Some(current), Some(old)) => Arc::ptr_eq(current, old),
                    (Some(_), None) => false,
                };
                if unchanged {
                    cache.pop(&key);
                    while cache
                        .iter()
                        .map(|(_, s)| s.resident)
                        .sum::<usize>()
                        .saturating_add(resident)
                        > CACHE_BYTES
                    {
                        cache.pop_lru();
                    }
                    cache.put(key, Arc::clone(&out));
                }
            }
            return Ok(out);
        }
        Err(Error::new(ErrorKind::Backend(format!(
            "reftable {}: stack kept changing; retry the operation",
            dir.display()
        ))))
    }

    pub fn reflog(
        &self,
        name: &[u8],
        limit: Option<usize>,
        cancel: &CancellationToken,
    ) -> Result<Vec<gix::refs::log::Line>> {
        if limit == Some(0) {
            return Ok(Vec::new());
        }
        let mut merged = BTreeMap::new();
        for table in &self.tables {
            for (update, line) in table.logs(name, cancel)? {
                merged.insert(update, line);
            }
        }
        Ok(merged
            .into_iter()
            .rev()
            .filter_map(|(_, line)| line)
            .filter(|line| !(line.previous_oid.is_null() && line.new_oid.is_null()))
            .take(limit.unwrap_or(usize::MAX))
            .collect())
    }
}

#[test]
fn snapshots_reuse_immutable_tables_and_detect_directory_replacement() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("reftable");
    std::fs::create_dir(&dir).unwrap();
    let fixture = include_bytes!("../../../tests/fixtures/reftable/sha1.ref");
    std::fs::write(dir.join("first.ref"), fixture).unwrap();
    std::fs::write(dir.join("tables.list"), "first.ref\n").unwrap();
    let cancel = CancellationToken::new();
    let first = Stack::load(&dir, gix::hash::Kind::Sha1, false, &cancel).unwrap();
    let mut header = fixture[..24].to_vec();
    let update = first.tables[0].max + 1;
    header[8..16].copy_from_slice(&update.to_be_bytes());
    header[16..24].copy_from_slice(&update.to_be_bytes());
    let mut footer = header.clone();
    footer.extend_from_slice(&[0; 40]);
    footer.extend_from_slice(&crc32fast::hash(&footer).to_be_bytes());
    header.extend(footer);
    std::fs::write(dir.join("next.ref"), &header).unwrap();
    std::fs::write(dir.join("tables.list"), "first.ref\nnext.ref\n").unwrap();
    let next = Stack::load(&dir, gix::hash::Kind::Sha1, false, &cancel).unwrap();
    assert!(!Arc::ptr_eq(&first, &next));
    assert!(Arc::ptr_eq(&first.tables[0], &next.tables[0]));
    assert_eq!(first.refs.len(), next.refs.len());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(Stack::load(&dir, gix::hash::Kind::Sha1, false, &cancelled).is_err());
    std::fs::rename(&dir, root.path().join("old-reftable")).unwrap();
    std::fs::create_dir(&dir).unwrap();
    // Reuse the old filename and list at the same path with a different stack.
    std::fs::write(dir.join("first.ref"), &header).unwrap();
    std::fs::write(dir.join("tables.list"), "first.ref\n").unwrap();
    let replacement = Stack::load(&dir, gix::hash::Kind::Sha1, false, &cancel).unwrap();
    assert!(replacement.refs.is_empty());
    assert!(!first.refs.is_empty());
}

#[test]
#[ignore = "set GITCOMET_REF_BENCH_REPO to measure a disposable repository"]
fn reference_snapshot_benchmark() {
    let path = std::env::var("GITCOMET_REF_BENCH_REPO").expect("GITCOMET_REF_BENCH_REPO");
    let repo = crate::open::open_worktree_repo(Path::new(&path)).unwrap();
    CACHE.lock().unwrap().clear();
    let start = std::time::Instant::now();
    let refs = crate::refs::view(&repo).unwrap();
    refs.head_oid().unwrap();
    eprintln!("cold_head_us={}", start.elapsed().as_micros());
    if let Some(stack) = &refs.common {
        eprintln!(
            "tables={} table_bytes={} retained_bytes={} refs={}",
            stack.tables.len(),
            stack.tables.iter().map(|t| t.bytes.len()).sum::<usize>(),
            stack.resident,
            stack.refs.len()
        );
    }
    let start = std::time::Instant::now();
    let (_, commands) = crate::command_trace::capture(|| {
        for _ in 0..1000 {
            crate::refs::view(&repo).unwrap().head_oid().unwrap();
        }
    });
    eprintln!(
        "warm_head_mean_us={:.2} spawns={}",
        start.elapsed().as_micros() as f64 / 1000.0,
        commands.len()
    );
    let start = std::time::Instant::now();
    let branches = refs
        .local_branches()
        .unwrap()
        .chain(refs.remote_branches().unwrap())
        .collect::<Result<Vec<_>>>()
        .unwrap();
    eprintln!(
        "branches={} branches_us={}",
        branches.len(),
        start.elapsed().as_micros()
    );
    let start = std::time::Instant::now();
    let tags = refs
        .tags()
        .unwrap()
        .peeled()
        .unwrap()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    eprintln!(
        "tags={} tags_us={}",
        tags.len(),
        start.elapsed().as_micros()
    );
    let start = std::time::Instant::now();
    let log = refs.reflog("HEAD", Some(50)).unwrap();
    eprintln!(
        "reflog_entries={} reflog_us={}",
        log.len(),
        start.elapsed().as_micros()
    );
    if std::env::var_os("GITCOMET_REF_BENCH_MUTATE").is_some() {
        // Opt in only for a disposable fixture, never the source benchmark repo.
        for args in [
            vec![
                "commit",
                "--allow-empty",
                "-m",
                "reference refresh benchmark",
            ],
            vec!["pack-refs", "--all"],
        ] {
            let output = crate::util::git_workdir_cmd_for(Path::new(&path))
                .args(&args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let start = std::time::Instant::now();
            let updated = crate::refs::view(&repo).unwrap();
            updated.head_oid().unwrap();
            eprintln!("after_{}_head_us={}", args[0], start.elapsed().as_micros());
            if let (Some(old), Some(new)) = (&refs.common, &updated.common) {
                let reused = new
                    .tables
                    .iter()
                    .filter(|table| old.tables.iter().any(|old| Arc::ptr_eq(old, table)))
                    .count();
                eprintln!(
                    "tables={} reused={} retained_bytes={}",
                    new.tables.len(),
                    reused,
                    new.resident
                );
            }
        }
    }
}
