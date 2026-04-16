//! Outbound fetch() — real HTTP client via reqwest.
//!
//! Matches the Workers fetch() API: takes a Request, returns a Response.

use crate::headers::Headers;
use crate::request::WorkexRequest;
use crate::response::WorkexResponse;
use bytes::Bytes;

/// Execute an outbound HTTP request using reqwest.
pub async fn fetch(request: WorkexRequest) -> anyhow::Result<WorkexResponse> {
    let client = reqwest::Client::new();

    let method = match request.method {
        crate::request::Method::GET => reqwest::Method::GET,
        crate::request::Method::POST => reqwest::Method::POST,
        crate::request::Method::PUT => reqwest::Method::PUT,
        crate::request::Method::DELETE => reqwest::Method::DELETE,
        crate::request::Method::PATCH => reqwest::Method::PATCH,
        crate::request::Method::HEAD => reqwest::Method::HEAD,
        crate::request::Method::OPTIONS => reqwest::Method::OPTIONS,
    };

    let mut builder = client.request(method, &request.url);

    // Copy headers
    for (k, v) in request.headers.entries() {
        builder = builder.header(k, v);
    }

    // Set body if present
    if let Some(body) = request.body {
        builder = builder.body(body);
    }

    let resp = builder.send().await?;

    let status = resp.status().as_u16();
    let mut headers = Headers::new();
    for (k, v) in resp.headers() {
        if let Ok(v_str) = v.to_str() {
            headers.append(k.as_str(), v_str);
        }
    }

    let body = resp.bytes().await?;

    Ok(WorkexResponse::with_init(
        Bytes::from(body),
        status,
        headers,
    ))
}

/// Create a mock fetch handler for testing (keeps backward compat).
pub fn mock_fetch(
    handler: impl Fn(WorkexRequest) -> WorkexResponse + Send + Sync + 'static,
) -> Box<
    dyn Fn(
            WorkexRequest,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<WorkexResponse>> + Send>>
        + Send
        + Sync,
> {
    Box::new(move |req| {
        let resp = handler(req);
        Box::pin(async move { Ok(resp) })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_fetch_works() {
        let handler = mock_fetch(|_req| {
            let mut headers = Headers::new();
            headers.set("content-type", "application/json");
            WorkexResponse::with_init(r#"{"ok":true}"#, 200, headers)
        });

        let req = WorkexRequest::get("https://api.example.com/data");
        let resp = handler(req).await.unwrap();
        assert_eq!(resp.status, 200);
        let body: serde_json::Value = resp.json_body().unwrap();
        assert_eq!(body["ok"], true);
    }

    // Real fetch test — requires network, so only run manually:
    // cargo test -p workex-runtime fetch::tests::real_fetch -- --ignored
    #[tokio::test]
    #[ignore]
    async fn real_fetch() {
        let req = WorkexRequest::get("https://httpbin.org/get");
        let resp = fetch(req).await.unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.ok());
    }
}
