//! WorkexEngine: QuickJS-based JavaScript engine for executing Workers scripts.
//!
//! Uses rquickjs (QuickJS) for fast JS execution.
//! Response/Request are JS-side objects whose properties are read back by Rust.

use rquickjs::{Context, Ctx, Object, Runtime, Value};

use crate::headers::Headers;
use crate::request::WorkexRequest;
use crate::response::WorkexResponse;
use bytes::Bytes;

/// JS polyfill: Response and Request constructors defined in pure JS.
/// Properties are read back by Rust after execution.
pub const WORKER_POLYFILL: &str = r#"
function Response(body, init) {
    this.__body = (body === undefined || body === null) ? "" : String(body);
    this.__status = (init && init.status) ? init.status : 200;
    this.__headers = (init && init.headers) ? init.headers : {};
    this.__is_response = true;
}

function Request(url, init) {
    this.url = url || "";
    this.method = (init && init.method) ? init.method : "GET";
    this.headers = (init && init.headers) ? init.headers : {};
}
"#;

/// The Workex JS engine backed by QuickJS.
pub struct WorkexEngine {
    rt: Runtime,
    ctx: Context,
}

impl WorkexEngine {
    /// Create a new engine with Response/Request polyfills registered.
    pub fn new() -> anyhow::Result<Self> {
        let rt = Runtime::new().map_err(|e| anyhow::anyhow!("QuickJS runtime error: {e}"))?;
        let ctx = Context::full(&rt).map_err(|e| anyhow::anyhow!("QuickJS context error: {e}"))?;

        // Register polyfills
        ctx.with(|ctx| {
            ctx.eval::<(), _>(WORKER_POLYFILL)
                .map_err(|e| anyhow::anyhow!("polyfill error: {e}"))
        })?;

        Ok(WorkexEngine { rt, ctx })
    }

    /// Execute a Worker script and call its fetch handler.
    ///
    /// Accepts TypeScript or JavaScript. TS type annotations are stripped via oxc.
    pub fn execute_worker(
        &mut self,
        source: &str,
        request: WorkexRequest,
    ) -> anyhow::Result<WorkexResponse> {
        let js_source = strip_ts_annotations(source);

        // Wrap `export default { ... }` → evaluable JS
        let wrapped = if js_source.contains("export default") {
            let mut s = js_source.replace("export default", "var __workex_mod__ =");
            s.push_str("\nvoid 0;");
            s
        } else {
            format!("var __workex_mod__ = {js_source};\nvoid 0;")
        };

        let method = request.method.as_str().to_string();
        let url = request.url.clone();

        self.ctx.with(|ctx| {
            // Set request as global
            let req_obj = Object::new(ctx.clone())
                .map_err(|e| anyhow::anyhow!("request object: {e}"))?;
            req_obj
                .set("url", url.as_str())
                .map_err(|e| anyhow::anyhow!("set url: {e}"))?;
            req_obj
                .set("method", method.as_str())
                .map_err(|e| anyhow::anyhow!("set method: {e}"))?;
            ctx.globals()
                .set("__workex_request__", req_obj)
                .map_err(|e| anyhow::anyhow!("set request: {e}"))?;

            // Eval the Worker module
            ctx.eval::<(), _>(wrapped.as_bytes())
                .map_err(|e| anyhow::anyhow!("JS eval error: {e}"))?;

            // Call fetch handler
            let call_result: Value = ctx
                .eval("__workex_mod__.fetch(__workex_request__)")
                .map_err(|e| anyhow::anyhow!("fetch call error: {e}"))?;

            // Resolve promise if async
            while ctx.execute_pending_job() {}

            // Try direct Response first
            if let Some(resp) = try_extract_response(&ctx, &call_result) {
                return Ok(resp);
            }

            // If it was a Promise, resolve it
            if call_result.is_object() {
                // Set up promise resolution
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
                .map_err(|e| anyhow::anyhow!("promise resolve: {e}"))?;

                // Execute promise jobs
                while ctx.execute_pending_job() {}

                let resolved: Value = ctx
                    .eval("__workex_resolved__")
                    .map_err(|e| anyhow::anyhow!("get resolved: {e}"))?;

                if let Some(resp) = try_extract_response(&ctx, &resolved) {
                    return Ok(resp);
                }
            }

            // Fallback: treat as string
            if let Some(s) = call_result.as_string() {
                let text = s
                    .to_string()
                    .map_err(|e| anyhow::anyhow!("string convert: {e}"))?;
                return Ok(WorkexResponse::new(text));
            }

            anyhow::bail!("Worker fetch() did not return a Response object")
        })
    }
}

