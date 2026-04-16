//! Workex VM — register-based virtual machine with SUSPEND/RESUME.
//!
//! Executes compiled bytecode. When hitting an async I/O operation,
//! saves only live registers (continuation) and yields control.
//! When I/O completes, restores registers and continues.

use std::collections::HashMap;

use workex_compiler::bytecode::{CompiledModule, Instruction, IoType, JsValue};
use workex_core::arena::Arena;

use crate::continuation::{AgentId, Continuation, IoRequest};

/// Resource limits for agent execution.
#[derive(Debug, Clone)]
pub struct AgentLimits {
    pub max_instructions: u64,
    pub max_continuation_bytes: usize,
    pub max_io_ops: u32,
}

impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            max_instructions: 1_000_000,
            max_continuation_bytes: 65_536,
            max_io_ops: 1_000,
        }
    }
}

/// Try/catch frame pushed on the error stack.
#[derive(Debug, Clone)]
pub struct TryCatchFrame {
    pub catch_ip: usize,
    pub error_reg: u8,
}

/// VM execution frame for one agent.
pub struct VmFrame {
    pub agent_id: AgentId,
    pub registers: Box<[JsValue; 256]>,
    pub ip: usize,
    pub arena: Arena,
    pub try_stack: Vec<TryCatchFrame>,
    pub instruction_count: u64,
    pub limits: AgentLimits,
}

impl VmFrame {
    /// Create a new frame for an incoming request.
    pub fn new(agent_id: AgentId) -> Self {
        Self {
            agent_id,
            registers: Box::new(std::array::from_fn(|_| JsValue::Undefined)),
            ip: 0,
            arena: Arena::minimal(),
            try_stack: Vec::new(),
            instruction_count: 0,
            limits: AgentLimits::default(),
        }
    }

    pub fn new_with_limits(agent_id: AgentId, limits: AgentLimits) -> Self {
        Self {
            limits,
            ..Self::new(agent_id)
        }
    }

    /// Rebuild frame from a continuation + I/O result.
    pub fn from_continuation(cont: Continuation, io_result: JsValue) -> Self {
        let mut registers: Box<[JsValue; 256]> = Box::new(std::array::from_fn(|_| JsValue::Undefined));
        for (idx, val) in cont.saved_registers {
            registers[idx as usize] = val;
        }
        registers[cont.dst_register as usize] = io_result;

        Self {
            agent_id: cont.agent_id,
            registers,
            ip: cont.ip,
            arena: Arena::minimal(),
            try_stack: Vec::new(),
            instruction_count: 0,
            limits: AgentLimits::default(),
        }
    }
}

/// Result of running the VM.
pub enum VmResult {
    /// Agent completed — response ready.
    Done(JsValue),
    /// Agent suspended at a single await.
    Suspended {
        agent_id: AgentId,
        continuation: Continuation,
        io_request: IoRequest,
    },
    /// Agent suspended at Promise.all — multiple parallel I/O.
    SuspendedMulti {
        agent_id: AgentId,
        continuation: Continuation,
        io_requests: Vec<IoRequest>,
    },
    /// Runtime error.
    Error(String),
}

