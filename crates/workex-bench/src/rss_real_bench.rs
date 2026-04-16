//! 10K Real Worker RSS Benchmark
//!
//! Each isolate runs an actual Worker (parse + compile + execute).
//! Measures real OS-level RSS, not struct estimates.
//!
//! Usage: cargo run -p workex-bench --release --bin rss-real-bench

use std::path::Path;
use std::time::Instant;

use workex_core::rss;
use workex_runtime::engine::WorkexEngine;
use workex_runtime::request::WorkexRequest;

const COUNT: usize = 10_000;

const WORKER_SOURCE: &str = r#"
export default {
    fetch(request) {
        var url = request.url;
        var body = JSON.stringify({ status: "ok", path: url, ts: Date.now() });
        return new Response(body, {
            headers: { "content-type": "application/json" }
        });
    }
};
"#;

fn main() -> anyhow::Result<()> {
    println!();
    println!("+======================================================+");
    println!("|  10K Real Worker RSS — Actual Code Execution          |");
    println!("+======================================================+");
    println!();

    // ── Workex 10K with real Workers ──
    println!("[WORKEX] Spawning {COUNT} real Workers...");

    let rss_before = rss::get_rss_bytes();
    let start = Instant::now();

    let mut engines: Vec<WorkexEngine> = Vec::with_capacity(COUNT);
    for i in 0..COUNT {
        let mut engine = WorkexEngine::new()?;
        // Actually execute the Worker once
        let req = WorkexRequest::get(&format!("https://example.com/{i}"));
        let resp = engine.execute_worker(WORKER_SOURCE, req)?;
        // Verify correctness on first
        if i == 0 {
            assert_eq!(resp.status, 200);
            let body: serde_json::Value = resp.json_body()?;
            assert_eq!(body["status"], "ok");
            println!("[WORKEX] First Worker verified: status={}, body OK", resp.status);
        }
        engines.push(engine);

        if (i + 1) % 2500 == 0 {
            let current_rss = rss::get_rss_bytes();
            println!(
                "  {} Workers: {} MB RSS",
                i + 1,
                current_rss / 1024 / 1024
            );
        }
    }

    let elapsed = start.elapsed();
    let rss_after = rss::get_rss_bytes();
    let rss_delta = rss_after.saturating_sub(rss_before);
    let per_isolate = rss_delta / COUNT;

    println!("[WORKEX] RSS before:  {} MB", rss_before / 1024 / 1024);
    println!("[WORKEX] RSS after:   {} MB", rss_after / 1024 / 1024);
    println!("[WORKEX] RSS delta:   {} MB", rss_delta / 1024 / 1024);
    println!("[WORKEX] Per Worker:  {} KB", per_isolate / 1024);
    println!("[WORKEX] Time:        {:.2?}", elapsed);

    // Keep alive
    std::hint::black_box(&engines);
    drop(engines);

    // ── V8 10K with real Workers ──
    println!();
    println!("[V8] Running Node.js 10K real Worker RSS benchmark...");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/scripts/v8-rss-real.mjs");

    let v8_output = std::process::Command::new("node")
        .arg("--expose-gc")
        .arg(&script)
        .output();

    let v8: Option<serde_json::Value> = match v8_output {
        Ok(out) if out.status.success() => serde_json::from_slice(&out.stdout).ok(),
        Ok(out) => {
            eprintln!("[V8] Error: {}", String::from_utf8_lossy(&out.stderr));
            None
        }
        Err(e) => { eprintln!("[V8] Failed: {e}"); None }
    };

    let (v8_delta_mb, v8_per_kb) = if let Some(ref v) = v8 {
        let d = v["rss_delta_bytes"].as_u64().unwrap_or(0);
        let p = v["rss_per_context_bytes"].as_u64().unwrap_or(0);
        println!("[V8] RSS delta:   {} MB", d / 1024 / 1024);
        println!("[V8] Per context: {} KB", p / 1024);
        (d as f64 / 1024.0 / 1024.0, p as f64 / 1024.0)
    } else {
        (0.0, 0.0)
    };

    // ── 3/3: Workers (Miniflare/workerd) ──
    println!();
    println!("[WORKERS] Running Miniflare RSS benchmark (100 workers, extrapolated)...");
    let workers_script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/scripts/workers-rss-real.mjs");

    let workers_output = std::process::Command::new("node")
        .arg("--expose-gc")
        .arg(&workers_script)
        .arg("100")
        .output();

    let workers: Option<serde_json::Value> = match workers_output {
        Ok(out) if out.status.success() => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.is_empty() { eprint!("{stderr}"); }
            serde_json::from_slice(&out.stdout).ok()
        }
        Ok(out) => {
            eprintln!("[WORKERS] Error: {}", String::from_utf8_lossy(&out.stderr));
            None
        }
        Err(e) => { eprintln!("[WORKERS] Failed: {e}"); None }
    };

    let (wkr_per_kb, wkr_10k_mb) = if let Some(ref w) = workers {
        let p = w["rss_per_worker_bytes"].as_u64().unwrap_or(0);
        let ext = w["extrapolated_10k_mb"].as_str().unwrap_or("0").parse::<f64>().unwrap_or(0.0);
        println!("[WORKERS] Per Worker:      {} KB (measured on {})", p / 1024, w["actual_count"]);
        println!("[WORKERS] 10K extrapolated: {:.0} MB", ext);
        (p as f64 / 1024.0, ext)
    } else {
        (0.0, 0.0)
    };

    let workex_delta_mb = rss_delta as f64 / 1024.0 / 1024.0;
    let workex_per_kb = per_isolate as f64 / 1024.0;

    // ── 3-Way Comparison ──
    let fac = |w: f64, o: f64| -> String {
        if w > 0.0 && o > 0.0 { format!("{:.1}x", o / w) } else { "-".into() }
    };

    println!();
    println!("+=======================================================================================+");
    println!("|  10K REAL WORKERS — 3-WAY RSS COMPARISON                                              |");
    println!("+=======================================================================================+");
    println!("  {:<25} {:>10} {:>10} {:>12} {:>8} {:>8}",
        "METRIC", "WORKEX", "V8", "WORKERS", "vs V8", "vs WKR");
    println!("  {}", "-".repeat(80));
    println!("  {:<25} {:>8.0} MB {:>8.0} MB {:>10.0} MB {:>8} {:>8}",
        "10K Total RSS",
        workex_delta_mb, v8_delta_mb, wkr_10k_mb,
        fac(workex_delta_mb, v8_delta_mb), fac(workex_delta_mb, wkr_10k_mb));
    println!("  {:<25} {:>8.0} KB {:>8.0} KB {:>10.0} KB {:>8} {:>8}",
        "Per Worker",
        workex_per_kb, v8_per_kb, wkr_per_kb,
        fac(workex_per_kb, v8_per_kb), fac(workex_per_kb, wkr_per_kb));
    println!("  {:<25} {:>10} {:>10} {:>12}",
        "Worker count", COUNT, COUNT, format!("{}*", workers.as_ref().map(|w| w["actual_count"].as_u64().unwrap_or(0)).unwrap_or(0)));
    println!("+=======================================================================================+");
    println!("  * Workers count is actual measured, RSS extrapolated to 10K");

    // ── Save ──
    let results_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/results");
    let report = serde_json::json!({
        "benchmark": "10k_real_worker_rss",
        "count": COUNT,
        "worker_source": "JSON API handler (parse URL, stringify response)",
        "workex": {
            "rss_delta_bytes": rss_delta,
            "rss_delta_mb": format!("{:.1}", workex_delta_mb),
            "rss_per_worker_bytes": per_isolate,
            "rss_per_worker_kb": format!("{:.1}", workex_per_kb),
            "spawn_time_ms": elapsed.as_millis(),
        },
        "v8": v8,
        "workers": workers,
    });
    let path = results_dir.join("v3-real-workers.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report)?)?;
    println!();
    println!("Saved: {}", path.display());
    println!();

    Ok(())
}
