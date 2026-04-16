//! `workex` CLI — wrangler-compatible local dev server.
//!
//! Usage:
//!   workex dev              # Start dev server (reads wrangler.toml)
//!   workex dev --port 3000  # Custom port

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

mod config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let command = args.get(1).map(|s| s.as_str()).unwrap_or("dev");

    match command {
        "dev" => cmd_dev(&args).await,
        "--help" | "-h" | "help" => {
            println!("workex — Agent-native JS runtime");
            println!();
            println!("Usage:");
            println!("  workex dev [--port PORT]   Start local dev server");
            println!();
            println!("Reads wrangler.toml from current directory.");
            Ok(())
        }
        other => {
            eprintln!("Unknown command: {other}");
            eprintln!("Run `workex --help` for usage.");
            std::process::exit(1);
        }
    }
}

async fn cmd_dev(args: &[String]) -> anyhow::Result<()> {
    let port: u16 = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(8787);

    // Load wrangler.toml
    let config = config::load_config(std::path::Path::new("."))
        .map_err(|e| anyhow::anyhow!("Failed to load wrangler.toml: {e}"))?;

    println!();
    println!("  workex dev v0.1.0");
    println!("  Worker:  {}", config.main);
    println!("  Name:    {}", config.name);

    // Show bindings
    for kv in &config.kv_namespaces {
        println!("  KV:      {} ({})", kv.binding, kv.id);
    }
    for d1 in &config.d1_databases {
        println!("  D1:      {} ({})", d1.binding, d1.database_name);
    }
    for (k, _) in &config.vars {
        println!("  Var:     {}", k);
    }

    // Read Worker source
    let source = std::fs::read_to_string(&config.main)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", config.main))?;

    // Create engine pool
    let pool = Arc::new(Mutex::new(
        WorkexEnginePool::new(&source, 10)
            .map_err(|e| anyhow::anyhow!("Failed to compile Worker: {e}"))?,
    ));

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;

    println!("  Ready:   http://localhost:{port}");
    println!();

    loop {
        let (stream, _) = listener.accept().await?;
        let pool = pool.clone();

        tokio::task::spawn(async move {
            let io = TokioIo::new(stream);
            let svc = service_fn(move |req: Request<Incoming>| {
                let pool = pool.clone();
                async move {
                    let workex_req = hyper_to_workex_request(&req);
                    let resp = {
                        let mut pool = pool.lock().unwrap();
                        pool.handle(&workex_req)
                    };

                    match resp {
                        Ok(r) => Ok::<_, Infallible>(workex_to_hyper_response(r)),
                        Err(e) => Ok(Response::builder()
                            .status(500)
                            .header("content-type", "text/plain")
                            .body(Full::new(Bytes::from(format!("Worker error: {e}"))))
                            .unwrap()),
                    }
                }
            });
            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                eprintln!("conn error: {e}");
            }
        });
    }
}

fn hyper_to_workex_request(req: &Request<Incoming>) -> WorkexRequest {
    let url = format!("http://localhost{}", req.uri());
    let method = match req.method().as_str() {
        "POST" => workex_runtime::request::Method::POST,
        "PUT" => workex_runtime::request::Method::PUT,
        "DELETE" => workex_runtime::request::Method::DELETE,
        "PATCH" => workex_runtime::request::Method::PATCH,
        "HEAD" => workex_runtime::request::Method::HEAD,
        "OPTIONS" => workex_runtime::request::Method::OPTIONS,
        _ => workex_runtime::request::Method::GET,
    };
    WorkexRequest {
        method,
        url,
        headers: workex_runtime::headers::Headers::new(),
        body: None,
    }
}

fn workex_to_hyper_response(
    resp: workex_runtime::response::WorkexResponse,
) -> Response<Full<Bytes>> {
    let mut builder = Response::builder()
        .status(resp.status)
        .header("server", "workex/0.1.0");

    for (k, v) in resp.headers.entries() {
        builder = builder.header(k, v);
    }

    builder.body(Full::new(resp.body)).unwrap()
}
