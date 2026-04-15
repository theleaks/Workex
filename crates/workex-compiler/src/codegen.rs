//! Cranelift code generation: HIR → native code.
//!
//! Compiles typed functions to native machine code using Cranelift JIT.
//! Key optimization: TypeScript type annotations → direct machine instructions.
//! `number + number` → `fadd`, no speculation, no deopt paths.

use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, UserFuncName, Value};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Linkage, Module};

use crate::hir::{BinOp, Type, TypedExpr, TypedFunction, TypedStmt};

/// A compiled native function ready for execution.
pub struct NativeFunction {
    /// Raw function pointer to JIT-compiled code.
    ptr: *const u8,
    /// Name of the function.
    pub name: String,
    /// Keep the module alive so the code memory isn't freed.
    _module: JITModule,
}

// Safety: the function pointer points to immutable code memory owned by _module.
unsafe impl Send for NativeFunction {}
unsafe impl Sync for NativeFunction {}

impl NativeFunction {
    /// Get the raw function pointer. Caller must transmute to the correct signature.
    pub fn ptr(&self) -> *const u8 {
        self.ptr
    }
}

/// Map our HIR Type to a Cranelift IR type.
fn cranelift_type(ty: &Type) -> types::Type {
    match ty {
        Type::Number => types::F64,
        Type::Boolean => types::I8,
        Type::String => types::I64, // pointer
        Type::Void => types::I8,    // placeholder
        Type::Any => types::I64,    // boxed value pointer
    }
}

/// Create a JIT module configured for the host machine.
fn create_jit_module() -> anyhow::Result<JITModule> {
    let mut flag_builder = settings::builder();
    flag_builder.set("use_colocated_libcalls", "false")?;
    flag_builder.set("is_pic", "false")?;

    let isa_builder = cranelift_native::builder()
        .map_err(|msg| anyhow::anyhow!("host not supported: {msg}"))?;
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .map_err(|e| anyhow::anyhow!("ISA error: {e}"))?;

    Ok(JITModule::new(JITBuilder::with_isa(
        isa,
        default_libcall_names(),
    )))
}

/// Compile a typed function to native code using Cranelift JIT.
pub fn compile_function(func: &TypedFunction) -> anyhow::Result<NativeFunction> {
    let mut module = create_jit_module()?;
    let mut ctx = module.make_context();
    let mut func_ctx = FunctionBuilderContext::new();

    // Build signature
    let mut sig = module.make_signature();
    for param in &func.params {
        sig.params.push(AbiParam::new(cranelift_type(&param.ty)));
    }
    if func.return_type != Type::Void {
        sig.returns
            .push(AbiParam::new(cranelift_type(&func.return_type)));
    }

    // Declare function
    let func_id = module.declare_function(&func.name, Linkage::Export, &sig)?;

    ctx.func.signature = sig;
    ctx.func.name = UserFuncName::user(0, func_id.as_u32());

    // Build function body
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);
        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        // Declare variables for parameters
        let mut param_vars = Vec::new();
        for (i, param) in func.params.iter().enumerate() {
            let var = Variable::from_u32(i as u32);
            let cl_type = cranelift_type(&param.ty);
            builder.declare_var(var, cl_type);
            let block_param = builder.block_params(entry_block)[i];
            builder.def_var(var, block_param);
            param_vars.push(var);
        }

        // Compile statements
        compile_body(&mut builder, &param_vars, func);

        // Finalize (consumes builder via seal_all_blocks + finalize)
        builder.seal_all_blocks();
        builder.finalize();
    }

    // Define and finalize
    module.define_function(func_id, &mut ctx)?;
    module.clear_context(&mut ctx);
    module.finalize_definitions()?;

    let code_ptr = module.get_finalized_function(func_id);

    Ok(NativeFunction {
        ptr: code_ptr,
        name: func.name.clone(),
        _module: module,
    })
}

