//! Agent Hibernation — persist continuations to disk, survive restarts.
//!
//! When an agent hits await, its continuation (~191 bytes) can be written to sled.
//! On server restart, all pending agents are loaded and resumed.
//! This is BEAM's killer feature — applied to Workers-compatible JS.

use serde::{Deserialize, Serialize};

use crate::continuation::{AgentId, Continuation, IoRequest};
use workex_compiler::bytecode::JsValue;

/// A hibernated agent — serializable to disk.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HibernatedAgent {
    pub agent_id: u64,
    pub resume_id: u32,
    pub saved_registers: Vec<(u8, JsValue)>,
    pub ip: usize,
    pub dst_register: u8,
    pub pending_io: SerializedIoRequest,
    pub hibernated_at_ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum SerializedIoRequest {
    Fetch { url: String, method: String, body: Option<String> },
    KvGet { binding: String, key: String },
    KvPut { binding: String, key: String, value: String },
    D1Query { binding: String, sql: String },
}

impl HibernatedAgent {
    pub fn from_continuation(cont: &Continuation, io: &IoRequest) -> Self {
        Self {
            agent_id: cont.agent_id.0,
            resume_id: cont.resume_id,
            saved_registers: cont.saved_registers.clone(),
            ip: cont.ip,
            dst_register: cont.dst_register,
            pending_io: serialize_io(io),
            hibernated_at_ms: now_ms(),
        }
    }

    pub fn to_continuation(&self) -> Continuation {
        Continuation {
            agent_id: AgentId(self.agent_id),
            resume_id: self.resume_id,
            saved_registers: self.saved_registers.clone(),
            ip: self.ip,
            dst_register: self.dst_register,
        }
    }
}

/// Persistent store for hibernated agents.
pub struct HibernationStore {
    db: sled::Db,
}

impl HibernationStore {
    pub fn new(path: &str) -> anyhow::Result<Self> {
        std::fs::create_dir_all(path)?;
        Ok(Self { db: sled::open(path)? })
    }

    /// Hibernate an agent — write continuation to disk.
    pub fn hibernate(&self, cont: &Continuation, io: &IoRequest) -> anyhow::Result<()> {
        let agent = HibernatedAgent::from_continuation(cont, io);
        let bytes = bincode::serialize(&agent)?;
        self.db.insert(agent.agent_id.to_le_bytes(), bytes)?;
        self.db.flush()?;
        Ok(())
    }

