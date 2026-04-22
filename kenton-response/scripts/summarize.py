#!/usr/bin/env python3
"""
Aggregate raw JSON in results/{run-N}/*.json into SUMMARY.md.

V2 — uses median across runs for headline numbers, reports IQR as noise
estimate, runs linear regression on memory data, and parses smaps
columns for the true COW upper-bound.

Usage:
    python3 scripts/summarize.py                    # results/run_*/*.json
    python3 scripts/summarize.py results/           # explicit base
"""

import glob
import json
import os
import statistics
import sys
from pathlib import Path


# ---------- formatters ----------

def fmt_ns(ns):
    if ns < 1e3: return f"{ns:.0f} ns"
    if ns < 1e6: return f"{ns/1e3:.2f} µs"
    if ns < 1e9: return f"{ns/1e6:.2f} ms"
    return f"{ns/1e9:.2f} s"

def fmt_bytes(b):
    if b is None: return "—"
    if b < 1024: return f"{b} B"
    if b < 1024 * 1024: return f"{b/1024:.1f} KB"
    return f"{b/(1024*1024):.1f} MB"


# ---------- multi-run aggregation ----------

def median_field(rows, *keys):
    """Pull rows[*][key0][key1]... values and return median."""
    vals = []
    for r in rows:
        v = r
        try:
            for k in keys:
                v = v[k]
            vals.append(float(v))
        except (KeyError, TypeError):
            pass
    return statistics.median(vals) if vals else None

def iqr_field(rows, *keys):
    """Inter-quartile range as % of median, as noise indicator."""
    vals = []
    for r in rows:
        v = r
        try:
            for k in keys:
                v = v[k]
            vals.append(float(v))
        except (KeyError, TypeError):
            pass
    if len(vals) < 2:
        return None
    qs = statistics.quantiles(vals, n=4)
    iqr = qs[2] - qs[0]
    med = statistics.median(vals)
    return (iqr / med * 100.0) if med else None


# ---------- linear regression ----------

def linreg(xs, ys):
    """Returns (slope, intercept, r_squared)."""
    n = len(xs)
    if n < 2: return None, None, None
    mx = sum(xs) / n
    my = sum(ys) / n
    num = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    denx = sum((x - mx) ** 2 for x in xs)
    deny = sum((y - my) ** 2 for y in ys)
    if denx == 0: return None, None, None
    slope = num / denx
    intercept = my - slope * mx
    r2 = (num ** 2) / (denx * deny) if deny > 0 else 0.0
    return slope, intercept, r2


# ---------- summarisers ----------

def summarize_p1(runs):
    """runs is list-of-list[dict]. each inner list = one run's p1.json."""
    if not runs:
        return "## P1 — no data\n"
    workloads = [r["workload"] for r in runs[0]]
    lines = [
        "## P1 — Serialization cost per I/O",
        "",
        "Per-iteration unique state (each round-trip rebuilds the object",
        "with a fresh seed so V8 inline-caches don't go hot).",
        f"Median across **{len(runs)} runs**, IQR-as-%-of-median in parens.",
        "",
        "| Workload | Serialized | Build | Serialize | Deserialize | Round-trip |",
        "|---|---|---|---|---|---|",
    ]
    for wl in workloads:
        # find the matching record in each run
        per_run = [next((x for x in run if x["workload"] == wl), None)
                   for run in runs]
        per_run = [x for x in per_run if x is not None]
        if not per_run:
            continue
        ser_med = median_field(per_run, "serialize_ns", "median")
        ser_iqr = iqr_field(per_run, "serialize_ns", "median")
        des_med = median_field(per_run, "deserialize_ns", "median")
        des_iqr = iqr_field(per_run, "deserialize_ns", "median")
        rt_med = median_field(per_run, "roundtrip_ns", "median")
        rt_iqr = iqr_field(per_run, "roundtrip_ns", "median")
        bl_med = median_field(per_run, "build_ns", "median")
        bytes_ = per_run[0].get("serialized_bytes_last")
        def fmt(med, iqr):
            if med is None: return "—"
            iqr_str = f" (±{iqr:.0f}%)" if iqr is not None and iqr > 1 else ""
            return f"{fmt_ns(med)}{iqr_str}"
        lines.append(
            f"| {wl} | {fmt_bytes(bytes_)} | {fmt(bl_med, None)} | "
            f"{fmt(ser_med, ser_iqr)} | {fmt(des_med, des_iqr)} | "
            f"{fmt(rt_med, rt_iqr)} |"
        )
    return "\n".join(lines)


