//! WorkexEngine: Boa-based JavaScript engine for executing Workers scripts.
//!
//! Registers `Response`, `Request`, `Headers` as global JS classes
//! backed by our Rust implementations, then executes Worker source code.

use boa_engine::class::{Class, ClassBuilder};
use boa_engine::property::Attribute;
use boa_engine::{
    js_string, Context, JsArgs, JsData, JsNativeError, JsObject, JsResult, JsValue, Source,
};
use boa_gc::{Finalize, Trace};

use crate::headers::Headers;
use crate::request::WorkexRequest;
use crate::response::WorkexResponse;
use bytes::Bytes;

/// JS-side Response object backed by our Rust WorkexResponse.
#[derive(Debug, Trace, Finalize, JsData)]
pub struct JsResponse {
    #[unsafe_ignore_trace]
    pub body: String,
    pub status: u16,
    #[unsafe_ignore_trace]
    pub headers: Vec<(String, String)>,
}

impl Class for JsResponse {
    const NAME: &'static str = "Response";
    const LENGTH: usize = 1;

    fn data_constructor(
        _new_target: &JsValue,
        args: &[JsValue],
        context: &mut Context,
    ) -> JsResult<Self> {
        // new Response(body?, init?)
        let body = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();

        let mut status = 200u16;
        let mut headers = Vec::new();

        // Parse init object: { status, headers }
        if let Some(init) = args.get(1) {
            if let Some(obj) = init.as_object() {
                // status
                if let Ok(s) = obj.get(js_string!("status"), context) {
                    if !s.is_undefined() && !s.is_null() {
                        if let Ok(n) = s.to_number(context) {
                            if !n.is_nan() && n > 0.0 {
                                status = n as u16;
                            }
                        }
                    }
                }
                // headers
                if let Ok(h) = obj.get(js_string!("headers"), context) {
                    if let Some(h_obj) = h.as_object() {
                        // Iterate own properties
                        let keys = h_obj.own_property_keys(context)?;
                        for key in keys {
                            if let Ok(val) = h_obj.get(key.clone(), context) {
                                let k = key.to_string();
                                let v = val.to_string(context)?.to_std_string_escaped();
                                headers.push((k, v));
                            }
                        }
                    }
                }
            }
        }

        Ok(JsResponse {
            body,
            status,
            headers,
        })
    }

    fn init(_class: &mut ClassBuilder<'_>) -> JsResult<()> {
        Ok(())
    }
}

impl JsResponse {
    /// Convert to our Rust WorkexResponse.
    pub fn to_workex_response(&self) -> WorkexResponse {
        let mut h = Headers::new();
        for (k, v) in &self.headers {
            h.set(k, v);
        }
        WorkexResponse::with_init(Bytes::from(self.body.clone()), self.status, h)
    }
}

/// JS-side Request object backed by our Rust WorkexRequest.
#[derive(Debug, Trace, Finalize, JsData)]
pub struct JsRequest {
    #[unsafe_ignore_trace]
    pub method: String,
    #[unsafe_ignore_trace]
    pub url: String,
}

impl Class for JsRequest {
    const NAME: &'static str = "Request";
    const LENGTH: usize = 1;

    fn data_constructor(
        _new_target: &JsValue,
        args: &[JsValue],
        context: &mut Context,
    ) -> JsResult<Self> {
        let url = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();
        Ok(JsRequest {
            method: "GET".into(),
            url,
        })
    }

    fn object_constructor(
        instance: &JsObject,
        _args: &[JsValue],
        context: &mut Context,
    ) -> JsResult<()> {
        let req = instance.downcast_ref::<JsRequest>().ok_or_else(|| {
            JsNativeError::typ().with_message("invalid Request")
        })?;
        instance.set(js_string!("method"), js_string!(req.method.as_str()), false, context)?;
        instance.set(js_string!("url"), js_string!(req.url.as_str()), false, context)?;
        Ok(())
    }

    fn init(_class: &mut ClassBuilder<'_>) -> JsResult<()> {
        Ok(())
    }
}

/// The Workex JS engine. Creates a Boa context with Workers globals registered.
pub struct WorkexEngine {
    context: Context,
}

impl WorkexEngine {
    /// Create a new engine with Response and Request classes registered.
    pub fn new() -> JsResult<Self> {
        let mut context = Context::default();

        // Register Response class
        context.register_global_class::<JsResponse>()?;

        // Register Request class
        context.register_global_class::<JsRequest>()?;

        Ok(WorkexEngine { context })
    }

