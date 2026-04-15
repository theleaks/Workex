//! KV Namespace implementation (in-memory mock for development).

use std::collections::HashMap;

/// In-memory KV namespace matching the Workers KV API.
#[derive(Debug, Clone)]
pub struct KvNamespace {
    pub binding_name: String,
    store: HashMap<String, String>,
}

impl KvNamespace {
    pub fn new(binding_name: &str) -> Self {
        KvNamespace {
            binding_name: binding_name.to_string(),
            store: HashMap::new(),
        }
    }

    /// Get a value by key. Returns None if not found.
    pub async fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(self.store.get(key).cloned())
    }

    /// Put a key-value pair.
    pub async fn put(&mut self, key: &str, value: &str) -> anyhow::Result<()> {
        self.store.insert(key.to_string(), value.to_string());
        Ok(())
    }

    /// Delete a key.
    pub async fn delete(&mut self, key: &str) -> anyhow::Result<()> {
        self.store.remove(key);
        Ok(())
    }

    /// List keys with an optional prefix.
    pub async fn list(
        &self,
        prefix: Option<&str>,
    ) -> anyhow::Result<Vec<String>> {
        let keys: Vec<String> = self
            .store
            .keys()
            .filter(|k| prefix.map_or(true, |p| k.starts_with(p)))
            .cloned()
            .collect();
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_and_get() {
        let mut kv = KvNamespace::new("MY_KV");
        kv.put("key1", "value1").await.unwrap();

        let val = kv.get("key1").await.unwrap();
        assert_eq!(val, Some("value1".to_string()));
    }

    #[tokio::test]
    async fn get_missing_key() {
        let kv = KvNamespace::new("MY_KV");
        let val = kv.get("nonexistent").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn delete_key() {
        let mut kv = KvNamespace::new("MY_KV");
        kv.put("key1", "value1").await.unwrap();
        kv.delete("key1").await.unwrap();

        let val = kv.get("key1").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn list_with_prefix() {
        let mut kv = KvNamespace::new("MY_KV");
        kv.put("user:1", "alice").await.unwrap();
        kv.put("user:2", "bob").await.unwrap();
        kv.put("config:theme", "dark").await.unwrap();

        let mut users = kv.list(Some("user:")).await.unwrap();
        users.sort();
        assert_eq!(users, vec!["user:1", "user:2"]);

        let all = kv.list(None).await.unwrap();
        assert_eq!(all.len(), 3);
    }
}
