//! workex-compiler: TypeScript Workers script parser and AOT compiler.
//!
//! Pipeline: TypeScript source → oxc parse → HIR (typed) → Cranelift → native code.

pub mod bytecode;
pub mod codegen;
pub mod cps;
pub mod hir;
pub mod hybrid;
pub mod lower;

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_parser::Parser;
use oxc_span::SourceType;

/// A compiled module ready for execution by an Isolate.
/// Currently holds the parse result; will hold Cranelift native code in Phase 5.
#[derive(Debug, Clone)]
pub struct CompiledModule {
    pub parse_result: ParseResult,
    pub source_hash: u64,
}

impl CompiledModule {
    /// Compile a TypeScript Workers script into a CompiledModule.
    pub fn compile(source: &str) -> anyhow::Result<Self> {
        let parse_result = parse_worker(source)?;
        let source_hash = Self::hash_source(source);
        Ok(CompiledModule {
            parse_result,
            source_hash,
        })
    }

    fn hash_source(source: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut hasher);
        hasher.finish()
    }
}

/// Result of parsing a Workers TypeScript script.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ParseResult {
    pub functions: Vec<FunctionInfo>,
    pub exports: Vec<ExportInfo>,
    pub imports: Vec<ImportInfo>,
}

/// Information about a discovered function.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FunctionInfo {
    pub name: String,
    pub params: Vec<ParamInfo>,
    pub return_type: Option<String>,
    pub is_async: bool,
}

/// Information about a function parameter with its type annotation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ParamInfo {
    pub name: String,
    pub type_annotation: Option<String>,
}

/// Information about an export declaration.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExportInfo {
    pub is_default: bool,
    pub handlers: Vec<String>,
}

/// Information about an import statement.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportInfo {
    pub source: String,
    pub specifiers: Vec<String>,
}

/// Parse a TypeScript Workers script and extract type information.
pub fn parse_worker(source: &str) -> anyhow::Result<ParseResult> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path("worker.ts")
        .map_err(|e| anyhow::anyhow!("invalid source type: {e}"))?;
    let parser_return = Parser::new(&allocator, source, source_type).parse();

    if !parser_return.errors.is_empty() {
        let msgs: Vec<String> = parser_return.errors.iter().map(|e| e.to_string()).collect();
        anyhow::bail!("parse errors: {}", msgs.join("; "));
    }

    let program = &parser_return.program;
    let mut result = ParseResult {
        functions: Vec::new(),
        exports: Vec::new(),
        imports: Vec::new(),
    };

    for stmt in &program.body {
        match stmt {
            Statement::FunctionDeclaration(func) => {
                if let Some(info) = extract_function_info(source, func) {
                    result.functions.push(info);
                }
            }
            Statement::ExportDefaultDeclaration(export_default) => {
                let export_info = extract_export_default(source, export_default, &mut result);
                result.exports.push(export_info);
            }
            Statement::ImportDeclaration(import) => {
                result.imports.push(extract_import(import));
            }
            _ => {}
        }
    }

    Ok(result)
}

/// Extract function info from a Function AST node.
fn extract_function_info(source: &str, func: &Function<'_>) -> Option<FunctionInfo> {
    let name = func.id.as_ref()?.name.as_str().to_string();
    let params = extract_params(source, &func.params);
    let return_type = func.return_type.as_ref().map(|rt| span_text(source, rt.span));
    let is_async = func.r#async;

    Some(FunctionInfo {
        name,
        params,
        return_type,
        is_async,
    })
}

/// Extract function info from a Function that may not have an id (e.g., method).
fn extract_method_info(source: &str, name: &str, func: &Function<'_>) -> FunctionInfo {
    let params = extract_params(source, &func.params);
    let return_type = func.return_type.as_ref().map(|rt| span_text(source, rt.span));

    FunctionInfo {
        name: name.to_string(),
        params,
        return_type,
        is_async: func.r#async,
    }
}

/// Extract parameter info from formal parameters.
fn extract_params(source: &str, params: &FormalParameters<'_>) -> Vec<ParamInfo> {
    params
        .items
        .iter()
        .map(|param| {
            let name = match &param.pattern {
                BindingPattern::BindingIdentifier(id) => id.name.as_str().to_string(),
                _ => "<pattern>".to_string(),
            };
            let type_annotation = param
                .type_annotation
                .as_ref()
                .map(|ta| span_text(source, ta.span));

            ParamInfo {
                name,
                type_annotation,
            }
        })
        .collect()
}

