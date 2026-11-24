use super::{
    PathDisplayBenchSnapshot, PathDisplayCache, bench_reset, bench_snapshot, cached_path_display,
    format_windows_path_for_display, path_display_string, repo_path_name,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

#[test]
fn empty_cache_reserves_no_table_storage() {
    let cache = PathDisplayCache::default();
    assert_eq!(cache.recent.capacity(), 0);
    assert_eq!(cache.previous.capacity(), 0);
    #[cfg(not(windows))]
    assert_eq!(cache.present_hash_counts.capacity(), 0);
}

#[test]
fn a_small_working_set_keeps_the_second_generation_unallocated() {
    let mut cache = PathDisplayCache::default();
    for ix in 0..32 {
        cached_path_display(&mut cache, Path::new(&format!("src/{ix}.rs")));
    }
    assert_eq!(cache.len(), 32);
    assert_eq!(cache.previous.capacity(), 0);
    assert!(cache.recent.capacity() < PathDisplayCache::RECENT_MAX_ENTRIES);
    #[cfg(not(windows))]
    assert!(cache.present_hash_counts.capacity() < PathDisplayCache::MAX_ENTRIES);
}

#[cfg(not(windows))]
#[test]
fn colliding_paths_survive_promotion_and_generation_rotation() {
    let mut cache = PathDisplayCache::default();
    let promoted = "src/promoted.rs";
    let collision = "src/collision.rs";
    let hash = super::path_display_cache_key(promoted);
    for text in [promoted, collision] {
        super::cache_insert(
            &mut cache.recent,
            &mut cache.recent_entries,
            &mut cache.present_hash_counts,
            hash,
            text.into(),
        );
    }
    cache.rotate_generations();
    assert_eq!(
        cached_path_display(&mut cache, Path::new(promoted)).as_ref(),
        promoted
    );
    assert!(super::cache_lookup(&cache.previous, hash, collision).is_some());
    assert_eq!(cache.present_hash_counts[&hash], 2);
    cache.rotate_generations();
    assert!(super::cache_lookup(&cache.previous, hash, promoted).is_some());
    assert!(super::cache_lookup(&cache.previous, hash, collision).is_none());
    assert_eq!(cache.present_hash_counts[&hash], 1);
    assert_eq!(cache.len(), 1);
}

#[cfg(not(windows))]
fn cache_contains(cache_map: &super::PathDisplayCacheMap, path: &Path) -> bool {
    let path_key = path.to_str().expect("test paths should be utf-8");
    let path_key_hash = super::path_display_cache_key(path_key);
    cache_map
        .get(&path_key_hash)
        .is_some_and(|bucket| bucket.iter().any(|label| label.as_ref() == path_key))
}

#[cfg(not(windows))]
fn cache_contains_recent(cache: &PathDisplayCache, path: &Path) -> bool {
    cache_contains(&cache.recent, path)
}

#[cfg(windows)]
fn cache_contains_recent(cache: &PathDisplayCache, path: &Path) -> bool {
    cache.recent.contains_key(path)
}

#[cfg(not(windows))]
fn cache_contains_previous(cache: &PathDisplayCache, path: &Path) -> bool {
    cache_contains(&cache.previous, path)
}

#[cfg(windows)]
fn cache_contains_previous(cache: &PathDisplayCache, path: &Path) -> bool {
    cache.previous.contains_key(path)
}

#[test]
fn strips_verbatim_disk_prefix_and_uses_forward_slashes() {
    let formatted = format_windows_path_for_display(r"\\?\C:\Users\sanni\git\GitComet".to_string());
    assert_eq!(formatted, "C:/Users/sanni/git/GitComet");
}

#[test]
fn strips_verbatim_unc_prefix_and_uses_forward_slashes() {
    let formatted = format_windows_path_for_display(r"\\?\UNC\server\share\repo".to_string());
    assert_eq!(formatted, "//server/share/repo");
}

#[test]
fn bench_counters_track_hits_misses_and_clears() {
    bench_reset();

    let mut cache = PathDisplayCache::default();
    let path = PathBuf::from("src/lib.rs");
    let _ = cached_path_display(&mut cache, &path);
    let _ = cached_path_display(&mut cache, &path);

    for ix in 0..8_193 {
        let extra = PathBuf::from(format!("src/module_{ix}/file_{ix}.rs"));
        let _ = cached_path_display(&mut cache, &extra);
    }

    assert_eq!(
        bench_snapshot(),
        PathDisplayBenchSnapshot {
            cache_hits: 1,
            cache_misses: 8_194,
            cache_clears: 1,
        }
    );
    assert!(cache.len() <= PathDisplayCache::MAX_ENTRIES);
}

#[cfg(not(windows))]
#[test]
fn utf8_cache_reuses_shared_string_for_entry_and_return_value() {
    let mut cache = PathDisplayCache::default();
    // `SharedString` is backed by `SmolStr`, which stores strings up to 23
    // bytes inline (copied, not `Arc`-shared). To exercise the zero-copy
    // path the label must exceed that inline capacity.
    let path = PathBuf::from("src/deeply/nested/module/path/file.rs");
    let display = cached_path_display(&mut cache, &path);

    let cached = cache
        .recent
        .values()
        .next()
        .and_then(|bucket| bucket.first())
        .cloned()
        .expect("cached recent label");
    let cached_arc: Arc<str> = cached.into();
    let display_arc: Arc<str> = display.into();

    assert!(Arc::ptr_eq(&cached_arc, &display_arc));
}

#[test]
fn previous_generation_hit_promotes_into_recent() {
    let mut cache = PathDisplayCache::default();
    let promoted = PathBuf::from("src/promoted.rs");

    for ix in 0..PathDisplayCache::RECENT_MAX_ENTRIES {
        let path = if ix == 0 {
            promoted.clone()
        } else {
            PathBuf::from(format!("src/previous_{ix}.rs"))
        };
        let _ = cached_path_display(&mut cache, &path);
    }
    let _ = cached_path_display(&mut cache, Path::new("src/rotate.rs"));

    assert!(
        !cache_contains_recent(&cache, &promoted),
        "promoted path should start in the previous generation"
    );
    assert!(cache_contains_previous(&cache, &promoted));

    let display = cached_path_display(&mut cache, &promoted);
    assert_eq!(display.as_ref(), promoted.to_str().unwrap_or_default());
    assert!(
        cache_contains_recent(&cache, &promoted),
        "previous-generation hits should be promoted into recent"
    );
    assert!(
        !cache_contains_previous(&cache, &promoted),
        "promoted entries should be removed from the previous generation"
    );
}

#[test]
fn overflow_miss_preserves_full_two_generation_hot_set() {
    bench_reset();

    let mut cache = PathDisplayCache::default();
    let previous_hot = PathBuf::from("src/previous_hot.rs");
    let recent_hot = PathBuf::from("src/recent_hot.rs");

    for ix in 0..PathDisplayCache::RECENT_MAX_ENTRIES {
        let path = if ix == 0 {
            previous_hot.clone()
        } else {
            PathBuf::from(format!("src/previous_{ix}.rs"))
        };
        let _ = cached_path_display(&mut cache, &path);
    }
    for ix in 0..PathDisplayCache::RECENT_MAX_ENTRIES {
        let path = if ix == 0 {
            recent_hot.clone()
        } else {
            PathBuf::from(format!("src/recent_{ix}.rs"))
        };
        let _ = cached_path_display(&mut cache, &path);
    }

    let before = bench_snapshot();
    assert_eq!(cache.len(), PathDisplayCache::MAX_ENTRIES);
    assert!(cache_contains_previous(&cache, &previous_hot));
    assert!(cache_contains_recent(&cache, &recent_hot));

    let overflow = PathBuf::from("src/overflow.rs");
    let _ = cached_path_display(&mut cache, &overflow);

    let after = bench_snapshot();
    assert_eq!(cache.len(), PathDisplayCache::MAX_ENTRIES);
    assert_eq!(
        after.cache_clears, before.cache_clears,
        "overflow misses should not rotate a full two-generation cache"
    );
    assert_eq!(after.cache_misses, before.cache_misses + 1);
    assert!(cache_contains_previous(&cache, &previous_hot));
    assert!(cache_contains_recent(&cache, &recent_hot));
    assert!(!cache_contains_recent(&cache, &overflow));
    assert!(!cache_contains_previous(&cache, &overflow));
}

#[test]
fn overflow_tail_misses_still_allow_cached_hits() {
    bench_reset();

    let mut cache = PathDisplayCache::default();
    let previous_hot = PathBuf::from("src/previous_hot.rs");
    let recent_hot = PathBuf::from("src/recent_hot.rs");

    for ix in 0..PathDisplayCache::RECENT_MAX_ENTRIES {
        let path = if ix == 0 {
            previous_hot.clone()
        } else {
            PathBuf::from(format!("src/previous_{ix}.rs"))
        };
        let _ = cached_path_display(&mut cache, &path);
    }
    for ix in 0..PathDisplayCache::RECENT_MAX_ENTRIES {
        let path = if ix == 0 {
            recent_hot.clone()
        } else {
            PathBuf::from(format!("src/recent_{ix}.rs"))
        };
        let _ = cached_path_display(&mut cache, &path);
    }

    let _ = cached_path_display(&mut cache, Path::new("src/overflow_a.rs"));
    let _ = cached_path_display(&mut cache, Path::new("src/overflow_b.rs"));

    let display = cached_path_display(&mut cache, &recent_hot);
    assert_eq!(display.as_ref(), recent_hot.to_str().unwrap_or_default());
    assert!(cache_contains_recent(&cache, &recent_hot));
    assert!(cache_contains_previous(&cache, &previous_hot));
}

#[test]
fn repo_path_name_uses_last_path_component_when_available() {
    assert_eq!(
        repo_path_name(Path::new("/tmp/repo-name")).as_ref(),
        "repo-name"
    );
}

#[test]
fn repo_path_name_falls_back_to_full_path_when_file_name_is_missing() {
    #[cfg(not(windows))]
    let path = Path::new("/");
    #[cfg(windows)]
    let path = Path::new(r"C:\");

    assert_eq!(repo_path_name(path).as_ref(), path_display_string(path));
}

#[test]
fn bench_counters_are_isolated_per_thread() {
    bench_reset();

    let ready = Arc::new(Barrier::new(2));
    let ready_thread = ready.clone();
    let handle = std::thread::spawn(move || {
        bench_reset();
        let mut cache = PathDisplayCache::default();
        let path = PathBuf::from("src/thread.rs");
        let _ = cached_path_display(&mut cache, &path);
        let _ = cached_path_display(&mut cache, &path);
        ready_thread.wait();
        assert_eq!(
            bench_snapshot(),
            PathDisplayBenchSnapshot {
                cache_hits: 1,
                cache_misses: 1,
                cache_clears: 0,
            }
        );
    });

    let mut cache = PathDisplayCache::default();
    let path = PathBuf::from("src/main.rs");
    let _ = cached_path_display(&mut cache, &path);
    ready.wait();
    assert_eq!(
        bench_snapshot(),
        PathDisplayBenchSnapshot {
            cache_hits: 0,
            cache_misses: 1,
            cache_clears: 0,
        }
    );

    handle.join().expect("join path_display test thread");
}
