use std::collections::HashMap;
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::semantic::env::{Env, FnSig, MethodDef, StructDef, ClassDef};
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
    fn_generics: HashMap<String, Vec<String>>,
    builtin_modules: std::collections::HashSet<String>,
    std_imported: bool,
    has_error: bool,
    current_fn_ret_type: Option<TypeExpr>,
}

fn types_compatible(expected: &TypeExpr, actual: &TypeExpr) -> bool {
    if *expected == TypeExpr::Infer || *actual == TypeExpr::Infer { return true; }
    if *actual == TypeExpr::Null { return true; }
    if matches!(expected, TypeExpr::Named(n) if n == "auto") { return true; }
    if matches!(actual, TypeExpr::Named(n) if n == "auto") { return true; }
    if expected == actual { return true; }
    // Generic("Pair", [int, str]) is compatible with Named("Pair")
    if let (TypeExpr::Generic(e_name, _), TypeExpr::Named(a_name)) = (expected, actual) {
        if e_name == a_name { return true; }
    }
    if let (TypeExpr::Named(e_name), TypeExpr::Generic(a_name, _)) = (expected, actual) {
        if e_name == a_name { return true; }
    }
    match (expected, actual) {
        (TypeExpr::Rint(_), TypeExpr::Int(_)) => true,
        (TypeExpr::Real(_), TypeExpr::Int(_) | TypeExpr::Rint(_)) => true,
        (TypeExpr::Nullable(inner), actual) if types_compatible(inner, actual) => true,
        (TypeExpr::Complex(er, ei), TypeExpr::Complex(ar, ai)) => {
            types_compatible(er, ar) && types_compatible(ei, ai)
        }
        (TypeExpr::Complex(_, _), TypeExpr::Int(_) | TypeExpr::Rint(_) | TypeExpr::Real(_)) => true,
        (TypeExpr::Union(elems), _) => elems.iter().any(|e| types_compatible(e, actual)),
        (_, TypeExpr::Union(elems)) => elems.iter().all(|a| types_compatible(expected, a)),
        _ => false,
    }
}

fn types_compatible_params(expected: &[TypeExpr], actual: &[TypeExpr]) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    expected.iter().zip(actual).all(|(e, a)| types_compatible(e, a))
}

fn wider_type(a: &TypeExpr, b: &TypeExpr) -> TypeExpr {
    if types_compatible(a, b) { return a.clone(); }
    if types_compatible(b, a) { return b.clone(); }
    match (a, b) {
        (TypeExpr::Complex(_, _), _) | (_, TypeExpr::Complex(_, _)) => {
            TypeExpr::Complex(Box::new(TypeExpr::Real(0)), Box::new(TypeExpr::Real(0)))
        }
        (TypeExpr::Real(_), _) | (_, TypeExpr::Real(_)) => TypeExpr::Real(0),
        (TypeExpr::Rint(_), _) | (_, TypeExpr::Rint(_)) => TypeExpr::Rint(0),
        _ => a.clone(),
    }
}

fn infer_type_args(param_type: &TypeExpr, arg_type: &TypeExpr, generic_params: &[String], inferred: &mut HashMap<String, TypeExpr>) {
    if let TypeExpr::Named(gname) = param_type {
        if generic_params.contains(gname) {
            inferred.entry(gname.clone()).or_insert_with(|| arg_type.clone());
        }
        return;
    }
    match (param_type, arg_type) {
        (TypeExpr::List(p), TypeExpr::List(a)) => infer_type_args(p, a, generic_params, inferred),
        (TypeExpr::Set(p), TypeExpr::Set(a)) => infer_type_args(p, a, generic_params, inferred),
        (TypeExpr::Vector(p), TypeExpr::Vector(a)) => infer_type_args(p, a, generic_params, inferred),
        (TypeExpr::Matrix(p), TypeExpr::Matrix(a)) => infer_type_args(p, a, generic_params, inferred),
        (TypeExpr::Nullable(p), TypeExpr::Nullable(a)) => infer_type_args(p, a, generic_params, inferred),
        (TypeExpr::Map(pk, pv), TypeExpr::Map(ak, av)) => {
            infer_type_args(pk, ak, generic_params, inferred);
            infer_type_args(pv, av, generic_params, inferred);
        }
        (TypeExpr::Tuple(pts), TypeExpr::Tuple(ats)) => {
            for (p, a) in pts.iter().zip(ats.iter()) {
                infer_type_args(p, a, generic_params, inferred);
            }
        }
        (TypeExpr::Generic(pn, pargs), TypeExpr::Generic(an, aargs)) if pn == an => {
            for (p, a) in pargs.iter().zip(aargs.iter()) {
                infer_type_args(p, a, generic_params, inferred);
            }
        }
        _ => {}
    }
}

