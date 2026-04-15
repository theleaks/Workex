//! Workex Benchmark Runner
//!
//! Runs ALL 3 runtimes every time: Workex + V8 (Node.js) + CF Workers (Miniflare).
//! Same 8 benchmarks across all runtimes. No static estimates.
//!
//! Usage:
//!   cargo run -p workex-bench --release              # Standard
//!   cargo run -p workex-bench --release -- --fast     # Quick
//!   cargo run -p workex-bench --release -- --heavy    # Publication-grade
//!   cargo run -p workex-bench --release -- --compare  # Compare last two versions
//!   cargo run -p workex-bench --release -- --list     # List saved versions

use workex_bench::measure::BenchConfig;
use workex_bench::results;
use workex_bench::v8_baseline;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--list") {
        return cmd_list();
    }
    if args.iter().any(|a| a == "--compare") {
        return cmd_compare(&args);
    }

    let (config, mode) = if args.iter().any(|a| a == "--fast") {
        (BenchConfig::fast(), "fast")
    } else if args.iter().any(|a| a == "--heavy") {
        (BenchConfig::heavy(), "heavy")
    } else {
        (BenchConfig::standard(), "standard")
    };

    cmd_run(&config, mode)
}

fn cmd_run(config: &BenchConfig, mode: &str) -> anyhow::Result<()> {
    let version = results::next_version();

    println!();
    println!("+======================================================+");
    println!("|  Workex Benchmark Suite — 3-Way Comparison            |");
    println!("|  Version: {:<44}|", &version);
    println!("|  Mode:    {:<44}|", mode);
    println!("+======================================================+");

    // ── 1/3: Workex ──
    println!();
    println!("[WORKEX] Running benchmarks...");
    let workex = workex_bench::benchmarks::run_all(config);

    // ── 2/3: V8 (Node.js) ──
    println!();
    println!("[V8] Running Node.js benchmarks...");
    let v8 = match v8_baseline::run_v8(mode) {
        Ok(r) => {
            println!("[V8] Done (Node {} / V8 {})",
                r.node_version.as_deref().unwrap_or("?"),
                r.v8_version.as_deref().unwrap_or("?"));
            Some(r)
        }
        Err(e) => { println!("[V8] FAILED: {e}"); None }
    };

    // ── 3/3: CF Workers (Miniflare) ──
    println!();
    println!("[WORKERS] Running Miniflare (workerd) benchmarks...");
    let workers = match v8_baseline::run_workers(mode) {
        Ok(r) => { println!("[WORKERS] Done"); Some(r) }
        Err(e) => { println!("[WORKERS] FAILED: {e}"); None }
    };

    // ── Build comparison ──
    let comparison = match (&v8, &workers) {
        (Some(v), Some(w)) => v8_baseline::build_comparison(&workex, v, w),
        _ => Vec::new(),
    };

    let report = results::BenchReport {
        version: version.clone(),
        timestamp: chrono_now(),
        machine: results::current_machine_info(),
        benchmarks: workex,
        comparison,
        v8_raw: v8.clone(),
        workers_raw: workers.clone(),
    };

    // ── Print 3-way table ──
    match (&v8, &workers) {
        (Some(v), Some(w)) => {
            v8_baseline::print_table(&report.benchmarks, v, w);
        }
        _ => {
            println!();
            println!("(3-way comparison incomplete — missing V8 or Workers results)");
        }
    }

    // ── Save ──
    let path = results::save_report(&report)?;
    println!();
    println!("Saved: {}", path.display());

    // ── Delta vs previous ──
    let versions = results::list_versions();
    if versions.len() >= 2 {
        let prev = &versions[versions.len() - 2];
        println!();
        println!("+-------------------------------------------------+");
        println!("|  Workex delta vs {:<32}|", prev);
        println!("+-------------------------------------------------+");
        if let Ok(old) = results::load_report(prev) {
            for c in &results::compare(&old, &report) {
                println!("{c}");
            }
        }
    }

    println!();
    Ok(())
}

fn cmd_list() -> anyhow::Result<()> {
    let versions = results::list_versions();
    if versions.is_empty() {
        println!("No benchmark results found.");
        return Ok(());
    }
    println!("Saved benchmark versions:");
    for v in &versions {
        if let Ok(r) = results::load_report(v) {
            let v8 = if r.v8_raw.is_some() { "+V8" } else { "" };
            let wk = if r.workers_raw.is_some() { "+Workers" } else { "" };
            println!("  {v}  ({} benchmarks {v8} {wk}, {})", r.benchmarks.len(), r.timestamp);
        }
    }
    Ok(())
}

fn cmd_compare(args: &[String]) -> anyhow::Result<()> {
    let idx = args.iter().position(|a| a == "--compare").unwrap();
    let rest: Vec<&str> = args[idx+1..].iter().filter(|a| !a.starts_with('-')).map(|s| s.as_str()).collect();

    let (old_v, new_v) = if rest.len() >= 2 {
        (rest[0].to_string(), rest[1].to_string())
    } else {
        let versions = results::list_versions();
        if versions.len() < 2 { anyhow::bail!("Need at least 2 versions."); }
        (versions[versions.len()-2].clone(), versions[versions.len()-1].clone())
    };

    let old = results::load_report(&old_v)?;
    let new = results::load_report(&new_v)?;

    println!("Comparing {old_v} vs {new_v}");
    println!("{}", "=".repeat(100));
    for c in &results::compare(&old, &new) { println!("{c}"); }

    if let (Some(v), Some(w)) = (&new.v8_raw, &new.workers_raw) {
        v8_baseline::print_table(&new.benchmarks, v, w);
    }

    Ok(())
}

fn chrono_now() -> String {
    std::process::Command::new("date").arg("+%Y-%m-%dT%H:%M:%S").output()
        .ok().filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
