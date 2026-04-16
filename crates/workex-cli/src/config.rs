//! wrangler.toml config parser — compatible with Cloudflare's wrangler CLI.

use std::collections::HashMap;
use std::path::Path;

/// Parsed wrangler.toml configuration.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct WranglerConfig {
    pub name: String,
    pub main: String,
    #[serde(default)]
    pub compatibility_date: Option<String>,

    #[serde(default)]
    pub kv_namespaces: Vec<KvBinding>,

    #[serde(default)]
    pub d1_databases: Vec<D1Binding>,

    #[serde(default)]
    pub vars: HashMap<String, String>,
}

/// A KV namespace binding from wrangler.toml.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct KvBinding {
    pub binding: String,
    pub id: String,
}

/// A D1 database binding from wrangler.toml.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct D1Binding {
    pub binding: String,
    pub database_name: String,
    pub database_id: String,
}

/// Load wrangler.toml from a directory.
pub fn load_config(dir: &Path) -> anyhow::Result<WranglerConfig> {
    let path = dir.join("wrangler.toml");
    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
    let config: WranglerConfig = toml::from_str(&content)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_config() {
        let toml = r#"
name = "my-worker"
main = "src/index.ts"
compatibility_date = "2026-01-01"
"#;
        let config: WranglerConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.name, "my-worker");
        assert_eq!(config.main, "src/index.ts");
        assert_eq!(config.compatibility_date.as_deref(), Some("2026-01-01"));
    }

    #[test]
    fn parse_kv_bindings() {
        let toml = r#"
name = "worker"
main = "src/index.ts"

[[kv_namespaces]]
binding = "MY_KV"
id = "abc123"

[[kv_namespaces]]
binding = "CACHE"
id = "def456"
"#;
        let config: WranglerConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.kv_namespaces.len(), 2);
        assert_eq!(config.kv_namespaces[0].binding, "MY_KV");
        assert_eq!(config.kv_namespaces[1].binding, "CACHE");
    }

    #[test]
    fn parse_d1_bindings() {
        let toml = r#"
name = "worker"
main = "src/index.ts"

[[d1_databases]]
binding = "DB"
database_name = "my-db"
database_id = "xxx-yyy"
"#;
        let config: WranglerConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.d1_databases.len(), 1);
        assert_eq!(config.d1_databases[0].binding, "DB");
        assert_eq!(config.d1_databases[0].database_name, "my-db");
    }

    #[test]
    fn parse_vars() {
        let toml = r#"
name = "worker"
main = "src/index.ts"

[vars]
API_KEY = "secret123"
ENV = "production"
"#;
        let config: WranglerConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.vars.get("API_KEY").unwrap(), "secret123");
        assert_eq!(config.vars.get("ENV").unwrap(), "production");
    }

    #[test]
    fn parse_full_config() {
        let toml = r#"
name = "my-api"
main = "src/index.ts"
compatibility_date = "2026-01-01"

[vars]
ENVIRONMENT = "staging"

[[kv_namespaces]]
binding = "SESSIONS"
id = "sess-123"

[[d1_databases]]
binding = "DB"
database_name = "app-db"
database_id = "db-456"
"#;
        let config: WranglerConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.name, "my-api");
        assert_eq!(config.kv_namespaces.len(), 1);
        assert_eq!(config.d1_databases.len(), 1);
        assert_eq!(config.vars.get("ENVIRONMENT").unwrap(), "staging");
    }
}