/// Extract export default declaration info.
/// Methods found in the default export object are also added to `result.functions`.
fn extract_export_default(
    source: &str,
    export: &ExportDefaultDeclaration<'_>,
    result: &mut ParseResult,
) -> ExportInfo {
    let mut handlers = Vec::new();

    match &export.declaration {
        ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
            if let Some(info) = extract_function_info(source, func) {
                handlers.push(info.name.clone());
                result.functions.push(info);
            }
        }
        ExportDefaultDeclarationKind::ObjectExpression(obj) => {
            for prop in &obj.properties {
                if let ObjectPropertyKind::ObjectProperty(prop) = prop {
                    if let Some(key_name) = property_key_name(&prop.key) {
                        handlers.push(key_name.clone());
                        // If the value is a function expression, extract its info
                        if let Expression::FunctionExpression(func) = &prop.value {
                            result
                                .functions
                                .push(extract_method_info(source, &key_name, func));
                        }
                    }
                }
            }
        }
        _ => {}
    }

    ExportInfo {
        is_default: true,
        handlers,
    }
}

/// Extract the name from a property key.
fn property_key_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.as_str().to_string()),
        _ => None,
    }
}

/// Extract import declaration info.
fn extract_import(import: &ImportDeclaration<'_>) -> ImportInfo {
    let source = import.source.value.as_str().to_string();
    let specifiers = import
        .specifiers
        .as_ref()
        .map(|specs| {
            specs
                .iter()
                .map(|s| match s {
                    ImportDeclarationSpecifier::ImportSpecifier(s) => {
                        s.local.name.as_str().to_string()
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                        s.local.name.as_str().to_string()
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                        format!("* as {}", s.local.name.as_str())
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    ImportInfo { source, specifiers }
}

/// Get source text for a span, trimming the leading `: ` from type annotations.
fn span_text(source: &str, span: oxc_span::Span) -> String {
    let text = &source[span.start as usize..span.end as usize];
    text.strip_prefix(": ").unwrap_or(text).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hello_worker() {
        let source = r#"
export default {
  async fetch(request: Request): Promise<Response> {
    return new Response("Hello from Workex!", {
      headers: { "content-type": "text/plain" },
    });
  },
};
"#;
        let result = parse_worker(source).expect("should parse");

        // Should find the fetch handler in exports
        assert_eq!(result.exports.len(), 1);
        assert!(result.exports[0].is_default);
        assert_eq!(result.exports[0].handlers, vec!["fetch"]);

        // Should extract the fetch function info
        assert_eq!(result.functions.len(), 1);
        let fetch_fn = &result.functions[0];
        assert_eq!(fetch_fn.name, "fetch");
        assert!(fetch_fn.is_async);

        // Should extract parameter type annotation
        assert_eq!(fetch_fn.params.len(), 1);
        assert_eq!(fetch_fn.params[0].name, "request");
        assert!(fetch_fn.params[0].type_annotation.is_some());
        let type_ann = fetch_fn.params[0].type_annotation.as_ref().unwrap();
        assert!(type_ann.contains("Request"), "type annotation should contain 'Request', got: {type_ann}");

        // Should extract return type
        assert!(fetch_fn.return_type.is_some());
        let ret_type = fetch_fn.return_type.as_ref().unwrap();
        assert!(ret_type.contains("Promise"), "return type should contain 'Promise', got: {ret_type}");
    }

    #[test]
    fn test_parse_with_imports() {
        let source = r#"
import { Router } from 'itty-router';
import KVStore from './kv';

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    return new Response("ok");
  },
};
"#;
        let result = parse_worker(source).expect("should parse");

        assert_eq!(result.imports.len(), 2);
        assert_eq!(result.imports[0].source, "itty-router");
        assert_eq!(result.imports[0].specifiers, vec!["Router"]);
        assert_eq!(result.imports[1].source, "./kv");
        assert_eq!(result.imports[1].specifiers, vec!["KVStore"]);
    }

    #[test]
    fn test_parse_typed_functions() {
        let source = r#"
function add(a: number, b: number): number {
  return a + b;
}

function greet(name: string): string {
  return `Hello, ${name}!`;
}

export default {
  async fetch(request: Request): Promise<Response> {
    return new Response("ok");
  },
};
"#;
        let result = parse_worker(source).expect("should parse");

        // Two top-level functions + one method from export default
        assert_eq!(result.functions.len(), 3);

        let add_fn = &result.functions[0];
        assert_eq!(add_fn.name, "add");
        assert!(!add_fn.is_async);
        assert_eq!(add_fn.params.len(), 2);
        assert_eq!(add_fn.params[0].name, "a");
        assert!(add_fn.params[0]
            .type_annotation
            .as_ref()
            .unwrap()
            .contains("number"));
        assert_eq!(add_fn.params[1].name, "b");
        assert!(add_fn.return_type.as_ref().unwrap().contains("number"));
    }
}