def summarize_p2(runs):
    if not runs:
        return "## P2 — no data\n"
    # Group by (config, workload)
    keys = []
    seen = set()
    for r in runs[0]:
        k = (r["config"], r.get("workload", "M"))
        if k not in seen:
            seen.add(k)
            keys.append(k)
    lines = [
        "## P2 — Resume cost across configs × state sizes",
        "",
        "- **A** = fresh Isolate + Context per resume (strict multi-tenant)",
        "- **B** = pooled Isolate, fresh Context per resume",
        "- **C** = pooled Context (NOT isolation-safe — leak verified)",
        "",
        f"Median across **{len(runs)} runs**.",
        "",
        "| Config | Workload | Iso-safe? | Setup | Deserialize | First-instr | Total mean | Total p99 |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for cfg, wl in keys:
        per_run = []
        for run in runs:
            for r in run:
                if r["config"] == cfg and r.get("workload") == wl:
                    per_run.append(r)
                    break
        if not per_run:
            continue
        safe = "✓" if per_run[0].get("isolation_safe") else "✗"
        lines.append(
            f"| {cfg} | {wl} | {safe} | "
            f"{fmt_ns(median_field(per_run, 'setup_ns', 'mean') or 0)} | "
            f"{fmt_ns(median_field(per_run, 'deserialize_ns', 'mean') or 0)} | "
            f"{fmt_ns(median_field(per_run, 'first_instruction_ns', 'mean') or 0)} | "
            f"{fmt_ns(median_field(per_run, 'total_resume_ns', 'mean') or 0)} | "
            f"{fmt_ns(median_field(per_run, 'total_resume_ns', 'p99') or 0)} |"
        )
    return "\n".join(lines)


def summarize_p3_memory(runs):
    if not runs:
        return "## P3 memory — no data\n"
    # Median per (pattern, count) cell across runs
    rows = {}
    for run in runs:
        for r in run:
            key = (r["pattern"], r["isolates"])
            rows.setdefault(key, []).append(r)

    lines = [
        "## P3 — Per-isolate memory (V8 14.7.173.16, smaps + regression)",
        "",
        "Median across runs. RSS-max from `getrusage`; smaps fields from",
        "`/proc/self/smaps_rollup`. PSS = proportional set size (Linux).",
        "",
        "| Pattern | Isolates | RSS-max | smaps-RSS | smaps-PSS | V8 total-heap | V8 used-heap |",
        "|---|---|---|---|---|---|---|",
    ]
    patterns = []
    pseen = set()
    counts = sorted({k[1] for k in rows.keys()})
    for run in runs:
        for r in run:
            if r["pattern"] not in pseen:
                pseen.add(r["pattern"])
                patterns.append(r["pattern"])

    for p in patterns:
        for n in counts:
            cell = rows.get((p, n), [])
            if not cell: continue
            lines.append(
                f"| {p} | {n} | "
                f"{fmt_bytes(int(median_field(cell, 'rss_max_kb') * 1024))} | "
                f"{fmt_bytes(int(median_field(cell, 'smaps_rss')) if median_field(cell, 'smaps_rss') else 0)} | "
                f"{fmt_bytes(int(median_field(cell, 'smaps_pss')) if median_field(cell, 'smaps_pss') else 0)} | "
                f"{fmt_bytes(int(median_field(cell, 'v8_total_heap')))} | "
                f"{fmt_bytes(int(median_field(cell, 'v8_used_heap')))} |"
            )

    # Linear regression: per-isolate RSS slope.
    lines.append("")
    lines.append("### Per-isolate cost via linear regression")
    lines.append("")
    lines.append("Fits `metric = base + per_isolate × N` across the count series.")
    lines.append("`per_isolate` is the slope — the true incremental cost of one isolate.")
    lines.append("")
    lines.append("| Pattern | Metric | Base (N=0) | Per-isolate | R² |")
    lines.append("|---|---|---|---|---|")
    for p in patterns:
        for metric_name, *path in [
            ("RSS-max", "rss_max_kb"),
            ("smaps-RSS", "smaps_rss"),
            ("smaps-PSS", "smaps_pss"),
            ("V8 used-heap", "v8_used_heap"),
        ]:
            xs, ys = [], []
            for n in counts:
                cell = rows.get((p, n), [])
                if not cell: continue
                v = median_field(cell, *path)
                if v is None: continue
                if metric_name == "RSS-max":
                    v = v * 1024  # kB → B
                xs.append(n)
                ys.append(v)
            if len(xs) < 3: continue
            slope, intercept, r2 = linreg(xs, ys)
            if slope is None: continue
            lines.append(
                f"| {p} | {metric_name} | "
                f"{fmt_bytes(int(intercept))} | "
                f"{fmt_bytes(int(slope))} | "
                f"{r2:.3f} |"
            )

    return "\n".join(lines)


def summarize_p3_heap(text):
    start = text.rfind("{")
    end = text.rfind("}")
    if start == -1 or end == -1:
        return "## P3 — Heap classifier\n\n_(no parseable JSON block)_"
    try:
        data = json.loads(text[start:end + 1])
    except json.JSONDecodeError:
        return "## P3 — Heap classifier\n\n_(unparseable JSON)_"
    lines = [
        "## P3 — Heap classifier (per-isolate heap stages)",
        "",
        "Single isolate, four stages of increasing user activity.",
        "",
        "| Stage | Used heap | Δ from previous |",
        "|---|---|---|",
    ]
    ordered = [
        ("stage_0_post_isolate", "isolate, no context"),
        ("stage_1_post_context", "+ context"),
        ("stage_2_post_minimal", "+ minimal JS"),
        ("stage_3_post_realistic", "+ realistic handler"),
    ]
    prev = None
    for key, label in ordered:
        if key not in data: continue
        cur = data[key]
        delta = "—" if prev is None else f"+{fmt_bytes(cur - prev)}"
        lines.append(f"| {label} | {fmt_bytes(cur)} | {delta} |")
        prev = cur
    if "stage_0_post_isolate" in data and "stage_3_post_realistic" in data:
        s0 = data["stage_0_post_isolate"]; s3 = data["stage_3_post_realistic"]
        pct = 100.0 * s0 / s3 if s3 else 0
        lines.append("")
        lines.append(
            f"**COW upper bound (V8 used-heap basis):** stage_0 = "
            f"{fmt_bytes(s0)} of {fmt_bytes(s3)} = "
            f"{pct:.1f}% of per-isolate used-heap is the cleanly shareable "
            f"snapshot-derived portion."
        )
    return "\n".join(lines)


def summarize_p3_prototype(text):
    return "\n".join([
        "## P3 — Standalone COW prototype (synthetic)",
        "",
        "Data-structure proof; not comparable to V8 numbers.",
        "",
        "```",
        text.strip(),
        "```",
    ])


def env_block(env_path):
    if not os.path.exists(env_path):
        return ""
    return "\n".join([
        "## Environment", "", "```",
        Path(env_path).read_text(encoding="utf-8", errors="ignore").strip(),
        "```",
    ])


# ---------- main ----------

def collect_runs(base):
    """Find all run_N/ subdirs under base, or treat base itself as a single run."""
    base = Path(base)
    runs_dirs = sorted(base.glob("run_*/"))
    if not runs_dirs:
        runs_dirs = [base]
    out = {"p1": [], "p2": [], "p3_memory": []}
    last_heap_text = None
    last_proto_text = None
    last_env = None
    for d in runs_dirs:
        for k in out.keys():
            f = d / f"{k}.json"
            if f.exists():
                try:
                    out[k].append(json.loads(f.read_text(encoding="utf-8")))
                except json.JSONDecodeError:
                    pass
        if (d / "p3_heap_classifier.txt").exists():
            last_heap_text = (d / "p3_heap_classifier.txt").read_text(encoding="utf-8", errors="ignore")
        if (d / "p3_prototype.txt").exists():
            last_proto_text = (d / "p3_prototype.txt").read_text(encoding="utf-8", errors="ignore")
        if (d / "env.txt").exists():
            last_env = d / "env.txt"
    return out, last_heap_text, last_proto_text, last_env


def main():
    base = Path(sys.argv[1] if len(sys.argv) > 1 else "results")
    if not base.exists():
        print(f"no results dir at {base}", file=sys.stderr); sys.exit(1)

    runs, heap_text, proto_text, env_path = collect_runs(base)
    n = max(len(runs["p1"]), len(runs["p2"]), len(runs["p3_memory"]))

    parts = [
        "# Benchmark Results",
        "",
        f"**Runs aggregated:** {n}.  V8 14.7.173.16 (workerd's pinned rev).",
        "Reproduce: `docker build -t kenton-bench . && for i in 1 2 3; do "
        "mkdir -p results/run_$i && docker run --rm "
        "-v \"$(pwd)/results/run_$i:/work/results\" -e KENTON_SEED=$i "
        "kenton-bench; done && python3 scripts/summarize.py`",
        "",
    ]
    parts.append(summarize_p1(runs["p1"])); parts.append("")
    parts.append(summarize_p2(runs["p2"])); parts.append("")
    parts.append(summarize_p3_memory(runs["p3_memory"])); parts.append("")
    if heap_text: parts.append(summarize_p3_heap(heap_text)); parts.append("")
    if proto_text: parts.append(summarize_p3_prototype(proto_text)); parts.append("")
    if env_path: parts.append(env_block(env_path))

    out = "\n".join(parts).rstrip() + "\n"
    out_path = base / "SUMMARY.md"
    out_path.write_text(out, encoding="utf-8")
    print(f"wrote {out_path}")
    try: sys.stdout.reconfigure(encoding="utf-8")
    except Exception: pass
    print()
    print(out)


if __name__ == "__main__":
    main()
