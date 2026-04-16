//! Hybrid execution: typed functions → Cranelift native, untyped → QuickJS.
//!
//! Analyzes a Worker script and decides execution strategy per function.
//! TypeScript-annotated functions compile to native code (~1ns/call).
//! Dynamic functions fall back to QuickJS (~10μs/call).

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::codegen::{compile_function, NativeFunction};
use crate::hir::Type;
use crate::lower::lower_function;

/// Execution strategy for a function.
pub enum ExecutionStrategy {
    /// All params + return typed → Cranelift native code.
    Native(NativeFunction),
    /// Dynamic or partially typed → QuickJS interprets.
    Interpreted,
}

/// Analysis result for a function.
pub struct FunctionStrategy {
    pub name: String,
    pub strategy: ExecutionStrategy,
    pub param_types: Vec<String>,
    pub return_type: String,
}

/// Analyze a Worker script and produce execution strategies.
pub fn analyze_worker(source: &str) -> Vec<FunctionStrategy> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path("worker.ts").unwrap_or_default();
    let parsed = Parser::new(&allocator, source, source_type).parse();

    if !parsed.errors.is_empty() {
        return Vec::new();
    }

    let mut strategies = Vec::new();

    for stmt in &parsed.program.body {
        if let Statement::FunctionDeclaration(func) = stmt {
            if let Some(id) = &func.id {
                let name = id.name.as_str().to_string();
                let lowered = lower_function(source, &name, func);

                let strategy = if let Some(ref typed_func) = lowered {
                    let all_typed = typed_func.params.iter().all(|p| p.ty != Type::Any)
                        && typed_func.return_type != Type::Any
                        && typed_func.return_type != Type::Void;

                    if all_typed && !typed_func.body.is_empty() {
                        match compile_function(typed_func) {
                            Ok(native) => {
                                let param_types = typed_func
                                    .params
                                    .iter()
                                    .map(|p| format!("{:?}", p.ty))
                                    .collect();
                                let return_type = format!("{:?}", typed_func.return_type);

                                strategies.push(FunctionStrategy {
                                    name: name.clone(),
                                    strategy: ExecutionStrategy::Native(native),
                                    param_types,
                                    return_type,
                                });
                                continue;
                            }
                            Err(_) => ExecutionStrategy::Interpreted,
                        }
                    } else {
                        ExecutionStrategy::Interpreted
                    }
                } else {
                    ExecutionStrategy::Interpreted
                };

                let (param_types, return_type) = if let Some(ref tf) = lowered {
                    (
                        tf.params.iter().map(|p| format!("{:?}", p.ty)).collect(),
                        format!("{:?}", tf.return_type),
                    )
                } else {
                    (Vec::new(), "Any".into())
                };

                strategies.push(FunctionStrategy {
                    name,
                    strategy,
                    param_types,
                    return_type,
                });
            }
        }
    }

    strategies
}

/// Summary of analysis results.
pub fn print_analysis(strategies: &[FunctionStrategy]) {
    for s in strategies {
        let mode = match &s.strategy {
            ExecutionStrategy::Native(_) => "NATIVE (Cranelift)",
            ExecutionStrategy::Interpreted => "INTERPRETED (QuickJS)",
        };
        println!(
            "  {} ({}) -> {} => {}",
            s.name,
            s.param_types.join(", "),
            s.return_type,
            mode,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fully_typed_goes_native() {
        let source = r#"
            function add(a: number, b: number): number {
                return a + b;
            }
        "#;
        let strategies = analyze_worker(source);
        assert_eq!(strategies.len(), 1);
        assert_eq!(strategies[0].name, "add");
        assert!(matches!(strategies[0].strategy, ExecutionStrategy::Native(_)));
    }

    #[test]
    fn untyped_goes_interpreted() {
        let source = r#"
            function greet(name) {
                return "hello " + name;
            }
        "#;
        let strategies = analyze_worker(source);
        assert_eq!(strategies.len(), 1);
        assert_eq!(strategies[0].name, "greet");
        assert!(matches!(strategies[0].strategy, ExecutionStrategy::Interpreted));
    }

    #[test]
    fn mixed_worker() {
        let source = r#"
            function add(a: number, b: number): number {
                return a + b;
            }

            function process(data) {
                return JSON.stringify(data);
            }

            function mul(x: number, y: number): number {
                return x * y;
            }
        "#;
        let strategies = analyze_worker(source);
        assert_eq!(strategies.len(), 3);

        assert!(matches!(strategies[0].strategy, ExecutionStrategy::Native(_)));
        assert_eq!(strategies[0].name, "add");

        assert!(matches!(strategies[1].strategy, ExecutionStrategy::Interpreted));
        assert_eq!(strategies[1].name, "process");

        assert!(matches!(strategies[2].strategy, ExecutionStrategy::Native(_)));
        assert_eq!(strategies[2].name, "mul");
    }

    #[test]
    fn native_function_callable() {
        let source = r#"
            function add(a: number, b: number): number {
                return a + b;
            }
        "#;
        let strategies = analyze_worker(source);
        if let ExecutionStrategy::Native(ref native) = strategies[0].strategy {
            let f: extern "C" fn(f64, f64) -> f64 =
                unsafe { std::mem::transmute(native.ptr()) };
            assert_eq!(f(10.0, 32.0), 42.0);
        } else {
            panic!("expected native");
        }
    }
}
