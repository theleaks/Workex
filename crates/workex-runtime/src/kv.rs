//! KV Namespace — sled-backed persistent storage.
//!
//! Drop-in replacement for Cloudflare Workers KV.
//! Data persists to `.workex/kv/<binding>/` on disk.

use std::path::PathBuf;

/// Persistent KV namespace backed by sled embedded database.
#[derive(Debug, Clone)]
pub struct KvNamespace {
    pub binding_name: String,
    db: sled::Db,
}

impl KvNamespace {
    /// Open or create a KV namespace. Persists to `.workex/kv/<name>/`.
    pub fn new(binding_name: &str) -> anyhow::Result<Self> {
        let path = kv_path(binding_name);
        std::fs::create_dir_all(&path)?;
        let db = sled::open(&path)?;
        Ok(KvNamespace {
            binding_name: binding_name.to_string(),
            db,
        })
    }

    /// Open a temporary in-memory KV (for testing).
    pub fn in_memory(binding_name: &str) -> Self {
        let db = sled::Config::new().temporary(true).open().unwrap();
        KvNamespace {
            binding_name: binding_name.to_string(),
            db,
        }
    }

    /// Get a value by key.
    pub async fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        match self.db.get(key.as_bytes())? {
            Some(v) => Ok(Some(String::from_utf8(v.to_vec())?)),
            None => Ok(None),
        }
    }

    /// Put a key-value pair.
    pub async fn put(&mut self, key: &str, value: &str) -> anyhow::Result<()> {
        self.db.insert(key.as_bytes(), value.as_bytes())?;
        Ok(())
    }

    /// Delete a key.
    pub async fn delete(&mut self, key: &str) -> anyhow::Result<()> {
        self.db.remove(key.as_bytes())?;
        Ok(())
    }

    /// List keys with an optional prefix.
    pub async fn list(&self, prefix: Option<&str>) -> anyhow::Result<Vec<String>> {
        let iter: Box<dyn Iterator<Item = sled::Result<(sled::IVec, sled::IVec)>>> =
            if let Some(p) = prefix {
                Box::new(self.db.scan_prefix(p.as_bytes()))
            } else {
                Box::new(self.db.iter())
            };

        let mut keys = Vec::new();
        for item in iter {
            let (k, _) = item?;
            keys.push(String::from_utf8(k.to_vec())?);
        }
        Ok(keys)
    }
}

fn kv_path(name: &str) -> PathBuf {
    PathBuf::from(".workex").join("kv").join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_and_get() {
        let mut kv = KvNamespace::in_memory("test_kv");
        kv.put("key1", "value1").await.unwrap();
        assert_eq!(kv.get("key1").await.unwrap(), Some("value1".to_string()));
    }

    #[tokio::test]
    async fn get_missing_key() {
        let kv = KvNamespace::in_memory("test_kv");
        assert_eq!(kv.get("nonexistent").await.unwrap(), None);
    }

    #[tokio::test]
    async fn delete_key() {
        let mut kv = KvNamespace::in_memory("test_kv");
        kv.put("key1", "value1").await.unwrap();
        kv.delete("key1").await.unwrap();
        assert_eq!(kv.get("key1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn list_with_prefix() {
        let mut kv = KvNamespace::in_memory("test_kv");
        kv.put("user:1", "alice").await.unwrap();
        kv.put("user:2", "bob").await.unwrap();
        kv.put("config:theme", "dark").await.unwrap();

        let mut users = kv.list(Some("user:")).await.unwrap();
        users.sort();
        assert_eq!(users, vec!["user:1", "user:2"]);

        let all = kv.list(None).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn persistence_across_instances() {
        let path = std::env::temp_dir().join("workex_kv_persist_test");
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();

        // Write
        {
            let db = sled::open(&path).unwrap();
            let mut kv = KvNamespace { binding_name: "test".into(), db };
            kv.put("persist_key", "persist_value").await.unwrap();
        }

        // Read from new instance
        {
            let db = sled::open(&path).unwrap();
            let kv = KvNamespace { binding_name: "test".into(), db };
            assert_eq!(kv.get("persist_key").await.unwrap(), Some("persist_value".to_string()));
        }

        let _ = std::fs::remove_dir_all(&path);
    }
}
