//! Continuation — serialized agent state at a suspension point.
//!
//! When an agent hits `await`, only live variables are saved here.
//! Typical size: 200-800 bytes (vs V8's 183KB per context).

use workex_compiler::bytecode::JsValue;

/// Unique agent identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AgentId(pub u64);

/// A serialized continuation — everything needed to resume an agent.
#[derive(Debug, Clone)]
pub struct Continuation {
    pub agent_id: AgentId,
    /// Which resume point to jump to.
    pub resume_id: u32,
    /// Only the live registers at this suspend point.
    pub saved_registers: Vec<(u8, JsValue)>,
    /// Instruction pointer to resume at.
    pub ip: usize,
    /// Which register receives the I/O result.
    pub dst_register: u8,
}

impl Continuation {
    /// Approximate memory footprint of this continuation.
    pub fn size_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.saved_registers.iter().map(|(_, v)| value_size(v)).sum::<usize>()
    }
}

fn value_size(v: &JsValue) -> usize {
    match v {
        JsValue::Str(s) => 8 + s.len(),
        JsValue::Object(map) => {
            8 + map.iter().map(|(k, v)| k.len() + value_size(v)).sum::<usize>()
        }
        _ => 8,
    }
}

/// What I/O the agent is waiting for.
#[derive(Debug, Clone)]
pub enum IoRequest {
    Fetch { url: String, method: String, body: Option<String> },
    KvGet { binding: String, key: String },
    KvPut { binding: String, key: String, value: String },
    D1Query { binding: String, sql: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_size_small() {
        let cont = Continuation {
            agent_id: AgentId(1),
            resume_id: 0,
            saved_registers: vec![
                (0, JsValue::Str("https://example.com/api".into())),
                (1, JsValue::Str("GET".into())),
                (2, JsValue::Number(200.0)),
            ],
            ip: 5,
            dst_register: 3,
        };

        let size = cont.size_bytes();
        println!("Continuation with 3 regs: {} bytes", size);
        assert!(size < 500, "continuation should be <500 bytes, got {size}");
    }

    #[test]
    fn continuation_with_request_data() {
        // Simulates a real agent suspended at fetch() with request context
        let mut headers = std::collections::HashMap::new();
        headers.insert("content-type".into(), JsValue::Str("application/json".into()));
        headers.insert("authorization".into(), JsValue::Str("Bearer tok123".into()));

        let cont = Continuation {
            agent_id: AgentId(42),
            resume_id: 1,
            saved_registers: vec![
                (0, JsValue::Str("https://api.openai.com/v1/chat".into())),
                (1, JsValue::Object(headers)),
                (2, JsValue::Str(r#"{"model":"gpt-4","messages":[]}"#.into())),
            ],
            ip: 8,
            dst_register: 4,
        };

        let size = cont.size_bytes();
        println!("Real agent continuation: {} bytes", size);
        assert!(size < 1000, "real continuation should be <1KB, got {size}");
    }
}
