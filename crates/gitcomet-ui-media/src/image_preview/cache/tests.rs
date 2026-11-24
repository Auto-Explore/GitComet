use super::*;

fn key(source: &Source, output: (u32, u32)) -> TileKey {
    TileKey {
        source: source.id,
        frame: 0,
        output,
        sampling: filter::Sampling::Nearest,
        column: 0,
        row: 0,
    }
}

fn originals(cache: &Cache) -> (usize, usize) {
    let inner = cache.originals.inner.lock().unwrap();
    (inner.lru.len(), inner.bytes)
}

#[test]
fn image_preview_lifecycle_finished_decode_survives_camera_cancellation() {
    let mut source = super::super::tests::source(64, 64);
    let (done, decoded) = std::sync::mpsc::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    Arc::get_mut(&mut source).unwrap().decode_gate = Some(Arc::new(source::DecodeGate {
        done,
        resume: Mutex::new(paused),
    }));
    let cache = Arc::new(Cache::default());
    cache.show(&[], &[source.id]);
    let generation = Arc::new(AtomicU64::new(0));
    let key = key(&source, (64, 64));
    let worker = {
        let source = source.clone();
        let cache = cache.clone();
        let generation = generation.clone();
        std::thread::spawn(move || cache.render(&source, &[key], &generation, 0).unwrap())
    };
    decoded
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    generation.store(1, Ordering::Relaxed);
    resume.send(()).unwrap();
    worker.join().unwrap();
    assert_eq!(
        originals(&cache),
        (1, 64 * 64 * 4),
        "a completed original remains useful after panning or zooming"
    );
    assert!(
        cache.inner.lock().unwrap().tiles.is_empty(),
        "obsolete tiles must still be canceled"
    );
    // The decoder gate cannot run twice. This request must reuse the
    // original from the canceled request and produce its current tile.
    cache.render(&source, &[key], &generation, 1).unwrap();
    assert!(cache.peek(&key).is_some());
}

#[test]
fn unfiltered_svg_reuses_its_parsed_tree_across_zoom_levels() {
    let source = Source::new(gpui::ImageFormat::Svg,
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10"/></svg>"#,
            1024 * 1024).unwrap();
    let cache = Cache::default();
    cache.show(&[], &[source.id]);
    let generation = AtomicU64::new(0);
    let key = key(&source, (20, 10));
    let original = cache
        .original(&source, &key, &generation, 0)
        .unwrap()
        .unwrap();
    assert!(matches!(*original, source::Decoded::Vector(..)));
    let zoomed = TileKey {
        output: (1280, 640),
        ..key
    };
    let zoomed = cache
        .original(&source, &zoomed, &generation, 0)
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&original, &zoomed));
    assert_eq!(originals(&cache).0, 1);
}

#[test]
fn filtered_svg_rasters_are_shared_accounted_and_bounded() {
    let source = Source::new(
        gpui::ImageFormat::Svg,
        super::super::tests::OFFSET_SVG.as_bytes(),
        1024 * 1024,
    )
    .unwrap();
    let cache = Cache::default();
    cache.show(&[], &[source.id]);
    let generation = AtomicU64::new(0);
    let key = TileKey {
        column: 2,
        ..key(&source, (1100, 300))
    };
    cache.render(&source, &[key], &generation, 0).unwrap();
    let tile = cache.peek(&key).unwrap();
    assert_eq!(
        &tile.as_bytes(0).unwrap()[80 * 76 * 4..80 * 76 * 4 + 4],
        &[16, 32, 239, 255]
    );
    let original = cache
        .original(&source, &key, &generation, 0)
        .unwrap()
        .unwrap();
    let other = TileKey { column: 1, ..key };
    cache.render(&source, &[other], &generation, 0).unwrap();
    assert!(Arc::ptr_eq(
        &original,
        &cache
            .original(&source, &other, &generation, 0)
            .unwrap()
            .unwrap()
    ));
    assert_eq!(originals(&cache), (1, 1100 * 300 * 4));
    let zoomed = TileKey {
        output: (2200, 600),
        ..key
    };
    cache.render(&source, &[zoomed], &generation, 0).unwrap();
    assert_eq!(originals(&cache).1, (1100 * 300 + 2200 * 600) * 4);
    let oversized = TileKey {
        output: (70400, 19200),
        ..key
    };
    assert!(
        cache
            .render(&source, &[oversized], &generation, 0)
            .unwrap_err()
            .contains("limit")
    );
    assert!(
        cache.peek(&oversized).is_none(),
        "keep the bounded preview instead of caching incorrect tiles"
    );
    assert_eq!(originals(&cache).0, 2);
}

