//! SharedRuntime: one QuickJS Runtime shared across many Contexts.
//!
//! QuickJS Runtime manages GC, atom table, and bytecode cache — shared resources.
//! Each Context has its own stack and global scope — isolated per request.
//! This is QuickJS's designed usage: many Contexts on one Runtime.
//!
//! Memory model:
//!   SharedRuntime (~50KB) — created once per Worker script
//!   ├── Context 1 (~15KB) — isolated global scope + stack
//!   ├── Context 2 (~15KB)
//!   └── ...N contexts
//!
//! vs old WorkexEnginePool:
//!   Engine 1 (Runtime ~50KB + Context ~15KB) — separate per instance
//!   Engine 2 (Runtime ~50KB + Context ~15KB)
//!   → 10K engines = 10K × 65KB = 650MB
//!
//! SharedRuntime:
//!   1 Runtime (~50KB) + 10K Contexts (~15KB each) = ~150MB

use std::sync::Mutex;

use rquickjs::{Context, Runtime};

use crate::engine::WORKER_POLYFILL;
use crate::headers::Headers;
use crate::request::WorkexRequest;
use crate::response::WorkexResponse;
use bytes::Bytes;

/// One QuickJS Runtime shared across all Contexts for the same Worker script.
pub struct SharedRuntime {
    rt: Runtime,
    compiled_source: String,
    idle: Mutex<Vec<Context>>,
    max_pool: usize,
    script_hash: u64,
}

impl SharedRuntime {
    /// Create a SharedRuntime. Compiles source once, pre-warms `pool_size` Contexts.
    pub fn new(source: &str, pool_size: usize) -> anyhow::Result<Self> {
        let rt = Runtime::new().map_err(|e| anyhow::anyhow!("Runtime: {e}"))?;
        // Memory limit scales with pool size
        let mem_limit = (pool_size as usize).max(10) * 256 * 1024; // ~256KB per context
        rt.set_memory_limit(mem_limit);
        rt.set_gc_threshold(512 * 1024);

        let js = crate::engine::prepare_source(source);
        let hash = hash_source(source);

        let shared = Self {
            rt,
            compiled_source: js,
            idle: Mutex::new(Vec::with_capacity(pool_size)),
            max_pool: pool_size,
            script_hash: hash,
        };

        // Pre-warm contexts
        {
            let mut idle = shared.idle.lock().unwrap();
            for _ in 0..pool_size {
                idle.push(shared.create_context()?);
            }
        }

        Ok(shared)
    }

    /// Create a new Context on this shared Runtime.
    fn create_context(&self) -> anyhow::Result<Context> {
        let ctx = Context::full(&self.rt).map_err(|e| anyhow::anyhow!("Context: {e}"))?;

        ctx.with(|ctx| -> anyhow::Result<()> {
            ctx.eval::<(), _>(WORKER_POLYFILL)
                .map_err(|e| anyhow::anyhow!("polyfill: {e}"))?;
            crate::fetch_bridge::register_fetch(&ctx)?;
            ctx.eval::<(), _>(self.compiled_source.as_bytes())
                .map_err(|e| anyhow::anyhow!("worker load: {e}"))?;
            Ok(())
        })?;

        Ok(ctx)
    }

    /// Acquire a Context from pool or create a new one.
    fn acquire(&self) -> anyhow::Result<Context> {
        let mut idle = self.idle.lock().unwrap();
        if let Some(ctx) = idle.pop() {
            return Ok(ctx);
        }
        drop(idle);
        self.create_context()
    }

    /// Return Context to pool.
    fn release(&self, ctx: Context) {
        let mut idle = self.idle.lock().unwrap();
        if idle.len() < self.max_pool {
            idle.push(ctx);
        }
    }

    /// Handle a request: acquire context → call fetch → release.
    pub fn handle(&self, request: &WorkexRequest) -> anyhow::Result<WorkexResponse> {
        let ctx = self.acquire()?;
        let result = call_fetch(&ctx, request);
        self.release(ctx);
        result
    }

    pub fn script_hash(&self) -> u64 {
        self.script_hash
    }

    pub fn idle_count(&self) -> usize {
        self.idle.lock().unwrap().len()
    }
}

