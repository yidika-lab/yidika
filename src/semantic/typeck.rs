use std::collections::HashMap;
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::semantic::env::{Env, FnSig};
use crate::syntax::ast::*;

#[derive(Clone)]
struct VarInfo {
    type_expr: TypeExpr,
    is_const: bool,
    moved: bool,
}

pub struct TypeChecker<'a> {
    env: &'a mut Env,
    types: HashMap<String, TypeExpr>,
    vars: HashMap<String, VarInfo>,
    local_fns: HashMap<String, FnSig>,
    builtin_modules: std::collections::HashSet<String>,
    std_imported: bool,
    has_error: bool,
}

fn is_copy_type(t: &TypeExpr) -> bool {
    matches!(t, TypeExpr::Int(_) | TypeExpr::Rint(_)
        | TypeExpr::Real(_) | TypeExpr::Bool
        | TypeExpr::Symbol | TypeExpr::Null | TypeExpr::None_)
}

impl<'a> TypeChecker<'a> {
    pub fn new(env: &'a mut Env) -> Self {
        let types = env.types.clone();
        Self { env, types, vars: HashMap::new(), local_fns: HashMap::new(), builtin_modules: std::collections::HashSet::new(), std_imported: false, has_error: false }
    }

    pub fn check_module(&mut self, module: &Module) -> Result<()> {
        // Register universally available builtins
        for name in &["print", "println", "len", "str", "input", "typeof", "int", "real", "bool"] {
            self.builtin_modules.insert(name.to_string());
        }
        // Register builtin module imports
        for import in &module.imports {
            match import.source.as_str() {
                "std" => {
                    self.std_imported = true;
                    for (name, _) in &import.names {
                        if name == "std" {
                            for sub in crate::stdlib::list_submodules() {
                                self.builtin_modules.insert(sub.to_string());
                            }
                        } else if crate::stdlib::list_submodules().any(|s| s == name.as_str()) {
                            self.builtin_modules.insert(name.clone());
                        }
                    }
                }
                "io" | "json" | "datetime" | "path" | "base64" | "re" | "math" | "time" => {
                    for (name, _) in &import.names { self.builtin_modules.insert(name.clone()); }
                }
                _ => {}
            }
        }
        let exported: std::collections::HashSet<&str> =
            module.exports.iter().map(|s| s.as_str()).collect();
        for item in &module.items {
            self.check_item(item, &exported)?;
            if self.has_error { break; }
        }
        if self.has_error {
            Err(error::err(ErrorKind::TypeError, module.span, "Type checking failed"))
        } else {
            Ok(())
        }
    }

    fn fail<T>(&mut self, kind: ErrorKind, span: crate::diagnostics::span::Span, msg: impl Into<String>) -> Result<T> {
        self.has_error = true;
        Err(error::err(kind, span, msg))
    }

