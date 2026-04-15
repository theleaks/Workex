//! Workers Response implementation.

use crate::headers::Headers;
use bytes::Bytes;

/// Response from a Worker, matching the Workers Response API.
#[derive(Debug, Clone)]
pub struct WorkexResponse {
    pub status: u16,
    pub headers: Headers,
    pub body: Bytes,
}

impl WorkexResponse {
    /// Create a response with a text body and 200 status.
    pub fn new(body: impl Into<Bytes>) -> Self {
        let mut headers = Headers::new();
        headers.set("content-type", "text/plain;charset=UTF-8");
        WorkexResponse {
            status: 200,
            headers,
            body: body.into(),
        }
    }

    /// Create a response with custom status and headers init.
    pub fn with_init(body: impl Into<Bytes>, status: u16, headers: Headers) -> Self {
        WorkexResponse {
            status,
            headers,
            body: body.into(),
        }
    }

    /// Create a JSON response.
    pub fn json(value: &impl serde::Serialize) -> anyhow::Result<Self> {
        let body = serde_json::to_vec(value)?;
        let mut headers = Headers::new();
        headers.set("content-type", "application/json");
        Ok(WorkexResponse {
            status: 200,
            headers,
            body: body.into(),
        })
    }

    /// Create a redirect response.
    pub fn redirect(url: &str, status: u16) -> Self {
        let mut headers = Headers::new();
        headers.set("location", url);
        WorkexResponse {
            status,
            headers,
            body: Bytes::new(),
        }
    }

    /// Get the response body as text.
    pub fn text(&self) -> anyhow::Result<String> {
        Ok(String::from_utf8(self.body.to_vec())?)
    }

    /// Parse the response body as JSON.
    pub fn json_body<T: serde::de::DeserializeOwned>(&self) -> anyhow::Result<T> {
        Ok(serde_json::from_slice(&self.body)?)
    }

    /// Whether this is a successful response (2xx).
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_response() {
        let resp = WorkexResponse::new("Hello from Workex!");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.text().unwrap(), "Hello from Workex!");
        assert_eq!(
            resp.headers.get("content-type"),
            Some("text/plain;charset=UTF-8")
        );
        assert!(resp.ok());
    }

    #[test]
    fn json_response() {
        let data = serde_json::json!({"message": "ok", "count": 42});
        let resp = WorkexResponse::json(&data).unwrap();
        assert_eq!(resp.headers.get("content-type"), Some("application/json"));

        let parsed: serde_json::Value = resp.json_body().unwrap();
        assert_eq!(parsed["count"], 42);
    }

    #[test]
    fn redirect_response() {
        let resp = WorkexResponse::redirect("https://example.com", 302);
        assert_eq!(resp.status, 302);
        assert_eq!(resp.headers.get("location"), Some("https://example.com"));
        assert!(!resp.ok());
    }

    #[test]
    fn with_init() {
        let mut headers = Headers::new();
        headers.set("x-custom", "value");
        let resp = WorkexResponse::with_init("body", 201, headers);
        assert_eq!(resp.status, 201);
        assert_eq!(resp.headers.get("x-custom"), Some("value"));
        assert!(resp.ok());
    }
}
