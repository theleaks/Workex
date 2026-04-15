//! Lowering pass: oxc AST → HIR.
//!
//! Resolves TypeScript type annotations to concrete types
//! and builds a typed intermediate representation.

use oxc_ast::ast::*;

use crate::hir::{BinOp, Type, TypedExpr, TypedFunction, TypedParam, TypedStmt};

/// Lower an oxc Function AST node to a TypedFunction HIR node.
pub fn lower_function(source: &str, name: &str, func: &Function<'_>) -> Option<TypedFunction> {
    let params = lower_params(source, &func.params);
    let return_type = func
        .return_type
        .as_ref()
        .map(|rt| resolve_type_annotation(source, rt))
        .unwrap_or(Type::Void);

    let body = func
        .body
        .as_ref()
        .map(|b| lower_body(source, &b.statements, &params))
        .unwrap_or_default();

    Some(TypedFunction {
        name: name.to_string(),
        params,
        return_type,
        body,
    })
}

/// Lower formal parameters to typed params.
fn lower_params(source: &str, params: &FormalParameters<'_>) -> Vec<TypedParam> {
    params
        .items
        .iter()
        .enumerate()
        .map(|(i, param)| {
            let name = match &param.pattern {
                BindingPattern::BindingIdentifier(id) => id.name.as_str().to_string(),
                _ => format!("_param{i}"),
            };
            let ty = param
                .type_annotation
                .as_ref()
                .map(|ta| resolve_type_annotation(source, ta))
                .unwrap_or(Type::Any);

            TypedParam {
                name,
                ty,
                index: i,
            }
        })
        .collect()
}

/// Resolve a TSTypeAnnotation to our Type enum using source text.
fn resolve_type_annotation(source: &str, ta: &TSTypeAnnotation<'_>) -> Type {
    let span = ta.span;
    let text = &source[span.start as usize..span.end as usize];
    // Strip leading `: ` from span
    let text = text.strip_prefix(": ").unwrap_or(text).trim();
    Type::from_annotation(text)
}

/// Lower a sequence of statements to typed statements.
fn lower_body(
    source: &str,
    stmts: &oxc_allocator::Vec<'_, Statement<'_>>,
    params: &[TypedParam],
) -> Vec<TypedStmt> {
    stmts.iter().filter_map(|s| lower_stmt(source, s, params)).collect()
}

/// Lower a single statement.
fn lower_stmt(source: &str, stmt: &Statement<'_>, params: &[TypedParam]) -> Option<TypedStmt> {
    match stmt {
        Statement::ReturnStatement(ret) => {
            let expr = ret
                .argument
                .as_ref()
                .map(|e| lower_expr(source, e, params))
                .unwrap_or(TypedExpr::NumberLit(0.0)); // void return
            Some(TypedStmt::Return(expr))
        }
        Statement::ExpressionStatement(es) => {
            Some(TypedStmt::Expr(lower_expr(source, &es.expression, params)))
        }
        _ => None, // Skip unsupported statements
    }
}

/// Lower an expression to a typed expression.
fn lower_expr(source: &str, expr: &Expression<'_>, params: &[TypedParam]) -> TypedExpr {
    match expr {
        Expression::NumericLiteral(lit) => TypedExpr::NumberLit(lit.value),

        Expression::StringLiteral(lit) => {
            TypedExpr::StringLit(lit.value.as_str().to_string())
        }

        Expression::BooleanLiteral(lit) => TypedExpr::BoolLit(lit.value),

        Expression::Identifier(id) => {
            let name = id.name.as_str();
            // Look up in params
            if let Some(param) = params.iter().find(|p| p.name == name) {
                TypedExpr::Ident {
                    name: name.to_string(),
                    ty: param.ty.clone(),
                    param_index: param.index,
                }
            } else {
                TypedExpr::Ident {
                    name: name.to_string(),
                    ty: Type::Any,
                    param_index: 0,
                }
            }
        }

        Expression::BinaryExpression(bin) => {
            let left = lower_expr(source, &bin.left, params);
            let right = lower_expr(source, &bin.right, params);

            let op = match bin.operator {
                BinaryOperator::Addition => BinOp::Add,
                BinaryOperator::Subtraction => BinOp::Sub,
                BinaryOperator::Multiplication => BinOp::Mul,
                BinaryOperator::Division => BinOp::Div,
                BinaryOperator::Remainder => BinOp::Mod,
                BinaryOperator::StrictEquality | BinaryOperator::Equality => BinOp::Eq,
                BinaryOperator::StrictInequality | BinaryOperator::Inequality => BinOp::Ne,
                BinaryOperator::LessThan => BinOp::Lt,
                BinaryOperator::LessEqualThan => BinOp::Le,
                BinaryOperator::GreaterThan => BinOp::Gt,
                BinaryOperator::GreaterEqualThan => BinOp::Ge,
                _ => BinOp::Add, // fallback
            };

            // Resolve result type from operand types
            let ty = resolve_binop_type(left.ty(), right.ty(), op);

            TypedExpr::BinaryOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
                ty,
            }
        }

        Expression::UnaryExpression(unary)
            if unary.operator == UnaryOperator::UnaryNegation =>
        {
            let operand = lower_expr(source, &unary.argument, params);
            let ty = operand.ty().clone();
            TypedExpr::Negate(Box::new(operand), ty)
        }

        Expression::ParenthesizedExpression(paren) => {
            lower_expr(source, &paren.expression, params)
        }

        // Fallback for unsupported expressions
        _ => TypedExpr::NumberLit(0.0),
    }
}

