//! Workex benchmarks — standard test suite.
//!
//! Same 8 benchmarks as V8 and Workers scripts.
//! All return BenchEntry with stats + metadata.

use std::collections::BTreeMap;
use std::sync::Arc;

use workex_compiler::codegen::compile_function;
use workex_compiler::hir::{BinOp, Type, TypedExpr, TypedFunction, TypedParam, TypedStmt};
use workex_core::arena::Arena;
use workex_core::isolate::{IsolateEnv, IsolatePool, ModuleHandle};

use crate::measure::{self, BenchConfig, Stats};
use crate::results::BenchEntry;

fn entry(stats: Stats, meta: Vec<(&str, String)>) -> BenchEntry {
    BenchEntry {
        stats,
        metadata: meta.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
    }
}

// ═══════════════════════════════════════════════════════════
// 1. COLD START — create isolate + compile + first execution
// ═══════════════════════════════════════════════════════════
pub fn cold_start(config: &BenchConfig) -> BenchEntry {
    let module = Arc::new(ModuleHandle {
        source_hash: 0xC01D,
        handler_names: vec!["fetch".into()],
    });
    let func = make_add_function();

    let stats = measure::bench(config, || {
        let iso = workex_core::isolate::Isolate::new(module.clone(), IsolateEnv::default());
        let native = compile_function(&func).unwrap();
        let f: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(native.ptr()) };
        std::hint::black_box(f(3.0, 4.0));
        std::hint::black_box(&iso);
    });

    entry(stats, vec![])
}

// ═══════════════════════════════════════════════════════════
// 2. WARM EXEC: add(a,b)
// ═══════════════════════════════════════════════════════════
pub fn warm_exec_add(config: &BenchConfig) -> BenchEntry {
    let func = make_add_function();
    let native = compile_function(&func).unwrap();
    let f: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(native.ptr()) };

    // Heavy warmup for fair comparison
    let heavy = BenchConfig::new(config.warmup.max(10_000), config.iterations.max(100_000));
    let stats = measure::bench(&heavy, || {
        std::hint::black_box(f(std::hint::black_box(3.0), std::hint::black_box(4.0)));
    });

    entry(stats, vec![])
}

// ═══════════════════════════════════════════════════════════
// 3. WARM EXEC: JSON parse + stringify (simulated)
// ═══════════════════════════════════════════════════════════
pub fn warm_exec_json(config: &BenchConfig) -> BenchEntry {
    let payload = r#"{"user":"alice","count":42,"tags":["a","b","c"]}"#;

    let stats = measure::bench(config, || {
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        let s = serde_json::to_string(&v).unwrap();
        std::hint::black_box(&s);
    });

    entry(stats, vec![])
}

// ═══════════════════════════════════════════════════════════
// 4. WARM EXEC: fibonacci(35) — CPU-bound
// ═══════════════════════════════════════════════════════════
pub fn warm_exec_fib35(_config: &BenchConfig) -> BenchEntry {
    fn fib(n: u32) -> u64 {
        if n <= 1 { return n as u64; }
        fib(n - 1) + fib(n - 2)
    }

    let stats = measure::bench(&BenchConfig::new(3, 10), || {
        std::hint::black_box(fib(35));
    });

    entry(stats, vec![("fib_n".into(), "35".into())])
}

