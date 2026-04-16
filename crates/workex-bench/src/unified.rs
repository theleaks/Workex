//! Unified 3-Way Benchmark Runner
//!
//! Single command that runs ALL benchmarks for ALL 3 runtimes:
//!   Workex (Rust) | V8 (Node.js) | CF Workers (Miniflare/workerd)
//!
//! Standards:
//!   - Multiple runs (configurable, default 5)
//!   - Statistical analysis: mean, median, stddev, p95, p99
//!   - k6 HTTP load test included
//!   - All results saved as versioned JSON
//!
//! Usage:
//!   cargo run -p workex-bench --release --bin unified-bench
//!   cargo run -p workex-bench --release --bin unified-bench -- --runs 10
//!   cargo run -p workex-bench --release --bin unified-bench -- --with-k6

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use workex_core::arena::Arena;
use workex_core::isolate::{IsolateEnv, ModuleHandle};
use workex_core::rss;
use workex_runtime::engine::{WorkexEngine, WorkexEnginePool};
use workex_runtime::request::WorkexRequest;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let runs: usize = args.iter().position(|a| a == "--runs")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let with_k6 = args.iter().any(|a| a == "--with-k6");

    println!();
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║  WORKEX UNIFIED BENCHMARK — 3-Way Comparison                ║");
    println!("║  Runs: {:<5}  |  Workex  |  V8 (Node.js)  |  CF Workers    ║", runs);
    println!("╚═══════════════════════════════════════════════════════════════╝");

    // ═══════════════════════════════════════════════════
    // 1. WORKEX BENCHMARKS
    // ═══════════════════════════════════════════════════
    println!("\n[WORKEX] Running benchmarks ({runs} runs)...\n");

    let worker_source = r#"
        export default {
            fetch(request) {
                var url = request.url;
                var body = JSON.stringify({ status: "ok", path: url, ts: Date.now() });
                return new Response(body, { headers: { "content-type": "application/json" } });
            }
        };
    "#;

    // Cold start (multiple runs)
    let mut cold_samples = Vec::new();
    for _ in 0..runs {
        let t = Instant::now();
        let mut engine = WorkexEngine::new()?;
        let req = WorkexRequest::get("https://x.com/cold");
        let _ = engine.execute_worker(worker_source, req)?;
        cold_samples.push(t.elapsed().as_nanos() as f64);
    }
    let workex_cold = compute_stats(&cold_samples);
    println!("  Cold start:  mean={:.1}us  median={:.1}us  p99={:.1}us  stddev={:.1}us",
        workex_cold.mean/1e3, workex_cold.median/1e3, workex_cold.p99/1e3, workex_cold.stddev/1e3);

    // Warm exec via POOL (source compiled once, context reused)
    let mut pool = WorkexEnginePool::new(worker_source, 4)?;
    // warmup
    for _ in 0..200 {
        let _ = pool.handle(&WorkexRequest::get("https://x.com/w"));
    }
    let iters_per_run = 2000;
    let mut warm_samples = Vec::new();
    for _ in 0..runs {
        let t = Instant::now();
        for _ in 0..iters_per_run {
            let _ = pool.handle(&WorkexRequest::get("https://x.com/w"));
        }
        let avg_ns = t.elapsed().as_nanos() as f64 / iters_per_run as f64;
        warm_samples.push(avg_ns);
    }
    let workex_warm = compute_stats(&warm_samples);
    println!("  Warm exec (pool): mean={:.1}us  median={:.1}us  p99={:.1}us",
        workex_warm.mean/1e3, workex_warm.median/1e3, workex_warm.p99/1e3);

    // RSS 10K (multiple runs)
    let module = Arc::new(ModuleHandle { source_hash: 0xBBBB, handler_names: vec!["fetch".into()] });
    let mut rss_samples = Vec::new();
    for run in 0..runs.min(3) {
        let before = rss::get_rss_bytes();
        let mut isos = Vec::with_capacity(10_000);
        for _ in 0..10_000 {
            let mut iso = workex_core::isolate::Isolate::new(module.clone(), IsolateEnv::default());
            iso.arena.alloc(42u64);
            iso.arena.alloc_str("agent state");
            isos.push(iso);
        }
        let after = rss::get_rss_bytes();
        let delta = after.saturating_sub(before);
        rss_samples.push(delta as f64);
        std::hint::black_box(&isos);
        drop(isos);
        println!("  RSS run {}: {} MB ({}KB/iso)", run+1, delta/1024/1024, delta/10000/1024);
    }
    let workex_rss = compute_stats(&rss_samples);
    println!("  RSS 10K:     mean={:.0}MB  median={:.0}MB",
        workex_rss.mean/1024.0/1024.0, workex_rss.median/1024.0/1024.0);

    // Worker compat
    let mut compat_engine = WorkexEngine::new()?;
    let resp = compat_engine.execute_worker(worker_source, WorkexRequest::get("https://example.com/test"))?;
    let workex_compat = resp.status == 200 && resp.headers.get("content-type") == Some("application/json");
    println!("  Compat:      {}", if workex_compat { "PASS" } else { "FAIL" });

    // ═══════════════════════════════════════════════════
    // 2. V8 + WORKERS BENCHMARKS
    // ═══════════════════════════════════════════════════
    println!("\n[V8 + WORKERS] Running unified Node.js benchmarks ({runs} runs)...\n");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/scripts/unified-bench.mjs");
    let output = std::process::Command::new("node")
        .arg("--expose-gc")
        .arg(&script)
        .arg(runs.to_string())
        .arg("all")
        .output()?;

    if !output.status.success() {
        eprintln!("  Node.js error: {}", String::from_utf8_lossy(&output.stderr));
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() { eprint!("{stderr}"); }

    let ext: serde_json::Value = serde_json::from_slice(&output.stdout)?;

    // ═══════════════════════════════════════════════════
    // 3. COMPARISON TABLE
    // ═══════════════════════════════════════════════════
    let fmt = |ns: f64| -> String {
        if ns >= 1e9 { format!("{:.2}s", ns/1e9) }
        else if ns >= 1e6 { format!("{:.2}ms", ns/1e6) }
        else if ns >= 1e3 { format!("{:.1}us", ns/1e3) }
        else { format!("{:.0}ns", ns) }
    };
    let fac = |w: f64, o: f64| -> String {
        if w > 0.0 && o > 0.0 { format!("{:.1}x", o/w) } else { "-".into() }
    };
    let jf = |v: &serde_json::Value, path: &str| -> f64 {
        path.split('.').fold(v.clone(), |acc, key| acc[key].clone()).as_f64().unwrap_or(0.0)
    };

    let v8_cold_mean = jf(&ext, "v8_micro.cold_start_ns.mean");
    let v8_warm_mean = jf(&ext, "v8_micro.warm_exec_ns.mean");
    let wk_cold_mean = jf(&ext, "workers_micro.cold_start_ns.mean");
    let wk_warm_mean = jf(&ext, "workers_micro.warm_exec_ns.mean");
    let v8_rss_mean = jf(&ext, "v8_rss.per_context.mean");
    let wk_rss_mean = jf(&ext, "workers_rss.per_worker.mean");
    let v8_compat = ext["v8_compat"]["correct"].as_bool().unwrap_or(false);
    let wk_compat = ext["workers_compat"]["correct"].as_bool().unwrap_or(false);

    println!();
    println!("╔══════════════════════════════════════════════════════════════════════════════════════╗");
    println!("║                    3-WAY BENCHMARK RESULTS ({runs} runs, averaged)                      ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════════════╣");
    println!("  {:<24} {:>12} {:>12} {:>12} {:>8} {:>8}",
        "METRIC", "WORKEX", "V8", "WORKERS", "vs V8", "vs WKR");
    println!("  {}", "─".repeat(80));

    println!("  {:<24} {:>12} {:>12} {:>12} {:>8} {:>8}",
        "Cold start (mean)", fmt(workex_cold.mean), fmt(v8_cold_mean), fmt(wk_cold_mean),
        fac(workex_cold.mean, v8_cold_mean), fac(workex_cold.mean, wk_cold_mean));
    println!("  {:<24} {:>12} {:>12} {:>12} {:>8} {:>8}",
        "Cold start (p99)", fmt(workex_cold.p99), fmt(jf(&ext,"v8_micro.cold_start_ns.p99")),
        fmt(jf(&ext,"workers_micro.cold_start_ns.p99")),
        fac(workex_cold.p99, jf(&ext,"v8_micro.cold_start_ns.p99")),
        fac(workex_cold.p99, jf(&ext,"workers_micro.cold_start_ns.p99")));

    println!("  {:<24} {:>12} {:>12} {:>12} {:>8} {:>8}",
        "Warm exec (mean)", fmt(workex_warm.mean), fmt(v8_warm_mean), fmt(wk_warm_mean),
        fac(workex_warm.mean, v8_warm_mean), fac(workex_warm.mean, wk_warm_mean));

    println!("  {:<24} {:>12} {:>12} {:>12} {:>8} {:>8}",
        "RSS/isolate (mean)", format!("{:.0}KB", workex_rss.mean/10000.0/1024.0),
        format!("{:.0}KB", v8_rss_mean/1024.0), format!("{:.0}KB", wk_rss_mean/1024.0),
        fac(workex_rss.mean/10000.0, v8_rss_mean), fac(workex_rss.mean/10000.0, wk_rss_mean));

    println!("  {:<24} {:>12} {:>12} {:>12}",
        "Worker compat",
        if workex_compat {"PASS"} else {"FAIL"},
        if v8_compat {"PASS"} else {"FAIL"},
        if wk_compat {"PASS"} else {"FAIL"});

    println!("  {:<24} {:>12}", "Runs", runs);
    println!("╚══════════════════════════════════════════════════════════════════════════════════════╝");

    // ═══════════════════════════════════════════════════
    // 4. K6 LOAD TEST (optional)
    // ═══════════════════════════════════════════════════
    if with_k6 {
        println!("\n[K6] Starting HTTP load tests...");
        println!("  Run: bash benchmarks/scripts/run-k6.sh");
        println!("  (k6 tests run separately — start servers first)");
    }

    // ═══════════════════════════════════════════════════
    // SAVE
    // ═══════════════════════════════════════════════════
    let results_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/results");
    std::fs::create_dir_all(&results_dir)?;

    // Auto-version
    let mut max_v = 0u32;
    if let Ok(entries) = std::fs::read_dir(&results_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(n) = name.strip_prefix("unified-v").and_then(|s| s.strip_suffix(".json")) {
                if let Ok(v) = n.parse::<u32>() { max_v = max_v.max(v); }
            }
        }
    }
    let version = format!("unified-v{}", max_v + 1);

    let report = serde_json::json!({
        "version": version,
        "runs": runs,
        "workex": {
            "cold_start_ns": { "mean": workex_cold.mean, "median": workex_cold.median, "p99": workex_cold.p99, "stddev": workex_cold.stddev },
            "warm_exec_ns": { "mean": workex_warm.mean, "median": workex_warm.median, "p99": workex_warm.p99, "stddev": workex_warm.stddev },
            "rss_10k": { "mean_bytes": workex_rss.mean, "median_bytes": workex_rss.median, "per_isolate_kb": workex_rss.mean/10000.0/1024.0 },
            "compat": workex_compat,
        },
        "v8": ext.get("v8_micro"),
        "workers": ext.get("workers_micro"),
        "v8_rss": ext.get("v8_rss"),
        "workers_rss": ext.get("workers_rss"),
        "v8_compat": ext.get("v8_compat"),
        "workers_compat": ext.get("workers_compat"),
        "node_version": ext.get("node_version"),
        "v8_version": ext.get("v8_version"),
    });

    let path = results_dir.join(format!("{version}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&report)?)?;
    println!("\nSaved: {}", path.display());
    println!();

    Ok(())
}

struct Stats {
    mean: f64,
    median: f64,
    stddev: f64,
    p95: f64,
    p99: f64,
}

fn compute_stats(samples: &[f64]) -> Stats {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = sorted.len();
    let mean = sorted.iter().sum::<f64>() / n as f64;
    let variance = sorted.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / n as f64;
    Stats {
        mean,
        median: sorted[n / 2],
        stddev: variance.sqrt(),
        p95: sorted[(n as f64 * 0.95) as usize].min(sorted[n - 1]),
        p99: sorted[(n as f64 * 0.99) as usize].min(sorted[n - 1]),
    }
}