/// A pre-warmed engine: Worker source compiled once, context reused across requests.
/// The hot path is just: set request globals → call fetch → read response.
struct WarmContext {
    rt: Runtime,
    ctx: Context,
}

impl WarmContext {
    /// Create a warm context: polyfill + fetch bridge + Worker source pre-compiled.
    fn new(js_source: &str) -> anyhow::Result<Self> {
        let rt = Runtime::new().map_err(|e| anyhow::anyhow!("{e}"))?;
        let ctx = Context::full(&rt).map_err(|e| anyhow::anyhow!("{e}"))?;

        ctx.with(|ctx| -> anyhow::Result<()> {
            ctx.eval::<(), _>(WORKER_POLYFILL)
                .map_err(|e| anyhow::anyhow!("polyfill: {e}"))?;
            // Register real fetch() bridge
            crate::fetch_bridge::register_fetch(&ctx)?;
            ctx.eval::<(), _>(js_source.as_bytes())
                .map_err(|e| anyhow::anyhow!("worker source: {e}"))?;
            Ok(())
        })?;

        Ok(WarmContext { rt, ctx })
    }

    /// Execute a request on this pre-warmed context.
    /// Only sets request, calls fetch, reads response — no source eval.
    fn handle_request(&self, request: &WorkexRequest) -> anyhow::Result<WorkexResponse> {
        self.ctx.with(|ctx| {
            // Set request
            let req_obj = Object::new(ctx.clone())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            req_obj.set("url", request.url.as_str()).map_err(|e| anyhow::anyhow!("{e}"))?;
            req_obj.set("method", request.method.as_str()).map_err(|e| anyhow::anyhow!("{e}"))?;
            ctx.globals().set("__workex_request__", req_obj)
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            // Call fetch — module already compiled
            let result: Value = ctx.eval("__workex_mod__.fetch(__workex_request__)")
                .map_err(|e| anyhow::anyhow!("fetch: {e}"))?;

            while ctx.execute_pending_job() {}

            // Direct response
            if let Some(resp) = try_extract_response(&ctx, &result) {
                return Ok(resp);
            }

            // Promise resolution
            if result.is_object() {
                ctx.eval::<(), _>(
                    "var __workex_resolved__ = null;\
                     var __p__ = __workex_mod__.fetch(__workex_request__);\
                     if (__p__ && typeof __p__.then === 'function') { __p__.then(function(r) { __workex_resolved__ = r; }); }\
                     else { __workex_resolved__ = __p__; }"
                ).map_err(|e| anyhow::anyhow!("{e}"))?;
                while ctx.execute_pending_job() {}

                let resolved: Value = ctx.eval("__workex_resolved__")
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                if let Some(resp) = try_extract_response(&ctx, &resolved) {
                    return Ok(resp);
                }
            }

            anyhow::bail!("fetch() did not return a Response")
        })
    }
}

/// Pool of pre-warmed QuickJS contexts for the same Worker script.
///
/// Worker source is compiled once. Each request takes a context from the pool,
/// calls `fetch()`, and returns it. No re-parsing, no re-eval.
pub struct WorkexEnginePool {
    /// Pre-compiled JS source (TS stripped, export default wrapped).
    compiled_source: String,
    /// Idle warm contexts ready for requests.
    pool: Vec<WarmContext>,
    /// Max idle contexts.
    max_idle: usize,
}

impl WorkexEnginePool {
    /// Create a pool for a Worker script. Pre-warms `pool_size` contexts.
    pub fn new(source: &str, pool_size: usize) -> anyhow::Result<Self> {
        let js = prepare_source(source);
        let mut pool = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            pool.push(WarmContext::new(&js)?);
        }
        Ok(WorkexEnginePool {
            compiled_source: js,
            pool,
            max_idle: pool_size,
        })
    }

    /// Handle a request using a pre-warmed context.
    pub fn handle(&mut self, request: &WorkexRequest) -> anyhow::Result<WorkexResponse> {
        let ctx = if let Some(c) = self.pool.pop() {
            c
        } else {
            WarmContext::new(&self.compiled_source)?
        };

        let resp = ctx.handle_request(request);

        // Return context to pool
        if self.pool.len() < self.max_idle {
            self.pool.push(ctx);
        }

        resp
    }

    /// Number of idle contexts.
    pub fn idle_count(&self) -> usize {
        self.pool.len()
    }
}

