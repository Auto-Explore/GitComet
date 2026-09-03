//! Which allocator tree-sitter's C uses, and optional accounting on top of it.
//!
//! `#[global_allocator]` replaces Rust's `GlobalAlloc` and nothing else. tree-sitter
//! is C: its `ts_malloc` calls plain `malloc`, which the linker resolves to libc
//! unless mimalloc is built to interpose the symbol -- and the `mimalloc` crate
//! does not do that by default. Left alone, a GitComet process runs two
//! allocators, with every subtree, parse stack and lexer buffer on libc's while
//! the Rust side is on mimalloc.
//!
//! `ts_set_allocator` is the supported way to fix that. The hooks installed before
//! `main` always route to `mi_*` through the same wrappers; benchmarks atomically
//! enable the wrappers' counters only while measuring. The function-pointer globals
//! are therefore written once, before parser threads exist, rather than being
//! swapped underneath concurrent C reads.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, Once};

use libmimalloc_sys::{mi_calloc, mi_free, mi_malloc, mi_realloc, mi_usable_size};

static INSTALL: Once = Once::new();
static INSTALLED: AtomicBool = AtomicBool::new(false);
static MEASUREMENT_LOCK: Mutex<()> = Mutex::new(());
static MEASUREMENT_ENABLED: AtomicBool = AtomicBool::new(false);
static COUNTERS: AllocCounters = AllocCounters::new();
#[cfg(test)]
static HOOK_INSTALL_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AllocMetrics {
    pub alloc_ops: u64,
    pub dealloc_ops: u64,
    pub realloc_ops: u64,
    pub alloc_bytes: u64,
    pub dealloc_bytes: u64,
    pub realloc_bytes_delta: i64,
    pub net_alloc_bytes: i64,
}

impl AllocMetrics {
    pub fn is_zero(self) -> bool {
        self.alloc_ops == 0
            && self.dealloc_ops == 0
            && self.realloc_ops == 0
            && self.alloc_bytes == 0
            && self.dealloc_bytes == 0
            && self.realloc_bytes_delta == 0
            && self.net_alloc_bytes == 0
    }

    pub fn delta_since(self, earlier: Self) -> Self {
        let alloc_bytes = self.alloc_bytes.saturating_sub(earlier.alloc_bytes);
        let dealloc_bytes = self.dealloc_bytes.saturating_sub(earlier.dealloc_bytes);
        Self {
            alloc_ops: self.alloc_ops.saturating_sub(earlier.alloc_ops),
            dealloc_ops: self.dealloc_ops.saturating_sub(earlier.dealloc_ops),
            realloc_ops: self.realloc_ops.saturating_sub(earlier.realloc_ops),
            alloc_bytes,
            dealloc_bytes,
            realloc_bytes_delta: clamp_i128_to_i64(
                i128::from(self.realloc_bytes_delta) - i128::from(earlier.realloc_bytes_delta),
            ),
            net_alloc_bytes: clamp_i128_to_i64(i128::from(alloc_bytes) - i128::from(dealloc_bytes)),
        }
    }
}

/// Point tree-sitter at mimalloc before `main`, in every binary that links this.
///
/// The ordering rule on [`install_mimalloc_allocator`] says the first switch has
/// to precede tree-sitter's first allocation. A caller *can* satisfy that by
/// installing from whatever lazily builds its parsers -- `gitcomet-ui-gpui` does
/// -- but only for routes it controls. Test binaries are where that breaks down:
/// a test that builds a `Query` straight off a `LANGUAGE` allocates through
/// `ts_malloc` without passing any such funnel, and libtest runs it in parallel
/// with tests that do, so a lazy install lands mid-flight and those blocks get
/// freed by `mi_free`. With `MI_DEBUG` off that is not survivable: mimalloc's
/// `mi_validate_ptr_page` compiles to `_mi_ptr_page(p)` and checks nothing, so a
/// foreign pointer yields a garbage page and corrupts the heap in silence.
///
/// Running before `main` deletes the question rather than answering it -- there
/// is no "before" left for an allocation to happen in. It also means the whole
/// of a crate's test suite exercises the same allocator pairing production uses,
/// so a mimalloc bump is covered by every test that parses anything instead of
/// only by the ones in this crate.
///
/// Nothing here can allocate or fail: it stores four function pointers into
/// tree-sitter's globals through an uncontended [`Once`] and never calls them,
/// so mimalloc is not initialised at this point either.
#[ctor::ctor(unsafe)]
fn install_before_main() {
    install_mimalloc_allocator();
}