    /// Wake a specific agent.
    pub fn wake(&self, agent_id: AgentId) -> anyhow::Result<Option<HibernatedAgent>> {
        match self.db.remove(agent_id.0.to_le_bytes())? {
            Some(bytes) => Ok(Some(bincode::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Get all pending hibernated agents.
    pub fn all_pending(&self) -> anyhow::Result<Vec<HibernatedAgent>> {
        self.db
            .iter()
            .filter_map(|r| r.ok())
            .map(|(_, v)| bincode::deserialize(&v).map_err(Into::into))
            .collect()
    }

    pub fn hibernated_count(&self) -> usize {
        self.db.len()
    }

    pub fn disk_usage_bytes(&self) -> u64 {
        self.db.size_on_disk().unwrap_or(0)
    }
}

fn serialize_io(io: &IoRequest) -> SerializedIoRequest {
    match io {
        IoRequest::Fetch { url, method, body } => SerializedIoRequest::Fetch {
            url: url.clone(),
            method: method.clone(),
            body: body.clone(),
        },
        IoRequest::KvGet { binding, key } => SerializedIoRequest::KvGet {
            binding: binding.clone(),
            key: key.clone(),
        },
        IoRequest::KvPut { binding, key, value } => SerializedIoRequest::KvPut {
            binding: binding.clone(),
            key: key.clone(),
            value: value.clone(),
        },
        IoRequest::D1Query { binding, sql } => SerializedIoRequest::D1Query {
            binding: binding.clone(),
            sql: sql.clone(),
        },
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::collections::HashMap;

    use workex_compiler::bytecode::*;
    use crate::scheduler::AgentScheduler;
    use crate::vm::VmFrame;

    fn make_fetch_module() -> CompiledModule {
        CompiledModule {
            source_hash: 0,
            instructions: vec![
                Instruction::LoadConst { dst: 0, idx: 0 },
                Instruction::Suspend { resume_id: 0, live_regs: 0b1, io_type: IoType::Fetch },
                Instruction::Return { val: 0 },
            ],
            constants: vec![JsValue::Str("https://api.openai.com".into())],
            strings: Vec::new(),
            resume_table: HashMap::from([(0, 2)]),
            live_reg_masks: HashMap::from([(0, 0b1)]),
        }
    }

    #[test]
    fn hibernate_and_wake() {
        let path = std::env::temp_dir().join("workex-test-hibernate-1");
        let _ = std::fs::remove_dir_all(&path);
        let store = HibernationStore::new(path.to_str().unwrap()).unwrap();

        let cont = Continuation {
            agent_id: AgentId(42),
            resume_id: 0,
            saved_registers: vec![(0, JsValue::Str("data".into()))],
            ip: 5,
            dst_register: 1,
        };
        let io = IoRequest::Fetch {
            url: "https://api.example.com".into(),
            method: "POST".into(),
            body: Some("payload".into()),
        };

        store.hibernate(&cont, &io).unwrap();
        assert_eq!(store.hibernated_count(), 1);

        let woken = store.wake(AgentId(42)).unwrap().unwrap();
        assert_eq!(woken.agent_id, 42);
        assert_eq!(woken.resume_id, 0);
        assert_eq!(woken.ip, 5);

        // After wake, agent is removed
        assert_eq!(store.hibernated_count(), 0);

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn agent_survives_server_restart() {
        let path = std::env::temp_dir().join("workex-test-restart");
        let _ = std::fs::remove_dir_all(&path);
        let module = Arc::new(make_fetch_module());

        // "Server 1": dispatch agent, it suspends, hibernate it
        {
            let scheduler = AgentScheduler::new(module.clone());
            let agent_id = scheduler.dispatch_and_suspend().unwrap();

            // Get the continuation from scheduler (slab uses index)
            let cont = {
                let mut suspended = scheduler.suspended.lock().unwrap();
                suspended.remove(agent_id.0 as usize).unwrap()
            };
            let io = IoRequest::Fetch {
                url: "https://api.openai.com".into(),
                method: "GET".into(),
                body: None,
            };

            let store = HibernationStore::new(path.to_str().unwrap()).unwrap();
            store.hibernate(&cont, &io).unwrap();
            println!("Hibernated. Disk: {} bytes", store.disk_usage_bytes());
        } // scheduler dropped — "server crashed"

        // "Server 2": load from disk, inject, resume
        {
            let store = HibernationStore::new(path.to_str().unwrap()).unwrap();
            let pending = store.all_pending().unwrap();
            assert_eq!(pending.len(), 1, "should find 1 hibernated agent");

            let scheduler2 = AgentScheduler::new(module.clone());
            let mut agent_slot = 0;
            for h in &pending {
                let cont = h.to_continuation();
                let mut slab = scheduler2.suspended.lock().unwrap();
                agent_slot = slab.insert(cont);
            }
            assert_eq!(scheduler2.suspended_count(), 1);

            let agent_id = AgentId(agent_slot as u64);
            let result = scheduler2.resume(agent_id, JsValue::Str("LLM response".into()));
            match result {
                crate::scheduler::DispatchResult::Done(val) => {
                    println!("Agent survived restart! Result: {:?}", val);
                }
                other => panic!("expected done, got: {:?}", other),
            }
        }

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn disk_size_per_agent() {
        let path = std::env::temp_dir().join("workex-test-disk-size");
        let _ = std::fs::remove_dir_all(&path);
        let store = HibernationStore::new(path.to_str().unwrap()).unwrap();

        // Hibernate 100 agents
        for i in 0..100u64 {
            let cont = Continuation {
                agent_id: AgentId(i),
                resume_id: 0,
                saved_registers: vec![
                    (0, JsValue::Str(format!("https://api.example.com/{i}"))),
                    (1, JsValue::Str("Bearer sk-xxx".into())),
                ],
                ip: 5,
                dst_register: 2,
            };
            let io = IoRequest::Fetch {
                url: format!("https://api.example.com/{i}"),
                method: "POST".into(),
                body: Some(r#"{"prompt":"hello"}"#.into()),
            };
            store.hibernate(&cont, &io).unwrap();
        }

        let disk = store.disk_usage_bytes();
        let per_agent = disk / 100;
        println!("100 hibernated agents: {} bytes disk, {} bytes/agent", disk, per_agent);
        assert!(per_agent < 2000, "per agent disk should be <2KB, got {per_agent}");

        let _ = std::fs::remove_dir_all(&path);
    }
}
