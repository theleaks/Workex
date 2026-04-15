//! End-to-end test: TypeScript source → parse → HIR → Cranelift → native execution.
//!
//! This proves the full AOT pipeline works:
//! 1. oxc parses TypeScript with type annotations
//! 2. Lowering resolves types to concrete HIR
//! 3. Cranelift compiles to native machine code
//! 4. We call the native function and verify the result

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_parser::Parser;
use oxc_span::SourceType;
use workex_compiler::codegen::compile_function;
use workex_compiler::lower::lower_function;

/// Helper: parse TypeScript, lower first function to HIR, compile to native.
fn compile_ts_function(source: &str) -> workex_compiler::codegen::NativeFunction {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path("test.ts").unwrap();
    let ret = Parser::new(&allocator, source, source_type).parse();
    assert!(ret.errors.is_empty(), "parse errors: {:?}", ret.errors);

    for stmt in &ret.program.body {
        if let Statement::FunctionDeclaration(func) = stmt {
            let name = func.id.as_ref().unwrap().name.as_str();
            let typed_func = lower_function(source, name, func).unwrap();
            return compile_function(&typed_func).expect("compilation failed");
        }
    }
    panic!("no function found in source");
}

#[test]
fn e2e_add() {
    let native = compile_ts_function(
        "function add(a: number, b: number): number { return a + b; }",
    );
    let add: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(native.ptr()) };

    assert_eq!(add(3.0, 4.0), 7.0);
    assert_eq!(add(100.0, 200.0), 300.0);
    assert_eq!(add(-5.0, 5.0), 0.0);
}

#[test]
fn e2e_subtract() {
    let native = compile_ts_function(
        "function sub(x: number, y: number): number { return x - y; }",
    );
    let sub: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(native.ptr()) };

    assert_eq!(sub(10.0, 3.0), 7.0);
    assert_eq!(sub(0.0, 1.0), -1.0);
}

#[test]
fn e2e_multiply_divide() {
    let native_mul = compile_ts_function(
        "function mul(a: number, b: number): number { return a * b; }",
    );
    let mul: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(native_mul.ptr()) };
    assert_eq!(mul(6.0, 7.0), 42.0);

    let native_div = compile_ts_function(
        "function div(a: number, b: number): number { return a / b; }",
    );
    let div: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(native_div.ptr()) };
    assert_eq!(div(42.0, 6.0), 7.0);
    assert_eq!(div(1.0, 3.0), 1.0 / 3.0);
}

#[test]
fn e2e_complex_expression() {
    // Quadratic-like: (a + b) * (a - b)  = a² - b²
    let native = compile_ts_function(
        "function diff_squares(a: number, b: number): number { return (a + b) * (a - b); }",
    );
    let f: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(native.ptr()) };

    assert_eq!(f(5.0, 3.0), 16.0); // 25 - 9
    assert_eq!(f(10.0, 10.0), 0.0); // 100 - 100
    assert_eq!(f(7.0, 0.0), 49.0); // 49 - 0
}

#[test]
fn e2e_constant_function() {
    let native = compile_ts_function(
        "function answer(): number { return 42.0; }",
    );
    let f: extern "C" fn() -> f64 = unsafe { std::mem::transmute(native.ptr()) };
    assert_eq!(f(), 42.0);
}

#[test]
fn e2e_single_param() {
    let native = compile_ts_function(
        "function double(x: number): number { return x + x; }",
    );
    let f: extern "C" fn(f64) -> f64 = unsafe { std::mem::transmute(native.ptr()) };
    assert_eq!(f(21.0), 42.0);
    assert_eq!(f(0.0), 0.0);
    assert_eq!(f(-3.0), -6.0);
}