/// Prepare Worker source: strip TS, wrap export default.
fn prepare_source(source: &str) -> String {
    let js = strip_ts_annotations(source);
    if js.contains("export default") {
        let mut s = js.replace("export default", "var __workex_mod__ =");
        s.push_str("\nvoid 0;");
        s
    } else {
        format!("var __workex_mod__ = {js};\nvoid 0;")
    }
}

/// Extract a WorkexResponse from a JS value by reading __body, __status, __headers.
fn try_extract_response(ctx: &Ctx<'_>, value: &Value<'_>) -> Option<WorkexResponse> {
    let obj = value.as_object()?;

    // Check __is_response flag
    let is_resp: bool = obj.get("__is_response").ok()?;
    if !is_resp {
        return None;
    }

    let body: String = obj.get("__body").unwrap_or_default();
    let status: u16 = obj.get::<_, u32>("__status").unwrap_or(200) as u16;

    let mut headers = Headers::new();
    if let Ok(h_obj) = obj.get::<_, Object>("__headers") {
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

/// Strip TypeScript type annotations using oxc AST spans.
fn strip_ts_annotations(source: &str) -> String {
    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::from_path("worker.ts").unwrap_or_default();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    if !parsed.errors.is_empty() {
        return source.to_string();
    }

    // Collect type annotation spans to remove
    let mut remove_spans: Vec<(u32, u32)> = Vec::new();
    for stmt in &parsed.program.body {
        collect_type_spans(stmt, &mut remove_spans);
    }

    // Remove spans in reverse order
    remove_spans.sort_by_key(|&(start, _)| std::cmp::Reverse(start));
    let mut result = source.to_string();
    for (start, end) in remove_spans {
        // Also remove preceding `: `
        let actual_start = if start >= 2 {
            let prefix = &source[start as usize - 2..start as usize];
            if prefix == ": " {
                start - 2
            } else if source.as_bytes().get(start as usize - 1) == Some(&b':') {
                start - 1
            } else {
                start
            }
        } else {
            start
        };
        result.replace_range(actual_start as usize..end as usize, "");
    }

    result
}

fn collect_type_spans(stmt: &oxc_ast::ast::Statement<'_>, spans: &mut Vec<(u32, u32)>) {
    match stmt {
        oxc_ast::ast::Statement::ExportDefaultDeclaration(export) => {
            if let oxc_ast::ast::ExportDefaultDeclarationKind::ObjectExpression(obj) =
                &export.declaration
            {
                for prop in &obj.properties {
                    if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                        if let oxc_ast::ast::Expression::FunctionExpression(func) = &p.value {
                            collect_fn_types(func, spans);
                        }
                    }
                }
            }
        }
        oxc_ast::ast::Statement::FunctionDeclaration(func) => {
            collect_fn_types(func, spans);
        }
        _ => {}
    }
}

fn collect_fn_types(func: &oxc_ast::ast::Function<'_>, spans: &mut Vec<(u32, u32)>) {
    if let Some(rt) = &func.return_type {
        spans.push((rt.span.start, rt.span.end));
    }
    for param in &func.params.items {
        if let Some(ta) = &param.type_annotation {
            spans.push((ta.span.start, ta.span.end));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_response_construction() {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            ctx.eval::<(), _>(WORKER_POLYFILL).unwrap();
            let val: Value = ctx
                .eval(r#"new Response("hello", { status: 201 })"#)
                .unwrap();
            let resp = try_extract_response(&ctx, &val).unwrap();
            assert_eq!(resp.text().unwrap(), "hello");
            assert_eq!(resp.status, 201);
        });
    }

    #[test]
    fn response_with_headers() {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            ctx.eval::<(), _>(WORKER_POLYFILL).unwrap();
            let val: Value = ctx
                .eval(r#"new Response("ok", { headers: { "content-type": "text/plain" } })"#)
                .unwrap();
            let resp = try_extract_response(&ctx, &val).unwrap();
            assert_eq!(resp.headers.get("content-type"), Some("text/plain"));
        });
    }

    #[test]
    fn execute_hello_worker() {
        let source = r#"
            export default {
                fetch(request) {
                    return new Response("Hello from Workex!", {
                        headers: { "content-type": "text/plain" }
                    });
                }
            };
        "#;
        let mut engine = WorkexEngine::new().unwrap();
        let req = WorkexRequest::get("https://example.com/");
        let resp = engine.execute_worker(source, req).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.text().unwrap(), "Hello from Workex!");
        assert_eq!(resp.headers.get("content-type"), Some("text/plain"));
    }

    #[test]
    fn worker_reads_request_url() {
        let source = r#"
            export default {
                fetch(request) {
                    return new Response("url=" + request.url);
                }
            };
        "#;
        let mut engine = WorkexEngine::new().unwrap();
        let req = WorkexRequest::get("https://api.example.com/test");
        let resp = engine.execute_worker(source, req).unwrap();
        assert_eq!(resp.text().unwrap(), "url=https://api.example.com/test");
    }

    #[test]
    fn worker_reads_request_method() {
        let source = r#"
            export default {
                fetch(request) {
                    return new Response("method=" + request.method);
                }
            };
        "#;
        let mut engine = WorkexEngine::new().unwrap();
        let req = WorkexRequest::post("https://example.com/data", "body");
        let resp = engine.execute_worker(source, req).unwrap();
        assert_eq!(resp.text().unwrap(), "method=POST");
    }

    // ── Pool tests ──

    #[test]
    fn pool_basic() {
        let source = r#"
            export default {
                fetch(request) {
                    return new Response("pooled:" + request.url);
                }
            };
        "#;
        let mut pool = WorkexEnginePool::new(source, 3).unwrap();
        assert_eq!(pool.idle_count(), 3);

        let resp = pool.handle(&WorkexRequest::get("https://x.com/a")).unwrap();
        assert_eq!(resp.text().unwrap(), "pooled:https://x.com/a");
        assert_eq!(pool.idle_count(), 3); // returned to pool
    }

    #[test]
    fn pool_multiple_requests() {
        let source = r#"
            export default {
                fetch(request) {
                    return new Response("path=" + request.url);
                }
            };
        "#;
        let mut pool = WorkexEnginePool::new(source, 2).unwrap();

        for i in 0..100 {
            let resp = pool.handle(&WorkexRequest::get(&format!("https://x.com/{i}"))).unwrap();
            assert_eq!(resp.text().unwrap(), format!("path=https://x.com/{i}"));
        }
        assert_eq!(pool.idle_count(), 2);
    }

    #[test]
    fn pool_async_worker() {
        let source = r#"
            export default {
                async fetch(request) {
                    var data = await Promise.resolve({ url: request.url });
                    return new Response(JSON.stringify(data));
                }
            };
        "#;
        let mut pool = WorkexEnginePool::new(source, 2).unwrap();
        let resp = pool.handle(&WorkexRequest::get("https://x.com/async")).unwrap();
        let body: serde_json::Value = resp.json_body().unwrap();
        assert_eq!(body["url"], "https://x.com/async");
    }

    #[test]
    fn pool_with_headers() {
        let source = r#"
            export default {
                fetch(request) {
                    return new Response("ok", {
                        status: 201,
                        headers: { "x-custom": "value" }
                    });
                }
            };
        "#;
        let mut pool = WorkexEnginePool::new(source, 1).unwrap();
        let resp = pool.handle(&WorkexRequest::get("https://x.com/")).unwrap();
        assert_eq!(resp.status, 201);
        assert_eq!(resp.headers.get("x-custom"), Some("value"));
    }

    #[test]
    fn pool_ts_source() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/workers/hello.ts"),
        ).unwrap();
        let mut pool = WorkexEnginePool::new(&source, 2).unwrap();
        let resp = pool.handle(&WorkexRequest::get("https://x.com/")).unwrap();
        assert_eq!(resp.text().unwrap(), "Hello from Workex!");
        assert_eq!(resp.status, 200);
    }
}
