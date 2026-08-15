//! 压测装置的共享零件：计数分配器、admin 端点、以及把真 `gw-proxy` 装起来
//! 所需的最小 in-memory 端口实现。
//!
//! 这里的每一行都只服务于「量」，不进产品构建图。

pub mod admin;
pub mod counting_alloc;
/// 只服务于 `gateway` 被测端 —— 它是唯一需要把真 `gw-proxy` 装起来的进程。
/// 见 `Cargo.toml` 里 `gateway` feature 的说明。
#[cfg(feature = "gateway")]
pub mod stubs;

/// 压测 API key 明文。`loadgen`（每个被测端都打）与 `stubs::StaticDirectory`
/// （只有 `gateway` 装得起来）共用同一个值，所以它住在 crate 根而不是 `stubs`
/// —— 否则 `--no-default-features` 下 `loadgen` 就编不出来了。
pub const PERF_API_KEY: &str = "cpa-perfbaselinekey";

/// 三种负载形态的共享常量，`mock-upstream` 与 `loadgen` 都从这里读，
/// 免得两边各写一份魔数然后悄悄漂移。
pub mod scenario {
    /// a) 非流式小 body。
    pub const SMALL_REQ_BYTES: usize = 1024;
    /// a) 的响应大小。
    pub const SMALL_RESP_BYTES: usize = 2048;
    /// b) 非流式大 body。
    pub const LARGE_REQ_BYTES: usize = 256 * 1024;
    /// b) 的响应大小。
    pub const LARGE_RESP_BYTES: usize = 1024 * 1024;
    /// c) SSE chunk 数。
    pub const SSE_CHUNKS: usize = 500;
    /// c) 每 chunk 字节数。
    pub const SSE_CHUNK_BYTES: usize = 1024;
    /// c) chunk 间隔（微秒）。
    pub const SSE_INTERVAL_US: u64 = 1000;
}
