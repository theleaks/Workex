//! D1 Database — rusqlite-backed real SQL execution.
//!
//! Drop-in replacement for Cloudflare D1.
//! Data persists to `.workex/d1/<name>.db` on disk.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// A D1 database backed by SQLite via rusqlite.
#[derive(Debug, Clone)]
pub struct D1Database {
    pub binding_name: String,
    conn: Arc<Mutex<rusqlite::Connection>>,
}

/// A prepared SQL statement with bound parameters.
pub struct D1PreparedStatement {
    pub sql: String,
    pub bindings: Vec<D1Value>,
    conn: Arc<Mutex<rusqlite::Connection>>,
}

/// Values that can be bound to a D1 query.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(untagged)]
pub enum D1Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
}

/// Result of a D1 query.
#[derive(Debug, Clone, serde::Serialize)]
pub struct D1Result {
    pub results: Vec<serde_json::Value>,
    pub meta: D1Meta,
}

/// Metadata from a D1 query.
#[derive(Debug, Clone, serde::Serialize)]
pub struct D1Meta {
    pub changes: u64,
    pub rows_read: u64,
    pub rows_written: u64,
}

impl D1Database {
    /// Open or create a D1 database. Persists to `.workex/d1/<name>.db`.
    pub fn new(binding_name: &str) -> anyhow::Result<Self> {
        let path = d1_path(binding_name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = rusqlite::Connection::open(&path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Ok(D1Database {
            binding_name: binding_name.to_string(),
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory database (for testing).
    pub fn in_memory(binding_name: &str) -> Self {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        D1Database {
            binding_name: binding_name.to_string(),
            conn: Arc::new(Mutex::new(conn)),
        }
    }

    /// Prepare a SQL statement.
    pub fn prepare(&self, sql: &str) -> D1PreparedStatement {
        D1PreparedStatement {
            sql: sql.to_string(),
            bindings: Vec::new(),
            conn: self.conn.clone(),
        }
    }

    /// Execute raw SQL (CREATE TABLE, etc).
    pub async fn exec(&self, sql: &str) -> anyhow::Result<D1Result> {
        let conn = self.conn.lock().unwrap();
        let changes = conn.execute_batch(sql).map(|_| 0u64)?;
        Ok(D1Result {
            results: Vec::new(),
            meta: D1Meta { changes, rows_read: 0, rows_written: 0 },
        })
    }
}

impl D1PreparedStatement {
    /// Bind a value to the next parameter.
    pub fn bind(mut self, value: D1Value) -> Self {
        self.bindings.push(value);
        self
    }

    /// Execute and return all matching rows as JSON.
    pub async fn all(&self) -> anyhow::Result<D1Result> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&self.sql)?;

        // Bind parameters
        for (i, val) in self.bindings.iter().enumerate() {
            let idx = i + 1;
            match val {
                D1Value::Null => stmt.raw_bind_parameter(idx, rusqlite::types::Null)?,
                D1Value::Integer(v) => stmt.raw_bind_parameter(idx, v)?,
                D1Value::Real(v) => stmt.raw_bind_parameter(idx, v)?,
                D1Value::Text(v) => stmt.raw_bind_parameter(idx, v.as_str())?,
            }
        }

        let col_count = stmt.column_count();
        let col_names: Vec<String> = (0..col_count)
            .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
            .collect();

        let mut results = Vec::new();
        let mut rows = stmt.raw_query();
        while let Some(row) = rows.next()? {
            let mut obj = serde_json::Map::new();
            for (i, name) in col_names.iter().enumerate() {
                let val = match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                    rusqlite::types::ValueRef::Integer(v) => serde_json::json!(v),
                    rusqlite::types::ValueRef::Real(v) => serde_json::json!(v),
                    rusqlite::types::ValueRef::Text(v) => {
                        serde_json::Value::String(String::from_utf8_lossy(v).to_string())
                    }
                    rusqlite::types::ValueRef::Blob(v) => {
                        serde_json::Value::String(base64_encode(v))
                    }
                };
                obj.insert(name.clone(), val);
            }
            results.push(serde_json::Value::Object(obj));
        }

        Ok(D1Result {
            results,
            meta: D1Meta { changes: 0, rows_read: 0, rows_written: 0 },
        })
    }

    /// Execute and return the first row.
    pub async fn first(&self) -> anyhow::Result<Option<serde_json::Value>> {
        let result = self.all().await?;
        Ok(result.results.into_iter().next())
    }

    /// Execute without returning rows (INSERT/UPDATE/DELETE).
    pub async fn run(&self) -> anyhow::Result<D1Meta> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&self.sql)?;
        for (i, val) in self.bindings.iter().enumerate() {
            let idx = i + 1;
            match val {
                D1Value::Null => stmt.raw_bind_parameter(idx, rusqlite::types::Null)?,
                D1Value::Integer(v) => stmt.raw_bind_parameter(idx, v)?,
                D1Value::Real(v) => stmt.raw_bind_parameter(idx, v)?,
                D1Value::Text(v) => stmt.raw_bind_parameter(idx, v.as_str())?,
            }
        }
        let changes = stmt.raw_execute()? as u64;
        Ok(D1Meta { changes, rows_read: 0, rows_written: changes })
    }
}