/// Point tree-sitter's C at mimalloc.
///
/// The installed wrappers contain the optional accounting, but its single atomic
/// branch is inactive outside [`measure_allocations`]. This function and
/// [`install_tracking_allocator`] share one [`Once`], so neither can rewrite
/// tree-sitter's non-atomic function-pointer globals after startup.
///
/// # The one ordering rule
///
/// The installer switches tree-sitter from libc to mimalloc, and
/// a block allocated before that switch would later be freed through `mi_free`
/// -- a foreign pointer, which corrupts the heap.
///
/// That rule is not left to callers to remember. [`install_before_main`] runs it
/// from a `#[ctor]`, so in any binary linking this crate the switch happens
/// while the process is still single-threaded and has executed no user code at
/// all. `gitcomet-ui-gpui` additionally calls this from the lazy initialisers
/// in front of its parsers -- the `TS_PARSER`/`TS_CURSOR` thread-locals and
/// `init_highlight_spec` -- which is a backstop for the linker dropping an
/// `.init_array` entry, not a second mechanism to keep in sync.
///
/// Do not replace either with a call in a `main`: that covers one entry point
/// and silently leaves every other binary and every test on libc.
pub fn install_mimalloc_allocator() {
    INSTALL.call_once(|| {
        unsafe {
            tree_sitter::set_allocator(Some(tree_sitter::Allocator {
                malloc: tree_sitter_malloc,
                calloc: tree_sitter_calloc,
                realloc: tree_sitter_realloc,
                free: tree_sitter_free,
            }));
        }
        #[cfg(test)]
        HOOK_INSTALL_COUNT.fetch_add(1, Ordering::SeqCst);
        INSTALLED.store(true, Ordering::Release);
    });
}

/// Ensure tree-sitter uses the wrappers that can count what it allocates.
///
/// Kept as the benchmark-facing entry point, but it deliberately installs no
/// different function pointers. [`measure_allocations`] toggles accounting with
/// an atomic flag, so calling this after worker threads start is idempotent and
/// cannot race a parser reading tree-sitter's allocator globals.
pub fn install_tracking_allocator() {
    install_mimalloc_allocator();
}

/// Whether tree-sitter's allocator has been pointed away from libc yet.
///
/// Exists so a caller that depends on the switch having already happened can
/// assert it instead of assuming it -- see the `#[ctor]` in `gitcomet-ui-gpui`,
/// whose whole job is to make this true before the test harness starts.
pub fn is_installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

pub fn measure_allocations<T>(f: impl FnOnce() -> T) -> (T, AllocMetrics) {
    let _guard = MeasurementGuard::new();
    let before = current_metrics();
    let value = f();
    std::hint::black_box(&value);
    let after = current_metrics();
    (value, after.delta_since(before))
}

#[derive(Debug)]
struct AllocCounters {
    alloc_ops: AtomicU64,
    dealloc_ops: AtomicU64,
    realloc_ops: AtomicU64,
    alloc_bytes: AtomicU64,
    dealloc_bytes: AtomicU64,
    realloc_bytes_delta: AtomicI64,
}

impl AllocCounters {
    const fn new() -> Self {
        Self {
            alloc_ops: AtomicU64::new(0),
            dealloc_ops: AtomicU64::new(0),
            realloc_ops: AtomicU64::new(0),
            alloc_bytes: AtomicU64::new(0),
            dealloc_bytes: AtomicU64::new(0),
            realloc_bytes_delta: AtomicI64::new(0),
        }
    }

