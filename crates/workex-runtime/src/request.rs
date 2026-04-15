//! Workers Request implementation.

use crate::headers::Headers;
use bytes::Bytes;

/// HTTP methods matching the Workers API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Method {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
    OPTIONS,
}

impl Method {
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::GET => "GET",
            Method::POST => "POST",
            Method::PUT => "PUT",
            Method::DELETE => "DELETE",
            Method::PATCH => "PATCH",
            Method::HEAD => "HEAD",
            Method::OPTIONS => "OPTIONS",
        }
    }
}

impl std::fmt::Display for Method {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Incoming request to a Worker, matching the Workers Request API.
#[derive(Debug, Clone)]
pub struct WorkexRequest {
    pub method: Method,
    pub url: String,
    pub headers: Headers,
    pub body: Option<Bytes>,
}

impl WorkexRequest {
    /// Create a simple GET request.
    pub fn get(url: &str) -> Self {
        WorkexRequest {
            method: Method::GET,
            url: url.to_string(),
            headers: Headers::new(),
            body: None,
        }
    }

    /// Create a POST request with a body.
    pub fn post(url: &str, body: impl Into<Bytes>) -> Self {
        WorkexRequest {
            method: Method::POST,
            url: url.to_string(),
            headers: Headers::new(),
            body: Some(body.into()),
        }
    }

    /// Parse the request body as JSON.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> anyhow::Result<T> {
        let body = self
            .body
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("request has no body"))?;
        Ok(serde_json::from_slice(body)?)
    }

    /// Get the request body as text.
    pub fn text(&self) -> anyhow::Result<String> {
        let body = self
            .body
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("request has no body"))?;
        Ok(String::from_utf8(body.to_vec())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_request() {
        let req = WorkexRequest::get("https://example.com/api");
        assert_eq!(req.method, Method::GET);
        assert_eq!(req.url, "https://example.com/api");
        assert!(req.body.is_none());
    }

    #[test]
    fn post_request_with_json_body() {
        let body = r#"{"key": "value"}"#;
        let req = WorkexRequest::post("https://example.com/api", body);
        assert_eq!(req.method, Method::POST);

        let parsed: serde_json::Value = req.json().unwrap();
        assert_eq!(parsed["key"], "value");
    }

    #[test]
    fn text_body() {
        let req = WorkexRequest::post("https://example.com", "hello");
        assert_eq!(req.text().unwrap(), "hello");
    }
}