    fn check_item(&mut self, item: &ItemNode, exported: &std::collections::HashSet<&str>) -> Result<()> {
        let is_exported = item.exported || exported.contains(item_name(&item.value).as_deref().unwrap_or(""));
        match &item.value {
            ItemKind::Fn { name, params, ret_type, body, .. } => {
                let sig = FnSig {
                    params: params.iter().map(|p| self.resolve_type(&p.type_expr.value)).collect(),
                    ret_type: ret_type.as_ref().map(|r| self.resolve_type(&r.value)).unwrap_or(TypeExpr::None_),
                };
                self.local_fns.insert(name.clone(), sig.clone());
                if is_exported {
                    self.env.add_fn(name.clone(), sig);
                }
                self.vars.clear();
                for p in params {
                    let t = self.resolve_type(&p.type_expr.value);
                    self.vars.insert(p.name.clone(), VarInfo { type_expr: t, is_const: false, moved: false });
                }
                for s in body {
                    self.check_stmt(s)?;
                    if self.has_error { break; }
                }
                Ok(())
            }
            ItemKind::Struct { name, .. } => {
                if is_exported {
                    self.env.add_type(name.clone(), TypeExpr::Named(name.clone()));
                }
                self.types.insert(name.clone(), TypeExpr::Named(name.clone()));
                Ok(())
            }
            ItemKind::TypeAlias { name, type_expr } => {
                let ty = self.resolve_type(&type_expr.value);
                if is_exported {
                    self.env.add_type(name.clone(), ty.clone());
                }
                self.types.insert(name.clone(), ty);
                Ok(())
            }
            ItemKind::Const { name, type_expr, value: _ } => {
                let t = self.resolve_type(&type_expr.value);
                if is_exported {
                    self.env.add_type(name.clone(), t.clone());
                }
                self.vars.insert(name.clone(), VarInfo { type_expr: t, is_const: true, moved: false });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn mark_moved(&mut self, expr: &ExprNode, ty: &TypeExpr) {
        if is_copy_type(ty) { return; }
        match &expr.value {
            Expr::Ident(src) => {
                if let Some(info) = self.vars.get_mut(src) {
                    if !info.moved {
                        info.moved = true;
                    }
                }
            }
            Expr::Field(obj, _) => {
                if let Expr::Ident(src) = &obj.value {
                    if let Some(info) = self.vars.get_mut(src) {
                        if !info.moved {
                            info.moved = true;
                        }
                    }
                }
            }
            Expr::AsConst(inner) => self.mark_moved(inner, ty),
            _ => {}
        }
    }

    fn check_stmt(&mut self, stmt: &StmtNode) -> Result<TypeExpr> {
        match &stmt.value {
            Stmt::Decl { name, type_expr, value, is_const } => {
                if self.vars.contains_key(name) {
                    return self.fail(ErrorKind::NameError, stmt.span,
                        format!("Variable '{}' already declared", name));
                }
                let t = match type_expr {
                    Some(te) => {
                        let resolved = self.resolve_type(&te.value);
                        // Even with explicit type, mark source as moved
                        if let Some(te_ref) = type_expr {
                            let _ = te_ref;
                        }
                        self.check_expr(value)?;
                        self.mark_moved(value, &resolved);
                        resolved
                    }
                    None => {
                        let inferred = self.check_expr(value)?;
                        self.mark_moved(value, &inferred);
                        inferred
                    }
                };
                let actual_const = *is_const || matches!(&value.value, Expr::AsConst(_));
                self.vars.insert(name.clone(), VarInfo { type_expr: t.clone(), is_const: actual_const, moved: false });
                Ok(t)
            }
            Stmt::Expr(e) => self.check_expr(e),
            Stmt::Return(e) => {
                if let Some(x) = e {
                    let t = self.check_expr(x)?;
                    self.mark_moved(x, &t);
                    Ok(t)
                } else {
                    Ok(TypeExpr::None_)
                }
            }
            Stmt::For(name, it, body) => {
                self.check_expr(it)?;
                let saved = self.vars.clone();
                self.vars.insert(name.clone(), VarInfo { type_expr: TypeExpr::Int(0), is_const: false, moved: false });
                for s in body { self.check_stmt(s)?; }
                self.vars = saved;
                Ok(TypeExpr::None_)
            }
            Stmt::While(c, body) => {
                self.check_expr(c)?;
                let saved = self.vars.clone();
                for s in body { self.check_stmt(s)?; }
                self.vars = saved;
                Ok(TypeExpr::None_)
            }
            Stmt::Loop(body) => {
                let saved = self.vars.clone();
                for s in body { self.check_stmt(s)?; }
                self.vars = saved;
                Ok(TypeExpr::Infer)
            }
            Stmt::If(c, then_body, else_body) => {
                self.check_expr(c)?;
                let saved = self.vars.clone();
                for s in then_body { self.check_stmt(s)?; }
                self.vars = saved.clone();
                if let Some(eb) = else_body {
                    for s in eb { self.check_stmt(s)?; }
                }
                self.vars = saved;
                Ok(TypeExpr::None_)
            }
            Stmt::Assign(name, expr) => {
                match self.vars.get(name) {
                    Some(info) if !info.is_const => {
                        let result = self.check_expr(expr)?;
                        self.mark_moved(expr, &result);
                        Ok(result)
                    }
                    Some(_) => self.fail(ErrorKind::TypeError, stmt.span,
                        format!("Cannot assign to const variable '{}'", name)),
                    None => self.fail(ErrorKind::NameError, stmt.span,
                        format!("Variable '{}' not found", name)),
                }
            }
            Stmt::Destruct(_, expr) => self.check_expr(expr),
        }
    }

    fn check_expr(&mut self, expr: &ExprNode) -> Result<TypeExpr> {
        match &expr.value {
            Expr::LitInt(_) | Expr::LitHex(_) => Ok(TypeExpr::Int(0)),
            Expr::LitReal(_) => Ok(TypeExpr::Real(0)),
            Expr::LitStr(_) => Ok(TypeExpr::Str),
            Expr::LitChar(_) => Ok(TypeExpr::Int(8)),
            Expr::LitBool(_) => Ok(TypeExpr::Bool),
            Expr::LitSymbol(_) => Ok(TypeExpr::Symbol),
            Expr::LitNull => Ok(TypeExpr::Null),
            Expr::LitNone => Ok(TypeExpr::None_),
            Expr::Ident(name) => {
                match self.vars.get(name) {
                    Some(var_info) => {
                        if var_info.moved {
                            return self.fail(ErrorKind::NameError, expr.span,
                                format!("Cannot use variable '{}' after it has been moved", name));
                        }
                        Ok(var_info.type_expr.clone())
                    }
                    None => self.fail(ErrorKind::NameError, expr.span,
                        format!("Variable '{}' not found", name)),
                }
            }
            Expr::BinOp(l, op, r) => {
                let lt = self.check_expr(l)?;
                let rt = self.check_expr(r)?;
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                        if lt == TypeExpr::Infer { Ok(rt) }
                        else if rt == TypeExpr::Infer { Ok(lt) }
                        else if lt != rt {
                            self.fail(ErrorKind::TypeError, expr.span,
                                format!("Type mismatch: cannot {:?} {:?} with {:?}", op, lt, rt))
                        } else { Ok(lt) }
                    }
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => Ok(TypeExpr::Bool),
                    BinOp::And | BinOp::Or => Ok(TypeExpr::Bool),
                    BinOp::Assign => {
                        let rt = self.check_expr(r)?;
                        self.mark_moved(r, &rt);
                        Ok(rt)
                    }
                }
            }
            Expr::UnOp(_, i) => self.check_expr(i),
            Expr::Call(callee, args) => {
                let mut arg_types = Vec::new();
                for a in args {
                    arg_types.push(self.check_expr(a)?);
                }
                match &callee.value {
                    Expr::Ident(name) => {
                        if self.builtin_modules.contains(name.as_str()) {
                            match name.as_str() {
                                "print" | "println" | "input" | "typeof" => Ok(TypeExpr::None_),
                                "cos" | "sin" | "sqrt" | "abs" | "floor" | "ceil" | "round" | "max" | "min" | "pow" | "rand" => Ok(TypeExpr::Real(0)),
                                "now" | "utc" => Ok(TypeExpr::Str),
                                "timestamp" => Ok(TypeExpr::Int(0)),
                                "format" => Ok(TypeExpr::Str),
                                "parse" => Ok(TypeExpr::Int(0)),
                                "sleep" => Ok(TypeExpr::None_),
                                "year" | "month" | "day" | "hour" | "minute" | "second" => Ok(TypeExpr::Int(0)),
                                "join" | "dirname" | "basename" | "extension" => Ok(TypeExpr::Str),
                                "is_absolute" => Ok(TypeExpr::Bool),
                                "encode" | "decode" | "stringify" => Ok(TypeExpr::Str),
                                "find" | "split" => Ok(TypeExpr::List(Box::new(TypeExpr::Str))),
                                "replace" => Ok(TypeExpr::Str),
                                "match" => Ok(TypeExpr::Bool),
                                "str" => Ok(TypeExpr::Str),
                                "int" => Ok(TypeExpr::Int(0)),
                                "real" => Ok(TypeExpr::Real(0)),
                                "bool" => Ok(TypeExpr::Bool),
                                "len" => Ok(TypeExpr::Int(0)),
                                _ => {
                                    for (a, t) in args.iter().zip(arg_types.iter()) {
                                        self.mark_moved(a, t);
                                    }
                                    if let Some(sig) = self.env.get_fn(name).or_else(|| self.local_fns.get(name)) { Ok(sig.ret_type.clone()) } else { Ok(TypeExpr::Infer) }
                                },
                            }
                        } else if let Some(sig) = self.env.get_fn(name).or_else(|| self.local_fns.get(name)) {
                            let param_types = sig.params.clone();
                            let ret_type = sig.ret_type.clone();
                            for (i, (a, t)) in args.iter().zip(arg_types.iter()).enumerate() {
                                if let Some(pt) = param_types.get(i) {
                                    if !is_copy_type(pt) {
                                        self.mark_moved(a, t);
                                    }
                                }
                            }
                            Ok(ret_type)
                        } else if self.vars.contains_key(name) {
                            Ok(TypeExpr::Infer)
                        } else {
                            return self.fail(ErrorKind::NameError, expr.span,
                                format!("Function '{}' not found or imported", name))
                        }
                    }
                    Expr::Field(obj, field) => {
                        let (mod_name, func_name) = match &obj.value {
                            Expr::Ident(mod_name) => (mod_name.clone(), field.clone()),
                            Expr::Field(inner, func_field) => {
                                match &inner.value {
                                    Expr::Ident(name) if name == "std" => (func_field.clone(), field.clone()),
                                    _ => return Ok(TypeExpr::Infer),
                                }
                            }
                            _ => return Ok(TypeExpr::Infer),
                        };
                        if self.builtin_modules.contains(&mod_name) || (self.std_imported && mod_name == "std") {
                            match mod_name.as_str() {
                                "fs" => match func_name.as_str() {
                                    "read" | "exists" | "is_dir" | "is_file" => Ok(TypeExpr::Str),
                                    "write" | "append" | "remove" => Ok(TypeExpr::None_),
                                    "list" => Ok(TypeExpr::List(Box::new(TypeExpr::Str))),
                                    _ => Ok(TypeExpr::Infer),
                                },
                                "sys" => match func_name.as_str() {
                                    "env" | "cwd" | "platform" => Ok(TypeExpr::Str),
                                    "args" => Ok(TypeExpr::List(Box::new(TypeExpr::Str))),
                                    "pid" => Ok(TypeExpr::Int(0)),
                                    "exit" => Ok(TypeExpr::None_),
                                    "sleep" => Ok(TypeExpr::None_),
                                    _ => Ok(TypeExpr::Infer),
                                },
                                "math" => match func_name.as_str() {
                                    "cos" | "sin" | "sqrt" | "abs" | "floor" | "ceil" | "round" | "max" | "min" | "pow" | "rand" => Ok(TypeExpr::Real(0)),
                                    _ => Ok(TypeExpr::Infer),
                                },
                                "time" => match func_name.as_str() {
                                    "now" | "utc" => Ok(TypeExpr::Str),
                                    "timestamp" => Ok(TypeExpr::Int(0)),
                                    "format" => Ok(TypeExpr::Str),
                                    "parse" => Ok(TypeExpr::Int(0)),
                                    "sleep" => Ok(TypeExpr::None_),
                                    "year" | "month" | "day" | "hour" | "minute" | "second" => Ok(TypeExpr::Int(0)),
                                    _ => Ok(TypeExpr::Infer),
                                },
                                "json" => match func_name.as_str() {
                                    "parse" => Ok(TypeExpr::Named("auto".into())),
                                    "stringify" => Ok(TypeExpr::Str),
                                    _ => Ok(TypeExpr::Infer),
                                },
                                "datetime" => match func_name.as_str() {
                                    "now" | "utc" => Ok(TypeExpr::Str),
                                    "format" => Ok(TypeExpr::Str),
                                    "parse" => Ok(TypeExpr::Int(0)),
                                    "timestamp" => Ok(TypeExpr::Int(0)),
                                    "year" | "month" | "day" | "hour" | "minute" | "second" => Ok(TypeExpr::Int(0)),
                                    _ => Ok(TypeExpr::Infer),
                                },
                                "path" => match func_name.as_str() {
                                    "join" | "dirname" | "basename" | "extension" => Ok(TypeExpr::Str),
                                    "is_absolute" => Ok(TypeExpr::Bool),
                                    _ => Ok(TypeExpr::Infer),
                                },
                                "base64" => match func_name.as_str() {
                                    "encode" | "decode" => Ok(TypeExpr::Str),
                                    _ => Ok(TypeExpr::Infer),
                                },
                                "re" => match func_name.as_str() {
                                    "match" => Ok(TypeExpr::Bool),
                                    "find" | "split" => Ok(TypeExpr::List(Box::new(TypeExpr::Str))),
                                    "replace" => Ok(TypeExpr::Str),
                                    _ => Ok(TypeExpr::Infer),
                                },
                                _ => Ok(TypeExpr::Infer),
                            }
                        } else {
                            Ok(TypeExpr::Infer)
                        }
                    }
                    _ => Ok(TypeExpr::Infer),
                }
            }
            Expr::Block(stmts) => {
                let saved = self.vars.clone();
                for s in stmts {
                    self.check_stmt(s)?;
                    if self.has_error { break; }
                }
                self.vars = saved;
                Ok(TypeExpr::None_)
            }
            Expr::If(c, t, e) => {
                self.check_expr(c)?;
                let r = self.check_expr(t)?;
                if let Some(x) = e { self.check_expr(x)?; }
                Ok(r)
            }
            Expr::ResultOk(i) => self.check_expr(i),
            Expr::ResultErr(i) => self.check_expr(i),
            Expr::Spawn(i) => self.check_expr(i),
            Expr::AsConst(i) => self.check_expr(i),
            Expr::TupleLit(items) => {
                let types: Result<Vec<TypeExpr>> = items.iter().map(|i| self.check_expr(i)).collect();
                Ok(TypeExpr::Tuple(types?))
            }
            Expr::MapLit(pairs) => {
                let mut key_ty = TypeExpr::Infer;
                let mut val_ty = TypeExpr::Infer;
                for (i, (k, v)) in pairs.iter().enumerate() {
                    let kt = self.check_expr(k)?;
                    let vt = self.check_expr(v)?;
                    if i == 0 { key_ty = kt; val_ty = vt; }
                }
                Ok(TypeExpr::Map(Box::new(key_ty), Box::new(val_ty)))
            }
            Expr::SetLit(items) => {
                let mut elem_ty = TypeExpr::Infer;
                for (i, item) in items.iter().enumerate() {
                    let t = self.check_expr(item)?;
                    if i == 0 { elem_ty = t; }
                }
                Ok(TypeExpr::Set(Box::new(elem_ty)))
            }
            Expr::FnLit(params, ret_type, body) => {
                let mut param_types = Vec::new();
                for p in params {
                    let pt = self.resolve_type(&p.type_expr.value);
                    param_types.push(pt);
                }
                let rt = ret_type.as_ref().map(|t| self.resolve_type(&t.value)).unwrap_or(TypeExpr::Infer);
                if !matches!(rt, TypeExpr::Infer) {
                    let _body_ty = self.check_expr(body)?;
                } else {
                    self.check_expr(body)?;
                }
                Ok(TypeExpr::Fn(param_types, Box::new(rt)))
            }
            _ => Ok(TypeExpr::Infer),
        }
    }

    fn resolve_type(&self, t: &TypeExpr) -> TypeExpr {
        match t {
            TypeExpr::Named(name) => self.types.get(name).cloned()
                .unwrap_or_else(|| {
                    self.env.get_type(name).cloned()
                        .unwrap_or_else(|| TypeExpr::Named(name.clone()))
                }),
            other => other.clone(),
        }
    }
}

fn item_name(kind: &ItemKind) -> Option<String> {
    match kind {
        ItemKind::Fn { name, .. } | ItemKind::Struct { name, .. }
        | ItemKind::Class { name, .. } | ItemKind::Interface { name, .. }
        | ItemKind::Union { name, .. } | ItemKind::TypeAlias { name, .. }
        | ItemKind::Const { name, .. } => Some(name.clone()),
    }
}
