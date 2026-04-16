//! RuntimeRegistry: one SharedRuntime per unique Worker script.
//!
//! Different Workers get different Runtimes (isolation).
//! Multiple concurrent requests for the SAME Worker share one Runtime.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::shared_runtime::SharedRuntime;

/// Global registry mapping Worker scripts to their SharedRuntime.
pub struct RuntimeRegistry {
    runtimes: RwLock<HashMap<u64, Arc<SharedRuntime>>>,
    pool_size: usize,
}

impl RuntimeRegistry {
    pub fn new(pool_size: usize) -> Self {
        Self {
            runtimes: RwLock::new(HashMap::new()),
            pool_size,
        }
    }

    /// Get the SharedRuntime for a Worker script, or create one.
    pub fn get_or_create(&self, source: &str) -> anyhow::Result<Arc<SharedRuntime>> {
        let hash = hash_source(source);

        // Fast path
        {
            let map = self.runtimes.read().unwrap();
            if let Some(rt) = map.get(&hash) {
                return Ok(rt.clone());
            }
        }

        // Slow path: create
        let rt = Arc::new(SharedRuntime::new(source, self.pool_size)?);
        {
            let mut map = self.runtimes.write().unwrap();
            map.entry(hash).or_insert_with(|| rt.clone());
        }
        Ok(rt)
    }

    pub fn runtime_count(&self) -> usize {
        self.runtimes.read().unwrap().len()
    }
}

fn hash_source(source: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::WorkexRequest;

    #[test]
    fn same_source_same_runtime() {
        let reg = RuntimeRegistry::new(2);
        let source = r#"export default { fetch(r) { return new Response("a"); } };"#;

        let rt1 = reg.get_or_create(source).unwrap();
        let rt2 = reg.get_or_create(source).unwrap();
        assert_eq!(rt1.script_hash(), rt2.script_hash());
        assert_eq!(reg.runtime_count(), 1);
    }

    #[test]
    fn different_source_different_runtime() {
        let reg = RuntimeRegistry::new(2);
        let s1 = r#"export default { fetch(r) { return new Response("a"); } };"#;
        let s2 = r#"export default { fetch(r) { return new Response("b"); } };"#;

        reg.get_or_create(s1).unwrap();
        reg.get_or_create(s2).unwrap();
        assert_eq!(reg.runtime_count(), 2);
    }

    #[test]
    fn registry_handles_requests() {
        let reg = RuntimeRegistry::new(2);
        let source = r#"export default { fetch(r) { return new Response("registry:" + r.url); } };"#;

        let rt = reg.get_or_create(source).unwrap();
        let resp = rt.handle(&WorkexRequest::get("https://x.com/test")).unwrap();
        assert_eq!(resp.text().unwrap(), "registry:https://x.com/test");
    }
}