fn is_copy_type(t: &TypeExpr) -> bool {
    matches!(t, TypeExpr::Int(_) | TypeExpr::Rint(_)
        | TypeExpr::Real(_) | TypeExpr::Bool
        | TypeExpr::Symbol | TypeExpr::Null | TypeExpr::None_
        | TypeExpr::Complex(_, _))
}

impl<'a> TypeChecker<'a> {
    pub fn new(env: &'a mut Env) -> Self {
        let types = env.types.clone();
        Self { env, types, vars: HashMap::new(), local_fns: HashMap::new(), fn_generics: HashMap::new(), builtin_modules: std::collections::HashSet::new(), std_imported: false, has_error: false, current_fn_ret_type: None }
    }

    pub fn check_module(&mut self, module: &Module) -> Result<()> {
        // Register universally available builtins
        for name in &["print", "println", "len", "str", "input", "fetch", "typeof", "int", "real", "bool"] {
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
                "io" | "json" | "datetime" | "path" | "base64" | "re" | "math" | "time" | "net" => {
                    for (name, _) in &import.names { self.builtin_modules.insert(name.clone()); }
                }
                _ => {
                    if let Some(lang) = &import.lang {
                        if lang == "json" {
                            for (i, (name, _)) in import.names.iter().enumerate() {
                                self.builtin_modules.insert(name.clone());
                                if import.is_const.get(i).copied().unwrap_or(false) {
                                    self.vars.insert(name.clone(), VarInfo { type_expr: TypeExpr::Infer, is_const: true, moved: false });
                                }
                            }
                        } else {
                            for (name, _) in &import.names {
                                self.builtin_modules.insert(name.clone());
                            }
                        }
                    }
                }
            }
        }
        let exported: std::collections::HashSet<&str> =
            module.exports.iter().map(|s| s.as_str()).collect();
        // Pass 1: register all interfaces first (so classes can reference them)
        for item in &module.items {
            if let ItemKind::Interface { name, methods } = &item.value {
                let mut method_map: HashMap<String, FnSig> = HashMap::new();
                for m in methods {
                    let params: Vec<TypeExpr> = m.params.iter().map(|p| self.resolve_type(&p.type_expr.value)).collect();
                    let ret = m.ret_type.as_ref().map(|r| self.resolve_type(&r.value)).unwrap_or(TypeExpr::None_);
                    method_map.insert(m.name.clone(), FnSig { params, ret_type: ret, self_is_ref: false });
                }
                self.env.add_interface(name.clone(), crate::semantic::env::InterfaceDef { methods: method_map });
                self.types.insert(name.clone(), TypeExpr::Named(name.clone()));
            }
        }
        // Pass 2: check all items (classes can now validate interface implementations)
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
        let is_exported = item.decorators.contains(&"export".to_string()) || exported.contains(item_name(&item.value).as_deref().unwrap_or(""));
        match &item.value {
            ItemKind::Fn { name, params, ret_type, body, generics, .. } => {
                let resolved_params: Vec<TypeExpr> = params.iter().map(|p| self.resolve_type(&p.type_expr.value)).collect();
                let sig = FnSig {
                    params: resolved_params,
                    ret_type: ret_type.as_ref().map(|r| self.resolve_type(&r.value)).unwrap_or(TypeExpr::None_),
                    self_is_ref: params.first().map(|p| p.is_ref).unwrap_or(false),
                };
                if !generics.is_empty() {
                    self.fn_generics.insert(name.clone(), generics.clone());
                }
                self.local_fns.insert(name.clone(), sig.clone());
                if is_exported {
                    self.env.add_fn(name.clone(), sig);
                }
                self.vars.clear();
                for p in params {
                    let t = self.resolve_type(&p.type_expr.value);
                    self.vars.insert(p.name.clone(), VarInfo { type_expr: t, is_const: false, moved: false });
                }
                self.current_fn_ret_type = Some(ret_type.as_ref().map(|r| self.resolve_type(&r.value)).unwrap_or(TypeExpr::Infer));
                for s in body {
                     self.check_stmt(s)?;
                     if self.has_error { break; }
                 }
                // Infer return type if not explicitly annotated
                if ret_type.is_none() {
                    let inferred = self.current_fn_ret_type.as_ref().map(|t| {
                        if *t == TypeExpr::Infer { TypeExpr::None_ } else { t.clone() }
                    }).unwrap_or(TypeExpr::None_);
                    self.local_fns.get_mut(name).map(|sig| sig.ret_type = inferred.clone());
                    if is_exported {
                        self.env.add_fn(name.clone(), self.local_fns[name].clone());
                    }
                }
                self.current_fn_ret_type = None;
                Ok(())
            }
            ItemKind::Struct { name, fields, generics } => {
                if is_exported {
                    self.env.add_type(name.clone(), TypeExpr::Named(name.clone()));
                }
                self.types.insert(name.clone(), TypeExpr::Named(name.clone()));
                self.env.add_struct(name.clone(), StructDef { fields: fields.clone(), generics: generics.clone() });
                Ok(())
            }
            ItemKind::Class { name, fields, methods, generics, implements, extends, .. } => {
                if is_exported {
                    self.env.add_type(name.clone(), TypeExpr::Named(name.clone()));
                }
                self.types.insert(name.clone(), TypeExpr::Named(name.clone()));
                let mut all_fields: Vec<Param> = Vec::new();
                let mut method_map: HashMap<String, MethodDef> = HashMap::new();
                if let Some(parent) = extends {
                    if let Some(parent_cls) = self.env.get_class(parent) {
                        all_fields.extend(parent_cls.fields.clone());
                        for (mname, mdef) in &parent_cls.methods {
                            method_map.insert(mname.clone(), mdef.clone());
                        }
                    } else {
                        return self.fail(ErrorKind::NameError, item.span,
                            format!("Parent class '{}' not found", parent));
                    }
                }
                all_fields.extend(fields.clone());
                for m in methods {
                    if let ItemKind::Fn { name: mname, params, ret_type, generics: mgens, .. } = m {
                        let rt = ret_type.as_ref().map(|r| self.resolve_type(&r.value)).unwrap_or(TypeExpr::None_);
                        method_map.insert(mname.clone(), MethodDef { params: params.clone(), ret_type: rt, generics: mgens.clone() });
                    }
                }
                // Validate that all interface methods are implemented
                for iface_name in implements {
                    self.env.add_class_interface(name.clone(), iface_name.clone());
                    if let Some(iface) = self.env.get_interface(iface_name) {
                        for (mname, isig) in &iface.methods {
                            let mmethod = method_map.get(mname);
                            match mmethod {
                                Some(mdef) => {
                                    let iface_params: Vec<TypeExpr> = isig.params.iter().map(|p| self.resolve_type(p)).collect();
                                    let impl_params: Vec<TypeExpr> = mdef.params.iter().map(|p| self.resolve_type(&p.type_expr.value)).collect();
                                    if !types_compatible_params(&iface_params, &impl_params) {
                                        return self.fail(ErrorKind::TypeError, item.span,
                                            format!("Class '{}' method '{}' parameter types don't match interface '{}'", name, mname, iface_name));
                                    }
                                    let iface_ret = isig.ret_type.clone();
                                    let impl_ret = mdef.ret_type.clone();
                                    if !types_compatible(&iface_ret, &impl_ret) {
                                        return self.fail(ErrorKind::TypeError, item.span,
                                            format!("Class '{}' method '{}' return type doesn't match interface '{}'", name, mname, iface_name));
                                    }
                                }
                                None => {
                                    return self.fail(ErrorKind::TypeError, item.span,
                                        format!("Class '{}' does not implement method '{}' required by interface '{}'", name, mname, iface_name));
                                }
                            }
                        }
                    } else {
                        return self.fail(ErrorKind::NameError, item.span,
                            format!("Interface '{}' not found", iface_name));
                    }
                }
                self.env.add_class(name.clone(), ClassDef { fields: all_fields, methods: method_map, generics: generics.clone(), interfaces: implements.clone(), extends: extends.clone() });
                Ok(())
            }
            ItemKind::Enum { name, variants } => {
                self.env.add_enum(name.clone(), variants.clone());
                if is_exported {
                    self.env.add_type(name.clone(), TypeExpr::Named(name.clone()));
                }
                self.types.insert(name.clone(), TypeExpr::Named(name.clone()));
                Ok(())
            }
            ItemKind::Union { name, variants } => {
                let variant_types: Vec<TypeExpr> = variants.iter().map(|p| self.resolve_type(&p.type_expr.value)).collect();
                if is_exported {
                    self.env.add_type(name.clone(), TypeExpr::Union(variant_types.clone()));
                }
                self.types.insert(name.clone(), TypeExpr::Union(variant_types));
                Ok(())
            }
            ItemKind::Object { name, fields, methods, init_body } => {
                if is_exported {
                    self.env.add_type(name.clone(), TypeExpr::Named(name.clone()));
                }
                self.types.insert(name.clone(), TypeExpr::Named(name.clone()));
                let saved = self.vars.clone();
                for f in fields {
                    let t = self.resolve_type(&f.type_expr.value);
                    self.vars.insert(f.name.clone(), VarInfo { type_expr: t, is_const: false, moved: false });
                }
                for s in init_body {
                    self.check_stmt(s)?;
                }
                for m in methods {
                    self.check_item(&ItemNode::new(0, crate::diagnostics::span::Span::new(0,0), m.clone()), &std::collections::HashSet::new())?;
                }
                self.vars = saved;
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
            Expr::Variant(_, _, args) => {
                for arg in args { self.mark_moved(arg, &TypeExpr::Infer); }
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
                        let expr_ty = self.check_expr(value)?;
                        self.mark_moved(value, &resolved);
                        if !types_compatible(&resolved, &expr_ty) {
                            // Check if expr_ty implements resolved as an interface
                            let implements_iface = match (&resolved, &expr_ty) {
                                (TypeExpr::Named(iface_name), TypeExpr::Named(type_name)) => {
                                    self.env.get_class_interfaces(type_name)
                                        .map(|ifaces| ifaces.contains(iface_name))
                                        .unwrap_or(false)
                                }
                                _ => false,
                            };
                            if !implements_iface {
                                return self.fail(ErrorKind::TypeError, stmt.span,
                                    format!("Type mismatch: variable '{}' declared as {} but expression has type {}", name, resolved, expr_ty));
                            }
                        }
                        // Validate struct/class literal fields with type args from annotation
                        if let Expr::StructLit(struct_name, struct_fields) = &value.value {
                            self.check_struct_fields_with_type_args(struct_name, struct_fields, &resolved, stmt.span)?;
                        }
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
                    let should_update = self.current_fn_ret_type.as_ref().map_or(false, |rt| *rt == TypeExpr::Infer);
                    if should_update {
                        self.current_fn_ret_type = Some(t.clone());
                    } else if let Some(ref ret_ty) = self.current_fn_ret_type {
                        if !types_compatible(ret_ty, &t) {
                            return self.fail(ErrorKind::TypeError, stmt.span,
                                format!("Type mismatch: function returns {} but expression has type {}", ret_ty, t));
                        }
                    }
                    Ok(t)
                } else {
                    let should_update = self.current_fn_ret_type.as_ref().map_or(false, |rt| *rt == TypeExpr::Infer);
                    if should_update {
                        self.current_fn_ret_type = Some(TypeExpr::None_);
                    } else if let Some(ref ret_ty) = self.current_fn_ret_type {
                        if !types_compatible(ret_ty, &TypeExpr::None_) {
                            return self.fail(ErrorKind::TypeError, stmt.span,
                                format!("Type mismatch: function returns {} but no value returned", ret_ty));
                        }
                    }
                    Ok(TypeExpr::None_)
                }
            }
            Stmt::For(name, it, body, is_for_of) => {
                let iter_ty = self.check_expr(it)?;
                let var_ty = match (&iter_ty, *is_for_of) {
                    (TypeExpr::List(elem), true) => *elem.clone(),
                    (TypeExpr::List(_), false) => TypeExpr::Int(0),
                    (TypeExpr::Set(elem), _) => *elem.clone(),
                    (TypeExpr::Map(_, v), true) => *v.clone(),
                    (TypeExpr::Map(k, _), false) => *k.clone(),
                    _ => TypeExpr::Int(0),
                };
                let saved = self.vars.clone();
                self.vars.insert(name.clone(), VarInfo { type_expr: var_ty, is_const: false, moved: false });
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
                let var_type = self.vars.get(name).map(|info| (info.type_expr.clone(), info.is_const));
                match var_type {
                    Some((ty, false)) => {
                        let result = self.check_expr(expr)?;
                        self.mark_moved(expr, &result);
                        if !types_compatible(&ty, &result) {
                            return self.fail(ErrorKind::TypeError, stmt.span,
                                format!("Type mismatch: variable '{}' has type {} but assigned expression has type {}", name, ty, result));
                        }
                        Ok(result)
                    }
                    Some((_, true)) => self.fail(ErrorKind::TypeError, stmt.span,
                        format!("Cannot assign to const variable '{}'", name)),
                    None => {
                        let inferred = self.check_expr(expr)?;
                        self.mark_moved(expr, &inferred);
                        self.vars.insert(name.clone(), VarInfo { type_expr: inferred.clone(), is_const: false, moved: false });
                        Ok(inferred)
                    }
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
                    None => {
                        if let Some(sig) = self.env.get_fn(name).or_else(|| self.local_fns.get(name)) {
                            Ok(sig.ret_type.clone())
                        } else {
                            self.fail(ErrorKind::NameError, expr.span,
                                format!("Variable '{}' not found", name))
                        }
                    }
                }
            }
            Expr::BinOp(l, op, r) => {
                let lt = self.check_expr(l)?;
                let rt = self.check_expr(r)?;
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                        if lt == TypeExpr::Infer { Ok(rt) }
                        else if rt == TypeExpr::Infer { Ok(lt) }
                        else if types_compatible(&lt, &rt) || types_compatible(&rt, &lt) {
                            Ok(wider_type(&lt, &rt))
                        } else {
                            self.fail(ErrorKind::TypeError, expr.span,
                                format!("Type mismatch: cannot {} {} with {}", op, lt, rt))
                        }
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
            Expr::UnOp(op, i) => {
                let t = self.check_expr(i)?;
                match op {
                    UnOp::Neg => match &t {
                        TypeExpr::Int(_) => Ok(TypeExpr::Rint(0)),
                        _ => Ok(t),
                    }
                    UnOp::Not => Ok(t),
                }
            }
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
                                "encode" | "decode" | "stringify" | "fetch" => Ok(TypeExpr::Str),
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
                            let type_args = self.fn_generics.get(name).map(|generic_params| {
                                let mut inferred: HashMap<String, TypeExpr> = HashMap::new();
                                for (pt, t) in param_types.iter().zip(arg_types.iter()) {
                                    infer_type_args(pt, t, generic_params, &mut inferred);
                                }
                                inferred
                            });
                            if let Some(ref type_args) = type_args {
                                for (i, (a, t)) in args.iter().zip(arg_types.iter()).enumerate() {
                                    if let Some(pt) = param_types.get(i) {
                                        let substituted_pt = substitute_type(pt, type_args);
                                        if !types_compatible(&substituted_pt, t) {
                                            return self.fail(ErrorKind::TypeError, expr.span,
                                                format!("Type mismatch: parameter {} of function '{}' expects {} but got {}", i, name, substituted_pt, t));
                                        }
                                        if !is_copy_type(&substituted_pt) {
                                            self.mark_moved(a, t);
                                        }
                                    }
                                }
                                Ok(substitute_type(&ret_type, &type_args))
                            } else {
                                for (i, (a, t)) in args.iter().zip(arg_types.iter()).enumerate() {
                                    if let Some(pt) = param_types.get(i) {
                                        if !types_compatible(pt, t) {
                                            return self.fail(ErrorKind::TypeError, expr.span,
                                                format!("Type mismatch: parameter {} of function '{}' expects {} but got {}", i, name, pt, t));
                                        }
                                        if !is_copy_type(pt) {
                                            self.mark_moved(a, t);
                                        }
                                    }
                                }
                                Ok(ret_type)
                            }
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
                            // Check class method calls (on variables) or type.method calls (on types/objects)
                            let obj_type = match &obj.value {
                                Expr::Ident(obj_name) => {
                                    // Try variable lookup first
                                    if let Some(var_info) = self.vars.get(obj_name) {
                                        Some(var_info.type_expr.clone())
                                    } else if let Some(ty) = self.types.get(obj_name).or_else(|| self.env.get_type(obj_name)) {
                                        Some(ty.clone())
                                    } else {
                                        None
                                    }
                                }
                                _ => match self.check_expr(obj) { Ok(t) => Some(t), Err(_) => None },
                            };
                            if let Some(obj_type) = obj_type {
                                let type_name = match &obj_type {
                                    TypeExpr::Named(n) => Some(n.clone()),
                                    TypeExpr::Generic(n, _) => Some(n.clone()),
                                    _ => None,
                                };
                                if let Some(cls_name) = type_name {
                                    let cls_def_opt = self.env.get_class(&cls_name).cloned();
                                    if let Some(cls_def) = cls_def_opt {
                                        if let Some(method) = cls_def.methods.get(&func_name) {
                                            let mut type_args = HashMap::new();
                                            if let TypeExpr::Generic(_, cls_type_args) = &obj_type {
                                                for (g, ta) in cls_def.generics.iter().zip(cls_type_args.iter()) {
                                                    type_args.insert(g.clone(), ta.clone());
                                                }
                                            }
                                            let method_params: Vec<&Param> = method.params.iter().skip(1).collect();
                                            for (i, (a, t)) in args.iter().zip(arg_types.iter()).enumerate() {
                                                if let Some(param_def) = method_params.get(i) {
                                                    let param_ty = substitute_type(&param_def.type_expr.value, &type_args);
                                                    if !types_compatible(&param_ty, t) {
                                                        return self.fail(ErrorKind::TypeError, expr.span,
                                                            format!("Type mismatch: parameter {} of method '{}' expects {} but got {}", i, func_name, param_ty, t));
                                                    }
                                                    if !is_copy_type(&param_ty) {
                                                        self.mark_moved(a, t);
                                                    }
                                                }
                                            }
                                            let ret_ty = substitute_type(&method.ret_type, &type_args);
                                            return Ok(ret_ty);
                                        } else {
                                            return self.fail(ErrorKind::NameError, expr.span,
                                                format!("Class '{}' has no method '{}'", cls_name, func_name));
                                        }
                                    }
                                }
                            }
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
            Expr::ResultOk(i) => { self.check_expr(i)?; Ok(TypeExpr::Named("auto".into())) }
            Expr::ResultErr(i) => { self.check_expr(i)?; Ok(TypeExpr::Named("auto".into())) }
            Expr::Spawn(i) => self.check_expr(i),
            Expr::AsConst(i) => self.check_expr(i),
            Expr::PostInc(i) | Expr::PostDec(i) => self.check_expr(i),
            Expr::LitComplex(r, im) => {
                let rt = self.check_expr(r)?;
                let it = self.check_expr(im)?;
                Ok(TypeExpr::Complex(Box::new(rt), Box::new(it)))
            }
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
            Expr::Variant(enum_name, _variant_name, args) => {
                let mut arg_types = Vec::new();
                for a in args {
                    let t = self.check_expr(a)?;
                    self.mark_moved(a, &t);
                    arg_types.push(t);
                }
                Ok(TypeExpr::Named(enum_name.clone()))
            }
            Expr::ListLit(items) => {
                let mut elem_ty = TypeExpr::Infer;
                for (i, item) in items.iter().enumerate() {
                    let t = self.check_expr(item)?;
                    if i == 0 { elem_ty = t; }
                }
                Ok(TypeExpr::List(Box::new(elem_ty)))
            }
            Expr::SetLit(items) => {
                let mut elem_ty = TypeExpr::Infer;
                for (i, item) in items.iter().enumerate() {
                    let t = self.check_expr(item)?;
                    if i == 0 { elem_ty = t; }
                }
                Ok(TypeExpr::Set(Box::new(elem_ty)))
            }
            Expr::VectorLit(items) => {
                let mut elem_ty = TypeExpr::Infer;
                for (i, item) in items.iter().enumerate() {
                    let t = self.check_expr(item)?;
                    if i == 0 { elem_ty = t; }
                }
                Ok(TypeExpr::Vector(Box::new(elem_ty)))
            }
            Expr::MatrixLit(rows) => {
                let mut elem_ty = TypeExpr::Infer;
                for row in rows {
                    for (i, item) in row.iter().enumerate() {
                        let t = self.check_expr(item)?;
                        if i == 0 { elem_ty = t; }
                    }
                }
                Ok(TypeExpr::Matrix(Box::new(elem_ty)))
            }
            Expr::FnLit(params, ret_type, body) => {
                let saved = self.vars.clone();
                let mut param_types = Vec::new();
                for p in params {
                    let pt = self.resolve_type(&p.type_expr.value);
                    param_types.push(pt);
                    self.vars.insert(p.name.clone(), VarInfo { type_expr: self.resolve_type(&p.type_expr.value), is_const: false, moved: false });
                }
                let rt = ret_type.as_ref().map(|t| self.resolve_type(&t.value)).unwrap_or(TypeExpr::Infer);
                if !matches!(rt, TypeExpr::Infer) {
                    let _body_ty = self.check_expr(body)?;
                } else {
                    self.check_expr(body)?;
                }
                self.vars = saved;
                Ok(TypeExpr::Fn(param_types, Box::new(rt)))
            }
            Expr::Closure(params, body) => {
                let saved = self.vars.clone();
                let mut param_types = Vec::new();
                for p in params {
                    let pt = self.resolve_type(&p.type_expr.value);
                    param_types.push(pt);
                    self.vars.insert(p.name.clone(), VarInfo { type_expr: self.resolve_type(&p.type_expr.value), is_const: false, moved: false });
                }
                self.check_expr(body)?;
                self.vars = saved;
                Ok(TypeExpr::Fn(param_types, Box::new(TypeExpr::Infer)))
            }
            Expr::StructLit(name, fields) => {
                let struct_def_opt = self.env.get_struct(name).cloned();
                if let Some(struct_def) = struct_def_opt {
                    for (field_name, field_expr) in fields {
                        self.check_expr(field_expr)?;
                        if !struct_def.fields.iter().any(|f| f.name == *field_name) {
                            return self.fail(ErrorKind::TypeError, expr.span,
                                format!("Unknown field '{}' for struct '{}'", field_name, name));
                        }
                    }
                    Ok(TypeExpr::Named(name.clone()))
                } else {
                    for (_, field_expr) in fields {
                        self.check_expr(field_expr)?;
                    }
                    Ok(TypeExpr::Infer)
                }
            }
            Expr::As(inner, target_type) => {
                let inner_ty = self.check_expr(inner)?;
                let target_ty = self.resolve_type(&target_type.value);
                // Allow casts from generic type params (Named) to anything
                if matches!(&inner_ty, TypeExpr::Named(_)) {
                    return Ok(target_ty);
                }
                match (&inner_ty, &target_ty) {
                    (TypeExpr::Int(_), TypeExpr::Str) => Ok(TypeExpr::Str),
                    (TypeExpr::Rint(_), TypeExpr::Str) => Ok(TypeExpr::Str),
                    (TypeExpr::Real(_), TypeExpr::Str) => Ok(TypeExpr::Str),
                    (TypeExpr::Bool, TypeExpr::Str) => Ok(TypeExpr::Str),
                    (TypeExpr::Real(_), TypeExpr::Int(_)) => Ok(TypeExpr::Int(0)),
                    (TypeExpr::Rint(_), TypeExpr::Int(_)) => Ok(TypeExpr::Int(0)),
                    (TypeExpr::Int(_), TypeExpr::Int(_)) => Ok(TypeExpr::Int(0)),
                    (TypeExpr::Str, TypeExpr::Named(n)) if n == "symbol" => Ok(TypeExpr::Symbol),
                    _ => self.fail(ErrorKind::TypeError, expr.span,
                        format!("Cannot cast {} to {}", inner_ty, target_ty)),
                }
            }
            Expr::Match(scrutinee, arms) => {
                let scrutinee_ty = self.check_expr(scrutinee)?;
                self.check_match_exhaustive(&scrutinee_ty, arms, expr.span)?;
                let mut result_ty = TypeExpr::Infer;
                for (i, arm) in arms.iter().enumerate() {
                    if let Some(guard) = &arm.guard {
                        self.check_expr(guard)?;
                    }
                    let arm_ty = self.check_expr(&arm.body)?;
                    if i == 0 {
                        result_ty = arm_ty;
                    }
                }
                Ok(result_ty)
            }
            _ => Ok(TypeExpr::Infer),
        }
    }

