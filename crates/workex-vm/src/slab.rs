//! ContinuationSlab — contiguous memory, O(1) insert/remove.
//! No HashMap bucket overhead — just continuation data.

use crate::continuation::Continuation;

/// Slab allocator for continuations.
/// AgentId maps to slot index. Freed slots go into free_list.
pub struct ContinuationSlab {
    slots: Vec<Option<Continuation>>,
    free_list: Vec<usize>,
    count: usize,
}

impl ContinuationSlab {
    pub fn new() -> Self {
        Self { slots: Vec::new(), free_list: Vec::new(), count: 0 }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self { slots: Vec::with_capacity(cap), free_list: Vec::new(), count: 0 }
    }

    pub fn insert(&mut self, cont: Continuation) -> usize {
        self.count += 1;
        if let Some(idx) = self.free_list.pop() {
            self.slots[idx] = Some(cont);
            idx
        } else {
            let idx = self.slots.len();
            self.slots.push(Some(cont));
            idx
        }
    }

    pub fn remove(&mut self, idx: usize) -> Option<Continuation> {
        if idx >= self.slots.len() { return None; }
        let cont = self.slots[idx].take()?;
        self.free_list.push(idx);
        self.count -= 1;
        Some(cont)
    }

    pub fn get(&self, idx: usize) -> Option<&Continuation> {
        self.slots.get(idx)?.as_ref()
    }

    pub fn len(&self) -> usize { self.count }
    pub fn is_empty(&self) -> bool { self.count == 0 }

    pub fn memory_bytes(&self) -> usize {
        self.slots.iter().filter_map(|s| s.as_ref()).map(|c| c.size_bytes()).sum()
    }

    /// Iterate over all stored continuations.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &Continuation)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|c| (i, c)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::continuation::AgentId;
    use workex_compiler::bytecode::JsValue;

    fn make_cont(id: u64) -> Continuation {
        Continuation {
            agent_id: AgentId(id),
            resume_id: 0,
            saved_registers: vec![
                (0, JsValue::str(format!("https://api.example.com/{id}"))),
                (1, JsValue::str("Bearer token")),
            ],
            ip: 5,
            dst_register: 2,
        }
    }

    #[test]
    fn insert_remove() {
        let mut slab = ContinuationSlab::new();
        let idx = slab.insert(make_cont(1));
        assert_eq!(slab.len(), 1);

        let cont = slab.remove(idx).unwrap();
        assert_eq!(cont.agent_id.0, 1);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn reuse_freed_slots() {
        let mut slab = ContinuationSlab::new();
        let a = slab.insert(make_cont(1));
        let b = slab.insert(make_cont(2));
        slab.remove(a);
        // New insert should reuse slot a
        let c = slab.insert(make_cont(3));
        assert_eq!(c, a);
        assert_eq!(slab.len(), 2);
    }

    #[test]
    fn slab_minimal_overhead() {
        let mut slab = ContinuationSlab::with_capacity(100_000);
        let single = make_cont(0);
        let logical_size = single.size_bytes();

        for i in 0..100_000u64 {
            slab.insert(make_cont(i));
        }

        let total = slab.memory_bytes();
        let per = total / 100_000;
        let overhead = per.saturating_sub(logical_size);
        println!("Slab: {} bytes/entry, logical: {}, overhead: {}", per, logical_size, overhead);
        assert!(overhead < 16, "overhead should be <16 bytes, got {overhead}");
    }
}
