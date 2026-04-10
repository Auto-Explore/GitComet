use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, Once};

static INSTALL: Once = Once::new();
static MEASUREMENT_LOCK: Mutex<()> = Mutex::new(());
static MEASUREMENT_ENABLED: AtomicBool = AtomicBool::new(false);
static COUNTERS: AllocCounters = AllocCounters::new();

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
unsafe extern "C" {
    fn malloc_usable_size(ptr: *mut c_void) -> usize;
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn malloc_size(ptr: *const c_void) -> usize;
}

#[cfg(windows)]
unsafe extern "C" {
    fn _msize(ptr: *mut c_void) -> usize;
}

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

pub fn install_tracking_allocator() {
    INSTALL.call_once(|| unsafe {
        tree_sitter::set_allocator(
            Some(tree_sitter_malloc),
            Some(tree_sitter_calloc),
            Some(tree_sitter_realloc),
            Some(tree_sitter_free),
        );
    });
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
    let ptr = unsafe { malloc(size) };
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
    let ptr = unsafe { calloc(count, size) };
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
    let result = unsafe { realloc(ptr, size) };
    if size > 0 && result.is_null() {
        abort_alloc("reallocate", size);
    }
    if measured {
        if result.is_null() {
            if size == 0 && !ptr.is_null() {
                COUNTERS.record_realloc(old_bytes, 0);
            }
        } else {
            COUNTERS.record_realloc(old_bytes, measured_bytes(result, size));
        }
    }
    result
}

unsafe extern "C" fn tree_sitter_free(ptr: *mut c_void) {
    if measurement_enabled() && !ptr.is_null() {
        COUNTERS.record_dealloc(unsafe { usable_size(ptr) });
    }
    unsafe { free(ptr) };
}

fn abort_alloc(kind: &str, size: usize) -> ! {
    eprintln!("tree-sitter failed to {kind} {size} bytes");
    std::process::abort();
}

fn measured_bytes(ptr: *mut c_void, fallback: usize) -> usize {
    let usable = unsafe { usable_size(ptr) };
    usable.max(fallback)
}

unsafe fn usable_size(ptr: *mut c_void) -> usize {
    if ptr.is_null() {
        return 0;
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
    {
        unsafe { malloc_usable_size(ptr) }
    }

    #[cfg(target_os = "macos")]
    {
        unsafe { malloc_size(ptr.cast_const()) }
    }

    #[cfg(windows)]
    {
        unsafe { _msize(ptr) as usize }
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "macos",
        windows
    )))]
    {
        0
    }
}
