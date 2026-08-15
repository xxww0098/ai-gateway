//! 自写 tokio 负载生成器（裸 TCP + 手写 HTTP/1.1）。
//!
//! # 为什么不用 oha / wrk / bombardier
//!
//! 1. 本机三个都没装，而装它们要么走网络、要么走包管理器，都不属于"一条命令
//!    能复现"的范围；
//! 2. 更重要的是：本任务要的 **SSE 首字节延迟与 chunk 间抖动**，上面三个都
//!    不给。wrk 要写 Lua、oha 只报整请求耗时。裸 TCP 能在每个 chunk 边界打
//!    时间戳，这是 c) 场景唯一诚实的量法。
//!
//! 连接全程 keep-alive 复用，所以量到的是稳态转发开销，不含 TCP 握手。
//!
//! # 用法
//!
//! ```text
//! loadgen --port 18080 --path '/v1/chat/completions?resp_bytes=2048' \
//!         --body-bytes 1024 --concurrency 16 --duration 10 --mode unary
//! loadgen --port 18080 --path '/v1/chat/completions?stream=1&chunks=500&chunk_bytes=1024&interval_us=1000' \
//!         --body-bytes 1024 --concurrency 8 --requests 400 --mode sse
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// 全进程单调的幂等 key 序号，预热与正式测量共用它，见 `worker` 里的注释。
static KEY_SEQ: AtomicU64 = AtomicU64::new(0);


// ---------------------------------------------------------------- 参数

struct Args {
    host: String,
    port: u16,
    path: String,
    body_bytes: usize,
    concurrency: usize,
    duration: Option<Duration>,
    requests: Option<usize>,
    warmup: Duration,
    stream: bool,
    idempotency: bool,
    label: String,
    out: Option<String>,
    /// 单请求超时。压测客户端**必须**有这个：一次卡住的读会让整轮永远挂住，
    /// 而挂住的样本本身是数据（`stalls` 字段），不是理由去阻塞整个基线。
    timeout: Duration,
}

fn parse_args() -> Args {
    let mut map: HashMap<String, String> = HashMap::new();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let key = argv[i].trim_start_matches("--").to_owned();
        let value = argv.get(i + 1).cloned().unwrap_or_default();
        map.insert(key, value);
        i += 2;
    }
    let get = |k: &str, d: &str| map.get(k).cloned().unwrap_or_else(|| d.to_owned());
    let mode = get("mode", "unary");
    Args {
        host: get("host", "127.0.0.1"),
        port: get("port", "18080").parse().expect("--port"),
        path: get("path", "/v1/chat/completions"),
        body_bytes: get("body-bytes", "1024").parse().expect("--body-bytes"),
        concurrency: get("concurrency", "16").parse().expect("--concurrency"),
        duration: map
            .get("duration")
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs),
        requests: map.get("requests").and_then(|v| v.parse::<usize>().ok()),
        warmup: Duration::from_millis(
            map.get("warmup-ms")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1500),
        ),
        stream: mode == "sse",
        idempotency: get("idempotency", "0") == "1",
        label: get("label", "run"),
        out: map.get("out").cloned(),
        timeout: Duration::from_millis(
            map.get("timeout-ms")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(10_000),
        ),
    }
}

// ---------------------------------------------------------------- 请求体

/// 造一个 `body_bytes` 字节的合法 chat/completions 请求体。
///
/// `model` / `stream` / `max_tokens` 三个字段必须是真的：hold 层会 peek 它们，
/// 少一个就走到另一条分支上去了。
fn build_body(bytes: usize, stream: bool) -> Vec<u8> {
    let head = format!(
        r#"{{"model":"gpt-4o","stream":{stream},"max_tokens":512,"messages":[{{"role":"user","content":""#
    );
    let tail = r#""}]}"#;
    let pad = bytes.saturating_sub(head.len() + tail.len());
    let mut out = Vec::with_capacity(bytes.max(head.len() + tail.len()));
    out.extend_from_slice(head.as_bytes());
    out.extend(std::iter::repeat_n(b'x', pad));
    out.extend_from_slice(tail.as_bytes());
    out
}

