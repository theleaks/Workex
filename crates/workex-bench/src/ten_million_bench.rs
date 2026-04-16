//! 10M Suspended Agents — Continuation Runtime.
//!
//! Usage: cargo run -p workex-bench --release --bin ten-million-bench

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use workex_compiler::bytecode::*;
use workex_core::rss;
use workex_vm::scheduler::AgentScheduler;

const TOTAL: usize = 10_000_000;

fn main() {
    println!();
    println!("+======================================================+");
    println!("|  10,000,000 SUSPENDED AGENTS                          |");
    println!("+======================================================+");
    println!();

    let module = Arc::new(CompiledModule {
        source_hash: 0x1000,
        instructions: vec![
            Instruction::LoadConst { dst: 0, idx: 0 },
            Instruction::LoadConst { dst: 1, idx: 1 },
            Instruction::LoadConst { dst: 2, idx: 2 },
            Instruction::Suspend {
                resume_id: 0,
                live_regs: 0b111,
                io_type: IoType::Fetch,
            },
            Instruction::Return { val: 0 },
        ],
        constants: vec![
            JsValue::str("https://api.openai.com/v1/chat"),
            JsValue::str("Bearer sk-xxx"),
            JsValue::str(r#"{"model":"gpt-4","messages":[{"role":"user","content":"hello"}]}"#),
        ],
        strings: Vec::new(),
        resume_table: HashMap::from([(0, 4)]),
        live_reg_masks: HashMap::from([(0, 0b111)]),
    });

    let scheduler = AgentScheduler::new(module);
    let rss_before = rss::get_rss_bytes();
    let start = Instant::now();

    for i in 0..TOTAL {
        scheduler.dispatch_and_suspend();
        if (i + 1) % 1_000_000 == 0 {
            let rss = rss::get_rss_bytes();
            let delta = rss.saturating_sub(rss_before);
            let count = i + 1;
            let per = delta / count;
            println!(
                "  {:>10} agents | {:>6} MB RSS | {:>4} bytes/agent | {:.1?}",
                count, rss / 1024 / 1024, per, start.elapsed()
            );
        }
    }

    let elapsed = start.elapsed();
    let rss_after = rss::get_rss_bytes();
    let delta = rss_after.saturating_sub(rss_before);
    let per = delta / TOTAL;
    let cont_mem = scheduler.suspended_memory_bytes();
    let v8_total_gb = 183.0 * 1024.0 * TOTAL as f64 / 1024.0 / 1024.0 / 1024.0;
    let workex_gb = delta as f64 / 1024.0 / 1024.0 / 1024.0;

    println!();
    println!("+======================================================================+");
    println!("|  10M SUSPENDED AGENTS — RESULTS                                      |");
    println!("+======================================================================+");
    println!("  Agents:       {:>12}", TOTAL);
    println!("  RSS delta:    {:>10.2} GB", workex_gb);
    println!("  Cont memory:  {:>10.1} GB", cont_mem as f64 / 1024.0/1024.0/1024.0);
    println!("  Per agent:    {:>10} bytes", per);
    println!("  Time:         {:>10.1?}", elapsed);
    println!("  Rate:         {:>10.0} agents/sec", TOTAL as f64 / elapsed.as_secs_f64());
    println!("+======================================================================+");
    println!();
    println!("  Workex 10M:  {:.2} GB", workex_gb);
    println!("  V8 10M:      {:.1} GB (impossible on any machine)", v8_total_gb);
    println!("  Factor:      {:.0}x", v8_total_gb / workex_gb);

    // Save
    let results_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/results");
    std::fs::create_dir_all(&results_dir).ok();
    let report = serde_json::json!({
        "benchmark": "10m_suspended_agents",
        "agents": TOTAL,
        "workex_gb": format!("{:.2}", workex_gb),
        "per_agent_bytes": per,
        "v8_gb": format!("{:.1}", v8_total_gb),
        "factor": format!("{:.0}", v8_total_gb / workex_gb),
        "time_secs": elapsed.as_secs_f64(),
    });
    let path = results_dir.join("10m-suspended-agents.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).ok();
    println!("\n  Saved: {}", path.display());
}
