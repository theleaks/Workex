//! Workex HTTP Server — for k6 benchmarking.
//!
//! Usage: workex-server [port]  (default: 3001)

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

use workex_core::isolate::{IsolateEnv, IsolatePool, ModuleHandle};

async fn handle(
    req: Request<Incoming>,
    pool: Arc<Mutex<IsolatePool>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path().to_string();

    let mut iso = pool.lock().unwrap().spawn();

    let (body, ct) = match path.as_str() {
        "/health" => {
            let s = iso.arena.alloc_str("ok");
            (s.to_string(), "text/plain")
        }
        "/json" => {
            let json = format!(
                r#"{{"status":"ok","path":"{}","runtime":"workex"}}"#,
                path
            );
            let s = iso.arena.alloc_str(&json);
            (s.to_string(), "application/json")
        }
        "/compute" => {
            fn fib(n: u32) -> u64 {
                if n <= 1 {
                    return n as u64;
                }
                fib(n - 1) + fib(n - 2)
            }
            let r = fib(30);
            let s = iso.arena.alloc_str(&format!(r#"{{"fib30":{r}}}"#));
            (s.to_string(), "application/json")
        }
        _ => {
            let s = iso.arena.alloc_str("Hello from Workex!");
            (s.to_string(), "text/plain")
        }
    };

    pool.lock().unwrap().recycle(iso);

    Ok(Response::builder()
        .header("content-type", ct)
        .header("server", "workex/0.1.0")
        .body(Full::new(Bytes::from(body)))
        .unwrap())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3001);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let module = Arc::new(ModuleHandle {
        source_hash: 0x1234,
        handler_names: vec!["fetch".to_string()],
    });
    let pool = Arc::new(Mutex::new(IsolatePool::new(module, IsolateEnv::default())));
    pool.lock().unwrap().warm();

    let listener = TcpListener::bind(addr).await?;
    eprintln!("Workex server listening on http://{addr}");

    loop {
        let (stream, _) = listener.accept().await?;
        let pool = pool.clone();

        tokio::task::spawn(async move {
            let io = TokioIo::new(stream);
            let svc = service_fn(move |req| {
                let pool = pool.clone();
                async move { handle(req, pool).await }
            });
            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                eprintln!("conn error: {e}");
            }
        });
    }
}
