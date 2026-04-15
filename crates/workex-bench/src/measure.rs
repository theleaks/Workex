//! Statistical measurement framework for benchmarks.
//!
//! - Configurable warmup + measurement phases
//! - Collects all individual samples
//! - Computes mean, median, stddev, p50/p95/p99, min, max

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Configuration for a benchmark run.
pub struct BenchConfig {
    /// Number of warmup iterations (discarded).
    pub warmup: u64,
    /// Number of measured iterations.
    pub iterations: u64,
}

impl BenchConfig {
    pub fn new(warmup: u64, iterations: u64) -> Self {
        Self { warmup, iterations }
    }

    /// Fast config for quick checks.
    pub fn fast() -> Self {
        Self::new(100, 1_000)
    }

    /// Standard config for proper measurements.
    pub fn standard() -> Self {
        Self::new(1_000, 10_000)
    }

    /// Heavy config for publication-grade results.
    pub fn heavy() -> Self {
        Self::new(5_000, 50_000)
    }
}

/// Statistical summary of a benchmark run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub iterations: u64,
    pub warmup: u64,
    /// All values in nanoseconds.
    pub mean_ns: f64,
    pub median_ns: f64,
    pub stddev_ns: f64,
    pub min_ns: u64,
    pub max_ns: u64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    /// Total wall-clock time for the measured phase.
    pub total_ns: u64,
}

impl Stats {
    pub fn mean(&self) -> Duration {
        Duration::from_nanos(self.mean_ns as u64)
    }

    pub fn median(&self) -> Duration {
        Duration::from_nanos(self.median_ns as u64)
    }

    pub fn p95(&self) -> Duration {
        Duration::from_nanos(self.p95_ns)
    }

    pub fn p99(&self) -> Duration {
        Duration::from_nanos(self.p99_ns)
    }
}

impl std::fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "mean={:.2?}  median={:.2?}  p95={:.2?}  p99={:.2?}  stddev={:.2?}  min={:.2?}  max={:.2?}",
            self.mean(),
            self.median(),
            self.p95(),
            self.p99(),
            Duration::from_nanos(self.stddev_ns as u64),
            Duration::from_nanos(self.min_ns),
            Duration::from_nanos(self.max_ns),
        )
    }
}

/// Run a benchmark function with warmup and statistical collection.
pub fn bench<F: FnMut()>(config: &BenchConfig, mut f: F) -> Stats {
    // Warmup phase
    for _ in 0..config.warmup {
        f();
    }

    // Measurement phase
    let mut samples = Vec::with_capacity(config.iterations as usize);
    let wall_start = Instant::now();

    for _ in 0..config.iterations {
        let t = Instant::now();
        f();
        samples.push(t.elapsed().as_nanos() as u64);
    }

    let total_ns = wall_start.elapsed().as_nanos() as u64;

    compute_stats(samples, config.warmup, config.iterations, total_ns)
}

/// Compute statistics from collected samples.
fn compute_stats(mut samples: Vec<u64>, warmup: u64, iterations: u64, total_ns: u64) -> Stats {
    samples.sort_unstable();

    let n = samples.len() as f64;
    let sum: u64 = samples.iter().sum();
    let mean = sum as f64 / n;

    let variance = samples.iter().map(|&s| {
        let diff = s as f64 - mean;
        diff * diff
    }).sum::<f64>() / n;
    let stddev = variance.sqrt();

    let median = percentile(&samples, 50.0);
    let p50 = percentile(&samples, 50.0);
    let p95 = percentile(&samples, 95.0);
    let p99 = percentile(&samples, 99.0);

    Stats {
        iterations,
        warmup,
        mean_ns: mean,
        median_ns: median as f64,
        stddev_ns: stddev,
        min_ns: samples[0],
        max_ns: *samples.last().unwrap(),
        p50_ns: p50,
        p95_ns: p95,
        p99_ns: p99,
        total_ns,
    }
}

fn percentile(sorted: &[u64], pct: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((pct / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_stats() {
        let config = BenchConfig::new(10, 100);
        let stats = bench(&config, || {
            std::hint::black_box(42u64 * 42u64);
        });

        assert_eq!(stats.iterations, 100);
        assert!(stats.mean_ns > 0.0 || stats.min_ns == 0); // could be too fast to measure
        assert!(stats.p99_ns >= stats.p50_ns);
        assert!(stats.max_ns >= stats.min_ns);
    }
}
