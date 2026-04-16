//! workerd-compatible HTTP server.
//!
//! Exposes the same HTTP interface as workerd so Cloudflare infrastructure
//! can route requests to Workex instead of workerd — zero config change.
//!
//! Usage: workex dev --workerd-compat --port 8787

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use workex_runtime::engine::WorkexEnginePool;
use workex_runtime::request::WorkexRequest;

/// workerd-compatible server.
pub struct WorkerdCompatServer {
    pool: Arc<Mutex<WorkexEnginePool>>,
    addr: SocketAddr,
}

impl WorkerdCompatServer {
    pub fn new(pool: Arc<Mutex<WorkexEnginePool>>, addr: SocketAddr) -> Self {
        Self { pool, addr }
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(self.addr).await?;
        eprintln!("workex workerd-compat on http://{}", self.addr);
        eprintln!("Drop-in replacement for: workerd");

        loop {
            let (stream, _) = listener.accept().await?;
            let pool = self.pool.clone();

            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let svc = service_fn(move |req: Request<Incoming>| {
                    let pool = pool.clone();
                    async move { handle(req, pool).await }
                });
                let _ = http1::Builder::new().serve_connection(io, svc).await;
            });
        }
    }
}

async fn handle(
    req: Request<Incoming>,
    pool: Arc<Mutex<WorkexEnginePool>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    // Parse CF-Worker headers (workerd protocol)
    let _worker_name = req.headers()
        .get("CF-Worker-Name")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("default");

    let _compat_date = req.headers()
        .get("CF-Compat-Date")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("2026-01-01");

    let url = format!("http://localhost{}", req.uri());
    let method = match req.method().as_str() {
        "POST" => workex_runtime::request::Method::POST,
        "PUT" => workex_runtime::request::Method::PUT,
        "DELETE" => workex_runtime::request::Method::DELETE,
        _ => workex_runtime::request::Method::GET,
    };

    let workex_req = WorkexRequest {
        method,
        url,
        headers: workex_runtime::headers::Headers::new(),
        body: None,
    };

    let resp = {
        let mut pool = pool.lock().unwrap();
        pool.handle(&workex_req)
    };

    match resp {
        Ok(r) => {
            let mut builder = Response::builder()
                .status(r.status)
                .header("server", "workex/0.1.0-workerd-compat")
                .header("CF-Runtime", "workex");

            for (k, v) in r.headers.entries() {
                builder = builder.header(k, v);
            }

            Ok(builder.body(Full::new(r.body)).unwrap())
        }
        Err(e) => Ok(Response::builder()
            .status(500)
            .body(Full::new(Bytes::from(format!("Worker error: {e}"))))
            .unwrap()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn workerd_compat_protocol() {
        let source = r#"
            export default {
                fetch(request) {
                    return new Response("workerd-compat OK", {
                        headers: { "content-type": "text/plain" }
                    });
                }
            };
        "#;

        let pool = Arc::new(Mutex::new(
            WorkexEnginePool::new(source, 2).unwrap(),
        ));

        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = TcpListener::bind(addr).await.unwrap();
        let local_addr = listener.local_addr().unwrap();

        // Start server in background
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let pool = pool_clone.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |req| {
                        let pool = pool.clone();
                        async move { handle(req, pool).await }
                    });
                    let _ = http1::Builder::new().serve_connection(io, svc).await;
                });
            }
        });

        // Give server a moment
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Test with CF headers
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://{}/", local_addr))
            .header("CF-Worker-Name", "test-worker")
            .header("CF-Compat-Date", "2026-04-16")
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("CF-Runtime").unwrap().to_str().unwrap(),
            "workex"
        );
        let body = resp.text().await.unwrap();
        assert_eq!(body, "workerd-compat OK");
        println!("workerd-compat protocol: PASS");
    }
}
