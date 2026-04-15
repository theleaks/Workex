//! High-level Intermediate Representation (HIR).
//!
//! Typed IR where all TypeScript type annotations have been resolved.
//! No deoptimization paths — types are known at compile time.

/// Resolved type from TypeScript annotations.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// JavaScript `number` → f64
    Number,
    /// JavaScript `string` → heap-allocated string
    String,
    /// JavaScript `boolean` → i8 (0 or 1)
    Boolean,
    /// No return value
    Void,
    /// Type could not be resolved — needs dynamic dispatch
    Any,
}

impl Type {
    /// Parse a type annotation string into a resolved Type.
    pub fn from_annotation(s: &str) -> Self {
        let s = s.trim();
        match s {
            "number" => Type::Number,
            "string" => Type::String,
            "boolean" | "bool" => Type::Boolean,
            "void" => Type::Void,
            _ => Type::Any,
        }
    }
}

/// A typed function ready for codegen.
#[derive(Debug, Clone)]
pub struct TypedFunction {
    pub name: String,
    pub params: Vec<TypedParam>,
    pub return_type: Type,
    pub body: Vec<TypedStmt>,
}

/// A function parameter with a resolved type.
#[derive(Debug, Clone)]
pub struct TypedParam {
    pub name: String,
    pub ty: Type,
    /// Parameter index (position in the function signature).
    pub index: usize,
}

/// A typed statement.
#[derive(Debug, Clone)]
pub enum TypedStmt {
    /// `return expr;`
    Return(TypedExpr),
    /// Expression used as a statement.
    Expr(TypedExpr),
}

/// A typed expression — the core of the HIR.
#[derive(Debug, Clone)]
pub enum TypedExpr {
    /// Numeric literal: `42`, `3.14`
    NumberLit(f64),
    /// String literal: `"hello"`
    StringLit(String),
    /// Boolean literal: `true`, `false`
    BoolLit(bool),
    /// Variable reference (parameter or local).
    Ident {
        name: String,
        ty: Type,
        /// Index into the parameter list (for codegen).
        param_index: usize,
    },
    /// Binary operation: `a + b`, `x * y`
    BinaryOp {
        op: BinOp,
        left: Box<TypedExpr>,
        right: Box<TypedExpr>,
        /// Resolved result type.
        ty: Type,
    },
    /// Unary negation: `-x`
    Negate(Box<TypedExpr>, Type),
}

impl TypedExpr {
    /// Get the resolved type of this expression.
    pub fn ty(&self) -> &Type {
        match self {
            TypedExpr::NumberLit(_) => &Type::Number,
            TypedExpr::StringLit(_) => &Type::String,
            TypedExpr::BoolLit(_) => &Type::Boolean,
            TypedExpr::Ident { ty, .. } => ty,
            TypedExpr::BinaryOp { ty, .. } => ty,
            TypedExpr::Negate(_, ty) => ty,
        }
    }
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_from_annotation() {
        assert_eq!(Type::from_annotation("number"), Type::Number);
        assert_eq!(Type::from_annotation("string"), Type::String);
        assert_eq!(Type::from_annotation("boolean"), Type::Boolean);
        assert_eq!(Type::from_annotation("void"), Type::Void);
        assert_eq!(Type::from_annotation("Request"), Type::Any);
        assert_eq!(Type::from_annotation("Promise<Response>"), Type::Any);
    }

    #[test]
    fn typed_expr_type() {
        let expr = TypedExpr::NumberLit(42.0);
        assert_eq!(expr.ty(), &Type::Number);

        let expr = TypedExpr::BinaryOp {
            op: BinOp::Add,
            left: Box::new(TypedExpr::NumberLit(1.0)),
            right: Box::new(TypedExpr::NumberLit(2.0)),
            ty: Type::Number,
        };
        assert_eq!(expr.ty(), &Type::Number);
    }
}