/// Call the Worker's fetch handler on a Context.
fn call_fetch(ctx: &Context, request: &WorkexRequest) -> anyhow::Result<WorkexResponse> {
    ctx.with(|ctx| {
        let req_obj =
            rquickjs::Object::new(ctx.clone()).map_err(|e| anyhow::anyhow!("{e}"))?;
        req_obj
            .set("url", request.url.as_str())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        req_obj
            .set("method", request.method.as_str())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        ctx.globals()
            .set("__workex_request__", req_obj)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        ctx.eval::<(), _>(
            r#"
            var __workex_resolved__ = null;
            var __p__ = __workex_mod__.fetch(__workex_request__);
            if (__p__ && typeof __p__.then === 'function') {
                __p__.then(function(r) { __workex_resolved__ = r; });
            } else {
                __workex_resolved__ = __p__;
            }
            "#,
        )
        .map_err(|e| anyhow::anyhow!("fetch: {e}"))?;

        while ctx.execute_pending_job() {}

        let resolved: rquickjs::Value = ctx
            .eval("__workex_resolved__")
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        extract_response(&ctx, &resolved)
            .ok_or_else(|| anyhow::anyhow!("fetch() did not return Response"))
    })
}

/// Extract WorkexResponse from a JS value (reads __body, __status, __headers).
pub fn extract_response(
    _ctx: &rquickjs::Ctx<'_>,
    value: &rquickjs::Value<'_>,
) -> Option<WorkexResponse> {
    let obj = value.as_object()?;
    let is_resp: bool = obj.get("__is_response").ok()?;
    if !is_resp {
        return None;
    }

    let body: String = obj.get("__body").unwrap_or_default();
    let status: u16 = obj.get::<_, u32>("__status").unwrap_or(200) as u16;

    let mut headers = Headers::new();
    if let Ok(h_obj) = obj.get::<_, rquickjs::Object>("__headers") {
        let keys: rquickjs::object::ObjectKeysIter<std::string::String> = h_obj.keys();
        for key_result in keys {
            if let Ok(key) = key_result {
                let val: std::string::String = h_obj.get(&*key).unwrap_or_default();
                headers.set(&key, &val);
            }
        }
    }

    Some(WorkexResponse::with_init(
        Bytes::from(body),
        status,
        headers,
    ))
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

    const TEST_WORKER: &str = r#"
        export default {
            fetch(request) {
                return new Response("shared:" + request.url, {
                    headers: { "content-type": "text/plain" }
                });
            }
        };
    "#;

    #[test]
    fn basic_shared_runtime() {
        let rt = SharedRuntime::new(TEST_WORKER, 3).unwrap();
        assert_eq!(rt.idle_count(), 3);

        let resp = rt.handle(&WorkexRequest::get("https://x.com/a")).unwrap();
        assert_eq!(resp.text().unwrap(), "shared:https://x.com/a");
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn many_requests_same_runtime() {
        let rt = SharedRuntime::new(TEST_WORKER, 2).unwrap();

        for i in 0..200 {
            let resp = rt
                .handle(&WorkexRequest::get(&format!("https://x.com/{i}")))
                .unwrap();
            assert_eq!(resp.text().unwrap(), format!("shared:https://x.com/{i}"));
        }
    }

    #[test]
    fn async_worker_shared() {
        let source = r#"
            export default {
                async fetch(request) {
                    var data = await Promise.resolve({ url: request.url });
                    return new Response(JSON.stringify(data));
                }
            };
        "#;
        let rt = SharedRuntime::new(source, 2).unwrap();
        let resp = rt.handle(&WorkexRequest::get("https://x.com/async")).unwrap();
        let body: serde_json::Value = resp.json_body().unwrap();
        assert_eq!(body["url"], "https://x.com/async");
    }

    #[test]
    fn ts_worker_shared() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/workers/hello.ts"),
        )
        .unwrap();
        let rt = SharedRuntime::new(&source, 2).unwrap();
        let resp = rt.handle(&WorkexRequest::get("https://x.com/")).unwrap();
        assert_eq!(resp.text().unwrap(), "Hello from Workex!");
    }

    #[test]
    fn pool_refills() {
        let rt = SharedRuntime::new(TEST_WORKER, 1).unwrap();
        assert_eq!(rt.idle_count(), 1);

        let resp = rt.handle(&WorkexRequest::get("https://x.com/")).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(rt.idle_count(), 1); // returned to pool
    }
}
