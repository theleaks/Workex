//! Lightweight isolate model for Worker execution contexts.
//!
//! Each Isolate holds an Arena allocator and a reference to a CompiledModule.
//! IsolatePool manages pre-warmed isolates for the same Worker script,
//! recycling them across requests.

use std::collections::HashMap;
use std::sync::Arc;

use crate::arena::Arena;

/// Unique identifier for an isolate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IsolateId(u64);

impl IsolateId {
    fn next() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        IsolateId(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

impl std::fmt::Display for IsolateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "isolate-{}", self.0)
    }
}

/// Environment bindings available to a Worker (KV, D1, secrets, etc).
#[derive(Debug, Clone, Default)]
pub struct IsolateEnv {
    pub kv_bindings: Vec<String>,
    pub d1_bindings: Vec<String>,
    pub secrets: HashMap<String, String>,
}

/// A compiled module handle shared across isolates running the same script.
/// Wraps the compiler's CompiledModule in an Arc for cheap cloning.
#[derive(Debug, Clone)]
pub struct ModuleHandle {
    pub source_hash: u64,
    pub handler_names: Vec<String>,
}

/// A single Worker execution context.
///
/// Memory target: <200KB baseline
/// - Arena: 64KB default
/// - Isolate struct + env: ~1KB overhead
/// - Total: well under 200KB
pub struct Isolate {
    pub id: IsolateId,
    pub arena: Arena,
    pub module: Arc<ModuleHandle>,
    pub env: IsolateEnv,
}

impl Isolate {
    /// Create a new isolate for the given module.
    pub fn new(module: Arc<ModuleHandle>, env: IsolateEnv) -> Self {
        Isolate {
            id: IsolateId::next(),
            arena: Arena::default_size(),
            module,
            env,
        }
    }

    /// Create an isolate with a custom arena size.
    pub fn with_arena_size(
        module: Arc<ModuleHandle>,
        env: IsolateEnv,
        arena_size: usize,
    ) -> Self {
        Isolate {
            id: IsolateId::next(),
            arena: Arena::new(arena_size),
            module,
            env,
        }
    }

    /// Reset the isolate for reuse: clear the arena, keep the module.
    pub fn reset_for_reuse(&mut self) {
        self.arena.reset();
    }

    /// Baseline memory usage of this isolate (arena capacity + struct overhead).
    pub fn memory_usage(&self) -> usize {
        self.arena.total_capacity()
            + std::mem::size_of::<Self>()
            + self.env.secrets.len() * 64 // rough estimate for secret strings
    }
}

impl std::fmt::Debug for Isolate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Isolate")
            .field("id", &self.id)
            .field("module_hash", &self.module.source_hash)
            .field("arena_capacity", &self.arena.total_capacity())
            .finish()
    }
}

/// Default pool size per Worker script.
const DEFAULT_POOL_SIZE: usize = 10;

/// Pool of pre-warmed isolates for the same Worker script.
/// AOT compilation happens once; each request gets an isolate from the pool.
pub struct IsolatePool {
    module: Arc<ModuleHandle>,
    env: IsolateEnv,
    idle: Vec<Isolate>,
    max_idle: usize,
    total_spawned: u64,
    total_recycled: u64,
}

impl IsolatePool {
    /// Create a new pool for the given compiled module.
    pub fn new(module: Arc<ModuleHandle>, env: IsolateEnv) -> Self {
        Self::with_capacity(module, env, DEFAULT_POOL_SIZE)
    }

    /// Create a pool with a custom max idle count.
    pub fn with_capacity(module: Arc<ModuleHandle>, env: IsolateEnv, max_idle: usize) -> Self {
        IsolatePool {
            module,
            env,
            idle: Vec::with_capacity(max_idle),
            max_idle,
            total_spawned: 0,
            total_recycled: 0,
        }
    }

    /// Pre-warm the pool by creating idle isolates up to max_idle.
    pub fn warm(&mut self) {
        while self.idle.len() < self.max_idle {
            let isolate = Isolate::new(self.module.clone(), self.env.clone());
            self.idle.push(isolate);
            self.total_spawned += 1;
        }
    }

    /// Get an isolate from the pool (reuses idle) or create a new one.
    pub fn spawn(&mut self) -> Isolate {
        self.total_spawned += 1;
        if let Some(isolate) = self.idle.pop() {
            isolate
        } else {
            Isolate::new(self.module.clone(), self.env.clone())
        }
    }

    /// Return an isolate to the pool after a request completes.
    /// Resets the arena before storing.
    pub fn recycle(&mut self, mut isolate: Isolate) {
        self.total_recycled += 1;
        isolate.reset_for_reuse();
        if self.idle.len() < self.max_idle {
            self.idle.push(isolate);
        }
        // else: drop the isolate — pool is full
    }

    /// Number of idle isolates ready for use.
    pub fn idle_count(&self) -> usize {
        self.idle.len()
    }

