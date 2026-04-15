//! Integration test: execute real Workers scripts through WorkexEngine.

use workex_runtime::engine::WorkexEngine;
use workex_runtime::request::WorkexRequest;

#[test]
fn execute_hello_ts_from_disk() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/workers/hello.ts"),
    )
    .expect("should read hello.ts");

    let mut engine = WorkexEngine::new().unwrap();
    let req = WorkexRequest::get("https://example.com/");
    let resp = engine.execute_worker(&source, req).unwrap();

    assert_eq!(resp.status, 200);
    assert_eq!(resp.text().unwrap(), "Hello from Workex!");
    assert_eq!(resp.headers.get("content-type"), Some("text/plain"));
}

#[test]
fn execute_json_api_worker() {
    let source = r#"
        export default {
            fetch(request) {
                var url = request.url;
                var body = JSON.stringify({ status: "ok", url: url });
                return new Response(body, {
                    status: 200,
                    headers: { "content-type": "application/json" }
                });
            }
        };
    "#;

    let mut engine = WorkexEngine::new().unwrap();
    let req = WorkexRequest::get("https://api.example.com/data");
    let resp = engine.execute_worker(source, req).unwrap();

    assert_eq!(resp.status, 200);
    assert_eq!(resp.headers.get("content-type"), Some("application/json"));

    let body: serde_json::Value = resp.json_body().unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["url"], "https://api.example.com/data");
}

#[test]
fn execute_custom_status_worker() {
    let source = r#"
        export default {
            fetch(request) {
                return new Response("not found", { status: 404 });
            }
        };
    "#;

    let mut engine = WorkexEngine::new().unwrap();
    let req = WorkexRequest::get("https://example.com/missing");
    let resp = engine.execute_worker(source, req).unwrap();

    assert_eq!(resp.status, 404);
    assert_eq!(resp.text().unwrap(), "not found");
}

#[test]
fn execute_computation_worker() {
    let source = r#"
        export default {
            fetch(request) {
                function fib(n) {
                    if (n <= 1) return n;
                    return fib(n - 1) + fib(n - 2);
                }
                var result = fib(10);
                return new Response("fib10=" + result);
            }
        };
    "#;

    let mut engine = WorkexEngine::new().unwrap();
    let req = WorkexRequest::get("https://example.com/compute");
    let resp = engine.execute_worker(source, req).unwrap();

    assert_eq!(resp.text().unwrap(), "fib10=55");
}

#[test]
fn execute_method_routing_worker() {
    let source = r#"
        export default {
            fetch(request) {
                if (request.method === "POST") {
                    return new Response("created", { status: 201 });
                }
                return new Response("ok");
            }
        };
    "#;

    let mut engine = WorkexEngine::new().unwrap();

    let get_resp = engine
        .execute_worker(source, WorkexRequest::get("https://example.com/"))
        .unwrap();
    assert_eq!(get_resp.text().unwrap(), "ok");
    assert_eq!(get_resp.status, 200);

    // Need a fresh engine since globals persist
    let mut engine2 = WorkexEngine::new().unwrap();
    let post_resp = engine2
        .execute_worker(source, WorkexRequest::post("https://example.com/", "data"))
        .unwrap();
    assert_eq!(post_resp.text().unwrap(), "created");
    assert_eq!(post_resp.status, 201);
}