    fn check_struct_fields_with_type_args(&mut self, struct_name: &str, fields: &[(String, ExprNode)], resolved_annotation: &TypeExpr, span: crate::diagnostics::span::Span) -> Result<()> {
        let type_args = match resolved_annotation {
            TypeExpr::Generic(base_name, args) if base_name == struct_name => {
                let mut map = HashMap::new();
                if let Some(struct_def) = self.env.get_struct(struct_name) {
                    for (g, a) in struct_def.generics.iter().zip(args.iter()) {
                        map.insert(g.clone(), a.clone());
                    }
                }
                map
            }
            _ => HashMap::new(),
        };
        if type_args.is_empty() { return Ok(()); }
        let struct_def_opt = self.env.get_struct(struct_name).cloned();
        if let Some(struct_def) = struct_def_opt {
            for (field_name, _field_expr) in fields {
                if let Some(field_def) = struct_def.fields.iter().find(|f| f.name == *field_name) {
                    let expected_ty = substitute_type(&field_def.type_expr.value, &type_args);
                    let field_expr_ty = self.check_expr(_field_expr)?;
                    if !types_compatible(&expected_ty, &field_expr_ty) {
                        return self.fail(ErrorKind::TypeError, span,
                            format!("Type mismatch: field '{}' of struct '{}' expects {} but got {}", field_name, struct_name, expected_ty, field_expr_ty));
                    }
                }
            }
        }
        Ok(())
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

    fn check_match_exhaustive(&self, scrutinee_ty: &TypeExpr, arms: &[MatchArm], span: crate::diagnostics::span::Span) -> Result<()> {
        let has_catch_all = arms.iter().any(|arm| matches!(&arm.pattern, Pattern::Ignore | Pattern::Ident(_) | Pattern::Rest(_)));
        if has_catch_all {
            return Ok(());
        }
        match scrutinee_ty {
            TypeExpr::Bool => {
                let has_true = arms.iter().any(|arm| matches!(&arm.pattern, Pattern::LitBool(true)));
                let has_false = arms.iter().any(|arm| matches!(&arm.pattern, Pattern::LitBool(false)));
                if !has_true {
                    return Err(error::err(ErrorKind::TypeError, span, format!("Non-exhaustive match: missing arm for `true`")));
                }
                if !has_false {
                    return Err(error::err(ErrorKind::TypeError, span, format!("Non-exhaustive match: missing arm for `false`")));
                }
            }
            TypeExpr::Named(name) => {
                if let Some(variants) = self.env.get_enum(name) {
                    for variant in variants {
                        let covered = arms.iter().any(|arm| matches!(&arm.pattern, Pattern::Variant(vn, _) if vn == &variant.name));
                        if !covered {
                            return Err(error::err(ErrorKind::TypeError, span,
                                format!("Non-exhaustive match: missing arm for `{}::{}`", name, variant.name)));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn item_name(kind: &ItemKind) -> Option<String> {
    match kind {
        ItemKind::Fn { name, .. } | ItemKind::Struct { name, .. }
        | ItemKind::Class { name, .. } | ItemKind::Interface { name, .. }
        | ItemKind::Union { name, .. } | ItemKind::Enum { name, .. } | ItemKind::Object { name, .. } | ItemKind::TypeAlias { name, .. }
        | ItemKind::Const { name, .. } => Some(name.clone()),
    }
}
