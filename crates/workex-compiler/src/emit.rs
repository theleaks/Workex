//! BytecodeEmitter: TypeScript Worker → CompiledModule.
//!
//! This closes the pipeline:
//!   worker.ts → [CPS Transformer] → SuspendPoints
//!                                        ↓
//!                           [BytecodeEmitter] → CompiledModule
//!                                        ↓
//!                                   [Workex VM] → Response

use std::collections::HashMap;

use oxc_allocator::Allocator;

use crate::bytecode::{CompiledModule, Instruction, IoType, JsValue};
use crate::cps::transform::{AsyncCallType, CpsTransformer, SuspendPoint, TransformResult};

/// Compile a TypeScript/JavaScript Worker to a CompiledModule.
/// This is the single public API for the entire compilation pipeline.
pub fn compile_worker(source: &str) -> anyhow::Result<CompiledModule> {
    let allocator = Allocator::default();
    let mut transformer = CpsTransformer::new(&allocator);
    let analysis = transformer.transform(source);
    BytecodeEmitter::new(source, analysis).emit()
}

struct BytecodeEmitter {
    source: String,
    suspend_points: Vec<SuspendPoint>,
    all_vars: Vec<String>,
    instructions: Vec<Instruction>,
    constants: Vec<JsValue>,
    strings: Vec<String>,
    resume_table: HashMap<u32, usize>,
    live_reg_masks: HashMap<u32, u64>,
    var_regs: HashMap<String, u8>,
    next_reg: u8,
}

impl BytecodeEmitter {
    fn new(source: &str, analysis: TransformResult) -> Self {
        Self {
            source: source.to_string(),
            suspend_points: analysis.suspend_points,
            all_vars: analysis.all_vars,
            instructions: Vec::new(),
            constants: Vec::new(),
            strings: Vec::new(),
            resume_table: HashMap::new(),
            live_reg_masks: HashMap::new(),
            var_regs: HashMap::new(),
            next_reg: 0,
        }
    }

    fn emit(mut self) -> anyhow::Result<CompiledModule> {
        // 1. Assign registers to all variables
        self.assign_registers();

        // 2. If no suspend points (sync worker), emit simple response + return
        if self.suspend_points.is_empty() {
            self.emit_sync_worker();
        } else {
            // 3. Async worker: emit request setup + suspend/resume chain
            self.emit_request_setup();
            self.emit_suspend_chain();
            self.emit_response_build();
        }

        Ok(CompiledModule {
            source_hash: hash_str(&self.source),
            instructions: self.instructions,
            constants: self.constants,
            strings: self.strings,
            resume_table: self.resume_table,
            live_reg_masks: self.live_reg_masks,
        })
    }

    fn assign_registers(&mut self) {
        for var in self.all_vars.clone() {
            let reg = self.alloc_reg();
            self.var_regs.insert(var, reg);
        }
    }

    fn emit_request_setup(&mut self) {
        // Load request URL and method into known registers
        let url_str_idx = self.add_string("__request_url__");
        let method_str_idx = self.add_string("__request_method__");
        let url_reg = self.alloc_reg();
        let method_reg = self.alloc_reg();
        self.instructions.push(Instruction::LoadGlobal { dst: url_reg, name: url_str_idx });
        self.instructions.push(Instruction::LoadGlobal { dst: method_reg, name: method_str_idx });
    }

    fn emit_suspend_chain(&mut self) {
        let points = self.suspend_points.clone();
        for (i, point) in points.iter().enumerate() {
            // Load I/O call arguments into registers
            self.emit_io_args(&point);

            // Compute live variable mask
            let live_mask = self.compute_live_mask(&point.live_vars);
            let io_type = call_type_to_io_type(&point.call_type);

            // Emit SUSPEND instruction
            self.instructions.push(Instruction::Suspend {
                resume_id: point.id,
                live_regs: live_mask,
                io_type,
            });

            // Record resume point (instruction after SUSPEND)
            self.resume_table.insert(point.id, self.instructions.len());
            self.live_reg_masks.insert(point.id, live_mask);

            // After resume, r0 has the I/O result — save it
            let result_reg = self.alloc_reg();
            self.instructions.push(Instruction::Move { dst: result_reg, src: 0 });
            self.var_regs.insert(format!("__io_result_{i}"), result_reg);
        }
    }

