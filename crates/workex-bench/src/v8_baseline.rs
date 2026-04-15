//! Live V8 + Workers benchmarks and 3-way comparison table.
//!
//! Runs Node.js and Miniflare (workerd) scripts every time.
//! All 3 runtimes run the same 8 benchmarks.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::measure::Stats;
use crate::results::BenchEntry;

/// External runtime benchmark results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalBenchResult {
    pub runtime: String,
    #[serde(default)]
    pub node_version: Option<String>,
    #[serde(default)]
    pub v8_version: Option<String>,
    pub benchmarks: BTreeMap<String, ExternalBenchEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalBenchEntry {
    pub stats: Stats,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

fn scripts_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/scripts")
}

/// Run V8 benchmarks via Node.js.
pub fn run_v8(mode: &str) -> anyhow::Result<ExternalBenchResult> {
    let script = scripts_dir().join("v8-bench.mjs");
    let flag = match mode { "fast" => "--fast", "heavy" => "--heavy", _ => "--standard" };

    let output = Command::new("node")
        .arg("--expose-gc")
        .arg(&script)
        .arg(flag)
        .output()
        .map_err(|e| anyhow::anyhow!("node not found: {e}"))?;

    if !output.status.success() {
        anyhow::bail!("V8 bench failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    Ok(serde_json::from_slice(&output.stdout)?)
}

/// Run Workers benchmarks via Miniflare.
pub fn run_workers(mode: &str) -> anyhow::Result<ExternalBenchResult> {
    let script = scripts_dir().join("workers-bench.mjs");
    let flag = match mode { "fast" => "--fast", "heavy" => "--heavy", _ => "--standard" };

    let output = Command::new("node")
        .arg("--expose-gc")
        .arg(&script)
        .arg(flag)
        .output()
        .map_err(|e| anyhow::anyhow!("node not found: {e}"))?;

    if !output.status.success() {
        anyhow::bail!("Workers bench failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    Ok(serde_json::from_slice(&output.stdout)?)
}

/// One row in the 3-way comparison table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TripleComparison {
    pub metric: String,
    pub workex_value: String,
    pub v8_value: String,
    pub workers_value: String,
    pub vs_v8: String,
    pub vs_workers: String,
}

impl std::fmt::Display for TripleComparison {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "  {:<28} {:>12}  {:>12}  {:>12}  {:>8}  {:>8}",
            self.metric, self.workex_value, self.v8_value,
            self.workers_value, self.vs_v8, self.vs_workers,
        )
    }
}

/// Build the 3-way comparison from all results.
pub fn build_comparison(
    workex: &BTreeMap<String, BenchEntry>,
    v8: &ExternalBenchResult,
    workers: &ExternalBenchResult,
) -> Vec<TripleComparison> {
    let mut rows = Vec::new();

    // Standard benchmark names
    let benches = [
        ("cold_start",          "Cold Start"),
        ("warm_exec_add",       "Warm add(a,b)"),
        ("warm_exec_json",      "Warm JSON roundtrip"),
        ("warm_exec_fib35",     "Fibonacci(35)"),
        ("request_throughput",  "Request latency"),
        ("memory_per_isolate",  "Memory / isolate"),
        ("concurrency_10k",     "Concurrency RAM"),
        ("gc_pressure",         "GC/Reset pressure"),
    ];

    for (key, label) in benches {
        let w = workex.get(key);
        let v = v8.benchmarks.get(key);
        let cf = workers.benchmarks.get(key);

        // Median latency row
        rows.push(TripleComparison {
            metric: format!("{label} (p50)"),
            workex_value: w.map(|e| fmt_ns(e.stats.median_ns as f64)).unwrap_or("-".into()),
            v8_value: v.map(|e| fmt_ns(e.stats.median_ns as f64)).unwrap_or("-".into()),
            workers_value: cf.map(|e| fmt_ns(e.stats.median_ns as f64)).unwrap_or("-".into()),
            vs_v8: factor_str(
                w.map(|e| e.stats.median_ns as f64),
                v.map(|e| e.stats.median_ns as f64),
            ),
            vs_workers: factor_str(
                w.map(|e| e.stats.median_ns as f64),
                cf.map(|e| e.stats.median_ns as f64),
            ),
        });

        // p99 tail latency row
        rows.push(TripleComparison {
            metric: format!("{label} (p99)"),
            workex_value: w.map(|e| fmt_ns(e.stats.p99_ns as f64)).unwrap_or("-".into()),
            v8_value: v.map(|e| fmt_ns(e.stats.p99_ns as f64)).unwrap_or("-".into()),
            workers_value: cf.map(|e| fmt_ns(e.stats.p99_ns as f64)).unwrap_or("-".into()),
            vs_v8: factor_str(
                w.map(|e| e.stats.p99_ns as f64),
                v.map(|e| e.stats.p99_ns as f64),
            ),
            vs_workers: factor_str(
                w.map(|e| e.stats.p99_ns as f64),
                cf.map(|e| e.stats.p99_ns as f64),
            ),
        });
    }

    // Memory comparison (from metadata)
    if let (Some(w), Some(v), Some(cf)) = (
        workex.get("memory_per_isolate"),
        v8.benchmarks.get("memory_per_isolate"),
        workers.benchmarks.get("memory_per_isolate"),
    ) {
        let wk = meta_f64(w, "memory_kb");
        let vk = meta_f64_ext(v, "memory_kb");
        let ck = meta_f64_ext(cf, "memory_kb");
        rows.push(TripleComparison {
            metric: "Memory KB / isolate".into(),
            workex_value: wk.map(|v| format!("{v:.0}KB")).unwrap_or("-".into()),
            v8_value: vk.map(|v| format!("{v:.0}KB")).unwrap_or("-".into()),
            workers_value: ck.map(|v| format!("{v:.0}KB")).unwrap_or("-".into()),
            vs_v8: factor_str(wk, vk),
            vs_workers: factor_str(wk, ck),
        });
    }

    rows
}

/// Print the 3-way table.
pub fn print_table(
    workex: &BTreeMap<String, BenchEntry>,
    v8: &ExternalBenchResult,
    workers: &ExternalBenchResult,
) {
    let rows = build_comparison(workex, v8, workers);

    println!();
    println!("+{:-<95}+", "");
    println!("|{:^95}|", " WORKEX  vs  V8 (Node.js)  vs  CF Workers (Miniflare) ");
    println!("+{:-<95}+", "");
    println!(
        "  {:<28} {:>12}  {:>12}  {:>12}  {:>8}  {:>8}",
        "METRIC", "WORKEX", "V8", "WORKERS", "vs V8", "vs WKR"
    );
    println!("  {}", "-".repeat(90));

    for row in &rows {
        println!("{row}");
    }

    println!("+{:-<95}+", "");
}

fn fmt_ns(ns: f64) -> String {
    if ns >= 1e9 { format!("{:.2}s", ns / 1e9) }
    else if ns >= 1e6 { format!("{:.2}ms", ns / 1e6) }
    else if ns >= 1e3 { format!("{:.2}us", ns / 1e3) }
    else { format!("{:.0}ns", ns) }
}

fn factor_str(workex: Option<f64>, other: Option<f64>) -> String {
    match (workex, other) {
        (Some(w), Some(o)) if w > 0.0 && o > 0.0 => {
            let f = o / w;
            if f >= 1.05 { format!("{f:.1}x") }
            else if f <= 0.95 { format!("{f:.2}x") }
            else { "~1x".into() }
        }
        _ => "-".into(),
    }
}

fn meta_f64(e: &BenchEntry, key: &str) -> Option<f64> {
    e.metadata.get(key).and_then(|s| s.parse().ok())
}

fn meta_f64_ext(e: &ExternalBenchEntry, key: &str) -> Option<f64> {
    e.metadata.get(key).and_then(|s| s.parse().ok())
}
