//! Versioned benchmark result storage and comparison.
//!
//! Results are saved as JSON to `benchmarks/results/vN.json`.
//! Each run auto-detects the next version number.
//! Comparison mode shows deltas between any two versions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::measure::Stats;
use crate::v8_baseline::{ExternalBenchResult, TripleComparison};

/// Full benchmark report for one run.
#[derive(Debug, Serialize, Deserialize)]
pub struct BenchReport {
    pub version: String,
    pub timestamp: String,
    pub machine: MachineInfo,
    pub benchmarks: BTreeMap<String, BenchEntry>,
    #[serde(default)]
    pub comparison: Vec<TripleComparison>,
    #[serde(default)]
    pub v8_raw: Option<ExternalBenchResult>,
    #[serde(default)]
    pub workers_raw: Option<ExternalBenchResult>,
}

/// Machine information for reproducibility.
#[derive(Debug, Serialize, Deserialize)]
pub struct MachineInfo {
    pub os: String,
    pub arch: String,
    pub rustc: String,
}

/// A single benchmark entry with stats and optional metadata.
#[derive(Debug, Serialize, Deserialize)]
pub struct BenchEntry {
    pub stats: Stats,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// Get the results directory path.
pub fn results_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/results")
}

/// Detect the next version number by scanning existing files.
pub fn next_version() -> String {
    let dir = results_dir();
    if !dir.exists() {
        return "v1".to_string();
    }

    let mut max = 0u32;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(num_str) = name.strip_prefix('v').and_then(|s| s.strip_suffix(".json")) {
                if let Ok(n) = num_str.parse::<u32>() {
                    max = max.max(n);
                }
            }
        }
    }
    format!("v{}", max + 1)
}

/// Load a benchmark report from a version file.
pub fn load_report(version: &str) -> anyhow::Result<BenchReport> {
    let path = results_dir().join(format!("{version}.json"));
    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
    Ok(serde_json::from_str(&content)?)
}

/// Save a benchmark report.
pub fn save_report(report: &BenchReport) -> anyhow::Result<PathBuf> {
    let dir = results_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", report.version));
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&path, &json)?;
    Ok(path)
}

/// List all available versions.
pub fn list_versions() -> Vec<String> {
    let dir = results_dir();
    let mut versions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(v) = name.strip_suffix(".json") {
                versions.push(v.to_string());
            }
        }
    }
    versions.sort();
    versions
}

/// Get machine info for the current system.
pub fn current_machine_info() -> MachineInfo {
    let rustc = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    MachineInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        rustc,
    }
}

/// Comparison between two benchmark entries.
pub struct Comparison {
    pub name: String,
    pub old_mean_ns: f64,
    pub new_mean_ns: f64,
    pub old_p99_ns: u64,
    pub new_p99_ns: u64,
    pub speedup: f64,      // >1.0 = faster
    pub p99_speedup: f64,
}

impl std::fmt::Display for Comparison {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let arrow = if self.speedup > 1.05 {
            "FASTER"
        } else if self.speedup < 0.95 {
            "SLOWER"
        } else {
            "~same"
        };

        write!(
            f,
            "  {:<35} mean: {:>10.0}ns -> {:>10.0}ns  ({:>6.2}x {})  p99: {:>10}ns -> {:>10}ns",
            self.name,
            self.old_mean_ns,
            self.new_mean_ns,
            self.speedup,
            arrow,
            self.old_p99_ns,
            self.new_p99_ns,
        )
    }
}

/// Compare two reports and return per-benchmark comparisons.
pub fn compare(old: &BenchReport, new: &BenchReport) -> Vec<Comparison> {
    let mut comparisons = Vec::new();

    for (name, new_entry) in &new.benchmarks {
        if let Some(old_entry) = old.benchmarks.get(name) {
            let speedup = if new_entry.stats.mean_ns > 0.0 {
                old_entry.stats.mean_ns / new_entry.stats.mean_ns
            } else {
                f64::INFINITY
            };
            let p99_speedup = if new_entry.stats.p99_ns > 0 {
                old_entry.stats.p99_ns as f64 / new_entry.stats.p99_ns as f64
            } else {
                f64::INFINITY
            };

            comparisons.push(Comparison {
                name: name.clone(),
                old_mean_ns: old_entry.stats.mean_ns,
                new_mean_ns: new_entry.stats.mean_ns,
                old_p99_ns: old_entry.stats.p99_ns,
                new_p99_ns: new_entry.stats.p99_ns,
                speedup,
                p99_speedup,
            });
        }
    }

    comparisons
}

/// Print comparison report between two versions.
pub fn print_comparison(old_version: &str, new_version: &str) -> anyhow::Result<()> {
    let old = load_report(old_version)?;
    let new = load_report(new_version)?;

    println!("Comparing {old_version} vs {new_version}");
    println!("{}", "=".repeat(100));

    let comparisons = compare(&old, &new);
    for c in &comparisons {
        println!("{c}");
    }
    println!("{}", "=".repeat(100));

    Ok(())
}
