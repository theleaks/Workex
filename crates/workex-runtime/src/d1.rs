//! D1 Database implementation (mock for development).

/// A mock D1 database.
#[derive(Debug, Clone)]
pub struct D1Database {
    pub binding_name: String,
    // In a real implementation, this would connect to D1's SQLite backend.
    // For now, this is a structural placeholder that validates the API surface.
}

/// A prepared SQL statement with bound parameters.
#[derive(Debug)]
pub struct D1PreparedStatement {
    pub sql: String,
    pub bindings: Vec<D1Value>,
}

/// Values that can be bound to a D1 query.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(untagged)]
pub enum D1Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// Result of a D1 query execution.
#[derive(Debug, Clone, serde::Serialize)]
pub struct D1Result {
    pub results: Vec<serde_json::Value>,
    pub meta: D1Meta,
}

/// Metadata from a D1 query.
#[derive(Debug, Clone, serde::Serialize)]
pub struct D1Meta {
    pub changes: u64,
    pub duration: f64,
    pub rows_read: u64,
    pub rows_written: u64,
}

impl D1Database {
    pub fn new(binding_name: &str) -> Self {
        D1Database {
            binding_name: binding_name.to_string(),
        }
    }

    /// Prepare a SQL statement for execution.
    pub fn prepare(&self, sql: &str) -> D1PreparedStatement {
        D1PreparedStatement {
            sql: sql.to_string(),
            bindings: Vec::new(),
        }
    }

    /// Execute raw SQL directly (no parameter binding).
    pub async fn exec(&self, _sql: &str) -> anyhow::Result<D1Result> {
        // Mock: return empty result
        Ok(D1Result {
            results: Vec::new(),
            meta: D1Meta {
                changes: 0,
                duration: 0.0,
                rows_read: 0,
                rows_written: 0,
            },
        })
    }
}

impl D1PreparedStatement {
    /// Bind a value to the next parameter placeholder.
    pub fn bind(mut self, value: D1Value) -> Self {
        self.bindings.push(value);
        self
    }

    /// Execute the statement and return all results.
    pub async fn all(&self) -> anyhow::Result<D1Result> {
        // Mock: return empty result
        Ok(D1Result {
            results: Vec::new(),
            meta: D1Meta {
                changes: 0,
                duration: 0.0,
                rows_read: 0,
                rows_written: 0,
            },
        })
    }

    /// Execute and return only the first row.
    pub async fn first(&self) -> anyhow::Result<Option<serde_json::Value>> {
        let result = self.all().await?;
        Ok(result.results.into_iter().next())
    }

    /// Execute the statement without returning results (for INSERT/UPDATE/DELETE).
    pub async fn run(&self) -> anyhow::Result<D1Meta> {
        let result = self.all().await?;
        Ok(result.meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn prepare_and_bind() {
        let db = D1Database::new("DB");
        let stmt = db
            .prepare("SELECT * FROM users WHERE id = ? AND name = ?")
            .bind(D1Value::Integer(1))
            .bind(D1Value::Text("alice".to_string()));

        assert_eq!(stmt.bindings.len(), 2);
        assert_eq!(stmt.sql, "SELECT * FROM users WHERE id = ? AND name = ?");
    }

    #[tokio::test]
    async fn exec_returns_result() {
        let db = D1Database::new("DB");
        let result = db.exec("CREATE TABLE test (id INTEGER)").await.unwrap();
        assert_eq!(result.results.len(), 0);
    }

    #[tokio::test]
    async fn first_returns_none_on_empty() {
        let db = D1Database::new("DB");
        let row = db.prepare("SELECT * FROM empty").first().await.unwrap();
        assert!(row.is_none());
    }
}
