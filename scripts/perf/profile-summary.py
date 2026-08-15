#!/usr/bin/env python3
"""把 `/usr/bin/sample` 的调用图折成"按类别的 CPU 占比"表。

用法：
    python3 scripts/perf/profile-summary.py            # 打印
    python3 scripts/perf/profile-summary.py --save     # 同时写 results/profile-summary.txt

# 归因方法（写清楚，免得数字被当成比它更精确的东西）

`sample` 的调用图里，一行的样本数 = 「这个帧及其所有子帧」的样本数。所以
**只有叶子帧的计数才是自身 CPU 时间**（下一行缩进不比自己深 = 叶子）。本脚本
只统计叶子，这与 `sample` 自己的语义一致，不会出现自身时间为负这种把和算爆的
情况。

`__psynch_cvwait` / park 是线程**空闲**在等活干，不是开销。表里单列，并额外给
一份「扣掉 park 之后的有效 CPU 占比」—— 那才是可比的口径。

两个 profile 是在**同样的 15 秒、同样 concurrency=8 的负载**下采的。因为网关
每秒服务的请求数比对照组少，所以这里的百分比是"每单位 CPU 花在哪"，不是
"每请求花了多少"。要每请求的量，看分配计数表。
"""

import argparse
import collections
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RESULTS = os.environ.get("RESULTS", os.path.join(HERE, "results"))

GROUPS = [
    ("分配器 malloc/free/realloc", ["malloc", "free", "xzm", "bzero"]),
    ("memmove / memset / memcpy", ["memmove", "memset", "memcpy"]),
    ("getentropy（uuid v4 取熵）", ["getentropy"]),
    ("serde_json", ["serde_json"]),
    ("网络系统调用 recv/writev/kevent", ["recvfrom", "writev", "kevent"]),
    ("线程 park / 条件变量（空闲）", ["psynch", "park"]),
    ("取时间 mach_absolute_time / clock", ["mach_absolute_time", "clock_gettime", "Timespec"]),
]

BUSY_GROUPS = [g for g, _ in GROUPS if "空闲" not in g]


def leaf_samples(path):
    rows = []
    with open(path, errors="replace") as fh:
        for line in fh:
            m = re.match(r"^([ +!:|]*?)(\d+)\s+(.+)$", line.rstrip("\n"))
            if not m:
                continue
            indent, cnt, rest = m.group(1), int(m.group(2)), m.group(3)
            name = re.sub(r"\s+\[0x[0-9a-f,\s.]+\]$", "", rest)
            name = re.sub(r"\s+\+\s+[\d,.\s]+$", "", name).strip()
            rows.append((len(indent), cnt, name))
    leaf = collections.Counter()
    total = 0
    for i, (depth, cnt, name) in enumerate(rows):
        is_leaf = (i + 1 >= len(rows)) or (rows[i + 1][0] <= depth)
        if is_leaf:
            leaf[name] += cnt
            total += cnt
    return leaf, total


def report(path, label, out):
    if not os.path.exists(path):
        out.append(f"（缺 {path}，跳过）")
        return
    leaf, total = leaf_samples(path)
    if total == 0:
        out.append(f"（{path} 里没有可解析的样本，跳过）")
        return
    used, acc = set(), {}
    out.append(f"\n##### {label}  叶子样本合计 = {total} #####")
    for name, keys in GROUPS:
        hit = [n for n in leaf if n not in used and any(k in n for k in keys)]
        acc[name] = sum(leaf[n] for n in hit)
        used.update(hit)
        out.append(f"  {name:34} {acc[name]:7}  {100 * acc[name] / total:6.2f}%")
    rest = sum(c for n, c in leaf.items() if n not in used)
    out.append(f"  {'其余':34} {rest:7}  {100 * rest / total:6.2f}%")

    busy = total - acc["线程 park / 条件变量（空闲）"]
    out.append(f"  --- 扣掉 park 之后的有效 CPU 样本 = {busy} ---")
    for name in BUSY_GROUPS:
        out.append(f"    {name:32} 占有效 CPU {100 * acc[name] / busy:6.2f}%")

    out.append("  自身耗时 top 12 叶子帧：")
    for name, cnt in leaf.most_common(12):
        out.append(f"    {cnt:7} {100 * cnt / total:5.2f}%  {name[:92]}")
    return acc, busy


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--save", action="store_true")
    args = ap.parse_args()

    out = [__doc__.split("\n\n", 1)[1].strip(), ""]
    report(os.path.join(RESULTS, "profile-gateway-full.txt"), "网关 全链路 (full)", out)
    report(os.path.join(RESULTS, "profile-floor.txt"), "对照组下界 (floor)", out)
    text = "\n".join(out)
    print(text)
    if args.save:
        with open(os.path.join(RESULTS, "profile-summary.txt"), "w") as fh:
            fh.write(text + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