/// Compile the function body statements.
fn compile_body(
    builder: &mut FunctionBuilder<'_>,
    param_vars: &[Variable],
    func: &TypedFunction,
) {
    let mut has_return = false;

    for stmt in &func.body {
        match stmt {
            TypedStmt::Return(expr) => {
                let val = compile_expr(builder, param_vars, expr);
                builder.ins().return_(&[val]);
                has_return = true;
            }
            TypedStmt::Expr(expr) => {
                compile_expr(builder, param_vars, expr);
            }
        }
    }

    // If no explicit return, add a default
    if !has_return {
        if func.return_type == Type::Void {
            builder.ins().return_(&[]);
        } else {
            let zero = builder.ins().f64const(0.0);
            builder.ins().return_(&[zero]);
        }
    }
}

/// Compile an expression to a Cranelift IR value.
fn compile_expr(
    builder: &mut FunctionBuilder<'_>,
    param_vars: &[Variable],
    expr: &TypedExpr,
) -> Value {
    match expr {
        TypedExpr::NumberLit(val) => builder.ins().f64const(*val),

        TypedExpr::BoolLit(val) => builder.ins().iconst(types::I8, *val as i64),

        TypedExpr::StringLit(_) => {
            // Strings not yet supported in codegen — return null pointer
            builder.ins().iconst(types::I64, 0)
        }

        TypedExpr::Ident { param_index, .. } => builder.use_var(param_vars[*param_index]),

        TypedExpr::BinaryOp {
            op,
            left,
            right,
            ty,
        } => {
            let lhs = compile_expr(builder, param_vars, left);
            let rhs = compile_expr(builder, param_vars, right);
            compile_binop(builder, *op, lhs, rhs, ty, left.ty())
        }

        TypedExpr::Negate(inner, ty) => {
            let val = compile_expr(builder, param_vars, inner);
            match ty {
                Type::Number => builder.ins().fneg(val),
                _ => val,
            }
        }
    }
}