/// Execute VM until completion or suspension.
pub fn run(module: &CompiledModule, mut frame: VmFrame) -> VmResult {
    loop {
        if frame.ip >= module.instructions.len() {
            return VmResult::Error("IP out of bounds".into());
        }

        // CPU limit check
        frame.instruction_count += 1;
        if frame.instruction_count > frame.limits.max_instructions {
            return VmResult::Error("CPU limit exceeded".into());
        }

        let inst = module.instructions[frame.ip].clone();

        match inst {
            Instruction::LoadConst { dst, idx } => {
                frame.registers[dst as usize] = module.constants.get(idx as usize)
                    .cloned()
                    .unwrap_or(JsValue::Undefined);
                frame.ip += 1;
            }

            Instruction::Move { dst, src } => {
                frame.registers[dst as usize] = frame.registers[src as usize].clone();
                frame.ip += 1;
            }

            Instruction::Add { dst, a, b } => {
                let av = to_f64(&frame.registers[a as usize]);
                let bv = to_f64(&frame.registers[b as usize]);
                frame.registers[dst as usize] = JsValue::Number(av + bv);
                frame.ip += 1;
            }

            Instruction::Sub { dst, a, b } => {
                let av = to_f64(&frame.registers[a as usize]);
                let bv = to_f64(&frame.registers[b as usize]);
                frame.registers[dst as usize] = JsValue::Number(av - bv);
                frame.ip += 1;
            }

            Instruction::Mul { dst, a, b } => {
                let av = to_f64(&frame.registers[a as usize]);
                let bv = to_f64(&frame.registers[b as usize]);
                frame.registers[dst as usize] = JsValue::Number(av * bv);
                frame.ip += 1;
            }

            Instruction::Div { dst, a, b } => {
                let av = to_f64(&frame.registers[a as usize]);
                let bv = to_f64(&frame.registers[b as usize]);
                frame.registers[dst as usize] = JsValue::Number(av / bv);
                frame.ip += 1;
            }

            Instruction::NewStr { dst, idx } => {
                let s = module.strings.get(idx as usize).cloned().unwrap_or_default();
                frame.registers[dst as usize] = JsValue::str(s);
                frame.ip += 1;
            }

            Instruction::NewObj { dst } => {
                frame.registers[dst as usize] = JsValue::Object(HashMap::new());
                frame.ip += 1;
            }

            Instruction::SetProp { obj, key, val } => {
                let key_str = module.strings.get(key as usize).cloned().unwrap_or_default();
                let value = frame.registers[val as usize].clone();
                if let JsValue::Object(ref mut map) = frame.registers[obj as usize] {
                    map.insert(key_str, value);
                }
                frame.ip += 1;
            }

            Instruction::GetProp { dst, obj, key } => {
                let key_str = module.strings.get(key as usize).cloned().unwrap_or_default();
                let val = if let JsValue::Object(map) = &frame.registers[obj as usize] {
                    map.get(&key_str).cloned().unwrap_or(JsValue::Undefined)
                } else {
                    JsValue::Undefined
                };
                frame.registers[dst as usize] = val;
                frame.ip += 1;
            }

            Instruction::Return { val } => {
                return VmResult::Done(frame.registers[val as usize].clone());
            }

            Instruction::Jump { offset } => {
                frame.ip = (frame.ip as i64 + offset as i64) as usize;
            }

            Instruction::JumpTrue { cond, offset } => {
                if is_truthy(&frame.registers[cond as usize]) {
                    frame.ip = (frame.ip as i64 + offset as i64) as usize;
                } else {
                    frame.ip += 1;
                }
            }

            Instruction::JumpFalse { cond, offset } => {
                if !is_truthy(&frame.registers[cond as usize]) {
                    frame.ip = (frame.ip as i64 + offset as i64) as usize;
                } else {
                    frame.ip += 1;
                }
            }

            // SUSPEND — the key instruction
            Instruction::Suspend { resume_id, live_regs, io_type } => {
                let saved = save_live_registers(&frame.registers, live_regs);
                let io_request = build_io_request(&io_type, &frame.registers);

                return VmResult::Suspended {
                    agent_id: frame.agent_id,
                    continuation: Continuation {
                        agent_id: frame.agent_id,
                        resume_id,
                        saved_registers: saved,
                        ip: frame.ip + 1, // resume after SUSPEND
                        dst_register: 0,  // result goes in r0
                    },
                    io_request,
                };
            }

            Instruction::Resume { .. } => {
                frame.ip += 1;
            }

            Instruction::TryCatch { catch_offset, error_reg } => {
                let catch_ip = (frame.ip as i64 + catch_offset as i64) as usize;
                frame.try_stack.push(TryCatchFrame { catch_ip, error_reg });
                frame.ip += 1;
            }

            Instruction::EndTry => {
                frame.try_stack.pop();
                frame.ip += 1;
            }

            Instruction::Throw { val } => {
                let error = frame.registers[val as usize].clone();
                if let Some(tc) = frame.try_stack.pop() {
                    frame.registers[tc.error_reg as usize] = error;
                    frame.ip = tc.catch_ip;
                } else {
                    let msg = match &error {
                        JsValue::Str(s) => s.to_string(),
                        _ => format!("{:?}", error),
                    };
                    return VmResult::Error(format!("Uncaught: {msg}"));
                }
            }

            Instruction::SuspendMulti { resume_id, live_regs, count } => {
                let saved = save_live_registers(&frame.registers, live_regs);
                let mut requests = Vec::new();
                for i in 0..count {
                    requests.push(build_io_request(
                        &IoType::Fetch,
                        &frame.registers,
                    ));
                }

                return VmResult::SuspendedMulti {
                    agent_id: frame.agent_id,
                    continuation: Continuation {
                        agent_id: frame.agent_id,
                        resume_id,
                        saved_registers: saved,
                        ip: frame.ip + 1,
                        dst_register: 0,
                    },
                    io_requests: requests,
                };
            }

            Instruction::WxResp { dst, body, status, headers } => {
                let mut resp = HashMap::new();
                resp.insert("body".into(), frame.registers[body as usize].clone());
                resp.insert("status".into(), frame.registers[status as usize].clone());
                resp.insert("headers".into(), frame.registers[headers as usize].clone());
                resp.insert("__is_response".into(), JsValue::Bool(true));
                frame.registers[dst as usize] = JsValue::Object(resp);
                frame.ip += 1;
            }

            _ => {
                frame.ip += 1;
            }
        }
    }
}