// ---------------------------------------------------------------- 统计

#[derive(Default)]
struct Samples {
    /// 整请求耗时（纳秒）。
    total: Vec<u64>,
    /// 首字节（纳秒），仅 SSE 有意义（非流式两者相同）。
    ttfb: Vec<u64>,
    /// chunk 间隔（纳秒），跨请求汇总。
    gaps: Vec<u64>,
    /// 每请求解出的 chunk 数，用于校验流没被截断。
    chunks: Vec<u64>,
}

impl Samples {
    fn merge(&mut self, other: Samples) {
        self.total.extend(other.total);
        self.ttfb.extend(other.ttfb);
        self.gaps.extend(other.gaps);
        self.chunks.extend(other.chunks);
    }
}

fn pct(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = p / 100.0 * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f64;
    (sorted[lo] as f64 * (1.0 - frac) + sorted[hi] as f64 * frac) / 1000.0 // → µs
}

fn mean_us(v: &[u64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().sum::<u64>() as f64 / v.len() as f64 / 1000.0
}

fn stddev_us(v: &[u64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = v.iter().sum::<u64>() as f64 / v.len() as f64;
    let var = v.iter().map(|&x| (x as f64 - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64;
    var.sqrt() / 1000.0
}

fn summarize(name: &str, v: &mut [u64]) -> serde_json::Value {
    v.sort_unstable();
    serde_json::json!({
        "metric": name,
        "n": v.len(),
        "mean_us": mean_us(v),
        "p50_us": pct(v, 50.0),
        "p90_us": pct(v, 90.0),
        "p95_us": pct(v, 95.0),
        "p99_us": pct(v, 99.0),
        "max_us": v.last().copied().unwrap_or(0) as f64 / 1000.0,
        "stddev_us": stddev_us(v),
    })
}

// ---------------------------------------------------------------- HTTP/1.1

struct Response {
    status: u16,
    ttfb: Duration,
    /// 每个 body chunk 读完的时刻（相对请求发出）。
    chunk_at: Vec<Duration>,
    body_len: usize,
}

/// 发一个请求并把响应读到底。`scratch` 跨请求复用，避免 loadgen 自己成为瓶颈。
async fn one_request(
    reader: &mut BufReader<TcpStream>,
    request: &[u8],
    line: &mut Vec<u8>,
    scratch: &mut Vec<u8>,
) -> std::io::Result<Response> {
    let started = Instant::now();
    reader.get_mut().write_all(request).await?;
    reader.get_mut().flush().await?;

    // 第一个可读字节 = TTFB。
    let ttfb = {
        let peeked = reader.fill_buf().await?;
        if peeked.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "upstream closed before responding",
            ));
        }
        started.elapsed()
    };

    // --- 状态行 ---
    line.clear();
    reader.read_until(b'\n', line).await?;
    let status = std::str::from_utf8(line)
        .ok()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .unwrap_or(0);

    // --- 头部 ---
    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    loop {
        line.clear();
        let n = reader.read_until(b'\n', line).await?;
        if n == 0 {
            break;
        }
        let trimmed = line.as_slice();
        if trimmed == b"\r\n" || trimmed == b"\n" {
            break;
        }
        let lower = String::from_utf8_lossy(trimmed).to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().ok();
        } else if let Some(v) = lower.strip_prefix("transfer-encoding:")
            && v.contains("chunked")
        {
            chunked = true;
        }
    }

    // --- body ---
    let mut chunk_at = Vec::new();
    let mut body_len = 0usize;
    if chunked {
        loop {
            line.clear();
            reader.read_until(b'\n', line).await?;
            let size_hex = String::from_utf8_lossy(line);
            let size = usize::from_str_radix(
                size_hex.trim().split(';').next().unwrap_or("0").trim(),
                16,
            )
            .unwrap_or(0);
            if size == 0 {
                // 结尾 CRLF（可能还有 trailer，本压测的上游不发）。
                line.clear();
                reader.read_until(b'\n', line).await?;
                break;
            }
            scratch.resize(size + 2, 0);
            reader.read_exact(scratch).await?;
            chunk_at.push(started.elapsed());
            body_len += size;
        }
    } else if let Some(len) = content_length {
        scratch.resize(len, 0);
        reader.read_exact(scratch).await?;
        chunk_at.push(started.elapsed());
        body_len = len;
    }

    Ok(Response {
        status,
        ttfb,
        chunk_at,
        body_len,
    })
}

