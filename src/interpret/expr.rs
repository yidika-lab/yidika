use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::Arc;
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::syntax::ast::*;
use crate::interpret::builtins;
use super::value::{Value, EvalResult, is_truthy, cmp_binop};
use super::env::Interpreter;

impl Interpreter {
    fn eval_binop(&self, lv: Value, op: BinOp, rv: Value, span: Span) -> Result<Value> {
        match op {
            BinOp::Add => {
                let to_c64 = |v: &Value| -> Option<(f64, f64)> {
                    match v { Value::Int(i) => Some((*i as f64, 0.0)), Value::Real(r) => Some((*r, 0.0)), Value::Complex(r, i) => Some((*r, *i)), _ => None }
                };
                match (&lv, &rv) {
                    (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
                    (Value::Real(a), Value::Real(b)) => Ok(Value::Real(a + b)),
                    (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{}{}", a, b))),
                    _ => {
                        if let (Some((r1, i1)), Some((r2, i2))) = (to_c64(&lv), to_c64(&rv)) {
                            Ok(Value::Complex(r1 + r2, i1 + i2))
                        } else {
                            Err(self.err(span, "Type mismatch in addition"))
                        }
                    }
                }
            }
            BinOp::Sub | BinOp::Mul | BinOp::Div => {
                let to_c64 = |v: &Value| -> Option<(f64, f64)> {
                    match v { Value::Int(i) => Some((*i as f64, 0.0)), Value::Real(r) => Some((*r, 0.0)), Value::Complex(r, i) => Some((*r, *i)), _ => None }
                };
                match (&lv, &rv) {
                    (Value::Int(a), Value::Int(b)) => {
                        let v = match op {
                            BinOp::Sub => Value::Int(a - b),
                            BinOp::Mul => Value::Int(a * b),
                            BinOp::Div => Value::Int(a / b),
                            _ => unreachable!(),
                        };
                        Ok(v)
                    }
                    (Value::Real(a), Value::Real(b)) => {
                        let v = match op {
                            BinOp::Sub => Value::Real(a - b),
                            BinOp::Mul => Value::Real(a * b),
                            BinOp::Div => Value::Real(a / b),
                            _ => unreachable!(),
                        };
                        Ok(v)
                    }
                    _ => {
                        if let (Some((r1, i1)), Some((r2, i2))) = (to_c64(&lv), to_c64(&rv)) {
                            let (r, i) = match op {
                                BinOp::Sub => (r1 - r2, i1 - i2),
                                BinOp::Mul => (r1 * r2 - i1 * i2, r1 * i2 + i1 * r2),
                                BinOp::Div => {
                                    let d = r2 * r2 + i2 * i2;
                                    if d == 0.0 { return Err(self.err(span, "Division by zero in complex arithmetic")); }
                                    ((r1 * r2 + i1 * i2) / d, (i1 * r2 - r1 * i2) / d)
                                }
                                _ => unreachable!(),
                            };
                            Ok(Value::Complex(r, i))
                        } else {
                            Err(self.err(span, "Type mismatch in arithmetic"))
                        }
                    }
                }
            }
            BinOp::Eq => Ok(Value::Bool(lv == rv)),
            BinOp::Ne => Ok(Value::Bool(lv != rv)),
            BinOp::Lt => cmp_binop(&lv, &rv, |a, b| a < b, span),
            BinOp::Gt => cmp_binop(&lv, &rv, |a, b| a > b, span),
            BinOp::Le => cmp_binop(&lv, &rv, |a, b| a <= b, span),
            BinOp::Ge => cmp_binop(&lv, &rv, |a, b| a >= b, span),
            BinOp::And => {
                let b = is_truthy(&lv) && is_truthy(&rv);
                Ok(Value::Bool(b))
            }
            BinOp::Or => {
                let b = is_truthy(&lv) || is_truthy(&rv);
                Ok(Value::Bool(b))
            }
            BinOp::Assign => Ok(rv),
        }
    }

    fn resolve_mod_call(&self, obj: &ExprNode, field: &str) -> Option<(String, String)> {
        match &obj.value {
            Expr::Ident(name) if self.builtin_modules.contains_key(name) => {
                Some((name.clone(), field.to_string()))
            }
            Expr::Field(inner, sub) => {
                match &inner.value {
                    Expr::Ident(name) if name == "std" && self.std_imported => {
                        Some((sub.clone(), field.to_string()))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn field_access_path(expr: &ExprNode) -> Vec<(String, String)> {
        match &expr.value {
            Expr::Field(parent, field_name) => {
                let mut path = Self::field_access_path(parent);
                if let Expr::Ident(var) = &parent.value {
                    path.insert(0, (var.clone(), field_name.clone()));
                } else {
                    path.push((field_name.clone(), String::new()));
                }
                path
            }
            _ => Vec::new(),
        }
    }

    fn set_field_path(&mut self, path: &[(String, String)], new_val: Value) {
        if path.is_empty() { return; }
        let (var_name, fname) = &path[0];
        if let Ok(mut parent) = self.get_var(var_name) {
            match &mut parent {
                Value::Struct(_, fields) => { fields.insert(fname.clone(), new_val); }
                Value::Instance(cls_name, fields) => {
                    if let Some(cls) = self.classes.get(cls_name) {
                        if let Some(pos) = cls.fields.iter().position(|f| f == fname) {
                            fields[pos] = new_val;
                        }
                    }
                }
                _ => {}
            }
            self.set_var(var_name, parent).ok();
        }
    }

    fn cast_value(&self, val: Value, target_type: &TypeNode, span: Span) -> Result<Value> {
        match (&val, &target_type.value) {
            (Value::Int(i), TypeExpr::Str) => Ok(Value::Str(i.to_string())),
            (Value::Int(i), TypeExpr::Int(_)) => Ok(Value::Int(*i)),
            (Value::Int(i), TypeExpr::Real(_)) => Ok(Value::Real(*i as f64)),
            (Value::Real(f), TypeExpr::Str) => Ok(Value::Str(f.to_string())),
            (Value::Real(f), TypeExpr::Int(_)) => Ok(Value::Int(*f as i64)),
            (Value::Bool(b), TypeExpr::Str) => Ok(Value::Str(b.to_string())),
            (Value::Complex(r, i), TypeExpr::Str) => {
                Ok(Value::Str(format!("{}{:+}i", r, i)))
            }
            (_, _) => Err(self.err(span, format!("Cannot cast {} to {:?}", val, target_type.value))),
        }
    }

    pub fn eval_expr(&mut self, expr: &ExprNode) -> Result<EvalResult> {
        macro_rules! eval {
            ($e:expr) => {
                match self.eval_expr($e)? {
                    EvalResult::Value(v) => v,
                    EvalResult::Return(v) => return Ok(EvalResult::Return(v)),
                }
            };
        }
        match &expr.value {
            Expr::LitInt(i) => Ok(EvalResult::Value(Value::Int(*i))),
            Expr::LitHex(h) => Ok(EvalResult::Value(Value::Int(*h))),
            Expr::LitReal(f) => Ok(EvalResult::Value(Value::Real(*f))),
            Expr::LitStr(s) => Ok(EvalResult::Value(Value::Str(s.clone()))),
            Expr::LitChar(c) => Ok(EvalResult::Value(Value::Char(*c))),
            Expr::LitBool(b) => Ok(EvalResult::Value(Value::Bool(*b))),
            Expr::LitNull => Ok(EvalResult::Value(Value::Null)),
            Expr::LitNone => Ok(EvalResult::Value(Value::None_)),
            Expr::LitComplex(r, im) => {
                let rv = eval!(r);
                let iv = eval!(im);
                let rf = match rv { Value::Int(i) => i as f64, Value::Real(f) => f, _ => return Err(self.err(expr.span, "Complex real part must be a number")) };
                let ri = match iv { Value::Int(i) => i as f64, Value::Real(f) => f, _ => return Err(self.err(expr.span, "Complex imag part must be a number")) };
                Ok(EvalResult::Value(Value::Complex(rf, ri)))
            }
            Expr::LitSymbol(s) => Ok(EvalResult::Value(Value::Str(format!(":{}", s)))),
            Expr::Ident(name) => {
                Ok(EvalResult::Value(self.get_var(name)?))
            }
            Expr::BinOp(l, op, r) => {
                let rv = eval!(r);
                if *op == BinOp::Assign {
                    if let Expr::Field(obj, field_name) = &l.value {
                        if let Expr::Ident(obj_name) = &obj.value {
                            if let Ok(val) = self.get_var(obj_name) {
                                if let Value::Instance(cls_name, mut cls_fields) = val {
                                    if let Some(cls) = self.classes.get(&cls_name) {
                                        if let Some(idx) = cls.fields.iter().position(|f| f == field_name) {
                                            cls_fields[idx] = rv.clone();
                                            self.set_var(obj_name, Value::Instance(cls_name, cls_fields))?;
                                        }
                                    }
                                }
                            }
                        }
                        return Ok(EvalResult::Value(rv));
                    }
                    return Ok(EvalResult::Value(rv));
                }
                let lv = eval!(l);
                Ok(EvalResult::Value(self.eval_binop(lv, *op, rv, expr.span)?))
            }
            Expr::UnOp(op, inner) => {
                let v = eval!(inner);
                match op {
                    UnOp::Neg => match v {
                        Value::Int(i) => Ok(EvalResult::Value(Value::Int(-i))),
                        Value::Real(r) => Ok(EvalResult::Value(Value::Real(-r))),
                        Value::Complex(r, i) => Ok(EvalResult::Value(Value::Complex(-r, -i))),
                        _ => Err(self.err(expr.span, "Cannot negate")),
                    },
                    UnOp::Not => match v {
                        Value::Bool(b) => Ok(EvalResult::Value(Value::Bool(!b))),
                        _ => Err(self.err(expr.span, "Cannot boolean not")),
                    },
                }
            }
            Expr::Call(callee, args) => {
                // Early check for Server method calls (before arg evaluation)
                if let Expr::Field(obj, field) = &callee.value {
                    if let Ok(EvalResult::Value(obj_val)) = self.eval_expr(obj) {
                        if let Value::Instance(cls_name, _) = &obj_val {
                            if cls_name == "Server" {
                                let mut processed_args = Vec::new();
                                for a in args.iter() {
                                    let val = match &a.value {
                                        Expr::Ident(fn_name) if self.functions.contains_key(fn_name) => {
                                            Value::Str(fn_name.clone())
                                        }
                                        _ => match self.eval_expr(a)? {
                                            EvalResult::Value(v) => v,
                                            EvalResult::Return(v) => v,
                                        },
                                    };
                                    processed_args.push(val);
                                }
                                let result = crate::netlib::call_net_method(field, args, &processed_args, obj_val, self, expr.span)?;
                                return Ok(EvalResult::Value(result));
                            }
                        }
                    }
                }
                let arg_vals: Result<Vec<Value>> = args.iter().map(|a| match self.eval_expr(a)? {
                    EvalResult::Value(v) => Ok(v),
                    EvalResult::Return(v) => Ok(v),
                }).collect();
                let arg_vals = arg_vals?;
                match &callee.value {
                    Expr::Ident(name) => {
                        match name.as_str() {
                            "print" | "println" => {
                                let parts: Vec<String> = arg_vals.iter().map(|v| v.to_string()).collect();
                                let s = if parts.is_empty() { "\n".into() } else { parts.join(" ") + "\n" };
                                if self.tui_mode {
                                    print!("{}", s);
                                    std::io::stdout().flush().ok();
                                } else {
                                    self.output.push_str(&s);
                                }
                                Ok(EvalResult::Value(Value::None_))
                            }
                            "len" => {
                                let v = arg_vals.into_iter().next()
                                    .ok_or_else(|| self.err(expr.span, "len() requires 1 argument"))?;
                                match v {
                                    Value::Str(s) => Ok(EvalResult::Value(Value::Int(s.chars().count() as i64))),
                                    Value::List(l) => Ok(EvalResult::Value(Value::Int(l.len() as i64))),
                                    _ => Err(self.err(expr.span, "len() requires a string or list")),
                                }
                            }
                            "str" => {
                                let v = arg_vals.into_iter().next()
                                    .ok_or_else(|| self.err(expr.span, "str() requires 1 argument"))?;
                                Ok(EvalResult::Value(Value::Str(v.to_string())))
                            }
                            "input" => {
                                if let Some(prompt) = arg_vals.first() {
                                    let p = prompt.to_string();
                                    if self.tui_mode {
                                        print!("{}", p);
                                        std::io::stdout().flush().ok();
                                    } else {
                                        self.output.push_str(&p);
                                    }
                                }
                                let mut line = String::new();
                                std::io::stdin().read_line(&mut line)
                                    .map_err(|e| self.err(expr.span, format!("input error: {}", e)))?;
                                if line.ends_with('\n') { line.pop(); }
                                if line.ends_with('\r') { line.pop(); }
                                Ok(EvalResult::Value(Value::Str(line)))
                            }
                            _ => {
                                if let Some(module) = self.builtin_funcs.get(name).cloned() {
                                    let span = expr.span;
                                    match module.as_str() {
                                        "math" => Ok(EvalResult::Value(builtins::call_math(name, arg_vals, span)?)),
                                        "time" => Ok(EvalResult::Value(builtins::call_time(name, arg_vals, span)?)),
                                        "net" => Ok(EvalResult::Value(crate::netlib::call_net(name, arg_vals, self, span)?)),
                                        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown function '{}'", name))),
                                    }
                                } else {
                                    let fndef = self.functions.get(name).cloned();
                                    match fndef {
                                        Some(fn_def) => {
                                            self.push_frame();
                                            for (i, param) in fn_def.params.iter().enumerate() {
                                                let val = arg_vals.get(i).cloned().unwrap_or(Value::None_);
                                                self.frames.last_mut().unwrap().insert(param.name.clone(), val);
                                            }
                                            let result = self.run_fn_body(&fn_def)?;
                                            self.pop_frame();
                                            Ok(EvalResult::Value(result.unwrap_or(Value::None_)))
                                        }
                                        None => Err(self.err(expr.span, format!("Unknown function '{}'", name))),
                                    }
                                }
                            }
                        }
                    }
                    Expr::Field(obj, field) => {
                        match self.resolve_mod_call(obj, field) {
                            Some((name, func)) => {
                                let mod_id = self.builtin_modules.get(&name).cloned().unwrap_or_else(|| name.clone());
                                match mod_id.as_str() {
                                    "io" | "fs" | "sys" => {
                                        match func.as_str() {
                                            "read" | "write" | "append" | "remove" | "exists" | "list" | "is_dir" | "is_file" => Ok(EvalResult::Value(builtins::call_fs(&func, arg_vals, expr.span)?)),
                                            _ => Ok(EvalResult::Value(builtins::call_sys(&func, arg_vals, expr.span)?)),
                                        }
                                    }
                                    "json" => Ok(EvalResult::Value(builtins::call_json(&func, arg_vals, expr.span)?)),
                                    "datetime" => Ok(EvalResult::Value(builtins::call_datetime(&func, arg_vals, expr.span)?)),
                                    "path" => Ok(EvalResult::Value(builtins::call_path_module(&func, arg_vals, expr.span)?)),
                                    "base64" => Ok(EvalResult::Value(builtins::call_base64(&func, arg_vals, expr.span)?)),
                                    "re" => Ok(EvalResult::Value(builtins::call_re(&func, arg_vals, expr.span)?)),
                                    "net" => Ok(EvalResult::Value(crate::netlib::call_net(&func, arg_vals, self, expr.span)?)),
                                    "math" => Ok(EvalResult::Value(builtins::call_math(&func, arg_vals, expr.span)?)),
                                    "time" => Ok(EvalResult::Value(builtins::call_time(&func, arg_vals, expr.span)?)),
                                    ffi if ffi.starts_with("rust:") || ffi.starts_with("c++:") => {
                                        let ffi_path = &ffi[ffi.find(':').unwrap() + 1..];
                                        Ok(EvalResult::Value(builtins::call_ffi(&name, &func, arg_vals, expr.span, ffi_path, &self.ffi_libs)?))
                                    }
                                    _ => Err(error::err(ErrorKind::Runtime, expr.span, format!("Unknown module '{}'", name))),
                                }
                            }
                            None => {
                                let var_name = match &obj.value { Expr::Ident(n) => Some(n.clone()), _ => None };
                                let field_path = if var_name.is_none() {
                                    Self::field_access_path(obj)
                                } else {
                                    Vec::new()
                                };
                                let mut receiver = eval!(obj);
                                let is_super = matches!(&obj.value, Expr::Ident(n) if n == "super");
                                if let Value::Instance(cls_name, _) = &receiver {
                                    let method = if is_super {
                                        self.classes.get(cls_name).and_then(|cls| {
                                            cls.extends.as_ref().and_then(|parent| {
                                                self.find_class_method(parent, field)
                                            })
                                        })
                                    } else {
                                        self.find_class_method(cls_name, field)
                                            .or_else(|| self.objects.get(cls_name).and_then(|obj| obj.methods.get(field).cloned()))
                                    };
                                    match method {
                                        Some(fn_def) => {
                                            self.push_frame();
                                            let cls_receiver = receiver;
                                            self.frames.last_mut().unwrap().insert("self".into(), cls_receiver.clone());
                                            self.frames.last_mut().unwrap().insert("super".into(), cls_receiver);
                                            let start_idx = if fn_def.params.first().map(|p| p.name.as_str()) == Some("self") { 1 } else { 0 };
                                            for (i, param) in fn_def.params.iter().enumerate().skip(start_idx) {
                                                let val = arg_vals.get(i - start_idx).cloned().unwrap_or(Value::None_);
                                                self.frames.last_mut().unwrap().insert(param.name.clone(), val);
                                            }
                                            let result = self.run_fn_body(&fn_def)?;
                                            let mutated = self.frames.last().and_then(|f| f.get("self")).cloned();
                                            self.pop_frame();
                                            if let Some(ref n) = var_name {
                                                if let Some(m) = mutated { self.set_var(n, m).ok(); }
                                            } else if let Some(m) = mutated {
                                                self.set_field_path(&field_path, m);
                                            }
                                            return Ok(EvalResult::Value(result.unwrap_or(Value::None_)));
                                        }
                                        None => {
                                            if let Value::Instance(cls_name, _) = &receiver {
                                                if let Some(cls) = self.classes.get(cls_name) {
                                                    if cls.is_data {
                                                        match field.as_str() {
                                                            "equals" if arg_vals.len() == 1 => {
                                                                let eq = match (&receiver, &arg_vals[0]) {
                                                                    (Value::Instance(_, a), Value::Instance(_, b)) => a == b,
                                                                    _ => false,
                                                                };
                                                                return Ok(EvalResult::Value(Value::Bool(eq)));
                                                            }
                                                            "toString" => {
                                                                let parts: Vec<String> = match &receiver {
                                                                    Value::Instance(_, fields) => cls.fields.iter().zip(fields.iter())
                                                                        .map(|(name, val)| format!("{}={}", name, val)).collect(),
                                                                    _ => vec![],
                                                                };
                                                                return Ok(EvalResult::Value(Value::Str(format!("{}({})", cls_name, parts.join(", ")))));
                                                            }
                                                            "copy" => {
                                                                return Ok(EvalResult::Value(receiver.clone()));
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                match field.as_str() {
                                    "push" => match &mut receiver {
                                        Value::List(items) => {
                                            if let Some(val) = arg_vals.into_iter().next() {
                                                items.push(val);
                                            }
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(EvalResult::Value(Value::None_))
                                        }
                                        _ => Err(self.err(expr.span, "push requires a list")),
                                    },
                                    "pop" => match &mut receiver {
                                        Value::List(items) => {
                                            let result = items.pop()
                                                .ok_or_else(|| self.err(expr.span, "pop from empty list"))?;
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(EvalResult::Value(result))
                                        }
                                        _ => Err(self.err(expr.span, "pop requires a list")),
                                    },
                                    "sort" => match &mut receiver {
                                        Value::List(items) => {
                                            items.sort_by(|a, b| a.to_string().cmp(&b.to_string()));
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(EvalResult::Value(Value::None_))
                                        }
                                        _ => Err(self.err(expr.span, "sort requires a list")),
                                    },
                                    "reverse" => match &mut receiver {
                                        Value::List(items) => {
                                            items.reverse();
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(EvalResult::Value(Value::None_))
                                        }
                                        _ => Err(self.err(expr.span, "reverse requires a list")),
                                    },
                                    "insert" => match &mut receiver {
                                        Value::List(items) => {
                                            let mut it = arg_vals.into_iter();
                                            let idx = it.next().ok_or_else(|| self.err(expr.span, "insert requires 2 arguments"))?;
                                            let val = it.next().ok_or_else(|| self.err(expr.span, "insert requires 2 arguments"))?;
                                            let i = match idx { Value::Int(i) => i as usize, _ => return Err(self.err(expr.span, "insert requires an integer index")) };
                                            items.insert(i, val);
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(EvalResult::Value(Value::None_))
                                        }
                                        _ => Err(self.err(expr.span, "insert requires a list")),
                                    },
                                    "remove" => match &mut receiver {
                                        Value::List(items) => {
                                            let idx = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "remove requires 1 argument"))?;
                                            let i = match idx { Value::Int(i) => {
                                                if i < 0 { (items.len() as i64 + i) as usize } else { i as usize }
                                            }, _ => return Err(self.err(expr.span, "remove requires an integer index")) };
                                            if i >= items.len() { return Err(self.err(expr.span, "remove index out of bounds")); }
                                            items.remove(i);
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(EvalResult::Value(Value::None_))
                                        }
                                        _ => Err(self.err(expr.span, "remove requires a list")),
                                    },
                                    "clear" => match &mut receiver {
                                        Value::List(items) => {
                                            items.clear();
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(EvalResult::Value(Value::None_))
                                        }
                                        _ => Err(self.err(expr.span, "clear requires a list")),
                                    },
                                    "len" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Int(s.chars().count() as i64))),
                                        Value::List(l) => Ok(EvalResult::Value(Value::Int(l.len() as i64))),
                                        Value::Map(m) => Ok(EvalResult::Value(Value::Int(m.len() as i64))),
                                        _ => Err(self.err(expr.span, "len requires a string, list, or map")),
                                    },
                                    "split" => match &receiver {
                                        Value::Str(s) => {
                                            let delim = match arg_vals.into_iter().next() {
                                                Some(Value::Str(d)) => d,
                                                Some(_) => return Err(self.err(expr.span, "split requires a string delimiter")),
                                                None => "".into(),
                                            };
                                            Ok(EvalResult::Value(Value::List(s.split(&delim).map(|p| Value::Str(p.to_string())).collect())))
                                        }
                                        _ => Err(self.err(expr.span, "split requires a string")),
                                    },
                                    "contains" => match &receiver {
                                        Value::Str(s) => {
                                            let sub = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "contains requires 1 argument"))?;
                                            match sub {
                                                Value::Str(p) => Ok(EvalResult::Value(Value::Bool(s.contains(&p)))),
                                                _ => Err(self.err(expr.span, "contains requires a string argument")),
                                            }
                                        }
                                        _ => Err(self.err(expr.span, "contains requires a string")),
                                    },
                                    "trim" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Str(s.trim().to_string()))),
                                        _ => Err(self.err(expr.span, "trim requires a string")),
                                    },
                                    "toUpper" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Str(s.to_uppercase()))),
                                        _ => Err(self.err(expr.span, "toUpper requires a string")),
                                    },
                                    "toLower" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Str(s.to_lowercase()))),
                                        _ => Err(self.err(expr.span, "toLower requires a string")),
                                    },
                                    "replace" => match &receiver {
                                        Value::Str(s) => {
                                            let mut it = arg_vals.into_iter();
                                            let from = it.next().ok_or_else(|| self.err(expr.span, "replace requires 2 arguments"))?;
                                            let to = it.next().ok_or_else(|| self.err(expr.span, "replace requires 2 arguments"))?;
                                            match (from, to) {
                                                (Value::Str(f), Value::Str(t)) => Ok(EvalResult::Value(Value::Str(s.replace(&f, &t)))),
                                                _ => Err(self.err(expr.span, "replace requires string arguments")),
                                            }
                                        }
                                        _ => Err(self.err(expr.span, "replace requires a string")),
                                    },
                                    "repeat" => match &receiver {
                                        Value::Str(s) => {
                                            let n = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "repeat requires 1 argument"))?;
                                            match n {
                                                Value::Int(i) => Ok(EvalResult::Value(Value::Str(s.repeat(i as usize)))),
                                                _ => Err(self.err(expr.span, "repeat requires an integer")),
                                            }
                                        }
                                        _ => Err(self.err(expr.span, "repeat requires a string")),
                                    },
                                    "startsWith" => match &receiver {
                                        Value::Str(s) => {
                                            let sub = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "startsWith requires 1 argument"))?;
                                            match sub {
                                                Value::Str(p) => Ok(EvalResult::Value(Value::Bool(s.starts_with(&p)))),
                                                _ => Err(self.err(expr.span, "startsWith requires a string argument")),
                                            }
                                        }
                                        _ => Err(self.err(expr.span, "startsWith requires a string")),
                                    },
                                    "endsWith" => match &receiver {
                                        Value::Str(s) => {
                                            let sub = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "endsWith requires 1 argument"))?;
                                            match sub {
                                                Value::Str(p) => Ok(EvalResult::Value(Value::Bool(s.ends_with(&p)))),
                                                _ => Err(self.err(expr.span, "endsWith requires a string argument")),
                                            }
                                        }
                                        _ => Err(self.err(expr.span, "endsWith requires a string")),
                                    },
                                    "isalpha" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Bool(s.chars().all(|c| c.is_alphabetic())))),
                                        _ => Err(self.err(expr.span, "isalpha requires a string")),
                                    },
                                    "isdigit" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Bool(s.chars().all(|c| c.is_ascii_digit())))),
                                        _ => Err(self.err(expr.span, "isdigit requires a string")),
                                    },
                                    "isalnum" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Bool(s.chars().all(|c| c.is_alphanumeric())))),
                                        _ => Err(self.err(expr.span, "isalnum requires a string")),
                                    },
                                    "isupper" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Bool(s.chars().any(|c| c.is_uppercase())))),
                                        _ => Err(self.err(expr.span, "isupper requires a string")),
                                    },
                                    "islower" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Bool(s.chars().any(|c| c.is_lowercase())))),
                                        _ => Err(self.err(expr.span, "islower requires a string")),
                                    },
                                    "isspace" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Bool(s.chars().all(|c| c.is_whitespace())))),
                                        _ => Err(self.err(expr.span, "isspace requires a string")),
                                    },
                                    "strip" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Str(s.trim().to_string()))),
                                        _ => Err(self.err(expr.span, "strip requires a string")),
                                    },
                                    "lstrip" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Str(s.trim_start().to_string()))),
                                        _ => Err(self.err(expr.span, "lstrip requires a string")),
                                    },
                                    "rstrip" => match &receiver {
                                        Value::Str(s) => Ok(EvalResult::Value(Value::Str(s.trim_end().to_string()))),
                                        _ => Err(self.err(expr.span, "rstrip requires a string")),
                                    },
                                    "capitalize" => match &receiver {
                                        Value::Str(s) => {
                                            let mut chars = s.chars();
                                            let first = chars.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                                            let rest = chars.as_str().to_lowercase();
                                            Ok(EvalResult::Value(Value::Str(first + &rest)))
                                        }
                                        _ => Err(self.err(expr.span, "capitalize requires a string")),
                                    },
                                    "swapcase" => match &receiver {
                                        Value::Str(s) => {
                                            Ok(EvalResult::Value(Value::Str(s.chars().map(|c| {
                                                if c.is_uppercase() { c.to_lowercase().next().unwrap_or(c) }
                                                else { c.to_uppercase().next().unwrap_or(c) }
                                            }).collect())))
                                        }
                                        _ => Err(self.err(expr.span, "swapcase requires a string")),
                                    },
                                    "count" => match &receiver {
                                        Value::Str(s) => {
                                            let sub = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "count requires 1 argument"))?;
                                            match sub {
                                                Value::Str(p) => Ok(EvalResult::Value(Value::Int(s.matches(&p).count() as i64))),
                                                _ => Err(self.err(expr.span, "count requires a string argument")),
                                            }
                                        }
                                        _ => Err(self.err(expr.span, "count requires a string")),
                                    },
                                    "index" => match &receiver {
                                        Value::Str(s) => {
                                            let sub = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "index requires 1 argument"))?;
                                            match sub {
                                                Value::Str(p) => s.find(&p).map(|i| EvalResult::Value(Value::Int(i as i64)))
                                                    .ok_or_else(|| self.err(expr.span, "substring not found")),
                                                _ => Err(self.err(expr.span, "index requires a string argument")),
                                            }
                                        }
                                        _ => Err(self.err(expr.span, "index requires a string")),
                                    },
                                    "mod" => match &receiver {
                                        Value::Complex(r, i) => Ok(EvalResult::Value(Value::Real((r * r + i * i).sqrt()))),
                                        _ => Err(self.err(expr.span, "mod requires a complex")),
                                    },
                                    "arg" => match &receiver {
                                        Value::Complex(r, i) => Ok(EvalResult::Value(Value::Real(i.atan2(*r)))),
                                        _ => Err(self.err(expr.span, "arg requires a complex")),
                                    },
                                    "conj" => match &receiver {
                                        Value::Complex(r, i) => Ok(EvalResult::Value(Value::Complex(*r, -i))),
                                        _ => Err(self.err(expr.span, "conj requires a complex")),
                                    },
                                    "norm" => match &receiver {
                                        Value::Complex(r, i) => Ok(EvalResult::Value(Value::Real(r * r + i * i))),
                                        _ => Err(self.err(expr.span, "norm requires a complex")),
                                    },
                                    "includes" => match &receiver {
                                        Value::List(items) => {
                                            let target = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "includes requires 1 argument"))?;
                                            Ok(EvalResult::Value(Value::Bool(items.contains(&target))))
                                        }
                                        _ => Err(self.err(expr.span, "includes requires a list")),
                                    },
                                    "join" => match &receiver {
                                        Value::List(items) => {
                                            let sep = arg_vals.into_iter().next()
                                                .map(|v| match v { Value::Str(s) => s, _ => ",".into() })
                                                .unwrap_or_else(|| ",".into());
                                            Ok(EvalResult::Value(Value::Str(items.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(&sep))))
                                        }
                                        _ => Err(self.err(expr.span, "join requires a list")),
                                    },
                                    "slice" => match &receiver {
                                        Value::List(items) => {
                                            let start = arg_vals.get(0).and_then(|v| if let Value::Int(i) = v { Some(*i as usize) } else { None }).unwrap_or(0);
                                            let end = arg_vals.get(1).and_then(|v| if let Value::Int(i) = v { Some(*i as usize) } else { None }).unwrap_or(items.len());
                                            let end = end.min(items.len());
                                            Ok(EvalResult::Value(Value::List(items[start..end].to_vec())))
                                        }
                                        Value::Str(s) => {
                                            let chars: Vec<char> = s.chars().collect();
                                            let start = arg_vals.get(0).and_then(|v| if let Value::Int(i) = v { Some(*i as usize) } else { None }).unwrap_or(0);
                                            let end = arg_vals.get(1).and_then(|v| if let Value::Int(i) = v { Some(*i as usize) } else { None }).unwrap_or(chars.len());
                                            let end = end.min(chars.len());
                                            Ok(EvalResult::Value(Value::Str(chars[start..end].iter().collect())))
                                        }
                                        _ => Err(self.err(expr.span, "slice requires a list or string")),
                                    },
                                    "shift" => match &mut receiver {
                                        Value::List(items) => {
                                            if items.is_empty() { return Err(self.err(expr.span, "shift from empty list")); }
                                            let result = items.remove(0);
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(EvalResult::Value(result))
                                        }
                                        _ => Err(self.err(expr.span, "shift requires a list")),
                                    },
                                    "unshift" => match &mut receiver {
                                        Value::List(items) => {
                                            let val = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "unshift requires 1 argument"))?;
                                            items.insert(0, val);
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(EvalResult::Value(Value::None_))
                                        }
                                        _ => Err(self.err(expr.span, "unshift requires a list")),
                                    },
                                    "keys" => match &receiver {
                                        Value::Map(m) => Ok(EvalResult::Value(Value::List(m.keys().map(|k| Value::Str(k.clone())).collect()))),
                                        _ => Err(self.err(expr.span, "keys requires a map")),
                                    },
                                    "values" => match &receiver {
                                        Value::Map(m) => Ok(EvalResult::Value(Value::List(m.values().cloned().collect()))),
                                        _ => Err(self.err(expr.span, "values requires a map")),
                                    },
                                    "entries" => match &receiver {
                                        Value::Map(m) => Ok(EvalResult::Value(Value::List(m.iter().map(|(k, v)| {
                                            Value::Tuple(vec![Value::Str(k.clone()), v.clone()])
                                        }).collect()))),
                                        _ => Err(self.err(expr.span, "entries requires a map")),
                                    },
                                    "has" => match &receiver {
                                        Value::Map(m) => {
                                            let key = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "has requires 1 argument"))?;
                                            match key {
                                                Value::Str(k) => Ok(EvalResult::Value(Value::Bool(m.contains_key(&k)))),
                                                _ => Err(self.err(expr.span, "has requires a string key")),
                                            }
                                        }
                                        _ => Err(self.err(expr.span, "has requires a map")),
                                    },
                                    "get" => match &receiver {
                                        Value::Map(m) => {
                                            let key = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "get requires 1 argument"))?;
                                            match key {
                                                Value::Str(k) => Ok(EvalResult::Value(m.get(&k).cloned().unwrap_or(Value::None_))),
                                                _ => Err(self.err(expr.span, "get requires a string key")),
                                            }
                                        }
                                        _ => Err(self.err(expr.span, "get requires a map")),
                                    },
                                    "set" => match &mut receiver {
                                        Value::Map(m) => {
                                            let mut args = arg_vals.into_iter();
                                            let key = args.next().ok_or_else(|| self.err(expr.span, "set requires 2 arguments"))?;
                                            let val = args.next().ok_or_else(|| self.err(expr.span, "set requires 2 arguments"))?;
                                            match key {
                                                Value::Str(k) => { m.insert(k, val); }
                                                _ => return Err(self.err(expr.span, "set key must be a string")),
                                            }
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(EvalResult::Value(Value::None_))
                                        }
                                        _ => Err(self.err(expr.span, "set requires a map")),
                                    },
                                    "delete" => match &mut receiver {
                                        Value::Map(m) => {
                                            let key = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "delete requires 1 argument"))?;
                                            match key {
                                                Value::Str(k) => { m.remove(&k); }
                                                _ => return Err(self.err(expr.span, "delete key must be a string")),
                                            }
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(EvalResult::Value(Value::None_))
                                        }
                                        _ => Err(self.err(expr.span, "delete requires a map")),
                                    },
                                    _ => Err(self.err(expr.span, format!("Unknown method '{}'", field))),
                                }
                            }
                        }
                    }
                    _ => Err(self.err(expr.span, "Invalid callee expression")),
                }
            }
            Expr::Block(stmts) => {
                self.push_frame();
                let mut result = Value::None_;
                for s in stmts {
                    if let Some(r) = self.exec_stmt(s)? {
                        result = r;
                        break;
                    }
                }
                self.pop_frame();
                Ok(EvalResult::Value(result))
            }
            Expr::If(cond, then_expr, else_expr) => {
                let c = eval!(cond);
                if is_truthy(&c) {
                    self.eval_expr(then_expr)
                } else if let Some(e) = else_expr {
                    self.eval_expr(e)
                } else {
                    Ok(EvalResult::Value(Value::None_))
                }
            }
            Expr::Range(start, end) => {
                let s = eval!(start);
                let e = eval!(end);
                match (s, e) {
                    (Value::Int(a), Value::Int(b)) => Ok(EvalResult::Value(Value::Range(a, b))),
                    _ => Err(self.err(expr.span, "Range bounds must be integers")),
                }
            }
            Expr::ListLit(items) => {
                let mut vals = self.value_pool.take_list_with_capacity(items.len());
                for i in items {
                    match self.eval_expr(i)? {
                        EvalResult::Value(v) => vals.push(v),
                        EvalResult::Return(v) => vals.push(v),
                    }
                }
                Ok(EvalResult::Value(Value::List(vals)))
            }
            Expr::Index(obj, idx_expr) => {
                let obj_val = eval!(obj);
                let idx = eval!(idx_expr);
                match (&obj_val, idx) {
                    (Value::List(items), Value::Int(i)) => {
                        let i = if i < 0 { (items.len() as i64 + i) as usize } else { i as usize };
                        items.get(i).cloned()
                            .ok_or_else(|| self.err(expr.span, format!("Index {} out of bounds for list of length {}", i, items.len())))
                            .map(EvalResult::Value)
                    }
                    (Value::Str(s), Value::Int(i)) => {
                        let i = if i < 0 { (s.len() as i64 + i) as usize } else { i as usize };
                        s.chars().nth(i).map(|c| EvalResult::Value(Value::Str(c.to_string())))
                            .ok_or_else(|| self.err(expr.span, format!("Index {} out of bounds for string of length {}", i, s.len())))
                    }
                    _ => Err(self.err(expr.span, "Cannot index non-indexable value")),
                }
            }
            Expr::Field(obj, field) => {
                let obj_val = eval!(obj);
                match obj_val {
                    Value::Struct(_, fields) => {
                        Ok(EvalResult::Value(fields.get(field).cloned()
                            .ok_or_else(|| self.err(expr.span, format!("Struct has no field '{}'", field)))?))
                    }
                    Value::Instance(cls_name, cls_fields) => {
                        let fields = if let Some(cls) = self.classes.get(&cls_name) {
                            cls.fields.clone()
                        } else if let Some(obj) = self.objects.get(&cls_name) {
                            obj.fields.clone()
                        } else {
                            return Err(self.err(expr.span, format!("Unknown class or object '{}'", cls_name)));
                        };
                        if let Some(idx) = fields.iter().position(|f| f == field) {
                            Ok(EvalResult::Value(cls_fields[idx].clone()))
                        } else {
                            if let Some(cls) = self.classes.get(&cls_name) {
                                if cls.methods.contains_key(field) {
                                    return Ok(EvalResult::Value(Value::Str(field.clone())));
                                }
                            }
                            if let Some(obj) = self.objects.get(&cls_name) {
                                if obj.methods.contains_key(field) {
                                    return Ok(EvalResult::Value(Value::Str(field.clone())));
                                }
                            }
                            Err(self.err(expr.span, format!("Instance '{}' has no field '{}'", cls_name, field)))
                        }
                    }
                    Value::Tuple(items) => {
                        let idx: usize = field.parse()
                            .map_err(|_| self.err(expr.span, format!("Invalid tuple index '{}'", field)))?;
                        Ok(EvalResult::Value(items.get(idx).cloned()
                            .ok_or_else(|| self.err(expr.span, format!("Tuple index {} out of bounds", idx)))?))
                    }
                    Value::Complex(r, i) => match field.as_str() {
                        "real" => Ok(EvalResult::Value(Value::Real(r))),
                        "img" => Ok(EvalResult::Value(Value::Real(i))),
                        "mod" | "norm" => Ok(EvalResult::Value(Value::Real((r * r + i * i).sqrt()))),
                        "arg" => Ok(EvalResult::Value(Value::Real(i.atan2(r)))),
                        "conj" => Ok(EvalResult::Value(Value::Complex(r, -i))),
                        _ => Err(self.err(expr.span, format!("Complex has no field '{}'", field))),
                    },
                    _ => Err(self.err(expr.span, "Cannot access field on non-struct value")),
                }
            }
            Expr::SafeCall(obj, field) => {
                let obj_val = eval!(obj);
                match obj_val {
                    Value::Null => Ok(EvalResult::Value(Value::Null)),
                    Value::Struct(_, fields) => {
                        Ok(EvalResult::Value(fields.get(field).cloned()
                            .ok_or_else(|| self.err(expr.span, format!("Struct has no field '{}'", field)))?))
                    }
                    Value::Instance(cls_name, cls_fields) => {
                        let cls = self.classes.get(&cls_name)
                            .ok_or_else(|| self.err(expr.span, format!("Unknown class '{}'", cls_name)))?;
                        if let Some(idx) = cls.fields.iter().position(|f| f == field) {
                            Ok(EvalResult::Value(cls_fields[idx].clone()))
                        } else {
                            Err(self.err(expr.span, format!("Class '{}' has no field '{}'", cls_name, field)))
                        }
                    }
                    Value::Tuple(items) => {
                        let idx: usize = field.parse()
                            .map_err(|_| self.err(expr.span, format!("Invalid tuple index '{}'", field)))?;
                        Ok(EvalResult::Value(items.get(idx).cloned()
                            .ok_or_else(|| self.err(expr.span, format!("Tuple index {} out of bounds", idx)))?))
                    }
                    _ => Err(self.err(expr.span, "Cannot access field on non-struct value")),
                }
            }
            Expr::Elvis(a, b) => {
                let a_val = eval!(a);
                match a_val {
                    Value::Null => {
                        match self.eval_expr(b)? {
                            EvalResult::Value(v) => Ok(EvalResult::Value(v)),
                            ret @ EvalResult::Return(_) => Ok(ret),
                        }
                    }
                    val => Ok(EvalResult::Value(val)),
                }
            }
            Expr::StructLit(name, field_exprs) => {
                if let Some(def_fields) = self.struct_defs.get(name).cloned() {
                    let mut fields = self.value_pool.take_map();
                    for (fname, fexpr) in field_exprs {
                        if !def_fields.contains(fname) {
                            self.value_pool.return_map(fields);
                            return Err(self.err(expr.span, format!("Struct '{}' has no field '{}'", name, fname)));
                        }
                        let val = eval!(fexpr);
                        fields.insert(fname.clone(), val);
                    }
                    let result = Ok(EvalResult::Value(Value::Struct(name.clone(), fields)));
                    return result;
                } else if let Some(cls) = self.classes.get(name).cloned() {
                    let mut vals = vec![Value::None_; cls.fields.len()];
                    for (fname, fexpr) in field_exprs {
                        let idx = cls.fields.iter().position(|f| f == fname)
                            .ok_or_else(|| self.err(expr.span, format!("Class '{}' has no field '{}'", name, fname)))?;
                        let val = eval!(fexpr);
                        vals[idx] = val;
                    }
                    let instance = Value::Instance(name.clone(), vals);
                    let instance = self.run_init_blocks(&cls, instance, expr.span)?;
                    Ok(EvalResult::Value(instance))
                } else {
                    Err(self.err(expr.span, format!("Unknown struct/class '{}'", name)))
                }
            }
            Expr::TupleLit(items) => {
                let mut vals = self.value_pool.take_list_with_capacity(items.len());
                for i in items {
                    match self.eval_expr(i)? {
                        EvalResult::Value(v) => vals.push(v),
                        EvalResult::Return(v) => vals.push(v),
                    }
                }
                Ok(EvalResult::Value(Value::Tuple(vals)))
            }
            Expr::MapLit(pairs) => {
                let mut dict = Vec::new();
                for (k_expr, v_expr) in pairs {
                    let k = eval!(k_expr);
                    let v = eval!(v_expr);
                    dict.push((k, v));
                }
                Ok(EvalResult::Value(Value::Dict(dict)))
            }
            Expr::SetLit(items) => {
                let mut set = Vec::new();
                for item in items {
                    set.push(eval!(item));
                }
                Ok(EvalResult::Value(Value::Set(set)))
            }
            Expr::VectorLit(items) => {
                let vals: Result<Vec<Value>> = items.iter().map(|i| match self.eval_expr(i)? {
                    EvalResult::Value(v) => Ok(v),
                    EvalResult::Return(v) => Ok(v),
                }).collect();
                Ok(EvalResult::Value(Value::List(vals?)))
            }
            Expr::MatrixLit(rows) => {
                let vals: Result<Vec<Vec<Value>>> = rows.iter().map(|r| r.iter().map(|i| -> Result<Value> {
                    match self.eval_expr(i)? {
                        EvalResult::Value(v) => Ok(v),
                        EvalResult::Return(v) => Ok(v),
                    }
                }).collect()).collect();
                Ok(EvalResult::Value(Value::List(vals?.into_iter().map(|r| Value::List(r)).collect())))
            }
            Expr::FnLit(_, _, body) => {
                Ok(EvalResult::Value(match self.eval_expr(body)? {
                    EvalResult::Value(v) => v,
                    EvalResult::Return(v) => v,
                }))
            }
            Expr::PostInc(i) | Expr::PostDec(i) => {
                let val = eval!(i);
                let one = Value::Int(1);
                let op = if matches!(&expr.value, Expr::PostInc(_)) { BinOp::Add } else { BinOp::Sub };
                let inc = self.eval_binop(val.clone(), op, one, expr.span)?;
                if let Expr::Ident(name) = &i.value {
                    self.set_var(name, inc)?;
                } else {
                    return Err(self.err(expr.span, "++/-- not supported on non-variable"));
                }
                Ok(EvalResult::Value(val))
            }
            Expr::ResultOk(i) => {
                let v = eval!(i);
                Ok(EvalResult::Value(Value::Result(true, Box::new(v))))
            }
            Expr::ResultErr(i) => {
                let v = eval!(i);
                Ok(EvalResult::Value(Value::Result(false, Box::new(v))))
            }
            Expr::Try(i) => {
                let val = eval!(i);
                match val {
                    Value::Result(true, v) => Ok(EvalResult::Value(*v)),
                    Value::Result(false, e) => Ok(EvalResult::Return(Value::Result(false, e))),
                    _ => Err(self.err(expr.span, "? used on non-Result value")),
                }
            }
            Expr::TryCatch(try_body, catch_var, catch_body) => {
                self.push_frame();
                let mut caught = None;
                for s in try_body {
                    match self.exec_stmt(s)? {
                        Some(v) => {
                            if matches!(&v, Value::Result(false, _)) {
                                caught = Some(v);
                            } else {
                                self.pop_frame();
                                return Ok(EvalResult::Return(v));
                            }
                            break;
                        }
                        None => {}
                    }
                }
                self.pop_frame();
                if let Some(err_val) = caught {
                    self.push_frame();
                    self.frames.last_mut().unwrap().insert(catch_var.clone(), err_val);
                    let mut result = Value::None_;
                    for s in catch_body {
                        match self.exec_stmt(s)? {
                            Some(v) => { result = v; break; }
                            None => {}
                        }
                    }
                    self.pop_frame();
                    Ok(EvalResult::Value(result))
                } else {
                    Ok(EvalResult::Value(Value::None_))
                }
            }
            Expr::Spawn(i) => {
                let expr_clone = i.clone();
                let tid = self.next_task_id;
                self.next_task_id += 1;
                let (tx, rx) = std::sync::mpsc::channel();
                let funcs = self.functions.clone();
                let structs = self.struct_defs.clone();
                let classes = self.classes.clone();
                let builtin_funcs = self.builtin_funcs.clone();
                let builtin_modules = self.builtin_modules.clone();
                let _task_rx = crate::runtime::virtual_task::global_virtual_scheduler().spawn(move || {
                    let mut mini = Interpreter {
                        globals: HashMap::new(),
                        const_vars: HashSet::new(),
                        struct_defs: structs,
                        classes,
                        objects: Arc::new(HashMap::new()),
                        functions: funcs,
                        builtin_modules,
                        builtin_funcs,
                        std_imported: false,
                        frames: vec![super::class::Frame::new()],
                        frame_pool: Vec::new(),
                        moved_frames: vec![HashSet::new()],
                        global_moved: HashSet::new(),
                        output: String::new(),
                        tui_mode: false,
                        ffi_libs: HashMap::new(),
                        next_task_id: 0,
                        task_rxs: HashMap::new(),
                        value_pool: crate::memory::arena::ValuePool::new(),
                    };
                    mini.push_frame();
                    let result = mini.eval_expr(&expr_clone);
                    mini.pop_frame();
                    let val = match result {
                        Ok(EvalResult::Value(v)) => v,
                        Ok(EvalResult::Return(v)) => v,
                        Err(e) => {
                            let msg = format!("Task {} failed: {:?}", tid, e);
                            Value::Str(msg)
                        }
                    };
                    let _ = tx.send(val);
                    Ok(())
                });
                self.task_rxs.insert(tid, rx);
                Ok(EvalResult::Value(Value::Int(tid as i64)))
            }
            Expr::AsConst(i) => {
                Ok(EvalResult::Value(match self.eval_expr(i)? {
                    EvalResult::Value(v) => v,
                    EvalResult::Return(v) => return Ok(EvalResult::Return(v)),
                }))
            }
            Expr::Await(i) => {
                let val = eval!(i);
                match val {
                    Value::Int(tid) if tid >= 0 => {
                        let tid_u64 = tid as u64;
                        if let Some(rx) = self.task_rxs.remove(&tid_u64) {
                            match rx.recv() {
                                Ok(result) => Ok(EvalResult::Value(result)),
                                Err(_) => Err(self.err(expr.span, "Task failed to produce a result")),
                            }
                        } else {
                            Ok(EvalResult::Value(Value::None_))
                        }
                    }
                    _ => Ok(EvalResult::Value(val)),
                }
            }
            Expr::As(inner, target_type) => {
                let val = eval!(inner);
                Ok(EvalResult::Value(self.cast_value(val, target_type, expr.span)?))
            }
            Expr::Match(scrutinee, arms) => {
                let sv = eval!(scrutinee);
                for arm in arms {
                    let mut bindings: HashMap<String, Value> = HashMap::new();
                    if self.match_pattern(&arm.pattern, &sv, &mut bindings) {
                        let guard_ok = match &arm.guard {
                            Some(guard_expr) => {
                                self.push_frame();
                                for (k, v) in &bindings { self.set_var(k, v.clone()).ok(); }
                                let result = eval!(guard_expr);
                                self.pop_frame();
                                match result {
                                    Value::Bool(b) => b,
                                    _ => return Err(self.err(scrutinee.span, "Match guard must evaluate to a boolean")),
                                }
                            }
                            None => true,
                        };
                        if guard_ok {
                            self.push_frame();
                            for (k, v) in &bindings { self.set_var(k, v.clone()).ok(); }
                            let result = self.eval_expr(&arm.body);
                            self.pop_frame();
                            return result;
                        }
                    }
                }
                Err(self.err(scrutinee.span, "Non-exhaustive patterns: no match arm matched the value"))
            }
            Expr::Variant(_enum_name, variant_name, args) => {
                let mut fields = Vec::new();
                for arg in args {
                    fields.push(eval!(arg));
                }
                Ok(EvalResult::Value(Value::Variant(variant_name.clone(), fields)))
            }
            Expr::ForIn(_, _, _) | Expr::While(_, _) | Expr::Loop(_) => {
                Err(self.err(expr.span, "Expression not supported in interpreter"))
            }
        }
    }
}