    fn emit_io_args(&mut self, point: &SuspendPoint) {
        match &point.call_type {
            AsyncCallType::Fetch { url_expr } => {
                let idx = self.add_constant(JsValue::str(url_expr.clone()));
                self.instructions.push(Instruction::LoadConst { dst: 0, idx: idx as u16 });
            }
            AsyncCallType::KvGet { key_expr } => {
                let idx = self.add_constant(JsValue::str(key_expr.clone()));
                self.instructions.push(Instruction::LoadConst { dst: 0, idx: idx as u16 });
            }
            AsyncCallType::KvPut { key_expr, value_expr } => {
                let kid = self.add_constant(JsValue::str(key_expr.clone()));
                let vid = self.add_constant(JsValue::str(value_expr.clone()));
                self.instructions.push(Instruction::LoadConst { dst: 0, idx: kid as u16 });
                self.instructions.push(Instruction::LoadConst { dst: 1, idx: vid as u16 });
            }
            AsyncCallType::D1Query { sql_expr } => {
                let idx = self.add_constant(JsValue::str(sql_expr.clone()));
                self.instructions.push(Instruction::LoadConst { dst: 0, idx: idx as u16 });
            }
            AsyncCallType::Other { .. } => {
                // Generic — put placeholder
                let idx = self.add_constant(JsValue::str("<other>"));
                self.instructions.push(Instruction::LoadConst { dst: 0, idx: idx as u16 });
            }
        }
    }

    fn emit_sync_worker(&mut self) {
        // Sync worker: just build a response and return
        let body_idx = self.add_constant(JsValue::str("ok"));
        let status_idx = self.add_constant(JsValue::Number(200.0));
        let body_reg = self.alloc_reg();
        let status_reg = self.alloc_reg();
        let headers_reg = self.alloc_reg();
        let resp_reg = self.alloc_reg();

        self.instructions.push(Instruction::LoadConst { dst: body_reg, idx: body_idx as u16 });
        self.instructions.push(Instruction::LoadConst { dst: status_reg, idx: status_idx as u16 });
        self.instructions.push(Instruction::NewObj { dst: headers_reg });
        self.instructions.push(Instruction::WxResp {
            dst: resp_reg,
            body: body_reg,
            status: status_reg,
            headers: headers_reg,
        });
        self.instructions.push(Instruction::Return { val: resp_reg });
    }

    fn emit_response_build(&mut self) {
        // Build Response from last I/O result
        let last_result_reg = self.next_reg.saturating_sub(1);
        let status_idx = self.add_constant(JsValue::Number(200.0));
        let status_reg = self.alloc_reg();
        let headers_reg = self.alloc_reg();
        let resp_reg = self.alloc_reg();

        self.instructions.push(Instruction::LoadConst { dst: status_reg, idx: status_idx as u16 });
        self.instructions.push(Instruction::NewObj { dst: headers_reg });
        self.instructions.push(Instruction::WxResp {
            dst: resp_reg,
            body: last_result_reg,
            status: status_reg,
            headers: headers_reg,
        });
        self.instructions.push(Instruction::Return { val: resp_reg });
    }

    fn compute_live_mask(&self, live_vars: &[String]) -> u64 {
        let mut mask = 0u64;
        for var in live_vars {
            if let Some(&reg) = self.var_regs.get(var) {
                if reg < 64 {
                    mask |= 1u64 << reg;
                }
            }
        }
        mask
    }

    fn alloc_reg(&mut self) -> u8 {
        let r = self.next_reg;
        self.next_reg = self.next_reg.saturating_add(1);
        r
    }

    fn add_constant(&mut self, val: JsValue) -> usize {
        let i = self.constants.len();
        self.constants.push(val);
        i
    }

    fn add_string(&mut self, s: &str) -> u16 {
        let i = self.strings.len() as u16;
        self.strings.push(s.to_string());
        i
    }
}

fn call_type_to_io_type(ct: &AsyncCallType) -> IoType {
    match ct {
        AsyncCallType::Fetch { .. } => IoType::Fetch,
        AsyncCallType::KvGet { .. } => IoType::KvGet,
        AsyncCallType::KvPut { .. } => IoType::KvPut,
        AsyncCallType::D1Query { .. } => IoType::D1Query,
        AsyncCallType::Other { .. } => IoType::Other,
    }
}

fn hash_str(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_fetch_worker() {
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
        assert!(!module.constants.is_empty());
        assert!(!module.resume_table.is_empty());
    }

    #[test]
    fn compile_kv_worker() {
        let source = r#"
            export default {
                async fetch(request, env) {
                    const value = await env.KV.get("config");
                    return new Response(value);
                }
            };
        "#;
        let module = compile_worker(source).unwrap();
        assert!(module.instructions.iter().any(|i| matches!(i, Instruction::Suspend { .. })));
    }

    #[test]
    fn compile_sync_worker() {
        let source = r#"
            export default {
                fetch(request) {
                    return new Response("sync");
                }
            };
        "#;
        let module = compile_worker(source).unwrap();
        assert!(!module.instructions.iter().any(|i| matches!(i, Instruction::Suspend { .. })));
        assert!(module.instructions.iter().any(|i| matches!(i, Instruction::Return { .. })));
    }

    #[test]
    fn compile_hello_ts() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/workers/hello.ts"),
        ).unwrap();
        let module = compile_worker(&source).expect("hello.ts should compile");
        assert!(!module.instructions.is_empty());
    }
}
