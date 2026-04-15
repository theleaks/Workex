//! Outbound fetch() implementation.
//!
//! In production, this will make real HTTP requests.
//! For now, provides a pluggable mock for testing.

use crate::request::WorkexRequest;
use crate::response::WorkexResponse;

/// Outbound fetch handler — can be swapped for testing.
pub type FetchHandler =
    Box<dyn Fn(WorkexRequest) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<WorkexResponse>> + Send>> + Send + Sync>;

/// Default fetch that returns a 501 Not Implemented (placeholder).
pub fn default_fetch() -> FetchHandler {
    Box::new(|req| {
        Box::pin(async move {
            anyhow::bail!(
                "outbound fetch not yet implemented: {} {}",
                req.method,
                req.url
            )
        })
    })
}

/// Create a mock fetch handler for testing.
pub fn mock_fetch(
    handler: impl Fn(WorkexRequest) -> WorkexResponse + Send + Sync + 'static,
) -> FetchHandler {
    Box::new(move |req| {
        let resp = handler(req);
        Box::pin(async move { Ok(resp) })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headers::Headers;

    #[tokio::test]
    async fn default_fetch_returns_error() {
        let fetch = default_fetch();
        let req = WorkexRequest::get("https://api.example.com/data");
        let result = fetch(req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mock_fetch_works() {
        let fetch = mock_fetch(|_req| {
            let mut headers = Headers::new();
            headers.set("content-type", "application/json");
            WorkexResponse::with_init(r#"{"ok":true}"#, 200, headers)
        });

        let req = WorkexRequest::get("https://api.example.com/data");
        let resp = fetch(req).await.unwrap();
        assert_eq!(resp.status, 200);

        let body: serde_json::Value = resp.json_body().unwrap();
        assert_eq!(body["ok"], true);
    }
}
