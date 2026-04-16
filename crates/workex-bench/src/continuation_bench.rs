//! 1M Suspended Agents Benchmark — The Continuation Advantage
//!
//! Scenario: 1M agents all suspended waiting for LLM/API response.
//! Each agent stores only its continuation (~48-300 bytes).
//! V8 would keep entire context alive (~183KB per agent).
//!
//! Usage: cargo run -p workex-bench --release --bin continuation-bench

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use workex_compiler::bytecode::*;
use workex_core::rss;
use workex_vm::continuation::AgentId;
use workex_vm::scheduler::AgentScheduler;

const TOTAL: usize = 1_000_000;

fn main() {
    println!();
    println!("+======================================================+");
    println!("|  1M SUSPENDED AGENTS — Continuation Runtime           |");
    println!("|  Each agent awaiting I/O, only continuation stored    |");
    println!("+======================================================+");
    println!();

    // Agent bytecode: load URL → SUSPEND (await fetch) → return result
    // This simulates an agent waiting for LLM API response
    let module = Arc::new(CompiledModule {
        source_hash: 0xA6E0,
        instructions: vec![
            Instruction::LoadConst { dst: 0, idx: 0 },  // r0 = request URL
            Instruction::LoadConst { dst: 1, idx: 1 },  // r1 = API key
            Instruction::LoadConst { dst: 2, idx: 2 },  // r2 = prompt
            Instruction::Suspend {
                resume_id: 0,
                live_regs: 0b111, // save r0, r1, r2
                io_type: IoType::Fetch,
            },
            Instruction::Return { val: 0 },
        ],
        constants: vec![
            JsValue::str("https://api.openai.com/v1/chat/completions"),
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

    // Suspend 1M agents
    for i in 0..TOTAL {
        scheduler.dispatch_and_suspend();

        if (i + 1) % 200_000 == 0 {
            let rss = rss::get_rss_bytes();
            let mem = scheduler.suspended_memory_bytes();
            let count = scheduler.suspended_count();
            let avg = if count > 0 { mem / count } else { 0 };
            println!(
                "  {:>9} agents | {:>5} MB RSS | {:>4} bytes/agent (continuation) | {:>5} MB (cont total)",
                count,
                rss / 1024 / 1024,
                avg,
                mem / 1024 / 1024,
            );
        }
    }

    let elapsed = start.elapsed();
    let rss_after = rss::get_rss_bytes();
    let rss_delta = rss_after.saturating_sub(rss_before);

    let cont_mem = scheduler.suspended_memory_bytes();
    let avg_cont = scheduler.avg_continuation_bytes();
    let count = scheduler.suspended_count();

    // V8 comparison
    let v8_per_agent: f64 = 183.0 * 1024.0; // 183KB in bytes
    let v8_total_gb = v8_per_agent * TOTAL as f64 / 1024.0 / 1024.0 / 1024.0;
    let workex_total_mb = cont_mem as f64 / 1024.0 / 1024.0;
    let factor = v8_per_agent / avg_cont as f64;

    println!();
    println!("+======================================================================+");
    println!("|  1M SUSPENDED AGENTS — RESULTS                                       |");
    println!("+======================================================================+");
    println!("  Agents suspended:    {:>12}", count);
    println!("  Continuation memory: {:>10.1} MB", workex_total_mb);
    println!("  Per agent (cont):    {:>10} bytes", avg_cont);
    println!("  RSS delta:           {:>10} MB", rss_delta / 1024 / 1024);
    println!("  Suspend time:        {:>10.2?}", elapsed);
    println!("  Suspend rate:        {:>10.0} agents/sec", TOTAL as f64 / elapsed.as_secs_f64());
    println!("+======================================================================+");
    println!();
    println!("+======================================================================+");
    println!("|  WORKEX vs V8 — 1M Suspended Agents                                  |");
    println!("+======================================================================+");
    println!("  {:<30} {:>12} {:>12} {:>10}", "METRIC", "WORKEX", "V8*", "FACTOR");
    println!("  {}", "-".repeat(70));
    println!("  {:<30} {:>10.1} MB {:>10.1} GB {:>9.0}x",
        "1M agents memory",
        workex_total_mb, v8_total_gb, factor);
    println!("  {:<30} {:>10} B {:>10} KB {:>9.0}x",
        "Per agent",
        avg_cont, 183, factor);
    println!("  {:<30} {:>10.2?}",
        "Suspend time", elapsed);
    println!("+======================================================================+");
    println!("  * V8 keeps full context alive (183KB) for each suspended agent");
    println!("  * Workex stores only live variables at the await point");

    // 24M projection
    let workex_24m_gb = avg_cont as f64 * 24_000_000.0 / 1024.0 / 1024.0 / 1024.0;
    let v8_24m_tb = v8_per_agent * 24_000_000.0 / 1024.0 / 1024.0 / 1024.0 / 1024.0;
    println!();
    println!("  Matthew Prince's 24M agents:");
    println!("    V8:     {:.1} TB", v8_24m_tb);
    println!("    Workex: {:.1} GB", workex_24m_gb);
    println!("    Factor: {:.0}x", v8_24m_tb * 1024.0 / workex_24m_gb);

    // Save
    let results_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/results");
    std::fs::create_dir_all(&results_dir).ok();
    let report = serde_json::json!({
        "benchmark": "1m_suspended_agents",
        "agents": count,
        "workex": {
            "continuation_memory_bytes": cont_mem,
            "continuation_memory_mb": format!("{:.1}", workex_total_mb),
            "per_agent_bytes": avg_cont,
            "rss_delta_mb": rss_delta / 1024 / 1024,
            "suspend_time_ms": elapsed.as_millis(),
        },
        "v8": {
            "per_agent_bytes": v8_per_agent as u64,
            "total_gb": format!("{:.1}", v8_total_gb),
        },
        "factor": format!("{:.0}", factor),
        "projection_24m": {
            "workex_gb": format!("{:.1}", workex_24m_gb),
            "v8_tb": format!("{:.1}", v8_24m_tb),
        },
    });
    let path = results_dir.join("1m-suspended-agents.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).ok();
    println!();
    println!("  Saved: {}", path.display());
    println!();
}
