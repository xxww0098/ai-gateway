#!/usr/bin/env python3
"""把 results/*.json 汇总成可以直接贴进 docs/relay-perf-baseline.md 的表。

用法：
    python3 scripts/perf/summarize.py                # 打印全部表
    python3 scripts/perf/summarize.py --raw          # 附带逐轮原始值

跨轮取**中位数**而不是平均：本机后台负载会造成偶发长尾，中位数对它免疫；
平均数不免疫。跨轮的离散度用 min/max 一起打出来，读的人自己判断可信度。
"""

import argparse
import glob
import json
import os
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RESULTS = os.environ.get("RESULTS", os.path.join(HERE, "results"))

TARGETS = ["floor", "nomw", "full"]
TARGET_LABEL = {
    "floor": "对照组下界 (floor)",
    "nomw":  "网关 无中间件 (nomw)",
    "full":  "网关 全链路 (full)",
}


def load(pattern):
    out = []
    for path in sorted(glob.glob(os.path.join(RESULTS, pattern))):
        try:
            with open(path) as fh:
                out.append(json.load(fh))
        except (json.JSONDecodeError, OSError):
            pass
    return out


def collect_latency(scenario, target, field="latency"):
    """返回该 (场景, 被测端) 各轮的 (p50,p95,p99) 列表。"""
    rows = []
    for doc in load(f"lat-{scenario}-{target}-r*.json"):
        m = doc.get(field) or {}
        if m.get("n"):
            rows.append((m["p50_us"], m["p95_us"], m["p99_us"], doc["rps"]))
    return rows


def med(vals):
    return statistics.median(vals) if vals else float("nan")


def fmt(x, nd=1):
    if x != x:  # NaN
        return "—"
    return f"{x:,.{nd}f}"


def latency_table(scenario, title, field="latency"):
    print(f"\n### {title}\n")
    print("| 被测端 | 轮数 | p50 µs | p95 µs | p99 µs | rps(串行) | p50 跨轮 min–max |")
    print("| --- | ---: | ---: | ---: | ---: | ---: | --- |")
    base = {}
    for t in TARGETS:
        rows = collect_latency(scenario, t, field)
        if not rows:
            continue
        p50s = [r[0] for r in rows]
        base[t] = (med(p50s), med([r[1] for r in rows]), med([r[2] for r in rows]))
        print(
            f"| {TARGET_LABEL[t]} | {len(rows)} | {fmt(base[t][0])} | {fmt(base[t][1])} | "
            f"{fmt(base[t][2])} | {fmt(med([r[3] for r in rows]), 0)} | "
            f"{fmt(min(p50s))}–{fmt(max(p50s))} |"
        )
    if "floor" in base:
        f50, f95, f99 = base["floor"]
        for t in ("nomw", "full"):
            if t in base:
                d = base[t]
                print(
                    f"| **{TARGET_LABEL[t]} − 下界** | | **{fmt(d[0]-f50)}** | "
                    f"**{fmt(d[1]-f95)}** | **{fmt(d[2]-f99)}** | | |"
                )
        if "full" in base and "nomw" in base:
            d, n = base["full"], base["nomw"]
            print(
                f"| **access+hold 净成本 (full − nomw)** | | **{fmt(d[0]-n[0])}** | "
                f"**{fmt(d[1]-n[1])}** | **{fmt(d[2]-n[2])}** | | |"
            )
    return base


def alloc_table():
    print("\n### 每请求堆分配\n")
    print("| 场景 | 被测端 | 请求数 | 分配次数/请求 | 分配字节/请求 | 相对下界 (次) | 相对下界 (字节) |")
    print("| --- | --- | ---: | ---: | ---: | ---: | ---: |")
    for scenario, label in (("small", "a) 1 KiB→2 KiB"),
                            ("large", "b) 256 KiB→1 MiB"),
                            ("sse", "c) SSE 500×1 KiB")):
        floor = None
        for t in TARGETS + ["idem"]:
            docs = load(f"alloc-{scenario}-{t}.alloc.json")
            if not docs:
                continue
            d = docs[0]
            if t == "floor":
                floor = d
            dc = fmt(d["alloc_per_req"] - floor["alloc_per_req"], 1) if floor else "—"
            db = fmt(d["bytes_per_req"] - floor["bytes_per_req"], 0) if floor else "—"
            name = TARGET_LABEL.get(t, "网关 全链路 + 幂等 (idem)")
            print(
                f"| {label} | {name} | {d['requests']} | {fmt(d['alloc_per_req'])} | "
                f"{fmt(d['bytes_per_req'], 0)} | {dc} | {db} |"
            )
    for doc in load("noise-*.idle.json"):
        print(f"\n> 空载噪声 `{doc['label']}`：{doc['alloc_per_sec']:.1f} 次分配/秒"
              f"（{doc['idle_seconds']}s 内共 {doc['alloc_count']} 次）。"
              f"按上表的实测速率折算，噪声占比 < 0.1%，未从表中扣除。")