/// Compile a binary operation to native instructions.
/// This is the core optimization: typed ops → single machine instructions.
fn compile_binop(
    builder: &mut FunctionBuilder<'_>,
    op: BinOp,
    lhs: Value,
    rhs: Value,
    result_ty: &Type,
    operand_ty: &Type,
) -> Value {
    match (operand_ty, op) {
        // Number arithmetic → direct float instructions, NO type checks, NO deopt
        (Type::Number, BinOp::Add) => builder.ins().fadd(lhs, rhs),
        (Type::Number, BinOp::Sub) => builder.ins().fsub(lhs, rhs),
        (Type::Number, BinOp::Mul) => builder.ins().fmul(lhs, rhs),
        (Type::Number, BinOp::Div) => builder.ins().fdiv(lhs, rhs),

        // Number comparisons → fcmp + extend to i8
        (Type::Number, BinOp::Eq) => {
            let cmp = builder.ins().fcmp(FloatCC::Equal, lhs, rhs);
            builder.ins().uextend(types::I8, cmp)
        }
        (Type::Number, BinOp::Ne) => {
            let cmp = builder.ins().fcmp(FloatCC::NotEqual, lhs, rhs);
            builder.ins().uextend(types::I8, cmp)
        }
        (Type::Number, BinOp::Lt) => {
            let cmp = builder.ins().fcmp(FloatCC::LessThan, lhs, rhs);
            builder.ins().uextend(types::I8, cmp)
        }
        (Type::Number, BinOp::Le) => {
            let cmp = builder.ins().fcmp(FloatCC::LessThanOrEqual, lhs, rhs);
            builder.ins().uextend(types::I8, cmp)
        }
        (Type::Number, BinOp::Gt) => {
            let cmp = builder.ins().fcmp(FloatCC::GreaterThan, lhs, rhs);
            builder.ins().uextend(types::I8, cmp)
        }
        (Type::Number, BinOp::Ge) => {
            let cmp = builder
                .ins()
                .fcmp(FloatCC::GreaterThanOrEqual, lhs, rhs);
            builder.ins().uextend(types::I8, cmp)
        }

        // Fallback: return zero
        _ => match result_ty {
            Type::Number => builder.ins().f64const(0.0),
            _ => builder.ins().iconst(types::I8, 0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::*;

    fn make_add_function() -> TypedFunction {
        TypedFunction {
            name: "add".to_string(),
            params: vec![
                TypedParam { name: "a".to_string(), ty: Type::Number, index: 0 },
                TypedParam { name: "b".to_string(), ty: Type::Number, index: 1 },
            ],
            return_type: Type::Number,
            body: vec![TypedStmt::Return(TypedExpr::BinaryOp {
                op: BinOp::Add,
                left: Box::new(TypedExpr::Ident { name: "a".to_string(), ty: Type::Number, param_index: 0 }),
                right: Box::new(TypedExpr::Ident { name: "b".to_string(), ty: Type::Number, param_index: 1 }),
                ty: Type::Number,
            })],
        }
    }

    #[test]
    fn compile_add_function() {
        let func = make_add_function();
        let native = compile_function(&func).expect("should compile");
        let add: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(native.ptr()) };

        assert_eq!(add(3.0, 4.0), 7.0);
        assert_eq!(add(0.0, 0.0), 0.0);
        assert_eq!(add(-1.5, 2.5), 1.0);
        assert_eq!(add(1e10, 1e10), 2e10);
    }

    #[test]
    fn compile_subtract() {
        let func = TypedFunction {
            name: "sub".to_string(),
            params: vec![
                TypedParam { name: "a".to_string(), ty: Type::Number, index: 0 },
                TypedParam { name: "b".to_string(), ty: Type::Number, index: 1 },
            ],
            return_type: Type::Number,
            body: vec![TypedStmt::Return(TypedExpr::BinaryOp {
                op: BinOp::Sub,
                left: Box::new(TypedExpr::Ident { name: "a".to_string(), ty: Type::Number, param_index: 0 }),
                right: Box::new(TypedExpr::Ident { name: "b".to_string(), ty: Type::Number, param_index: 1 }),
                ty: Type::Number,
            })],
        };
        let native = compile_function(&func).expect("should compile");
        let sub: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(native.ptr()) };
        assert_eq!(sub(10.0, 3.0), 7.0);
    }

    #[test]
    fn compile_multiply() {
        let func = TypedFunction {
            name: "mul".to_string(),
            params: vec![
                TypedParam { name: "a".to_string(), ty: Type::Number, index: 0 },
                TypedParam { name: "b".to_string(), ty: Type::Number, index: 1 },
            ],
            return_type: Type::Number,
            body: vec![TypedStmt::Return(TypedExpr::BinaryOp {
                op: BinOp::Mul,
                left: Box::new(TypedExpr::Ident { name: "a".to_string(), ty: Type::Number, param_index: 0 }),
                right: Box::new(TypedExpr::Ident { name: "b".to_string(), ty: Type::Number, param_index: 1 }),
                ty: Type::Number,
            })],
        };
        let native = compile_function(&func).expect("should compile");
        let mul: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(native.ptr()) };
        assert_eq!(mul(6.0, 7.0), 42.0);
    }

    #[test]
    fn compile_constant_return() {
        let func = TypedFunction {
            name: "pi".to_string(),
            params: vec![],
            return_type: Type::Number,
            body: vec![TypedStmt::Return(TypedExpr::NumberLit(3.14159))],
        };
        let native = compile_function(&func).expect("should compile");
        let pi: extern "C" fn() -> f64 = unsafe { std::mem::transmute(native.ptr()) };
        assert!((pi() - 3.14159).abs() < 1e-10);
    }

    #[test]
    fn compile_complex_expression() {
        // (a + b) * a
        let func = TypedFunction {
            name: "calc".to_string(),
            params: vec![
                TypedParam { name: "a".to_string(), ty: Type::Number, index: 0 },
                TypedParam { name: "b".to_string(), ty: Type::Number, index: 1 },
            ],
            return_type: Type::Number,
            body: vec![TypedStmt::Return(TypedExpr::BinaryOp {
                op: BinOp::Mul,
                left: Box::new(TypedExpr::BinaryOp {
                    op: BinOp::Add,
                    left: Box::new(TypedExpr::Ident { name: "a".to_string(), ty: Type::Number, param_index: 0 }),
                    right: Box::new(TypedExpr::Ident { name: "b".to_string(), ty: Type::Number, param_index: 1 }),
                    ty: Type::Number,
                }),
                right: Box::new(TypedExpr::Ident { name: "a".to_string(), ty: Type::Number, param_index: 0 }),
                ty: Type::Number,
            })],
        };
        let native = compile_function(&func).expect("should compile");
        let calc: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(native.ptr()) };
        assert_eq!(calc(3.0, 4.0), 21.0);
    }
}