#[test]
fn queued_obsolete_work_never_decodes_or_populates_cache() {
    let source = super::super::tests::source(64, 64);
    let cache = Arc::new(Cache::default());
    cache.show(&[], &[source.id]);
    let generation = Arc::new(AtomicU64::new(0));
    let first = cache.acquire();
    let second = cache.acquire();
    let (started, ready) = std::sync::mpsc::sync_channel(0);
    let worker = {
        let cache = cache.clone();
        let generation = generation.clone();
        std::thread::spawn(move || {
            started.send(()).unwrap();
            let key = key(&source, (128, 128));
            cache.render(&source, &[key], &generation, 0).unwrap();
        })
    };
    ready.recv().unwrap();
    assert_eq!(*cache.running.lock().unwrap(), 2);
    generation.store(1, Ordering::Relaxed);
    drop(first);
    drop(second);
    worker.join().unwrap();
    assert_eq!(originals(&cache).0, 0);
    assert!(cache.inner.lock().unwrap().tiles.is_empty());
}

#[gpui::test]
fn closing_releases_retained_pixels_and_rejects_late_tiles(cx: &mut gpui::TestAppContext) {
    let source = super::super::tests::source(64, 64);
    let cache = Cache::default();
    cache.show(&[], &[source.id]);
    let key = key(&source, (128, 128));
    cache
        .render(&source, &[key], &AtomicU64::new(0), 0)
        .unwrap();
    cache.show(&[source.id], &[]);
    cx.update(|cx| cache.close(cx));
    cache
        .render(&source, &[key], &AtomicU64::new(0), 0)
        .unwrap();
    let inner = cache.inner.lock().unwrap();
    assert!(inner.closed && inner.tiles.is_empty() && inner.dropped.is_empty());
    assert_eq!(originals(&cache), (0, 0));
}

#[test]
fn tiles_reuse_originals_and_lru_eviction_is_bounded() {
    let source = super::super::tests::source(64, 64);
    let cache = Cache::default();
    cache.show(&[], &[source.id]);
    let generation = AtomicU64::new(1);
    let key = key(&source, (64, 64));
    cache.render(&source, &[key], &generation, 0).unwrap();
    assert!(cache.peek(&key).is_none());
    cache.render(&source, &[key], &generation, 1).unwrap();
    let first = cache.peek(&key).unwrap();
    cache.render(&source, &[key], &generation, 1).unwrap();
    assert!(Arc::ptr_eq(&first, &cache.peek(&key).unwrap()));
    assert_eq!(originals(&cache).0, 1);
    for w in 65..130 {
        cache
            .render(
                &source,
                &[TileKey {
                    output: (w, 64),
                    ..key
                }],
                &generation,
                1,
            )
            .unwrap();
    }
    let inner = cache.inner.lock().unwrap();
    assert_eq!(inner.tiles.len(), TILE_COUNT);
    assert_eq!(inner.dropped.len(), 2);
    assert!(!inner.tiles.contains(&key));
    assert_eq!(originals(&cache).0, 1);
}

