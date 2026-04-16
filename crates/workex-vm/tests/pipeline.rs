//! Full pipeline tests: TypeScript → compile_worker → VM → result.

use workex_compiler::bytecode::{Instruction, JsValue};
use workex_compiler::emit::compile_worker;
use workex_vm::continuation::{AgentId, IoRequest};
use workex_vm::vm::{VmFrame, VmResult, run};

#[test]
fn full_pipeline_fetch_worker() {
    let source = r#"
        export default {
            async fetch(request) {
                const resp = await fetch("https://api.example.com/data");
                return new Response(resp);
            }
        };
    "#;
    let module = compile_worker(source).unwrap();
    assert!(module.instructions.iter().any(|i| matches!(i, Instruction::Suspend { .. })));

    let frame = VmFrame::new(AgentId(1));
    match run(&module, frame) {
        VmResult::Suspended { continuation, io_request, .. } => {
            assert!(matches!(io_request, IoRequest::Fetch { .. }));
            println!("Pipeline OK: {} bytes continuation", continuation.size_bytes());
        }
        VmResult::Error(e) => panic!("pipeline error: {e}"),
        _ => panic!("expected suspension"),
    }
}

#[test]
fn full_pipeline_kv_worker() {
    let source = r#"
        export default {
            async fetch(request, env) {
                const value = await env.KV.get("config");
                return new Response(value);
            }
        };
    "#;
    let module = compile_worker(source).unwrap();
    let frame = VmFrame::new(AgentId(1));
    match run(&module, frame) {
        VmResult::Suspended { io_request, .. } => {
            println!("KV suspend: {:?}", io_request);
        }
        VmResult::Error(e) => panic!("error: {e}"),
        _ => panic!("expected suspension"),
    }
}

#[test]
fn sync_worker_completes() {
    let source = r#"
        export default {
            fetch(request) {
                return new Response("sync");
            }
        };
    "#;
    let module = compile_worker(source).unwrap();
    let frame = VmFrame::new(AgentId(1));
    match run(&module, frame) {
        VmResult::Done(_) => println!("Sync worker completed"),
        VmResult::Error(e) => panic!("error: {e}"),
        _ => panic!("sync worker should complete"),
    }
}

#[test]
fn pipeline_hello_ts() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/workers/hello.ts"),
    )
    .unwrap();
    let module = compile_worker(&source).expect("compile failed");
    let result = run(&module, VmFrame::new(AgentId(1)));
    assert!(!matches!(result, VmResult::Error(_)), "hello.ts should not error");
    println!("V6: TypeScript → VM pipeline works.");
}

#[test]
fn pipeline_suspend_resume_cycle() {
    let source = r#"
        export default {
            async fetch(request) {
                const data = await fetch("https://api.example.com");
                return new Response(data);
            }
        };
    "#;
    let module = compile_worker(source).unwrap();

    let frame = VmFrame::new(AgentId(1));
    let VmResult::Suspended { continuation, .. } = run(&module, frame) else {
        panic!("expected suspend");
    };

    let resumed = VmFrame::from_continuation(continuation, JsValue::Str("api response".into()));
    match run(&module, resumed) {
        VmResult::Done(val) => println!("Resume OK: {:?}", val),
        VmResult::Error(e) => panic!("resume error: {e}"),
        _ => panic!("expected done"),
    }
}