// ---------------------------------------------------------------- 主循环

async fn connect(host: &str, port: u16) -> std::io::Result<BufReader<TcpStream>> {
    let stream = TcpStream::connect((host, port)).await?;
    stream.set_nodelay(true)?;
    Ok(BufReader::with_capacity(64 * 1024, stream))
}

#[allow(clippy::too_many_arguments)]
async fn worker(
    id: usize,
    args: Arc<Args>,
    body: Arc<Vec<u8>>,
    deadline: Option<Instant>,
    quota: Option<usize>,
    errors: Arc<AtomicU64>,
    non200: Arc<AtomicU64>,
    counter: Arc<AtomicU64>,
    stalls: Arc<AtomicU64>,
) -> Samples {
    let mut samples = Samples::default();
    let mut reader = match connect(&args.host, args.port).await {
        Ok(r) => r,
        Err(err) => {
            eprintln!("worker {id}: connect failed: {err}");
            errors.fetch_add(1, Ordering::Relaxed);
            return samples;
        }
    };
    let mut line = Vec::with_capacity(256);
    let mut scratch = Vec::with_capacity(64 * 1024);
    // 进程号进 key，防止同机先后两次 loadgen 撞到同一批幂等条目。
    let pid = std::process::id();

    let mut sent = 0usize;
    loop {
        if let Some(q) = quota
            && sent >= q
        {
            break;
        }
        if let Some(d) = deadline
            && Instant::now() >= d
        {
            break;
        }

        // 幂等 key 必须**全进程唯一**：预热和正式测量共用一个进程，如果两段
        // 各自从 0 开始计数，正式测量的每一发都会命中预热留下的缓存条目，
        // 于是根本不打上游 —— 量到的会是"重放有多快"，不是"幂等有多贵"。
        // 这个坑第一次跑基线时真的踩了（正式档比对照组还快 138 µs）。
        let seq = KEY_SEQ.fetch_add(1, Ordering::Relaxed);
        let _ = counter.fetch_add(1, Ordering::Relaxed);
        let mut request = Vec::with_capacity(body.len() + 512);
        request.extend_from_slice(
            format!(
                "POST {} HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {}\r\n\
                 Content-Type: application/json\r\nContent-Length: {}\r\n",
                args.path,
                args.host,
                args.port,
                perfkit::PERF_API_KEY,
                body.len()
            )
            .as_bytes(),
        );
        if args.idempotency {
            request
                .extend_from_slice(format!("Idempotency-Key: perf-{pid}-{id}-{seq}\r\n").as_bytes());
        }
        request.extend_from_slice(b"\r\n");
        request.extend_from_slice(&body);

        let outcome = tokio::time::timeout(
            args.timeout,
            one_request(&mut reader, &request, &mut line, &mut scratch),
        )
        .await;

        match outcome {
            Ok(Ok(resp)) => {
                if resp.status != 200 {
                    non200.fetch_add(1, Ordering::Relaxed);
                }
                let total = resp.chunk_at.last().copied().unwrap_or(resp.ttfb);
                samples.total.push(total.as_nanos() as u64);
                samples.ttfb.push(resp.ttfb.as_nanos() as u64);
                samples.chunks.push(resp.chunk_at.len() as u64);
                if args.stream && resp.chunk_at.len() > 1 {
                    for w in resp.chunk_at.windows(2) {
                        samples.gaps.push((w[1] - w[0]).as_nanos() as u64);
                    }
                }
                let _ = resp.body_len;
            }
            // 超时 / IO 错误：连接的协议状态已经不可信，换一条重连继续。
            // 计入 stalls/errors 并如实报出去，不当作没发生。
            other => {
                if other.is_err() {
                    stalls.fetch_add(1, Ordering::Relaxed);
                } else {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
                match connect(&args.host, args.port).await {
                    Ok(r) => reader = r,
                    Err(err) => {
                        eprintln!("worker {id}: reconnect failed: {err}");
                        break;
                    }
                }
            }
        }
        sent += 1;
    }
    samples
}

fn main() -> anyhow::Result<()> {
    let args = Arc::new(parse_args());
    let body = Arc::new(build_body(args.body_bytes, args.stream));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(std::cmp::max(2, args.concurrency.min(8)))
        .enable_all()
        .build()?;

    rt.block_on(async move {
        // --- 预热：把连接池、JIT-free 的分支预测、页表都跑热 ---
        if !args.warmup.is_zero() {
            let warm_deadline = Instant::now() + args.warmup;
            let mut handles = Vec::new();
            for id in 0..args.concurrency {
                let (a, b) = (args.clone(), body.clone());
                let noise = Arc::new(AtomicU64::new(0));
                handles.push(tokio::spawn(worker(
                    id,
                    a,
                    b,
                    Some(warm_deadline),
                    None,
                    noise.clone(),
                    noise.clone(),
                    noise.clone(),
                    noise,
                )));
            }
            for h in handles {
                let _ = h.await;
            }
        }

        // --- 正式测量 ---
        let errors = Arc::new(AtomicU64::new(0));
        let non200 = Arc::new(AtomicU64::new(0));
        let counter = Arc::new(AtomicU64::new(0));
        let stalls = Arc::new(AtomicU64::new(0));
        let deadline = args.duration.map(|d| Instant::now() + d);
        let quota = args
            .requests
            .map(|n| n.div_ceil(args.concurrency));

        let started = Instant::now();
        let mut handles = Vec::new();
        for id in 0..args.concurrency {
            handles.push(tokio::spawn(worker(
                id,
                args.clone(),
                body.clone(),
                deadline,
                quota,
                errors.clone(),
                non200.clone(),
                counter.clone(),
                stalls.clone(),
            )));
        }
        let mut all = Samples::default();
        for h in handles {
            all.merge(h.await.expect("worker joins"));
        }
        let wall = started.elapsed();

        let n = all.total.len();
        let chunks_min = all.chunks.iter().copied().min().unwrap_or(0);
        let chunks_max = all.chunks.iter().copied().max().unwrap_or(0);
        let report = serde_json::json!({
            "label": args.label,
            "target": format!("http://{}:{}{}", args.host, args.port, args.path),
            "mode": if args.stream { "sse" } else { "unary" },
            "concurrency": args.concurrency,
            "request_bytes": body.len(),
            "requests": n,
            "wall_seconds": wall.as_secs_f64(),
            "rps": n as f64 / wall.as_secs_f64(),
            "errors": errors.load(Ordering::Relaxed),
            "stalls": stalls.load(Ordering::Relaxed),
            "timeout_ms": args.timeout.as_millis() as u64,
            "non_200": non200.load(Ordering::Relaxed),
            "chunks_per_response": { "min": chunks_min, "max": chunks_max },
            "latency": summarize("total", &mut all.total),
            "ttfb": summarize("ttfb", &mut all.ttfb),
            "chunk_gap": summarize("chunk_gap", &mut all.gaps),
        });

        let rendered = serde_json::to_string_pretty(&report)?;
        if let Some(path) = &args.out {
            std::fs::write(path, format!("{rendered}\n"))?;
        }
        println!("{rendered}");
        Ok::<(), anyhow::Error>(())
    })
}