    /// Execute a Worker script and call its fetch handler.
    ///
    /// Accepts TypeScript or JavaScript. TS type annotations are stripped.
    /// Supports `export default { fetch(request) { ... } }` syntax.
    pub fn execute_worker(
        &mut self,
        source: &str,
        request: WorkexRequest,
    ) -> anyhow::Result<WorkexResponse> {
        // Strip TypeScript type annotations so Boa (JS-only) can parse it
        let source = &strip_ts_annotations(source);
        // Create request object in JS
        let req_obj = JsRequest {
            method: request.method.as_str().to_string(),
            url: request.url.clone(),
        };

        // Register the request as a global
        let js_req = JsObject::from_proto_and_data(
            self.context.intrinsics().constructors().object().prototype(),
            req_obj,
        );
        // Set properties on the JS object
        js_req
            .set(
                js_string!("method"),
                js_string!(request.method.as_str()),
                false,
                &mut self.context,
            )
            .map_err(|e| anyhow::anyhow!("set method: {e}"))?;
        js_req
            .set(
                js_string!("url"),
                js_string!(request.url.as_str()),
                false,
                &mut self.context,
            )
            .map_err(|e| anyhow::anyhow!("set url: {e}"))?;

        let _ = self.context.register_global_property(
            js_string!("__workex_request__"),
            js_req,
            Attribute::all(),
        );

        // Wrap ES module syntax into an evaluable expression.
        // `export default { fetch(req) { ... } }` → extract the object.
        let wrapped = if source.contains("export default") {
            source
                .replace("export default", "var __workex_mod__ =")
                .replace("};", "};\n__workex_mod__.fetch(__workex_request__);")
        } else {
            format!("var __workex_mod__ = {source};\n__workex_mod__.fetch(__workex_request__);")
        };

        // Execute
        let result = self
            .context
            .eval(Source::from_bytes(wrapped.as_bytes()))
            .map_err(|e| anyhow::anyhow!("JS eval error: {e}"))?;

        // Run pending promise jobs (for async fetch handlers)
        let _ = self.context.run_jobs();

        // Try to extract Response directly
        if let Some(resp) = try_extract_response(&result) {
            return Ok(resp);
        }

        // If result is a Promise, resolve it
        if let Some(obj) = result.as_object() {
            // Try to get the promise result via `.then()` — store in global
            let _ = self.context.eval(Source::from_bytes(
                b"var __workex_resolved__; void 0;",
            ));

            // Use a simpler approach: call the fetch synchronously
            // by evaluating the expression and checking __workex_result__
            let resolve_script = format!(
                "var __p__ = {};\
                 if (__p__ && typeof __p__.then === 'function') {{\
                   __p__.then(function(r) {{ __workex_resolved__ = r; }});\
                 }} else {{\
                   __workex_resolved__ = __p__;\
                 }}",
                "__workex_mod__.fetch(__workex_request__)"
            );

            let _ = self.context.eval(Source::from_bytes(resolve_script.as_bytes()));
            let _ = self.context.run_jobs();

            if let Ok(resolved) = self.context.eval(Source::from_bytes(b"__workex_resolved__")) {
                if let Some(resp) = try_extract_response(&resolved) {
                    return Ok(resp);
                }
            }
        }

        // If result is a string, wrap it in a basic Response
        if result.is_string() {
            let s = result
                .to_string(&mut self.context)
                .map_err(|e| anyhow::anyhow!("to_string: {e}"))?
                .to_std_string_escaped();
            return Ok(WorkexResponse::new(s));
        }

        anyhow::bail!("Worker fetch() did not return a Response object")
    }
}

/// Try to extract a WorkexResponse from a JsValue.
fn try_extract_response(value: &JsValue) -> Option<WorkexResponse> {
    if let Some(obj) = value.as_object() {
        if let Some(resp) = obj.downcast_ref::<JsResponse>() {
            return Some(resp.to_workex_response());
        }
    }
    None
}

/// Strip TypeScript type annotations using oxc AST spans.
/// Parses as TypeScript, identifies type annotation spans, removes them from source.
fn strip_ts_annotations(source: &str) -> String {
    use oxc_ast::ast::*;

    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::from_path("worker.ts").unwrap_or_default();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    if !parsed.errors.is_empty() {
        return source.to_string();
    }

    // Collect spans of type annotations to remove
    let mut remove_spans: Vec<(u32, u32)> = Vec::new();

    for stmt in &parsed.program.body {
        collect_type_spans(stmt, &mut remove_spans);
    }

    // Sort spans and remove from source (in reverse to preserve offsets)
    remove_spans.sort_by_key(|&(start, _)| std::cmp::Reverse(start));

    let mut result = source.to_string();
    for (start, end) in remove_spans {
        // Also remove the preceding `: ` or `: `
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

/// Walk all statements and collect type annotation spans from functions.
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
        let mut ctx = Context::default();
        ctx.register_global_class::<JsResponse>().unwrap();

        let result = ctx
            .eval(Source::from_bytes(
                b"new Response('hello', { status: 201 })",
            ))
            .unwrap();

        let obj = result.as_object().unwrap();
        let resp = obj.downcast_ref::<JsResponse>().unwrap();
        assert_eq!(resp.body, "hello");
        assert_eq!(resp.status, 201);
    }

    #[test]
    fn response_with_headers() {
        let mut ctx = Context::default();
        ctx.register_global_class::<JsResponse>().unwrap();

        let result = ctx
            .eval(Source::from_bytes(
                b"new Response('ok', { headers: { 'content-type': 'text/plain' } })",
            ))
            .unwrap();

        let obj = result.as_object().unwrap();
        let resp = obj.downcast_ref::<JsResponse>().unwrap();
        let workex_resp = resp.to_workex_response();
        assert_eq!(workex_resp.text().unwrap(), "ok");
        assert_eq!(
            workex_resp.headers.get("content-type"),
            Some("text/plain")
        );
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
        assert_eq!(
            resp.headers.get("content-type"),
            Some("text/plain")
        );
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
}