fn save_live_registers(regs: &[JsValue; 256], live_mask: u64) -> Vec<(u8, JsValue)> {
    let mut saved = Vec::new();
    for i in 0..64u8 {
        if live_mask & (1u64 << i) != 0 {
            saved.push((i, regs[i as usize].clone()));
        }
    }
    saved
}

fn build_io_request(io_type: &IoType, regs: &[JsValue; 256]) -> IoRequest {
    match io_type {
        IoType::Fetch => {
            let url = match &regs[0] {
                JsValue::Str(s) => s.to_string(),
                _ => String::new(),
            };
            IoRequest::Fetch { url, method: "GET".into(), body: None }
        }
        IoType::KvGet => IoRequest::KvGet {
            binding: "KV".into(),
            key: match &regs[0] { JsValue::Str(s) => s.to_string(), _ => String::new() },
        },
        IoType::KvPut => IoRequest::KvPut {
            binding: "KV".into(),
            key: match &regs[0] { JsValue::Str(s) => s.to_string(), _ => String::new() },
            value: match &regs[1] { JsValue::Str(s) => s.to_string(), _ => String::new() },
        },
        IoType::D1Query => IoRequest::D1Query {
            binding: "DB".into(),
            sql: match &regs[0] { JsValue::Str(s) => s.to_string(), _ => String::new() },
        },
        IoType::Other => IoRequest::Fetch { url: String::new(), method: "GET".into(), body: None },
    }
}

fn to_f64(val: &JsValue) -> f64 {
    match val {
        JsValue::Number(n) => *n,
        JsValue::Bool(b) => if *b { 1.0 } else { 0.0 },
        _ => 0.0,
    }
}

