//! 1M Isolate Benchmark — the killer number.
//!
//! Matthew Prince: "24 million simultaneous sessions"
//! V8: 183KB × 1M = 183GB → impossible on single machine
//! Workex: ? × 1M = ? → let's find out
//!
//! Usage: cargo run -p workex-bench --release --bin million-bench

use std::sync::Arc;
use std::time::Instant;

use workex_core::isolate::{Isolate, IsolateEnv, ModuleHandle};
use workex_core::rss;

const TOTAL: usize = 1_000_000;
const BATCH: usize = 50_000;
const ARENA_SIZE: usize = 4 * 1024;

fn main() {
    println!();
    println!("+======================================================+");
    println!("|  1,000,000 Isolate Benchmark                          |");
    println!("|  Arena: {}KB per isolate                               |", ARENA_SIZE / 1024);
    println!("+======================================================+");
    println!();

    let module = Arc::new(ModuleHandle {
        source_hash: 0x1111,
        handler_names: vec!["fetch".into()],
    });

    let rss_before = rss::get_rss_bytes();
    let start = Instant::now();

    let env = IsolateEnv::default();
    let mut isolates: Vec<Isolate> = Vec::with_capacity(TOTAL);

    for batch in 0..(TOTAL / BATCH) {
        for _ in 0..BATCH {
            let mut iso = Isolate::new_minimal(module.clone(), env.clone());
            iso.arena.alloc(0u64);
            isolates.push(iso);
        }

        let count = (batch + 1) * BATCH;
        let current_rss = rss::get_rss_bytes();
        let elapsed = start.elapsed();
        println!(
            "  {:>9} isolates | {:>7} MB RSS | {:>6.1} KB/iso | {:.1?}",
            count,
            current_rss / 1024 / 1024,
            (current_rss - rss_before) as f64 / count as f64 / 1024.0,
            elapsed,
        );
    }

    let elapsed = start.elapsed();
    let rss_after = rss::get_rss_bytes();
    let delta = rss_after.saturating_sub(rss_before);
    let per_iso = delta / TOTAL;

    // Keep alive
    std::hint::black_box(&isolates);

    println!();
    println!("+======================================================================+");
    println!("|  1M ISOLATE RESULTS                                                  |");
    println!("+======================================================================+");
    println!("  Isolate count:     {:>12}", TOTAL);
    println!("  Arena size:        {:>10} KB", ARENA_SIZE / 1024);
    println!("  RSS before:        {:>10} MB", rss_before / 1024 / 1024);
    println!("  RSS after:         {:>10} MB", rss_after / 1024 / 1024);
    println!("  RSS delta:         {:>10} MB", delta / 1024 / 1024);
    println!("  Per isolate:       {:>10} KB ({} bytes)", per_iso / 1024, per_iso);
    println!("  Spawn time:        {:>10.2?}", elapsed);
    println!("  Spawn rate:        {:>10.0} isolates/sec", TOTAL as f64 / elapsed.as_secs_f64());
    println!("+======================================================================+");

    // V8 extrapolation
    let v8_per_kb = 183.0; // from our measured 10K benchmark
    let v8_1m_gb = v8_per_kb * 1_000_000.0 / 1024.0 / 1024.0;
    let workex_1m_gb = delta as f64 / 1024.0 / 1024.0 / 1024.0;

    println!();
    println!("+======================================================================+");
    println!("|  1M EXTRAPOLATION — Workex vs V8                                     |");
    println!("+======================================================================+");
    println!("  {:<25} {:>12} {:>12} {:>10}", "METRIC", "WORKEX", "V8*", "FACTOR");
    println!("  {}", "-".repeat(60));
    println!("  {:<25} {:>10.1} GB {:>10.1} GB {:>9.1}x",
        "1M isolates RAM",
        workex_1m_gb, v8_1m_gb,
        v8_1m_gb / workex_1m_gb);
    println!("  {:<25} {:>10} KB {:>10} KB {:>9.1}x",
        "Per isolate",
        per_iso / 1024, v8_per_kb as u64,
        v8_per_kb * 1024.0 / per_iso as f64);
    println!("  {:<25} {:>10.2?} {:>12}", "Spawn time", elapsed, "N/A");
    println!("+======================================================================+");
    println!("  * V8 extrapolated from measured 10K benchmark (183KB/context)");

    // Save
    let results_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/results");
    let report = serde_json::json!({
        "benchmark": "1m_isolates",
        "count": TOTAL,
        "arena_size_bytes": ARENA_SIZE,
        "rss_before_bytes": rss_before,
        "rss_after_bytes": rss_after,
        "rss_delta_bytes": delta,
        "rss_delta_mb": delta / 1024 / 1024,
        "rss_delta_gb": format!("{:.2}", workex_1m_gb),
        "per_isolate_bytes": per_iso,
        "per_isolate_kb": per_iso / 1024,
        "spawn_time_ms": elapsed.as_millis(),
        "spawn_rate": format!("{:.0}", TOTAL as f64 / elapsed.as_secs_f64()),
        "v8_extrapolated_gb": format!("{:.1}", v8_1m_gb),
        "factor_vs_v8": format!("{:.1}", v8_1m_gb / workex_1m_gb),
    });
    let path = results_dir.join("v4-1m-isolates.json");
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap());
    println!();
    println!("  Saved: {}", path.display());
    println!();

    drop(isolates);
}