/// An original over the budget used to be decoded for one request and
/// dropped, so every pan or zoom decoded the whole image again (420 ms per
/// tile for an 8200² PNG). It now stays, alone.
#[test]
fn oversized_original_is_kept_alone_for_the_next_pan() {
    let small = super::super::tests::source(32, 32);
    let large = super::super::tests::source(64, 64);
    let cache = Cache::new(Arc::new(Originals {
        budget: 64 * 64 * 4 - 1,
        ..Default::default()
    }));
    cache.show(&[], &[small.id, large.id]);
    let generation = AtomicU64::new(0);
    cache
        .render(&small, &[key(&small, (32, 32))], &generation, 0)
        .unwrap();
    assert_eq!(originals(&cache), (1, 32 * 32 * 4));
    cache
        .render(&large, &[key(&large, (64, 64))], &generation, 0)
        .unwrap();
    assert_eq!(originals(&cache), (1, 64 * 64 * 4));
    let pan = TileKey {
        output: (128, 128),
        column: 0,
        ..key(&large, (64, 64))
    };
    assert!(
        cache
            .original(&large, &pan, &AtomicU64::new(1), 0)
            .unwrap()
            .is_some(),
        "a cached original answers even a superseded request"
    );
}

/// The decode lock was taken before the cache lookup, so a pane whose
/// original was already decoded waited behind another pane's decode.
#[test]
fn cached_original_does_not_wait_for_another_decode() {
    let mut slow = super::super::tests::source(64, 64);
    let fast = super::super::tests::source(32, 32);
    let (done, decoded) = std::sync::mpsc::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    Arc::get_mut(&mut slow).unwrap().decode_gate = Some(Arc::new(source::DecodeGate {
        done,
        resume: Mutex::new(paused),
    }));
    let cache = Arc::new(Cache::default());
    cache.show(&[], &[slow.id, fast.id]);
    cache
        .render(&fast, &[key(&fast, (32, 32))], &AtomicU64::new(0), 0)
        .unwrap();
    let worker = {
        let cache = cache.clone();
        let key = key(&slow, (64, 64));
        std::thread::spawn(move || cache.render(&slow, &[key], &AtomicU64::new(0), 0))
    };
    decoded
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    let (finished, pan) = std::sync::mpsc::channel();
    {
        let cache = cache.clone();
        let key = key(&fast, (64, 64));
        std::thread::spawn(move || {
            cache.render(&fast, &[key], &AtomicU64::new(0), 0).unwrap();
            finished.send(()).unwrap();
        });
    }
    let waited = pan.recv_timeout(std::time::Duration::from_secs(2)).is_err();
    resume.send(()).unwrap();
    worker.join().unwrap().unwrap();
    assert!(
        !waited,
        "a cached original waited behind another image's decode"
    );
}

/// Tiles and decoded pixels stayed resident for the session after the
/// pane moved on to a text diff, including a decode finishing late.
#[gpui::test]
fn leaving_an_image_releases_its_tiles_and_pixels(cx: &mut gpui::TestAppContext) {
    let image = super::super::tests::source(64, 64);
    let group = cx.new(Group::new);
    let cache = group.update(cx, |group, cx| {
        group.set_inputs(
            "image".into(),
            [
                Input {
                    source: Some(image.clone()),
                    ..Default::default()
                },
                Input::default(),
                Input::default(),
            ],
            false,
            cx,
        );
        group.cache.clone()
    });
    let key = key(&image, (64, 64));
    cache.render(&image, &[key], &AtomicU64::new(0), 0).unwrap();
    assert!(cache.peek(&key).is_some());
    assert_eq!(originals(&cache).0, 1);
    group.update(cx, |group, cx| {
        group.set_inputs("text diff".into(), Default::default(), false, cx)
    });
    cx.run_until_parked();
    assert!(
        cache.peek(&key).is_none(),
        "tiles of a closed image stay resident"
    );
    assert!(cache.inner.lock().unwrap().dropped.is_empty());
    assert_eq!(originals(&cache), (0, 0));
    let late = TileKey {
        output: (128, 128),
        ..key
    };
    cache
        .render(&image, &[late], &AtomicU64::new(0), 0)
        .unwrap();
    assert_eq!(
        originals(&cache),
        (0, 0),
        "a late decode cached pixels nobody shows"
    );
}