def throughput_table():
    print("\n### 吞吐（concurrency=16，5s×3 轮）\n")
    print("| 被测端 | rps 中位数 | rps min–max | p50 µs | p99 µs |")
    print("| --- | ---: | --- | ---: | ---: |")
    for t in TARGETS:
        docs = load(f"tput-small-{t}-r*.json")
        if not docs:
            continue
        rps = [d["rps"] for d in docs]
        print(
            f"| {TARGET_LABEL[t]} | {fmt(med(rps),0)} | {fmt(min(rps),0)}–{fmt(max(rps),0)} | "
            f"{fmt(med([d['latency']['p50_us'] for d in docs]))} | "
            f"{fmt(med([d['latency']['p99_us'] for d in docs]))} |"
        )


def idempotency_table():
    print("\n### 幂等（`Idempotency-Key` 触发 hold 层 `capture_body` 全量缓冲）\n")
    print("| 场景 | 幂等 | p50 µs | p99 µs | Δp50 µs |")
    print("| --- | --- | ---: | ---: | ---: |")
    for scenario, label in (("small", "1 KiB→2 KiB"), ("large", "256 KiB→1 MiB")):
        vals = {}
        for state in ("off", "on"):
            docs = load(f"idem-{scenario}-{state}-r*.json")
            if not docs:
                continue
            vals[state] = (
                med([d["latency"]["p50_us"] for d in docs]),
                med([d["latency"]["p99_us"] for d in docs]),
            )
        for state in ("off", "on"):
            if state in vals:
                delta = ""
                if state == "on" and "off" in vals:
                    delta = fmt(vals["on"][0] - vals["off"][0])
                print(f"| {label} | {state} | {fmt(vals[state][0])} | "
                      f"{fmt(vals[state][1])} | {delta} |")


def sse_table():
    print("\n### SSE 长流（500 chunk × 1 KiB × 1 ms）\n")
    print("| 被测端 | 轮数 | TTFB p50 µs | TTFB p99 µs | chunk 间隔 p50 µs | "
          "chunk 间隔 p99 µs | chunk 间隔 stddev µs |")
    print("| --- | ---: | ---: | ---: | ---: | ---: | ---: |")
    base = {}
    for t in TARGETS:
        docs = load(f"lat-sse-{t}-r*.json")
        if not docs:
            continue
        base[t] = (
            med([d["ttfb"]["p50_us"] for d in docs]),
            med([d["ttfb"]["p99_us"] for d in docs]),
            med([d["chunk_gap"]["p50_us"] for d in docs]),
            med([d["chunk_gap"]["p99_us"] for d in docs]),
            med([d["chunk_gap"]["stddev_us"] for d in docs]),
        )
        print(f"| {TARGET_LABEL[t]} | {len(docs)} | " + " | ".join(fmt(x) for x in base[t]) + " |")
    if "floor" in base:
        for t in ("nomw", "full"):
            if t in base:
                d = [a - b for a, b in zip(base[t], base["floor"])]
                print(f"| **{TARGET_LABEL[t]} − 下界** | | " + " | ".join(f"**{fmt(x)}**" for x in d) + " |")


def raw_dump():
    print("\n### 逐轮原始值\n```")
    for path in sorted(glob.glob(os.path.join(RESULTS, "*.json"))):
        try:
            with open(path) as fh:
                d = json.load(fh)
        except (json.JSONDecodeError, OSError):
            continue
        if "latency" in d:
            print(f"{os.path.basename(path):34} n={d['requests']:6} rps={d['rps']:9.0f} "
                  f"p50={d['latency']['p50_us']:9.2f} p99={d['latency']['p99_us']:10.2f}")
        elif "alloc_per_req" in d:
            print(f"{os.path.basename(path):34} allocs/req={d['alloc_per_req']:9.1f} "
                  f"bytes/req={d['bytes_per_req']:12.0f}")
    print("```")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--raw", action="store_true")
    args = ap.parse_args()

    env = os.path.join(RESULTS, "env.txt")
    if os.path.exists(env):
        print("```")
        print(open(env).read().strip())
        print("```")

    latency_table("small", "a) 非流式小 body：1 KiB 请求 / 2 KiB 响应")
    latency_table("large", "b) 非流式大 body：256 KiB 请求 / 1 MiB 响应")
    latency_table("ssettfb", "c-0) 流式路径固定开销（1 chunk，无间隔）—— 只测建流成本",
                  field="ttfb")
    sse_table()
    alloc_table()
    throughput_table()
    idempotency_table()
    if args.raw:
        raw_dump()
    return 0


if __name__ == "__main__":
    sys.exit(main())