    fn snapshot(&self) -> AllocMetrics {
        let alloc_bytes = self.alloc_bytes.load(Ordering::SeqCst);
        let dealloc_bytes = self.dealloc_bytes.load(Ordering::SeqCst);
        AllocMetrics {
            alloc_ops: self.alloc_ops.load(Ordering::SeqCst),
            dealloc_ops: self.dealloc_ops.load(Ordering::SeqCst),
            realloc_ops: self.realloc_ops.load(Ordering::SeqCst),
            alloc_bytes,
            dealloc_bytes,
            realloc_bytes_delta: self.realloc_bytes_delta.load(Ordering::SeqCst),
            net_alloc_bytes: clamp_i128_to_i64(i128::from(alloc_bytes) - i128::from(dealloc_bytes)),
        }
    }

    fn record_alloc(&self, bytes: usize) {
        self.alloc_ops.fetch_add(1, Ordering::SeqCst);
        self.alloc_bytes
            .fetch_add(bytes.min(u64::MAX as usize) as u64, Ordering::SeqCst);
    }

    fn record_dealloc(&self, bytes: usize) {
        self.dealloc_ops.fetch_add(1, Ordering::SeqCst);
        self.dealloc_bytes
            .fetch_add(bytes.min(u64::MAX as usize) as u64, Ordering::SeqCst);
    }

    fn record_realloc(&self, old_bytes: usize, new_bytes: usize) {
        self.realloc_ops.fetch_add(1, Ordering::SeqCst);
        match new_bytes.cmp(&old_bytes) {
            std::cmp::Ordering::Greater => {
                self.alloc_bytes.fetch_add(
                    (new_bytes - old_bytes).min(u64::MAX as usize) as u64,
                    Ordering::SeqCst,
                );
            }
            std::cmp::Ordering::Less => {
                self.dealloc_bytes.fetch_add(
                    (old_bytes - new_bytes).min(u64::MAX as usize) as u64,
                    Ordering::SeqCst,
                );
            }
            std::cmp::Ordering::Equal => {}
        }
        let delta = i128::try_from(new_bytes)
            .unwrap_or(i128::MAX)
            .saturating_sub(i128::try_from(old_bytes).unwrap_or(i128::MAX));
        self.realloc_bytes_delta
            .fetch_add(clamp_i128_to_i64(delta), Ordering::SeqCst);
    }
}

fn current_metrics() -> AllocMetrics {
    COUNTERS.snapshot()
}

fn measurement_enabled() -> bool {
    MEASUREMENT_ENABLED.load(Ordering::SeqCst)
}

fn clamp_i128_to_i64(value: i128) -> i64 {
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

struct MeasurementGuard<'a> {
    _lock: std::sync::MutexGuard<'a, ()>,
}

impl<'a> MeasurementGuard<'a> {
    fn new() -> Self {
        let lock = MEASUREMENT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        MEASUREMENT_ENABLED.store(true, Ordering::SeqCst);
        Self { _lock: lock }
    }
}

impl Drop for MeasurementGuard<'_> {
    fn drop(&mut self) {
        MEASUREMENT_ENABLED.store(false, Ordering::SeqCst);
    }
}

unsafe extern "C" fn tree_sitter_malloc(size: usize) -> *mut c_void {
    let ptr = unsafe { mi_malloc(size) };
    if size > 0 && ptr.is_null() {
        abort_alloc("allocate", size);
    }
    if measurement_enabled() && !ptr.is_null() {
        COUNTERS.record_alloc(measured_bytes(ptr, size));
    }
    ptr
}

unsafe extern "C" fn tree_sitter_calloc(count: usize, size: usize) -> *mut c_void {
    let requested = count.checked_mul(size).unwrap_or_else(|| {
        eprintln!("tree-sitter failed to allocate {count} * {size} bytes");
        std::process::abort();
    });
    let ptr = unsafe { mi_calloc(count, size) };
    if requested > 0 && ptr.is_null() {
        abort_alloc("allocate", requested);
    }
    if measurement_enabled() && !ptr.is_null() {
        COUNTERS.record_alloc(measured_bytes(ptr, requested));
    }
    ptr
}

