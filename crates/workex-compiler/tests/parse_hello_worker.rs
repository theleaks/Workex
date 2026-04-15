//! Integration test: parse tests/workers/hello.ts from disk and verify extracted types.

use workex_compiler::parse_worker;

#[test]
fn parse_hello_worker_from_file() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/workers/hello.ts"),
    )
    .expect("should read hello.ts");

    let result = parse_worker(&source).expect("should parse hello.ts");

    // Print discovered types for visibility
    println!("=== Workex Compiler: Parse Result ===");
    println!("Functions found: {}", result.functions.len());
    for func in &result.functions {
        println!("  fn {}({}) -> {:?} [async={}]",
            func.name,
            func.params.iter()
                .map(|p| format!("{}: {}", p.name, p.type_annotation.as_deref().unwrap_or("?")))
                .collect::<Vec<_>>()
                .join(", "),
            func.return_type.as_deref().unwrap_or("?"),
            func.is_async,
        );
    }
    println!("Exports found: {}", result.exports.len());
    for export in &result.exports {
        println!("  default={} handlers={:?}", export.is_default, export.handlers);
    }
    println!("Imports found: {}", result.imports.len());
    println!("=====================================");

    // Verify export default { fetch }
    assert_eq!(result.exports.len(), 1);
    assert!(result.exports[0].is_default);
    assert_eq!(result.exports[0].handlers, vec!["fetch"]);

    // Verify fetch handler function
    assert_eq!(result.functions.len(), 1);
    let fetch = &result.functions[0];
    assert_eq!(fetch.name, "fetch");
    assert!(fetch.is_async);

    // Verify parameter: request: Request
    assert_eq!(fetch.params.len(), 1);
    assert_eq!(fetch.params[0].name, "request");
    let param_type = fetch.params[0].type_annotation.as_ref().unwrap();
    assert!(param_type.contains("Request"));

    // Verify return type: Promise<Response>
    let return_type = fetch.return_type.as_ref().unwrap();
    assert!(return_type.contains("Promise"));
    assert!(return_type.contains("Response"));
}
