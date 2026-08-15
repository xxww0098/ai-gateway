//! 计数 `#[global_allocator]`。
//!
//! 为什么不用 dhat：dhat 要往被测进程里塞一层自己的 profiler，且它的输出是
//! 「整个进程」的堆快照 —— 我要的是**每请求**的增量，即
//! `(计数差) / (请求数)`。一个可清零的原子计数器把这件事做得更直接，
//! 也不用往 workspace 里加依赖（CONTRACT §3 禁止 audit-perf 动 Cargo.toml）。
//!
//! 计数默认**关闭**，由 `PERF_COUNT_ALLOC=1` 打开。原因：延迟档和分配档要
//! 分开跑。开着计数测延迟，两条 12 核争用的原子加会污染 p99；关掉之后
//! 每次分配只多一次 relaxed load，量不到。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static DEALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static DEALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static REALLOC_COUNT: AtomicU64 = AtomicU64::new(0);

/// 包了 `System` 的计数分配器。
pub struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ENABLED.load(Ordering::Relaxed) {
            DEALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            DEALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    /// realloc 算**一次**分配，字节数只记增量。把它算成 alloc+dealloc 会让
    /// `Vec::push` 驱动的增长路径虚高一倍，那不是「拷贝了几次」的真相。
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            REALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(new_size.saturating_sub(layout.size()) as u64, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// 一次计数快照。
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct Snapshot {
    pub alloc_count: u64,
    pub alloc_bytes: u64,
    pub dealloc_count: u64,
    pub dealloc_bytes: u64,
    pub realloc_count: u64,
    pub enabled: bool,
}

/// 读取当前计数。
#[must_use]
pub fn snapshot() -> Snapshot {
    Snapshot {
        alloc_count: ALLOC_COUNT.load(Ordering::Relaxed),
        alloc_bytes: ALLOC_BYTES.load(Ordering::Relaxed),
        dealloc_count: DEALLOC_COUNT.load(Ordering::Relaxed),
        dealloc_bytes: DEALLOC_BYTES.load(Ordering::Relaxed),
        realloc_count: REALLOC_COUNT.load(Ordering::Relaxed),
        enabled: ENABLED.load(Ordering::Relaxed),
    }
}

/// 清零计数（`/admin/reset` 用）。
pub fn reset() {
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    ALLOC_BYTES.store(0, Ordering::Relaxed);
    DEALLOC_COUNT.store(0, Ordering::Relaxed);
    DEALLOC_BYTES.store(0, Ordering::Relaxed);
    REALLOC_COUNT.store(0, Ordering::Relaxed);
}

/// 按 `PERF_COUNT_ALLOC` 决定是否开计数。进程启动时调一次。
pub fn init_from_env() {
    let on = std::env::var("PERF_COUNT_ALLOC").is_ok_and(|v| v == "1" || v == "true");
    ENABLED.store(on, Ordering::Relaxed);
}

/// 计数是否已开。
#[must_use]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}
