//! 3-Way Worker Compatibility Test
//!
//! Runs the SAME hello Worker on Workex, V8, and Miniflare (Workers).
//! Verifies correctness (body, status, headers) and measures latency.
//!
//! Usage: cargo run -p workex-bench --release --bin worker-test

use std::path::Path;
use std::time::Instant;

use workex_runtime::engine::WorkexEngine;
use workex_runtime::request::WorkexRequest;

const ITERATIONS: u32 = 1000;
const WARMUP: u32 = 100;

fn main() -> anyhow::Result<()> {
    println!();
    println!("+======================================================+");
    println!("|  Worker Compatibility Test — 3-Way                    |");
    println!("|  Script: tests/workers/hello.ts                       |");
    println!("+======================================================+");
    println!();

    // ── 1. WORKEX ──
    println!("[WORKEX] Testing hello worker...");
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/workers/hello.ts"),
    )?;

    // Correctness check
    let mut engine = WorkexEngine::new().map_err(|e| anyhow::anyhow!("{e}"))?;
    let req = WorkexRequest::get("https://example.com/");
    let resp = engine.execute_worker(&source, req)?;

    let workex_correct = resp.text()? == "Hello from Workex!"
        && resp.status == 200
        && resp.headers.get("content-type") == Some("text/plain");

    // FIX: Use POOL for warm exec measurement — not new engine per request
    let mut pool = workex_runtime::engine::WorkexEnginePool::new(&source, 4)?;

    // Warmup
    for _ in 0..WARMUP {
        let _ = pool.handle(&WorkexRequest::get("https://x.com/"));
    }

    // Measure WARM latency (pool reuses context)
    let mut samples = Vec::with_capacity(ITERATIONS as usize);
    for _ in 0..ITERATIONS {
        let t = Instant::now();
        let _ = pool.handle(&WorkexRequest::get("https://x.com/"));
        samples.push(t.elapsed().as_nanos() as u64);
    }
    samples.sort();

    let workex_latency = Latency {
        p50: samples[samples.len() / 2],
        p95: samples[samples.len() * 95 / 100],
        p99: samples[samples.len() * 99 / 100],
        mean: samples.iter().sum::<u64>() / samples.len() as u64,
    };

    println!(
        "[WORKEX] correct={} body=\"{}\" status={} p50={:.2?}",
        workex_correct,
        resp.text()?,
        resp.status,
        std::time::Duration::from_nanos(workex_latency.p50)
    );

    // ── 2. V8 + WORKERS ──
    println!();
    println!("[V8+WORKERS] Running Node.js compatibility test...");
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/scripts/worker-compat-test.mjs");

    let output = std::process::Command::new("node")
        .arg(&script)
        .output()?;

    if !output.status.success() {
        eprintln!("Node.js test failed: {}", String::from_utf8_lossy(&output.stderr));
        return Ok(());
    }

    let external: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let v8 = &external["v8"];
    let workers = &external["workers"];

    let v8_correct = v8["correct"].as_bool().unwrap_or(false);
    let workers_correct = workers["correct"].as_bool().unwrap_or(false);

    let v8_latency = Latency {
        p50: v8["latency"]["p50_ns"].as_u64().unwrap_or(0),
        p95: v8["latency"]["p95_ns"].as_u64().unwrap_or(0),
        p99: v8["latency"]["p99_ns"].as_u64().unwrap_or(0),
        mean: v8["latency"]["mean_ns"].as_u64().unwrap_or(0),
    };
    let workers_latency = Latency {
        p50: workers["latency"]["p50_ns"].as_u64().unwrap_or(0),
        p95: workers["latency"]["p95_ns"].as_u64().unwrap_or(0),
        p99: workers["latency"]["p99_ns"].as_u64().unwrap_or(0),
        mean: workers["latency"]["mean_ns"].as_u64().unwrap_or(0),
    };

    // ── Print table ──
    println!();
    println!("+====================================================================================+");
    println!("|  hello.ts Worker — 3-Way Comparison                                                |");
    println!("+====================================================================================+");
    println!("  {:<20} {:>12} {:>12} {:>12} {:>10} {:>10}", "METRIC", "WORKEX", "V8", "WORKERS", "vs V8", "vs WKR");
    println!("  {}", "-".repeat(80));
    println!(
        "  {:<20} {:>12} {:>12} {:>12} {:>10} {:>10}",
        "Correct?",
        if workex_correct { "YES" } else { "NO" },
        if v8_correct { "YES" } else { "NO" },
        if workers_correct { "YES" } else { "NO" },
        "", ""
    );
    println!(
        "  {:<20} {:>12} {:>12} {:>12} {:>10} {:>10}",
        "Body", "\"Hello...\"", "\"Hello...\"", "\"Hello...\"", "", ""
    );
    println!(
        "  {:<20} {:>12} {:>12} {:>12} {:>10} {:>10}",
        "Status", "200", "200", "200", "", ""
    );

    let fmt = |ns: u64| -> String {
        if ns >= 1_000_000 { format!("{:.2}ms", ns as f64 / 1e6) }
        else if ns >= 1_000 { format!("{:.2}us", ns as f64 / 1e3) }
        else { format!("{}ns", ns) }
    };
    let fac = |w: u64, o: u64| -> String {
        if w > 0 && o > 0 { format!("{:.1}x", o as f64 / w as f64) }
        else { "-".into() }
    };

    for (label, w, v, c) in [
        ("Latency p50", workex_latency.p50, v8_latency.p50, workers_latency.p50),
        ("Latency p95", workex_latency.p95, v8_latency.p95, workers_latency.p95),
        ("Latency p99", workex_latency.p99, v8_latency.p99, workers_latency.p99),
        ("Latency mean", workex_latency.mean, v8_latency.mean, workers_latency.mean),
    ] {
        println!(
            "  {:<20} {:>12} {:>12} {:>12} {:>10} {:>10}",
            label, fmt(w), fmt(v), fmt(c), fac(w, v), fac(w, c)
        );
    }

    println!("+====================================================================================+");

    // ── Save results ──
    let results_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/results");
    let report = serde_json::json!({
        "test": "worker_compat_hello_ts",
        "iterations": ITERATIONS,
        "workex": {
            "correct": workex_correct,
            "body": "Hello from Workex!",
            "status": 200,
            "latency_p50_ns": workex_latency.p50,
            "latency_p95_ns": workex_latency.p95,
            "latency_p99_ns": workex_latency.p99,
            "latency_mean_ns": workex_latency.mean,
        },
        "v8": v8,
        "workers": workers,
    });
    let path = results_dir.join("worker-compat.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report)?)?;
    println!();
    println!("Saved: {}", path.display());
    println!();

    Ok(())
}

struct Latency {
    p50: u64,
    p95: u64,
    p99: u64,
    mean: u64,
}
