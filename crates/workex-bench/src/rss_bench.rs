//! RSS Benchmark: 10K Workex isolates vs 10K V8 contexts.
//!
//! Measures REAL OS-level RSS (physical RAM), not struct estimates.
//! Runs Node.js script for V8 comparison.
//!
//! Usage: cargo run -p workex-bench --release --bin rss-bench

use std::path::Path;
use std::sync::Arc;

use workex_core::isolate::{Isolate, IsolateEnv, ModuleHandle};
use workex_core::rss;

const COUNT: usize = 10_000;

fn main() -> anyhow::Result<()> {
    println!();
    println!("+======================================================+");
    println!("|  RSS Benchmark: 10K Isolates — Real Memory            |");
    println!("+======================================================+");
    println!();

    // ── Workex 10K ──
    println!("[WORKEX] Spawning {COUNT} isolates...");

    let module = Arc::new(ModuleHandle {
        source_hash: 0xA551,
        handler_names: vec!["fetch".into()],
    });
    let env = IsolateEnv::default();

    let rss_before = rss::get_rss_bytes();

    let mut isolates: Vec<Isolate> = Vec::with_capacity(COUNT);
    for _ in 0..COUNT {
        let mut iso = Isolate::new(module.clone(), env.clone());
        // Simulate real work — alloc into arena like a Worker would
        iso.arena.alloc(42u64);
        iso.arena.alloc_str("Hello from Workex!");
        isolates.push(iso);
    }

    let rss_after = rss::get_rss_bytes();
    let rss_delta = rss_after.saturating_sub(rss_before);
    let per_isolate = rss_delta / COUNT;

    println!("[WORKEX] RSS before:  {} MB", rss_before / 1024 / 1024);
    println!("[WORKEX] RSS after:   {} MB", rss_after / 1024 / 1024);
    println!("[WORKEX] RSS delta:   {} MB ({} bytes)", rss_delta / 1024 / 1024, rss_delta);
    println!("[WORKEX] Per isolate: {} KB ({} bytes)", per_isolate / 1024, per_isolate);
    println!();

    // Keep isolates alive for measurement
    std::hint::black_box(&isolates);
    drop(isolates);

    // ── V8 10K ──
    println!("[V8] Running Node.js 10K context benchmark...");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/scripts/v8-rss-bench.mjs");

    let v8_output = std::process::Command::new("node")
        .arg("--expose-gc")
        .arg(&script)
        .output();

    let v8_result: Option<serde_json::Value> = match v8_output {
        Ok(out) if out.status.success() => {
            serde_json::from_slice(&out.stdout).ok()
        }
        Ok(out) => {
            eprintln!("[V8] Error: {}", String::from_utf8_lossy(&out.stderr));
            None
        }
        Err(e) => {
            eprintln!("[V8] Failed to run: {e}");
            None
        }
    };

    let v8_delta_mb;
    let v8_per_ctx_kb;

    if let Some(ref v8) = v8_result {
        let delta = v8["rss_delta_bytes"].as_u64().unwrap_or(0);
        let per_ctx = v8["rss_per_context_bytes"].as_u64().unwrap_or(0);
        v8_delta_mb = delta as f64 / 1024.0 / 1024.0;
        v8_per_ctx_kb = per_ctx as f64 / 1024.0;

        println!("[V8] RSS delta:   {:.1} MB", v8_delta_mb);
        println!("[V8] Per context: {:.1} KB", v8_per_ctx_kb);
        println!("[V8] Heap delta:  {} MB", v8["heap_delta_mb"].as_str().unwrap_or("?"));
    } else {
        v8_delta_mb = 0.0;
        v8_per_ctx_kb = 0.0;
        println!("[V8] Skipped (Node.js not available)");
    }

    // ── Comparison ──
    let workex_delta_mb = rss_delta as f64 / 1024.0 / 1024.0;
    let workex_per_kb = per_isolate as f64 / 1024.0;

    println!();
    println!("+======================================================================+");
    println!("|  10K ISOLATES — REAL RSS COMPARISON                                  |");
    println!("+======================================================================+");
    println!("  {:<25} {:>12} {:>12} {:>10}", "METRIC", "WORKEX", "V8 (Node)", "FACTOR");
    println!("  {}", "-".repeat(60));
    println!(
        "  {:<25} {:>10.1} MB {:>10.1} MB {:>9.1}x",
        "Total RSS delta",
        workex_delta_mb,
        v8_delta_mb,
        if workex_delta_mb > 0.0 { v8_delta_mb / workex_delta_mb } else { 0.0 }
    );
    println!(
        "  {:<25} {:>10.1} KB {:>10.1} KB {:>9.1}x",
        "Per isolate/context",
        workex_per_kb,
        v8_per_ctx_kb,
        if workex_per_kb > 0.0 { v8_per_ctx_kb / workex_per_kb } else { 0.0 }
    );
    println!(
        "  {:<25} {:>10} {:>13} {:>10}",
        "Isolate count", COUNT, COUNT, ""
    );
    println!("+======================================================================+");

    // ── Save to JSON ──
    let results_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/results");
    std::fs::create_dir_all(&results_dir)?;

    let report = serde_json::json!({
        "benchmark": "10k_isolate_rss",
        "count": COUNT,
        "workex": {
            "rss_before_bytes": rss_before,
            "rss_after_bytes": rss_after,
            "rss_delta_bytes": rss_delta,
            "rss_delta_mb": format!("{:.1}", workex_delta_mb),
            "rss_per_isolate_bytes": per_isolate,
            "rss_per_isolate_kb": format!("{:.1}", workex_per_kb),
        },
        "v8": v8_result,
        "comparison": {
            "rss_factor": if workex_delta_mb > 0.0 { v8_delta_mb / workex_delta_mb } else { 0.0 },
            "per_isolate_factor": if workex_per_kb > 0.0 { v8_per_ctx_kb / workex_per_kb } else { 0.0 },
        }
    });

    let path = results_dir.join("v2.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report)?)?;
    println!();
    println!("Saved: {}", path.display());
    println!();

    Ok(())
}
