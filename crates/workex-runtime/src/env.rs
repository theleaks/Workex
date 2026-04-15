//! Worker environment bindings (KV, D1, secrets).

use std::collections::HashMap;

use crate::d1::D1Database;
use crate::kv::KvNamespace;

/// Environment bindings available to a Worker.
/// Mirrors Cloudflare's `env` parameter in fetch handlers.
#[derive(Debug, Clone)]
pub struct Env {
    pub kv: HashMap<String, KvNamespace>,
    pub d1: HashMap<String, D1Database>,
    pub secrets: HashMap<String, String>,
}

impl Env {
    pub fn new() -> Self {
        Env {
            kv: HashMap::new(),
            d1: HashMap::new(),
            secrets: HashMap::new(),
        }
    }

    /// Add a KV namespace binding.
    pub fn add_kv(&mut self, name: &str) -> &mut KvNamespace {
        self.kv
            .entry(name.to_string())
            .or_insert_with(|| KvNamespace::new(name))
    }

    /// Add a D1 database binding.
    pub fn add_d1(&mut self, name: &str) -> &mut D1Database {
        self.d1
            .entry(name.to_string())
            .or_insert_with(|| D1Database::new(name))
    }

    /// Add a secret binding.
    pub fn add_secret(&mut self, name: &str, value: &str) {
        self.secrets.insert(name.to_string(), value.to_string());
    }

    /// Get a KV namespace by binding name.
    pub fn kv(&self, name: &str) -> Option<&KvNamespace> {
        self.kv.get(name)
    }

    /// Get a mutable KV namespace by binding name.
    pub fn kv_mut(&mut self, name: &str) -> Option<&mut KvNamespace> {
        self.kv.get_mut(name)
    }

    /// Get a D1 database by binding name.
    pub fn d1(&self, name: &str) -> Option<&D1Database> {
        self.d1.get(name)
    }

    /// Get a secret value.
    pub fn secret(&self, name: &str) -> Option<&str> {
        self.secrets.get(name).map(|s| s.as_str())
    }
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_env_with_bindings() {
        let mut env = Env::new();
        env.add_kv("MY_KV");
        env.add_d1("DB");
        env.add_secret("API_KEY", "secret123");

        assert!(env.kv("MY_KV").is_some());
        assert!(env.d1("DB").is_some());
        assert_eq!(env.secret("API_KEY"), Some("secret123"));
    }

    #[tokio::test]
    async fn kv_through_env() {
        let mut env = Env::new();
        env.add_kv("MY_KV");

        let kv = env.kv_mut("MY_KV").unwrap();
        kv.put("key", "value").await.unwrap();

        let val = env.kv("MY_KV").unwrap().get("key").await.unwrap();
        assert_eq!(val, Some("value".to_string()));
    }

    #[test]
    fn d1_through_env() {
        let mut env = Env::new();
        env.add_d1("DB");

        let db = env.d1("DB").unwrap();
        let stmt = db.prepare("SELECT 1");
        assert_eq!(stmt.sql, "SELECT 1");
    }
}