fn d1_path(name: &str) -> PathBuf {
    PathBuf::from(".workex").join("d1").join(format!("{name}.db"))
}

fn base64_encode(data: &[u8]) -> String {
    // Simple base64 without external dep
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 { result.push(CHARS[((n >> 6) & 63) as usize] as char); } else { result.push('='); }
        if chunk.len() > 2 { result.push(CHARS[(n & 63) as usize] as char); } else { result.push('='); }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_table_and_insert() {
        let db = D1Database::in_memory("test");
        db.exec("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)")
            .await
            .unwrap();

        db.prepare("INSERT INTO users (name, email) VALUES (?, ?)")
            .bind(D1Value::Text("alice".into()))
            .bind(D1Value::Text("alice@example.com".into()))
            .run()
            .await
            .unwrap();

        let row = db
            .prepare("SELECT * FROM users WHERE name = ?")
            .bind(D1Value::Text("alice".into()))
            .first()
            .await
            .unwrap();

        let row = row.expect("should find alice");
        assert_eq!(row["name"], "alice");
        assert_eq!(row["email"], "alice@example.com");
    }

    #[tokio::test]
    async fn query_multiple_rows() {
        let db = D1Database::in_memory("test");
        db.exec("CREATE TABLE items (id INTEGER PRIMARY KEY, val TEXT)")
            .await
            .unwrap();

        for i in 0..5 {
            db.prepare("INSERT INTO items (val) VALUES (?)")
                .bind(D1Value::Text(format!("item-{i}")))
                .run()
                .await
                .unwrap();
        }

        let result = db.prepare("SELECT * FROM items").all().await.unwrap();
        assert_eq!(result.results.len(), 5);
        assert_eq!(result.results[0]["val"], "item-0");
        assert_eq!(result.results[4]["val"], "item-4");
    }

    #[tokio::test]
    async fn integer_and_real_types() {
        let db = D1Database::in_memory("test");
        db.exec("CREATE TABLE data (i INTEGER, r REAL)").await.unwrap();

        db.prepare("INSERT INTO data VALUES (?, ?)")
            .bind(D1Value::Integer(42))
            .bind(D1Value::Real(3.14))
            .run()
            .await
            .unwrap();

        let row = db.prepare("SELECT * FROM data").first().await.unwrap().unwrap();
        assert_eq!(row["i"], 42);
        assert!((row["r"].as_f64().unwrap() - 3.14).abs() < 0.001);
    }

    #[tokio::test]
    async fn delete_and_update() {
        let db = D1Database::in_memory("test");
        db.exec("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)").await.unwrap();
        db.prepare("INSERT INTO t VALUES (1, 'a')").run().await.unwrap();
        db.prepare("INSERT INTO t VALUES (2, 'b')").run().await.unwrap();

        let meta = db.prepare("DELETE FROM t WHERE id = ?")
            .bind(D1Value::Integer(1))
            .run().await.unwrap();
        assert_eq!(meta.changes, 1);

        let result = db.prepare("SELECT * FROM t").all().await.unwrap();
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0]["v"], "b");
    }

    #[tokio::test]
    async fn first_returns_none_on_empty() {
        let db = D1Database::in_memory("test");
        db.exec("CREATE TABLE empty (id INTEGER)").await.unwrap();
        let row = db.prepare("SELECT * FROM empty").first().await.unwrap();
        assert!(row.is_none());
    }
}