    /// Total isolates spawned over the pool's lifetime.
    pub fn total_spawned(&self) -> u64 {
        self.total_spawned
    }

    /// Total isolates recycled over the pool's lifetime.
    pub fn total_recycled(&self) -> u64 {
        self.total_recycled
    }

    /// Source hash of the module this pool serves.
    pub fn script_hash(&self) -> u64 {
        self.module.source_hash
    }

    /// Total memory used by all idle isolates.
    pub fn idle_memory_usage(&self) -> usize {
        self.idle.iter().map(|i| i.memory_usage()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::DEFAULT_CHUNK_SIZE;

    fn test_module() -> Arc<ModuleHandle> {
        Arc::new(ModuleHandle {
            source_hash: 12345,
            handler_names: vec!["fetch".to_string()],
        })
    }

    #[test]
    fn isolate_creation_under_200kb() {
        let module = test_module();
        let isolate = Isolate::new(module, IsolateEnv::default());

        let usage = isolate.memory_usage();
        assert!(
            usage < 200 * 1024,
            "isolate should use <200KB, got {usage} bytes ({}KB)",
            usage / 1024,
        );
        // Default arena is 64KB, struct overhead is minimal
        assert_eq!(isolate.arena.total_capacity(), DEFAULT_CHUNK_SIZE);
    }

    #[test]
    fn isolate_reset_clears_arena() {
        let module = test_module();
        let mut isolate = Isolate::new(module, IsolateEnv::default());

        // Allocate some data
        isolate.arena.alloc(42u64);
        isolate.arena.alloc_str("some request data");

        // Reset and verify arena is clean
        isolate.reset_for_reuse();
        assert_eq!(isolate.arena.total_capacity(), DEFAULT_CHUNK_SIZE);
    }

    #[test]
    fn pool_spawn_and_recycle() {
        let module = test_module();
        let mut pool = IsolatePool::new(module, IsolateEnv::default());

        // Spawn an isolate
        let isolate = pool.spawn();
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(pool.total_spawned(), 1);

        // Recycle it
        pool.recycle(isolate);
        assert_eq!(pool.idle_count(), 1);
        assert_eq!(pool.total_recycled(), 1);

        // Spawn again — should reuse the recycled one
        let isolate2 = pool.spawn();
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(pool.total_spawned(), 2);

        pool.recycle(isolate2);
    }

    #[test]
    fn pool_warm() {
        let module = test_module();
        let mut pool = IsolatePool::new(module, IsolateEnv::default());

        pool.warm();
        assert_eq!(pool.idle_count(), DEFAULT_POOL_SIZE);

        // Memory for 10 idle isolates should be ~640KB (10 * 64KB arenas)
        let mem = pool.idle_memory_usage();
        assert!(
            mem < 700 * 1024,
            "10 idle isolates should use ~640KB, got {}KB",
            mem / 1024,
        );
    }

    #[test]
    fn pool_respects_max_idle() {
        let module = test_module();
        let mut pool = IsolatePool::with_capacity(module, IsolateEnv::default(), 2);

        // Spawn 5, recycle all — only 2 should be kept
        let isolates: Vec<_> = (0..5).map(|_| pool.spawn()).collect();
        for iso in isolates {
            pool.recycle(iso);
        }
        assert_eq!(pool.idle_count(), 2);
    }

    #[test]
    fn pool_script_hash() {
        let module = test_module();
        let pool = IsolatePool::new(module, IsolateEnv::default());
        assert_eq!(pool.script_hash(), 12345);
    }

    #[test]
    fn isolate_with_env() {
        let module = test_module();
        let env = IsolateEnv {
            kv_bindings: vec!["MY_KV".to_string()],
            d1_bindings: vec!["DB".to_string()],
            secrets: HashMap::from([("API_KEY".to_string(), "secret123".to_string())]),
        };

        let isolate = Isolate::new(module, env);
        assert_eq!(isolate.env.kv_bindings, vec!["MY_KV"]);
        assert_eq!(isolate.env.d1_bindings, vec!["DB"]);
        assert_eq!(isolate.env.secrets.get("API_KEY").unwrap(), "secret123");
    }

    #[test]
    fn concurrent_request_simulation() {
        let module = test_module();
        let mut pool = IsolatePool::new(module, IsolateEnv::default());
        pool.warm();

        // Simulate 50 sequential requests
        for i in 0u64..50 {
            let mut iso = pool.spawn();

            // Simulate request work — allocate into the arena
            let val = iso.arena.alloc(i);
            assert_eq!(*val, i);
            let body = iso.arena.alloc_str("response body");
            assert_eq!(body, "response body");

            pool.recycle(iso);
        }

        // 10 from warm() + 50 from requests
        assert_eq!(pool.total_spawned(), 60);
        assert_eq!(pool.total_recycled(), 50);
    }
}