unsafe extern "C" fn tree_sitter_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    let measured = measurement_enabled();
    let old_bytes = if measured && !ptr.is_null() {
        unsafe { usable_size(ptr) }
    } else {
        0
    };
    let result = unsafe { mi_realloc(ptr, size) };
    // mimalloc reallocates to a *zero-sized block* rather than freeing and
    // returning NULL, so unlike glibc a null result always means failure and
    // `ptr` has not been freed (`c_src/mimalloc/v3/src/alloc.c`, the comment on
    // `_mi_theap_realloc_zero`). That makes the abort below the only null case
    // to handle, and makes charging `size == 0` as a small live block correct:
    // the block really is still allocated.
    if result.is_null() {
        abort_alloc("reallocate", size);
    }
    if measured {
        COUNTERS.record_realloc(old_bytes, measured_bytes(result, size));
    }
    result
}

unsafe extern "C" fn tree_sitter_free(ptr: *mut c_void) {
    if measurement_enabled() && !ptr.is_null() {
        COUNTERS.record_dealloc(unsafe { usable_size(ptr) });
    }
    unsafe { mi_free(ptr) };
}

fn abort_alloc(kind: &str, size: usize) -> ! {
    eprintln!("tree-sitter failed to {kind} {size} bytes");
    std::process::abort();
}

fn measured_bytes(ptr: *mut c_void, fallback: usize) -> usize {
    let usable = unsafe { usable_size(ptr) };
    usable.max(fallback)
}

/// The block size mimalloc actually handed out, which is what the counters should
/// charge.
///
/// One call for every platform: the libc equivalents (`malloc_usable_size`,
/// `malloc_size`, `_msize`) would be the wrong question now, and would report
/// nothing useful for a pointer mimalloc owns.
///
/// `mi_usable_size` is only defined for a pointer mimalloc owns -- it reads page
/// metadata derived from the pointer's segment address, so a foreign block gives
/// garbage rather than a wrong-but-safe number the way glibc's does. Every
/// pointer that reaches here came from `mi_malloc`/`mi_calloc`/`mi_realloc`
/// above, which holds because the allocator switch happens before tree-sitter's
/// first allocation; see [`install_mimalloc_allocator`].
unsafe fn usable_size(ptr: *mut c_void) -> usize {
    if ptr.is_null() {
        return 0;
    }
    unsafe { mi_usable_size(ptr) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_json() -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_json::LANGUAGE.into())
            .expect("json grammar should load");
        parser
            .parse(r#"{"a":[1,2,{"b":"c"}],"d":null}"#, None)
            .expect("json should parse")
    }

    /// Runtime installer calls must never rewrite tree-sitter's global hook
    /// pointers while parsers on sibling threads are reading them.
    #[test]
    fn concurrent_installer_calls_do_not_swap_hooks_while_parsing() {
        assert!(
            is_installed(),
            "the constructor should install before tests"
        );
        let start = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads = (0..8)
            .map(|thread_ix| {
                let start = std::sync::Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    for iteration in 0..128 {
                        if thread_ix % 2 == 0 {
                            if iteration % 2 == 0 {
                                install_mimalloc_allocator();
                            } else {
                                install_tracking_allocator();
                            }
                        } else {
                            let tree = parse_json();
                            assert_eq!(tree.root_node().kind(), "document");
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().expect("allocator stress thread");
        }
        assert_eq!(
            HOOK_INSTALL_COUNT.load(Ordering::SeqCst),
            1,
            "all runtime installer calls must resolve through the constructor's one hook write"
        );
    }

    /// `measure_allocations` should see tree-sitter's C allocations, which it can
    /// only do if the counters sit on the allocator tree-sitter actually calls.
    #[test]
    fn measurement_sees_tree_sitter_allocations() {
        install_tracking_allocator();
        install_mimalloc_allocator();
        let (tree, metrics) = measure_allocations(parse_json);
        assert_eq!(tree.root_node().kind(), "document");
        assert!(
            metrics.alloc_ops > 0 && metrics.alloc_bytes > 0,
            "parsing should allocate through the installed allocator, got {metrics:?}"
        );
    }
}
