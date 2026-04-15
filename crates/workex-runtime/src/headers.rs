//! HTTP Headers implementation matching the Workers Headers API.

use std::collections::HashMap;

/// Case-insensitive HTTP headers.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Headers {
    inner: HashMap<String, Vec<String>>,
}

impl Headers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a header (replaces any existing values for this key).
    pub fn set(&mut self, key: &str, value: &str) {
        self.inner
            .insert(key.to_lowercase(), vec![value.to_string()]);
    }

    /// Append a value to a header (adds to existing values).
    pub fn append(&mut self, key: &str, value: &str) {
        self.inner
            .entry(key.to_lowercase())
            .or_default()
            .push(value.to_string());
    }

    /// Get the first value for a header.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.inner
            .get(&key.to_lowercase())
            .and_then(|v| v.first())
            .map(|s| s.as_str())
    }

    /// Check if a header exists.
    pub fn has(&self, key: &str) -> bool {
        self.inner.contains_key(&key.to_lowercase())
    }

    /// Delete a header.
    pub fn delete(&mut self, key: &str) {
        self.inner.remove(&key.to_lowercase());
    }

    /// Iterate over all header entries.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.inner
            .iter()
            .flat_map(|(k, vs)| vs.iter().map(move |v| (k.as_str(), v.as_str())))
    }
}

impl From<Vec<(&str, &str)>> for Headers {
    fn from(pairs: Vec<(&str, &str)>) -> Self {
        let mut h = Headers::new();
        for (k, v) in pairs {
            h.append(k, v);
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get() {
        let mut h = Headers::new();
        h.set("Content-Type", "text/plain");
        assert_eq!(h.get("content-type"), Some("text/plain"));
        assert_eq!(h.get("Content-Type"), Some("text/plain"));
    }

    #[test]
    fn append_multiple() {
        let mut h = Headers::new();
        h.append("Set-Cookie", "a=1");
        h.append("Set-Cookie", "b=2");
        let cookies: Vec<_> = h
            .entries()
            .filter(|(k, _)| *k == "set-cookie")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(cookies, vec!["a=1", "b=2"]);
    }

    #[test]
    fn has_and_delete() {
        let mut h = Headers::new();
        h.set("X-Custom", "value");
        assert!(h.has("x-custom"));
        h.delete("X-Custom");
        assert!(!h.has("x-custom"));
    }

    #[test]
    fn from_pairs() {
        let h = Headers::from(vec![
            ("content-type", "application/json"),
            ("authorization", "Bearer token"),
        ]);
        assert_eq!(h.get("content-type"), Some("application/json"));
        assert_eq!(h.get("authorization"), Some("Bearer token"));
    }
}
