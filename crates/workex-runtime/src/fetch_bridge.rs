//! fetch() bridge: JS → Rust → real HTTP via reqwest.
//!
//! Registers global fetch() in QuickJS. When Worker calls fetch("https://..."),
//! Rust makes the actual HTTP request via reqwest blocking client.

use rquickjs::{Ctx, Function};

/// Register global `fetch()` in a QuickJS context.
pub fn register_fetch(ctx: &Ctx<'_>) -> anyhow::Result<()> {
    // Native: __workex_native_fetch(url, method, body) → JSON string "{status,body,headers}"
    let native_fn = Function::new(
        ctx.clone(),
        |url: String, method: String, body: String| -> String {
            match make_http_request(&url, &method, &body) {
                Ok((status, resp_body, headers)) => {
                    let h: serde_json::Value = headers
                        .into_iter()
                        .collect::<serde_json::Map<String, serde_json::Value>>()
                        .into();
                    serde_json::json!({
                        "status": status,
                        "body": resp_body,
                        "headers": h,
                    })
                    .to_string()
                }
                Err(e) => {
                    serde_json::json!({ "status": 0, "body": e, "headers": {} }).to_string()
                }
            }
        },
    )
    .map_err(|e| anyhow::anyhow!("native fetch fn: {e}"))?;

    ctx.globals()
        .set("__workex_native_fetch", native_fn)
        .map_err(|e| anyhow::anyhow!("set native fetch: {e}"))?;

    // JS wrapper: parse JSON result, return Promise<Response>
    ctx.eval::<(), _>(
        r#"
        globalThis.fetch = function(url, init) {
            var method = (init && init.method) || "GET";
            var body = (init && init.body) || "";
            var raw = __workex_native_fetch(url, method, body);
            var result = JSON.parse(raw);
            var resp = new Response(result.body, {
                status: result.status,
                headers: result.headers || {}
            });
            return Promise.resolve(resp);
        };
        "#,
    )
    .map_err(|e| anyhow::anyhow!("fetch wrapper: {e}"))?;

    Ok(())
}

/// Make a blocking HTTP request.
fn make_http_request(
    url: &str,
    method: &str,
    body: &str,
) -> Result<(u32, String, Vec<(String, serde_json::Value)>), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let req_method: reqwest::Method = method.parse().unwrap_or(reqwest::Method::GET);
    let mut builder = client.request(req_method, url);

    if !body.is_empty() {
        builder = builder.body(body.to_string());
    }

    let resp = builder.send().map_err(|e| e.to_string())?;
    let status = resp.status().as_u16() as u32;
    let headers: Vec<(String, serde_json::Value)> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.to_string(), serde_json::Value::String(v.to_string())))
        })
        .collect();
    let resp_body = resp.text().map_err(|e| e.to_string())?;

    Ok((status, resp_body, headers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{Context, Runtime, Value};

    #[test]
    fn fetch_registered_as_function() {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            ctx.eval::<(), _>(crate::engine::WORKER_POLYFILL).unwrap();
            register_fetch(&ctx).unwrap();
            let val: Value = ctx.eval("typeof fetch").unwrap();
            assert_eq!(val.as_string().unwrap().to_string().unwrap(), "function");
        });
    }

    // Real network test — run with: cargo test -p workex-runtime fetch_bridge -- --ignored
    #[test]
    #[ignore]
    fn fetch_real_http() {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            ctx.eval::<(), _>(crate::engine::WORKER_POLYFILL).unwrap();
            register_fetch(&ctx).unwrap();

            ctx.eval::<(), _>(
                r#"
                var __test_result = null;
                fetch("https://httpbin.org/get").then(function(r) {
                    __test_result = { status: r.__status, hasBody: r.__body.length > 0 };
                });
                "#,
            )
            .unwrap();
            while ctx.execute_pending_job() {}

            let result: Value = ctx.eval("__test_result").unwrap();
            let obj = result.as_object().unwrap();
            let status: u32 = obj.get("status").unwrap();
            assert_eq!(status, 200);
        });
    }
}