// ═══════════════════════════════════════════════════════════
// 5. REQUEST THROUGHPUT — spawn/handle/recycle
// ═══════════════════════════════════════════════════════════
pub fn request_throughput(config: &BenchConfig) -> BenchEntry {
    let module = Arc::new(ModuleHandle {
        source_hash: 0xBEEF,
        handler_names: vec!["fetch".into()],
    });
    let mut pool = IsolatePool::new(module, IsolateEnv::default());
    pool.warm();

    let stats = measure::bench(config, || {
        let mut iso = pool.spawn();
        let _ = iso.arena.alloc_str(r#"{"status":"ok","path":"/api"}"#);
        pool.recycle(iso);
    });

    let rps = if stats.mean_ns > 0.0 { 1e9 / stats.mean_ns } else { f64::INFINITY };

    entry(stats, vec![("requests_per_sec".into(), format!("{rps:.0}"))])
}

// ═══════════════════════════════════════════════════════════
// 6. MEMORY PER ISOLATE
// ═══════════════════════════════════════════════════════════
pub fn memory_per_isolate(config: &BenchConfig) -> BenchEntry {
    let module = Arc::new(ModuleHandle {
        source_hash: 0xAAAA,
        handler_names: vec!["fetch".into()],
    });

    let stats = measure::bench(
        &BenchConfig::new(config.warmup.min(100), config.iterations.min(1000)),
        || {
            let iso = workex_core::isolate::Isolate::new(module.clone(), IsolateEnv::default());
            std::hint::black_box(&iso);
        },
    );

    let sample = workex_core::isolate::Isolate::new(module.clone(), IsolateEnv::default());
    let mem = sample.memory_usage();

    entry(stats, vec![
        ("memory_bytes".into(), mem.to_string()),
        ("memory_kb".into(), format!("{}", mem / 1024)),
    ])
}

// ═══════════════════════════════════════════════════════════
// 7. CONCURRENCY 10K
// ═══════════════════════════════════════════════════════════
pub fn concurrency_10k(_config: &BenchConfig) -> BenchEntry {
    let module = Arc::new(ModuleHandle {
        source_hash: 0xCAFE,
        handler_names: vec!["fetch".into()],
    });
    let env = IsolateEnv::default();
    let count = 10_000usize;

    let stats = measure::bench(&BenchConfig::new(1, 5), || {
        let isolates: Vec<_> = (0..count)
            .map(|_| workex_core::isolate::Isolate::new(module.clone(), env.clone()))
            .collect();
        std::hint::black_box(&isolates);
    });

    let isolates: Vec<_> = (0..count)
        .map(|_| workex_core::isolate::Isolate::new(module.clone(), env.clone()))
        .collect();
    let total_mem: usize = isolates.iter().map(|i| i.memory_usage()).sum();

    entry(stats, vec![
        ("isolate_count".into(), count.to_string()),
        ("total_memory_bytes".into(), total_mem.to_string()),
        ("total_memory_mb".into(), format!("{:.1}", total_mem as f64 / 1024.0 / 1024.0)),
        ("avg_memory_kb".into(), format!("{}", total_mem / count / 1024)),
    ])
}

// ═══════════════════════════════════════════════════════════
// 8. GC PRESSURE — arena alloc + reset
// ═══════════════════════════════════════════════════════════
pub fn gc_pressure(config: &BenchConfig) -> BenchEntry {
    let stats = measure::bench(config, || {
        let mut arena = Arena::new(4096);
        for i in 0u64..100 {
            arena.alloc(i);
        }
        arena.alloc_str("Hello from Workex benchmark!");
        arena.reset();
    });

    entry(stats, vec![])
}

// ═══════════════════════════════════════════════════════════
// RUN ALL — same 8 names as V8 and Workers scripts
// ═══════════════════════════════════════════════════════════
pub fn run_all(config: &BenchConfig) -> BTreeMap<String, BenchEntry> {
    let mut r = BTreeMap::new();

    println!("  [1/8] Cold start...");
    r.insert("cold_start".into(), cold_start(config));

    println!("  [2/8] Warm exec add...");
    r.insert("warm_exec_add".into(), warm_exec_add(config));

    println!("  [3/8] Warm exec JSON...");
    r.insert("warm_exec_json".into(), warm_exec_json(config));

    println!("  [4/8] Fibonacci(35)...");
    r.insert("warm_exec_fib35".into(), warm_exec_fib35(config));

    println!("  [5/8] Request throughput...");
    r.insert("request_throughput".into(), request_throughput(config));

    println!("  [6/8] Memory per isolate...");
    r.insert("memory_per_isolate".into(), memory_per_isolate(config));

    println!("  [7/8] Concurrency (10K)...");
    r.insert("concurrency_10k".into(), concurrency_10k(config));

    println!("  [8/8] GC pressure...");
    r.insert("gc_pressure".into(), gc_pressure(config));

    r
}

fn make_add_function() -> TypedFunction {
    TypedFunction {
        name: "add".to_string(),
        params: vec![
            TypedParam { name: "a".into(), ty: Type::Number, index: 0 },
            TypedParam { name: "b".into(), ty: Type::Number, index: 1 },
        ],
        return_type: Type::Number,
        body: vec![TypedStmt::Return(TypedExpr::BinaryOp {
            op: BinOp::Add,
            left: Box::new(TypedExpr::Ident { name: "a".into(), ty: Type::Number, param_index: 0 }),
            right: Box::new(TypedExpr::Ident { name: "b".into(), ty: Type::Number, param_index: 1 }),
            ty: Type::Number,
        })],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_under_200kb() {
        let e = memory_per_isolate(&BenchConfig::fast());
        let kb: usize = e.metadata["memory_kb"].parse().unwrap();
        assert!(kb < 200, "isolate should use <200KB, got {kb}KB");
    }

    #[test]
    fn concurrency_under_2gb() {
        let e = concurrency_10k(&BenchConfig::fast());
        let mb: f64 = e.metadata["total_memory_mb"].parse().unwrap();
        assert!(mb < 2048.0, "10K isolates should use <2GB, got {mb:.1}MB");
    }
}