/// Resolve the result type of a binary operation.
fn resolve_binop_type(left: &Type, right: &Type, op: BinOp) -> Type {
    match op {
        // Comparison operators always return boolean
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => Type::Boolean,
        // Arithmetic: if both are numbers, result is number
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
            if *left == Type::Number && *right == Type::Number {
                Type::Number
            } else if *left == Type::String || *right == Type::String {
                Type::String // string concatenation
            } else {
                Type::Any
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    fn parse_and_lower_first_function(source: &str) -> TypedFunction {
        let allocator = Allocator::default();
        let source_type = SourceType::from_path("test.ts").unwrap();
        let ret = Parser::new(&allocator, source, source_type).parse();
        assert!(ret.errors.is_empty(), "parse errors: {:?}", ret.errors);

        for stmt in &ret.program.body {
            if let Statement::FunctionDeclaration(func) = stmt {
                let name = func.id.as_ref().unwrap().name.as_str();
                return lower_function(source, name, func).unwrap();
            }
        }
        panic!("no function found");
    }

    #[test]
    fn lower_add_function() {
        let source = r#"function add(a: number, b: number): number { return a + b; }"#;
        let func = parse_and_lower_first_function(source);

        assert_eq!(func.name, "add");
        assert_eq!(func.params.len(), 2);
        assert_eq!(func.params[0].name, "a");
        assert_eq!(func.params[0].ty, Type::Number);
        assert_eq!(func.params[1].name, "b");
        assert_eq!(func.params[1].ty, Type::Number);
        assert_eq!(func.return_type, Type::Number);
        assert_eq!(func.body.len(), 1);

        match &func.body[0] {
            TypedStmt::Return(TypedExpr::BinaryOp { op, ty, .. }) => {
                assert_eq!(*op, BinOp::Add);
                assert_eq!(*ty, Type::Number);
            }
            other => panic!("expected Return(BinaryOp), got: {other:?}"),
        }
    }

    #[test]
    fn lower_comparison() {
        let source = r#"function gt(a: number, b: number): boolean { return a > b; }"#;
        let func = parse_and_lower_first_function(source);

        assert_eq!(func.return_type, Type::Boolean);
        match &func.body[0] {
            TypedStmt::Return(TypedExpr::BinaryOp { op, ty, .. }) => {
                assert_eq!(*op, BinOp::Gt);
                assert_eq!(*ty, Type::Boolean);
            }
            other => panic!("expected Return(BinaryOp::Gt), got: {other:?}"),
        }
    }

    #[test]
    fn lower_literal_return() {
        let source = r#"function pi(): number { return 3.14159; }"#;
        let func = parse_and_lower_first_function(source);

        assert_eq!(func.params.len(), 0);
        match &func.body[0] {
            TypedStmt::Return(TypedExpr::NumberLit(v)) => {
                assert!((v - 3.14159).abs() < 1e-10);
            }
            other => panic!("expected Return(NumberLit), got: {other:?}"),
        }
    }

    #[test]
    fn lower_complex_expr() {
        let source = r#"function calc(x: number, y: number): number { return (x + y) * x; }"#;
        let func = parse_and_lower_first_function(source);

        // Should be: Mul(Add(x, y), x)
        match &func.body[0] {
            TypedStmt::Return(TypedExpr::BinaryOp {
                op: BinOp::Mul,
                left,
                right,
                ..
            }) => {
                match left.as_ref() {
                    TypedExpr::BinaryOp {
                        op: BinOp::Add, ..
                    } => {}
                    other => panic!("expected Add in left, got: {other:?}"),
                }
                match right.as_ref() {
                    TypedExpr::Ident { name, .. } => assert_eq!(name, "x"),
                    other => panic!("expected Ident(x) in right, got: {other:?}"),
                }
            }
            other => panic!("expected Return(Mul(...)), got: {other:?}"),
        }
    }
}
