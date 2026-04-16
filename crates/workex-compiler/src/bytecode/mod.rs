//! Workex bytecode — register-based instruction set for the continuation VM.
//!
//! Designed for I/O-bound Workers: explicit SUSPEND/RESUME for async operations.
//! Each await compiles to a SUSPEND instruction that saves only live registers.

use std::collections::HashMap;

/// Register-based instruction set (~25 opcodes).
#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    // Data movement
    LoadConst { dst: u8, idx: u16 },
    Move { dst: u8, src: u8 },
    LoadGlobal { dst: u8, name: u16 },
    SetGlobal { name: u16, src: u8 },

    // Object ops
    GetProp { dst: u8, obj: u8, key: u16 },
    SetProp { obj: u8, key: u16, val: u8 },
    NewObj { dst: u8 },
    NewStr { dst: u8, idx: u16 },

    // Arithmetic
    Add { dst: u8, a: u8, b: u8 },
    Sub { dst: u8, a: u8, b: u8 },
    Mul { dst: u8, a: u8, b: u8 },
    Div { dst: u8, a: u8, b: u8 },

    // Control flow
    Jump { offset: i16 },
    JumpTrue { cond: u8, offset: i16 },
    JumpFalse { cond: u8, offset: i16 },
    Call { dst: u8, func: u8, argc: u8 },
    Return { val: u8 },

    // Suspension — the key innovation
    Suspend { resume_id: u32, live_regs: u64, io_type: IoType },
    Resume { resume_id: u32 },

    // Workers API primitives
    WxFetch { dst: u8, req: u8 },
    WxKvGet { dst: u8, binding: u8, key: u8 },
    WxKvPut { binding: u8, key: u8, val: u8 },
    WxD1Exec { dst: u8, stmt: u8 },
    WxResp { dst: u8, body: u8, status: u8, headers: u8 },
}

/// Type of async I/O that caused suspension.
#[derive(Debug, Clone, PartialEq)]
pub enum IoType {
    Fetch,
    KvGet,
    KvPut,
    D1Query,
    Other,
}

/// Compiled Worker module — everything needed to run a Worker.
#[derive(Debug, Clone)]
pub struct CompiledModule {
    pub source_hash: u64,
    pub instructions: Vec<Instruction>,
    pub constants: Vec<JsValue>,
    pub strings: Vec<String>,
    pub resume_table: HashMap<u32, usize>,
    pub live_reg_masks: HashMap<u32, u64>,
}

/// Minimal JS value for the bytecode VM.
#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Object(HashMap<String, JsValue>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_size() {
        // Verify instructions are reasonably small
        assert!(std::mem::size_of::<Instruction>() <= 24);
    }

    #[test]
    fn jsvalue_basic() {
        let v = JsValue::Str("hello".into());
        assert_eq!(v, JsValue::Str("hello".into()));

        let mut map = HashMap::new();
        map.insert("x".into(), JsValue::Number(42.0));
        let obj = JsValue::Object(map);
        if let JsValue::Object(m) = &obj {
            assert_eq!(m["x"], JsValue::Number(42.0));
        }
    }
}
