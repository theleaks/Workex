//! 1M Real Execution Contexts — SharedRuntime Architecture
//!
//! Each "isolate" is a real QuickJS Context with Worker source compiled,
//! fetch() handler ready to call, JS engine fully initialized.
//!
//! Architecture:
//!   1000 SharedRuntimes × 1000 Contexts each = 1,000,000 contexts
//!   (Different runtimes simulate different Worker scripts in production)
//!
//! Usage: cargo run -p workex-bench --release --bin million-real-bench

use std::time::Instant;

use workex_core::rss;
use workex_runtime::shared_runtime::SharedRuntime;
use workex_runtime::request::WorkexRequest;

const TOTAL: usize = 1_000_000;
const CONTEXTS_PER_RT: usize = 1_000;
const NUM_RUNTIMES: usize = TOTAL / CONTEXTS_PER_RT;

const WORKER_SOURCE: &str = r#"
export default {
    fetch(request) {
        var body = JSON.stringify({ status: "ok", path: request.url });
        return new Response(body, { headers: { "content-type": "application/json" } });
    }
};
"#;

fn main() -> anyhow::Result<()> {
    println!();
    println!("+======================================================+");
    println!("|  1,000,000 REAL Execution Contexts                    |");
    println!("|  {} SharedRuntimes x {} Contexts each       |", NUM_RUNTIMES, CONTEXTS_PER_RT);
    println!("|  Each context: compiled Worker + fetch() ready        |");
    println!("+======================================================+");
    println!();

    let rss_before = rss::get_rss_bytes();
    let start = Instant::now();

    let mut runtimes: Vec<SharedRuntime> = Vec::with_capacity(NUM_RUNTIMES);

    for i in 0..NUM_RUNTIMES {
        let rt = SharedRuntime::new(WORKER_SOURCE, CONTEXTS_PER_RT)?;

        // Verify first runtime works
        if i == 0 {
            let resp = rt.handle(&WorkexRequest::get("https://example.com/verify"))?;
            assert_eq!(resp.status, 200);
            println!("[OK] First runtime verified: status=200");
        }

        runtimes.push(rt);

        if (i + 1) % 100 == 0 {
            let contexts = (i + 1) * CONTEXTS_PER_RT;
            let current_rss = rss::get_rss_bytes();
            let delta = current_rss.saturating_sub(rss_before);
            let per_ctx = if contexts > 0 { delta / contexts } else { 0 };
            let elapsed = start.elapsed();
            println!(
                "  {:>9} contexts ({:>4} runtimes) | {:>7} MB RSS | {:>5} KB/ctx | {:.1?}",
                contexts,
                i + 1,
                current_rss / 1024 / 1024,
                per_ctx / 1024,
                elapsed,
            );
        }
    }

    let elapsed = start.elapsed();
    let rss_after = rss::get_rss_bytes();
    let delta = rss_after.saturating_sub(rss_before);
    let per_ctx = delta / TOTAL;

    // Keep alive for measurement
    std::hint::black_box(&runtimes);

    // V8 extrapolation (from measured 10K)
    let v8_per_kb: f64 = 178.0; // measured
    let v8_1m_gb = v8_per_kb * TOTAL as f64 / 1024.0 / 1024.0;
    let workex_gb = delta as f64 / 1024.0 / 1024.0 / 1024.0;
    let factor = v8_1m_gb / workex_gb;

    println!();
    println!("+======================================================================+");
    println!("|  1M REAL EXECUTION CONTEXTS — RESULTS                                |");
    println!("+======================================================================+");
    println!("  Total contexts:    {:>12}", TOTAL);
    println!("  SharedRuntimes:    {:>12}", NUM_RUNTIMES);
    println!("  Contexts/runtime:  {:>12}", CONTEXTS_PER_RT);
    println!("  RSS before:        {:>10} MB", rss_before / 1024 / 1024);
    println!("  RSS after:         {:>10} MB", rss_after / 1024 / 1024);
    println!("  RSS delta:         {:>10.1} GB", workex_gb);
    println!("  Per context:       {:>10} KB ({} bytes)", per_ctx / 1024, per_ctx);
    println!("  Spawn time:        {:>10.1?}", elapsed);
    println!("  Spawn rate:        {:>10.0} contexts/sec", TOTAL as f64 / elapsed.as_secs_f64());
    println!("+======================================================================+");
    println!();
    println!("+======================================================================+");
    println!("|  WORKEX vs V8 — 1M Contexts                                          |");
    println!("+======================================================================+");
    println!("  {:<25} {:>12} {:>12} {:>10}", "METRIC", "WORKEX", "V8*", "FACTOR");
    println!("  {}", "-".repeat(60));
    println!("  {:<25} {:>10.1} GB {:>10.1} GB {:>9.1}x",
        "1M contexts RAM", workex_gb, v8_1m_gb, factor);
    println!("  {:<25} {:>10} KB {:>10} KB {:>9.1}x",
        "Per context", per_ctx / 1024, v8_per_kb as u64, v8_per_kb * 1024.0 / per_ctx as f64);
    println!("  {:<25} {:>10.1?}", "Spawn time", elapsed);
    println!("+======================================================================+");
    println!("  * V8 extrapolated from measured 10K benchmark (178KB/context)");
    println!("  * Workex 1M is REAL — all contexts exist in memory during measurement");

    // Save
    let results_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/results");
    std::fs::create_dir_all(&results_dir)?;
    let report = serde_json::json!({
        "benchmark": "1m_real_execution_contexts",
        "architecture": format!("{NUM_RUNTIMES} SharedRuntimes x {CONTEXTS_PER_RT} Contexts"),
        "total_contexts": TOTAL,
        "runtimes": NUM_RUNTIMES,
        "contexts_per_runtime": CONTEXTS_PER_RT,
        "workex": {
            "rss_delta_bytes": delta,
            "rss_delta_gb": format!("{:.2}", workex_gb),
            "per_context_bytes": per_ctx,
            "per_context_kb": per_ctx / 1024,
            "spawn_time_secs": elapsed.as_secs_f64(),
            "spawn_rate": format!("{:.0}", TOTAL as f64 / elapsed.as_secs_f64()),
        },
        "v8_extrapolated": {
            "per_context_kb": v8_per_kb,
            "total_gb": format!("{:.1}", v8_1m_gb),
            "note": "extrapolated from measured 10K vm.createContext() benchmark",
        },
        "factor_vs_v8": format!("{:.1}", factor),
        "note": "Workex 1M is a real allocation. V8 1M is extrapolated.",
    });
    let path = results_dir.join("1m-real-contexts.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report)?)?;
    println!();
    println!("  Saved: {}", path.display());
    println!();

    drop(runtimes);
    Ok(())
}