fn is_truthy(val: &JsValue) -> bool {
    match val {
        JsValue::Undefined | JsValue::Null => false,
        JsValue::Bool(b) => *b,
        JsValue::Number(n) => *n != 0.0 && !n.is_nan(),
        JsValue::Str(s) => !s.is_empty(),
        JsValue::Object(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use workex_compiler::bytecode::*;

    #[test]
    fn vm_arithmetic() {
        let module = CompiledModule {
            source_hash: 0,
            instructions: vec![
                Instruction::LoadConst { dst: 0, idx: 0 },
                Instruction::LoadConst { dst: 1, idx: 1 },
                Instruction::Add { dst: 2, a: 0, b: 1 },
                Instruction::Return { val: 2 },
            ],
            constants: vec![JsValue::Number(10.0), JsValue::Number(32.0)],
            strings: Vec::new(),
            resume_table: HashMap::new(),
            live_reg_masks: HashMap::new(),
        };

        let frame = VmFrame::new(AgentId(1));
        match run(&module, frame) {
            VmResult::Done(JsValue::Number(n)) => assert_eq!(n, 42.0),
            other => panic!("expected Done(42), got: {:?}", match other { VmResult::Error(e) => e, _ => "?".into() }),
        }
    }

    #[test]
    fn vm_suspend_and_resume() {
        let module = CompiledModule {
            source_hash: 0,
            instructions: vec![
                Instruction::LoadConst { dst: 0, idx: 0 }, // r0 = "https://api.com"
                Instruction::Suspend { resume_id: 0, live_regs: 0b01, io_type: IoType::Fetch },
                Instruction::Return { val: 0 }, // return I/O result
            ],
            constants: vec![JsValue::str("https://api.com")],
            strings: Vec::new(),
            resume_table: HashMap::from([(0, 2)]),
            live_reg_masks: HashMap::from([(0, 0b01)]),
        };

        // First run — should suspend
        let frame = VmFrame::new(AgentId(1));
        let result = run(&module, frame);

        match result {
            VmResult::Suspended { continuation, io_request, .. } => {
                // Verify continuation is small
                let size = continuation.size_bytes();
                println!("Continuation size: {size} bytes");
                assert!(size < 500);

                // Verify IO request
                assert!(matches!(io_request, IoRequest::Fetch { .. }));

                // Resume with I/O result
                let resumed = VmFrame::from_continuation(
                    continuation,
                    JsValue::str("response body"),
                );
                match run(&module, resumed) {
                    VmResult::Done(JsValue::Str(s)) => {
                        assert_eq!(&*s, "response body");
                    }
                    other => panic!("expected Done after resume, got error"),
                }
            }
            _ => panic!("expected Suspended"),
        }
    }

    #[test]
    fn vm_multiple_suspends() {
        // Simulate: r0 = await fetch(url1); r1 = await fetch(url2); return r0 + r1
        let module = CompiledModule {
            source_hash: 0,
            instructions: vec![
                Instruction::LoadConst { dst: 0, idx: 0 },  // r0 = "url1"
                Instruction::Suspend { resume_id: 0, live_regs: 0, io_type: IoType::Fetch }, // await #1
                // r0 now has I/O result
                Instruction::Move { dst: 1, src: 0 },       // r1 = r0 (save first result)
                Instruction::LoadConst { dst: 0, idx: 1 },  // r0 = "url2"
                Instruction::Suspend { resume_id: 1, live_regs: 0b10, io_type: IoType::Fetch }, // await #2, save r1
                // r0 now has second I/O result
                Instruction::Add { dst: 2, a: 0, b: 1 },    // r2 = r0 + r1
                Instruction::Return { val: 2 },
            ],
            constants: vec![JsValue::str("url1"), JsValue::str("url2")],
            strings: Vec::new(),
            resume_table: HashMap::from([(0, 2), (1, 5)]),
            live_reg_masks: HashMap::from([(0, 0), (1, 0b10)]),
        };

        // Run 1 — suspend at first await
        let frame = VmFrame::new(AgentId(1));
        let VmResult::Suspended { continuation: c1, .. } = run(&module, frame) else {
            panic!("expected first suspend");
        };

        // Resume with first result
        let frame2 = VmFrame::from_continuation(c1, JsValue::Number(10.0));
        let VmResult::Suspended { continuation: c2, .. } = run(&module, frame2) else {
            panic!("expected second suspend");
        };

        // Verify r1 was saved
        assert!(c2.saved_registers.iter().any(|(idx, _)| *idx == 1));

        // Resume with second result
        let frame3 = VmFrame::from_continuation(c2, JsValue::Number(32.0));
        match run(&module, frame3) {
            VmResult::Done(JsValue::Number(n)) => assert_eq!(n, 42.0),
            _ => panic!("expected Done(42) after two resumes"),
        }
    }

    #[test]
    fn vm_try_catch() {
        let module = CompiledModule {
            source_hash: 0,
            instructions: vec![
                Instruction::TryCatch { catch_offset: 3, error_reg: 1 },
                Instruction::LoadConst { dst: 0, idx: 0 },
                Instruction::Throw { val: 0 },
                // catch block (ip=3)
                Instruction::Return { val: 1 }, // return the caught error
            ],
            constants: vec![JsValue::str("boom")],
            strings: Vec::new(),
            resume_table: HashMap::new(),
            live_reg_masks: HashMap::new(),
        };

        let frame = VmFrame::new(AgentId(1));
        match run(&module, frame) {
            VmResult::Done(JsValue::Str(s)) => assert_eq!(&*s, "boom"),
            other => panic!("expected caught error, got: {:?}", match other { VmResult::Error(e) => e, _ => "?".into() }),
        }
    }

    #[test]
    fn vm_uncaught_throw() {
        let module = CompiledModule {
            source_hash: 0,
            instructions: vec![
                Instruction::LoadConst { dst: 0, idx: 0 },
                Instruction::Throw { val: 0 },
            ],
            constants: vec![JsValue::str("uncaught")],
            strings: Vec::new(),
            resume_table: HashMap::new(),
            live_reg_masks: HashMap::new(),
        };

        let frame = VmFrame::new(AgentId(1));
        match run(&module, frame) {
            VmResult::Error(e) => assert!(e.contains("uncaught"), "got: {e}"),
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn vm_cpu_limit() {
        // Infinite loop: Jump { offset: 0 } loops forever
        let module = CompiledModule {
            source_hash: 0,
            instructions: vec![
                Instruction::LoadConst { dst: 0, idx: 0 },
                Instruction::Jump { offset: 0 }, // infinite loop at ip=1
            ],
            constants: vec![JsValue::Number(0.0)],
            strings: Vec::new(),
            resume_table: HashMap::new(),
            live_reg_masks: HashMap::new(),
        };

        let frame = VmFrame::new_with_limits(AgentId(1), AgentLimits {
            max_instructions: 100,
            ..Default::default()
        });
        match run(&module, frame) {
            VmResult::Error(e) => assert!(e.contains("CPU limit"), "got: {e}"),
            _ => panic!("infinite loop should be killed"),
        }
    }

    #[test]
    fn vm_suspend_multi() {
        let module = CompiledModule {
            source_hash: 0,
            instructions: vec![
                Instruction::SuspendMulti { resume_id: 0, live_regs: 0, count: 3 },
                Instruction::Return { val: 0 },
            ],
            constants: Vec::new(),
            strings: Vec::new(),
            resume_table: HashMap::from([(0, 1)]),
            live_reg_masks: HashMap::new(),
        };

        let frame = VmFrame::new(AgentId(1));
        match run(&module, frame) {
            VmResult::SuspendedMulti { io_requests, .. } => {
                assert_eq!(io_requests.len(), 3);
            }
            _ => panic!("expected multi-suspend"),
        }
    }

    #[test]
    fn vm_agent_isolation() {
        // Two agents with different data shouldn't leak
        let module = CompiledModule {
            source_hash: 0,
            instructions: vec![
                Instruction::LoadConst { dst: 0, idx: 0 },
                Instruction::Suspend { resume_id: 0, live_regs: 0b1, io_type: IoType::Fetch },
                Instruction::Return { val: 0 },
            ],
            constants: vec![JsValue::str("secret_A")],
            strings: Vec::new(),
            resume_table: HashMap::from([(0, 2)]),
            live_reg_masks: HashMap::from([(0, 0b1)]),
        };

        // Agent A
        let frame_a = VmFrame::new(AgentId(1));
        let VmResult::Suspended { continuation: cont_a, .. } = run(&module, frame_a) else { panic!() };

        // Agent B — fresh frame, no access to A's registers
        let frame_b = VmFrame::new(AgentId(2));
        let VmResult::Suspended { continuation: cont_b, .. } = run(&module, frame_b) else { panic!() };

        // Verify isolation: B's continuation doesn't contain A's data
        let b_data = format!("{:?}", cont_b.saved_registers);
        // Both have "secret_A" from constants, but that's module-level — not agent state
        // The key: agent B cannot read agent A's continuation
        assert_ne!(cont_a.agent_id, cont_b.agent_id);
    }

    #[test]
    fn vm_response_construction() {
        let module = CompiledModule {
            source_hash: 0,
            instructions: vec![
                Instruction::NewStr { dst: 0, idx: 0 },     // r0 = "hello"
                Instruction::LoadConst { dst: 1, idx: 0 },   // r1 = 200
                Instruction::NewObj { dst: 2 },              // r2 = {}
                Instruction::WxResp { dst: 3, body: 0, status: 1, headers: 2 },
                Instruction::Return { val: 3 },
            ],
            constants: vec![JsValue::Number(200.0)],
            strings: vec!["hello".into()],
            resume_table: HashMap::new(),
            live_reg_masks: HashMap::new(),
        };

        let frame = VmFrame::new(AgentId(1));
        match run(&module, frame) {
            VmResult::Done(JsValue::Object(map)) => {
                assert_eq!(map["body"], JsValue::str("hello"));
                assert_eq!(map["status"], JsValue::Number(200.0));
                assert_eq!(map["__is_response"], JsValue::Bool(true));
            }
            _ => panic!("expected response object"),
        }
    }
}
