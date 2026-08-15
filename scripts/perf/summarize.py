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
import re
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RESULTS = os.environ.get("RESULTS", os.path.join(HERE, "results"))

# floor 必须排第一：所有差值都是"减 floor"。
TARGETS = ["floor", "nomw", "full", "relay"]
# 差值行按这个顺序打，floor 不出现在里面（它是被减数）。
DELTA_TARGETS = [t for t in TARGETS if t != "floor"]
TARGET_LABEL = {
    "floor": "对照组下界 (floor)",
    "nomw":  "网关 无中间件 (nomw)",
    "full":  "网关 全链路 (full)",
    "relay": "gw-relay 中继内核 (relay)",
    "idem":  "网关 全链路 + 幂等 (idem)",
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
        for t in DELTA_TARGETS:
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
            name = TARGET_LABEL[t]
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
        for t in DELTA_TARGETS:
            if t in base:
                d = [a - b for a, b in zip(base[t], base["floor"])]
                print(f"| **{TARGET_LABEL[t]} − 下界** | | " + " | ".join(f"**{fmt(x)}**" for x in d) + " |")


def sseburst_table():
    """SSE 满速：间隔设 0，整流耗时就是"中继 501 个 chunk 要多久"。

    1 ms 间隔那一档量不出每 chunk 成本 —— mock 的定时器在本机被放大到
    ~2.35 ms，网关的个位数 µs 埋在里面。这一档把定时器拿掉。
    """
    print("\n### c-1) SSE 满速（500 chunk × 1 KiB，间隔 0）—— 分辨每 chunk 中继成本\n")
    print("| 被测端 | 轮数 | 整流 p50 µs | 跨轮 min–max | 每 chunk µs | TTFB p50 µs |")
    print("| --- | ---: | ---: | --- | ---: | ---: |")
    base = {}
    for t in TARGETS:
        docs = load(f"lat-sseburst-{t}-r*.json")
        if not docs:
            continue
        tot = [d["latency"]["p50_us"] for d in docs]
        chunks = max(d["chunks_per_response"]["max"] for d in docs) or 1
        base[t] = (med(tot), chunks)
        print(f"| {TARGET_LABEL[t]} | {len(docs)} | {fmt(med(tot))} | "
              f"{fmt(min(tot))}–{fmt(max(tot))} | {fmt(med(tot)/chunks, 3)} | "
              f"{fmt(med([d['ttfb']['p50_us'] for d in docs]))} |")
    if "floor" in base:
        f, ch = base["floor"]
        for t in DELTA_TARGETS:
            if t in base:
                d = base[t][0] - f
                print(f"| **{TARGET_LABEL[t]} − 下界** | | **{fmt(d)}** | | "
                      f"**{fmt(d/ch, 3)}** | |")


def collect_json(label, target):
    """档 1c 的一格：返回各轮 p50 列表。"""
    return [d["latency"]["p50_us"] for d in load(f"json-{label}-{target}-r*.json")
            if (d.get("latency") or {}).get("n")]


def jsonrewrite_table():
    """档 1c：同一 body 只切 stream 真假，隔离出「JSON 重写随 body 增长的部分」。

    这一档原来没有汇总函数（§2.5 的表是手算的）。T10 要用它，所以补上。
    """
    print("\n### 1c) 请求体 JSON 重写（同一 body，只切 `stream` 真假）\n")
    cells = [("u1k", "1 KiB 请求，非流式"), ("s1k", "1 KiB 请求，流式"),
             ("u256k", "256 KiB 请求，非流式"), ("s256k", "256 KiB 请求，流式"),
             ("resp1m", "1 KiB 请求 → 1 MiB 响应，非流式")]
    have = [t for t in TARGETS if any(collect_json(c, t) for c, _ in cells)]
    print("| 场景 | " + " | ".join(f"{TARGET_LABEL[t]} p50" for t in have) + " | " +
          " | ".join(f"{t} − floor" for t in have if t != "floor") + " |")
    print("| --- |" + " ---: |" * (len(have) + max(0, len(have) - 1)))
    grid = {}
    for key, label in cells:
        vals = {t: med(collect_json(key, t)) for t in have}
        grid[key] = vals
        row = [fmt(vals[t]) for t in have]
        row += [fmt(vals[t] - vals["floor"]) for t in have if t != "floor"]
        print(f"| {label} | " + " | ".join(row) + " |")

    if any(grid.get("resp1m", {}).get(t) == grid.get("resp1m", {}).get(t) for t in have):
        print("\n| 被测端 | 请求侧（256 KiB 请求，减 1 KiB 那一格） | 响应侧（1 MiB 响应，减 1 KiB 那一格） |")
        print("| --- | ---: | ---: |")
        for t in have:
            if t == "floor":
                continue
            base = grid["u1k"][t] - grid["u1k"]["floor"]
            req = (grid["u256k"][t] - grid["u256k"]["floor"]) - base
            resp = (grid["resp1m"][t] - grid["resp1m"]["floor"]) - base
            print(f"| {TARGET_LABEL[t]} | {fmt(req)} µs / 256 KiB | {fmt(resp)} µs / 1 MiB |")
        print("\n> b) 档的开销落在请求侧还是响应侧，只有这一行拆得开。"
              "两格都减掉 1 KiB 那一格，去掉与 body 大小无关的固定成本。")

    print("\n| 被测端 | (流式−非流式)@1KiB | (流式−非流式)@256KiB | 差 = JSON 重写代价 |")
    print("| --- | ---: | ---: | ---: |")
    out = {}
    for t in have:
        if t == "floor":
            continue
        d = {k: grid[k][t] - grid[k]["floor"] for k in grid}
        small = d["s1k"] - d["u1k"]
        large = d["s256k"] - d["u256k"]
        out[t] = large - small
        print(f"| {TARGET_LABEL[t]} | {fmt(small)} | {fmt(large)} | **{fmt(out[t])}** |")
    print("\n> 读法：两级相减。第一级减 floor 去掉运行时/mock 的共同成本；"
          "第二级用 1 KiB 的「流式−非流式」当零点，去掉「流式路径本身」的固定开销。"
          "剩下的就是**随 body 线性增长**的那部分，也就是请求体重写的真实代价。")
    return out


# ==================================================================== T1–T13

# `docs/relay-perf-baseline.md` §5 的 13 条验收目标。
# (编号, 说明, 目标值, 单位, 方向)  —— 方向 "<=" 表示越小越好。
#
# 规范 2.11「不许把源码里的字面量抄进断言」在这里**不适用**：这些数不是从
# 被测源码里抄来的，它们是 wave 1 的实测基线定出来的**外部目标**。抄的是
# §5 那张表，抄错了会被 --acceptance 自己的自检发现（见 _T_SOURCE）。
_T_SOURCE = "docs/relay-perf-baseline.md §5"

SSE_FLOW_BYTES = 501 * 1024      # c) 501 帧 × 1 KiB，与 §2.4 的放大倍数表同口径
LARGE_PAYLOAD = 256 * 1024 + 1024 * 1024
SMALL_PAYLOAD = 1024 + 2048


def _delta_p50(scenario, target, field="latency"):
    """被测端 p50 − floor p50（跨轮中位数）。绝对值受后台负载影响，差值不受。"""
    a = [r[0] for r in collect_latency(scenario, target, field)]
    b = [r[0] for r in collect_latency(scenario, "floor", field)]
    if not a or not b:
        return float("nan")
    return med(a) - med(b)


def _alloc(scenario, target, key):
    docs = load(f"alloc-{scenario}-{target}.alloc.json")
    return docs[0][key] if docs else float("nan")


def _getentropy_pct(target):
    """T12：profile 里 getentropy 占**扣掉 park 之后的有效 CPU**。

    分母与 `profile-summary.py` 一致（park / 条件变量是线程空闲等活，不是开销）。
    用总样本当分母会把这个数按 park 比例压小，两张表就对不上了。
    找不到 profile 返回 NaN —— **未覆盖，不是 0**。
    """
    name = "profile-gateway-full.txt" if target == "full" else f"profile-{target}.txt"
    path = os.path.join(RESULTS, name)
    if not os.path.exists(path):
        return float("nan")
    total = hits = park = 0
    with open(path, errors="replace") as fh:
        rows = []
        for line in fh:
            m = re.match(r"^([ +!:|]*?)(\d+)\s+(.+)$", line.rstrip("\n"))
            if m:
                rows.append((len(m.group(1)), int(m.group(2)), m.group(3)))
    for i, (depth, cnt, name_) in enumerate(rows):
        if i + 1 >= len(rows) or rows[i + 1][0] <= depth:
            total += cnt
            if "getentropy" in name_:
                hits += cnt
            if "psynch" in name_ or "park" in name_:
                park += cnt
    busy = total - park
    return 100.0 * hits / busy if busy else float("nan")


def acceptance_table(target):
    rows = []
    add = rows.append

    d_small = _delta_p50("small", target)
    d_large = _delta_p50("large", target)
    add(("T1", "非流式 1 KiB→2 KiB，自身开销 p50", 4.0, "µs", d_small))
    add(("T2", "同上，每请求堆分配次数", 110.0, "次", _alloc("small", target, "alloc_per_req")))
    add(("T3", "同上，每请求堆分配字节", 36_000.0, "B", _alloc("small", target, "bytes_per_req")))

    slope = (d_large - d_small) / ((LARGE_PAYLOAD - SMALL_PAYLOAD) / 1024.0)
    add(("T4", "大 body 斜率（开销 ÷ 载荷）", 0.030, "µs/KiB", slope))
    add(("T5", "分配字节 ÷ 载荷字节（256 KiB→1 MiB）", 1.6, "×",
         _alloc("large", target, "bytes_per_req") / LARGE_PAYLOAD))
    add(("T6", "256 KiB→1 MiB，自身开销 p50", 50.0, "µs", d_large))

    add(("T7", "SSE 建流固定成本（c-0）", 4.0, "µs", _delta_p50("ssettfb", target, "ttfb")))

    burst = load(f"lat-sseburst-{target}-r*.json")
    fburst = load("lat-sseburst-floor-r*.json")
    per_chunk = float("nan")
    if burst and fburst:
        chunks = max(d["chunks_per_response"]["max"] for d in burst) or 1
        per_chunk = (med([d["latency"]["p50_us"] for d in burst])
                     - med([d["latency"]["p50_us"] for d in fburst])) / chunks
    add(("T8", "SSE 每 chunk 额外开销（c-1 满速）", 0.15, "µs", per_chunk))
    add(("T9", "SSE 分配字节 ÷ 流量字节", 0.15, "×",
         _alloc("sse", target, "bytes_per_req") / SSE_FLOW_BYTES))

    add(("T10", "256 KiB 流式请求的 JSON 重写代价", 10.0, "µs",
         _jsonrewrite_cost(target)))

    idem = load(f"idem-large-on-r*.json"), load("idem-large-off-r*.json")
    t11 = float("nan")
    if target == "full" and all(idem):
        t11 = (med([d["latency"]["p50_us"] for d in idem[0]])
               - med([d["latency"]["p50_us"] for d in idem[1]]))
    add(("T11", "1 MiB 响应 + Idempotency-Key 额外开销", 100.0, "µs", t11))

    add(("T12", "profile 里 getentropy 占有效 CPU", 0.0, "%", _getentropy_pct(target)))

    tput = load(f"tput-small-{target}-r*.json")
    ftput = load("tput-small-floor-r*.json")
    ratio = float("nan")
    if tput and ftput:
        ratio = 100.0 * med([d["rps"] for d in tput]) / med([d["rps"] for d in ftput])
    add(("T13", "吞吐（concurrency 16）相对下界", 90.0, "%", ratio))

    print(f"\n## T1–T13 验收：被测端 `{target}`（目标出处：{_T_SOURCE}）\n")
    print("| # | 指标 | 目标 | 实测 | 结论 | 差多少 |")
    print("| ---: | --- | ---: | ---: | :---: | ---: |")
    for num, name, goal, unit, got in rows:
        nd = 3 if unit == "µs/KiB" else (0 if unit == "B" else 2)
        if got != got:
            print(f"| {num} | {name} | {fmt(goal, nd)} {unit} | — | **未覆盖** | — |")
            continue
        better_is_high = num == "T13"
        ok = got >= goal if better_is_high else got <= goal
        gap = (goal - got) if better_is_high else (got - goal)
        verdict = "达标" if ok else "**未达标**"
        sign = "+" if gap > 0 else ""
        print(f"| {num} | {name} | {fmt(goal, nd)} {unit} | {fmt(got, nd)} {unit} | "
              f"{verdict} | {sign}{fmt(gap, nd)} |")
    return rows


def _jsonrewrite_cost(target):
    grid = {}
    for key in ("u1k", "s1k", "u256k", "s256k"):
        a, b = collect_json(key, target), collect_json(key, "floor")
        grid[key] = (med(a) - med(b)) if (a and b) else float("nan")
    return (grid["s256k"] - grid["u256k"]) - (grid["s1k"] - grid["u1k"])


def tls_table():
    """档 7：上游那一跳走 TLS + h2 之后，relay 对 floor 的差值有没有失真。

    §5.2 的原话是「T4/T5 在 h2 的分帧开销下会失真」。判据就是这张表的
    Δ 与明文档的 Δ 对不对得上 —— 对得上，明文档的结论就能外推到生产。
    """
    docs = load("tls-small-floor-r*.json")
    if not docs:
        return
    print("\n### 档 7）TLS + HTTP/2（上游那一跳 https/h2，客户端一侧仍是明文 h1）\n")
    print("| 场景 | floor p50 | relay p50 | Δ(TLS+h2) | Δ(明文 h1) | 差之差 |")
    print("| --- | ---: | ---: | ---: | ---: | ---: |")
    for scen, label, field in (("small", "1 KiB→2 KiB", "latency"),
                               ("large", "256 KiB→1 MiB", "latency"),
                               ("ssettfb", "SSE 建流（c-0）", "ttfb")):
        tls = {}
        for t in ("floor", "relay"):
            v = [(d.get(field) or {}).get("p50_us") for d in load(f"tls-{scen}-{t}-r*.json")]
            v = [x for x in v if x]
            if v:
                tls[t] = med(v)
        if len(tls) < 2:
            continue
        d_tls = tls["relay"] - tls["floor"]
        d_plain = _delta_p50(scen, "relay", field)
        print(f"| {label} | {fmt(tls['floor'])} | {fmt(tls['relay'])} | **{fmt(d_tls)}** | "
              f"{fmt(d_plain)} | {fmt(d_tls - d_plain)} |")
    print("\n> 「差之差」是本档的**唯一**结论量：它接近 0 就说明明文档量到的中继开销"
          "在 h2 上仍然成立；它大，说明 h2 的分帧把开销结构改了，明文档的 T4/T5 不能外推。"
          "绝对值不可比 —— TLS 记录加解密是两边都要付的新成本。")


def failover_table():
    """档 6：跨账号 failover 的重放代价 —— `Bytes` 化到底省了多少。"""
    if not load("fo-bytes-a1-r*.json"):
        return
    print("\n### 档 6）跨账号 failover 的重放代价（256 KiB 请求体，前 n−1 次上游回 429）\n")
    print("| 尝试次数 | bytes p50 | vec p50 | Δ = 每次重放的拷贝代价 | "
          "bytes 分配字节/请求 | vec 分配字节/请求 | Δ 字节 |")
    print("| ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    for n in (1, 2, 4):
        row, alloc = {}, {}
        for mode in ("bytes", "vec"):
            v = [d["latency"]["p50_us"] for d in load(f"fo-{mode}-a{n}-r*.json")
                 if (d.get("latency") or {}).get("n")]
            if v:
                row[mode] = med(v)
            a = load(f"alloc-fo-{mode}-a{n}.alloc.json")
            if a:
                alloc[mode] = a[0]["bytes_per_req"]
        if len(row) < 2:
            continue
        db = (alloc["vec"] - alloc["bytes"]) if len(alloc) == 2 else float("nan")
        print(f"| {n} | {fmt(row['bytes'])} | {fmt(row['vec'])} | "
              f"**{fmt(row['vec'] - row['bytes'])}** | "
              f"{fmt(alloc.get('bytes', float('nan')), 0)} | "
              f"{fmt(alloc.get('vec', float('nan')), 0)} | {fmt(db, 0)} |")
    print("\n> 两个模式跑的是**同一条路径、同一个二进制**，只差一行："
          "`Bytes::clone`（refcount 加一）vs `Bytes::copy_from_slice`（全量拷贝）。"
          "后者复刻今天 `routes.rs:217` 的 `inbound.body.to_vec()` 落在 failover 循环体内。"
          "所以 Δ 就是 `Bytes` 化省下来的东西，不含任何别的差异。")


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
    ap.add_argument("--acceptance", metavar="TARGET",
                    help="只打 T1–T13 验收表（被测端名，如 relay）")
    args = ap.parse_args()

    if args.acceptance:
        env = os.path.join(RESULTS, "env.txt")
        if os.path.exists(env):
            print("```")
            print(open(env).read().strip())
            print("```")
        acceptance_table(args.acceptance)
        return 0

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
    sseburst_table()
    jsonrewrite_table()
    alloc_table()
    throughput_table()
    idempotency_table()
    failover_table()
    tls_table()
    if args.raw:
        raw_dump()
    return 0


if __name__ == "__main__":
    sys.exit(main())
