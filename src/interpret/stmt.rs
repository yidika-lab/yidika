use std::collections::HashMap;
use crate::diagnostics::error::Result;
use crate::diagnostics::span::Span;
use crate::syntax::ast::*;
use super::value::{Value, EvalResult, is_truthy};
use super::class::FnDef;
use super::env::Interpreter;

impl Interpreter {
    pub fn run_fn_body(&mut self, fndef: &FnDef) -> Result<Option<Value>> {
        for stmt in fndef.body.iter() {
            let r = self.exec_stmt(stmt)?;
            if r.is_some() { return Ok(r); }
        }
        Ok(None)
    }

    pub fn exec_stmt(&mut self, stmt: &StmtNode) -> Result<Option<Value>> {
        match &stmt.value {
            Stmt::Decl { name, value, is_const, .. } => {
                let val = match self.eval_expr(value)? {
                    EvalResult::Value(v) => v,
                    EvalResult::Return(v) => return Ok(Some(v)),
                };
                self.mark_moved_expr(value, &val);
                if *is_const || matches!(&value.value, Expr::AsConst(_)) {
                    self.const_vars.insert(name.clone());
                }
                self.frames.last_mut().unwrap().insert(name.clone(), val);
                Ok(None)
            }
            Stmt::Expr(e) => {
                match self.eval_expr(e)? {
                    EvalResult::Value(_) => Ok(None),
                    EvalResult::Return(v) => Ok(Some(v)),
                }
            }
            Stmt::Return(e) => {
                let val = match e {
                    Some(x) => {
                        let v = match self.eval_expr(x)? {
                            EvalResult::Value(v) => v,
                            EvalResult::Return(v) => return Ok(Some(v)),
                        };
                        self.mark_moved_expr(x, &v);
                        v
                    }
                    None => Value::None_,
                };
                return Ok(Some(val));
            }
            Stmt::For(var, iter, body, is_for_of) => {
                let iter_val = match self.eval_expr(iter)? {
                    EvalResult::Value(v) => v,
                    EvalResult::Return(v) => return Ok(Some(v)),
                };
                match iter_val {
                    Value::Range(s, e, is_char) => {
                        if is_char {
                            let start_cp = s;
                            let end_cp = e;
                            if s <= e {
                                for i in start_cp..end_cp {
                                    self.push_frame();
                                    self.frames.last_mut().unwrap().insert(var.clone(), Value::Char(char::from_u32(i as u32).unwrap_or(char::REPLACEMENT_CHARACTER)));
                                    for s in body {
                                        if let Some(r) = self.exec_stmt(s)? { self.pop_frame(); return Ok(Some(r)); }
                                    }
                                    self.pop_frame();
                                }
                            } else {
                                for i in ((end_cp + 1)..=start_cp).rev() {
                                    self.push_frame();
                                    self.frames.last_mut().unwrap().insert(var.clone(), Value::Char(char::from_u32(i as u32).unwrap_or(char::REPLACEMENT_CHARACTER)));
                                    for s in body {
                                        if let Some(r) = self.exec_stmt(s)? { self.pop_frame(); return Ok(Some(r)); }
                                    }
                                    self.pop_frame();
                                }
                            }
                        } else {
                            if s <= e {
                                for i in s..e {
                                    self.push_frame();
                                    self.frames.last_mut().unwrap().insert(var.clone(), Value::Int(i));
                                    for s in body {
                                        if let Some(r) = self.exec_stmt(s)? { self.pop_frame(); return Ok(Some(r)); }
                                    }
                                    self.pop_frame();
                                }
                            } else {
                                for i in ((e + 1)..=s).rev() {
                                    self.push_frame();
                                    self.frames.last_mut().unwrap().insert(var.clone(), Value::Int(i));
                                    for s in body {
                                        if let Some(r) = self.exec_stmt(s)? { self.pop_frame(); return Ok(Some(r)); }
                                    }
                                    self.pop_frame();
                                }
                            }
                        }
                    }
                    Value::Int(end) => {
                        for i in 0..end {
                            self.push_frame();
                            self.frames.last_mut().unwrap().insert(var.clone(), Value::Int(i));
                            for s in body {
                                if let Some(r) = self.exec_stmt(s)? { self.pop_frame(); return Ok(Some(r)); }
                            }
                            self.pop_frame();
                        }
                    }
                    Value::List(items) => {
                        let iterable: Vec<Value> = if *is_for_of {
                            items.iter().cloned().collect()
                        } else {
                            (0..items.len()).map(|i| Value::Int(i as i64)).collect()
                        };
                        for val in iterable {
                            self.push_frame();
                            self.frames.last_mut().unwrap().insert(var.clone(), val);
                            for s in body {
                                if let Some(r) = self.exec_stmt(s)? { self.pop_frame(); return Ok(Some(r)); }
                            }
                            self.pop_frame();
                        }
                    }
                    Value::Map(map) => {
                        let iterable: Vec<Value> = if *is_for_of {
                            map.values().cloned().collect()
                        } else {
                            map.keys().map(|k| Value::Str(k.clone())).collect()
                        };
                        for val in iterable {
                            self.push_frame();
                            self.frames.last_mut().unwrap().insert(var.clone(), val);
                            for s in body {
                                if let Some(r) = self.exec_stmt(s)? { self.pop_frame(); return Ok(Some(r)); }
                            }
                            self.pop_frame();
                        }
                    }
                    Value::Set(set) => {
                        for val in set {
                            self.push_frame();
                            self.frames.last_mut().unwrap().insert(var.clone(), val);
                            for s in body {
                                if let Some(r) = self.exec_stmt(s)? { self.pop_frame(); return Ok(Some(r)); }
                            }
                            self.pop_frame();
                        }
                    }
                    _ => return Err(self.err(stmt.span, "For-in requires a range, integer, list, map, or set")),
                }
                Ok(None)
            }
            Stmt::While(cond, body) => {
                loop {
                    let c = match self.eval_expr(cond)? {
                        EvalResult::Value(v) => v,
                        EvalResult::Return(v) => return Ok(Some(v)),
                    };
                    if !is_truthy(&c) { break; }
                    self.push_frame();
                    for s in body {
                        if let Some(r) = self.exec_stmt(s)? { self.pop_frame(); return Ok(Some(r)); }
                    }
                    self.pop_frame();
                }
                Ok(None)
            }
            Stmt::Loop(body) => {
                loop {
                    self.push_frame();
                    for s in body {
                        if let Some(r) = self.exec_stmt(s)? { self.pop_frame(); return Ok(Some(r)); }
                    }
                    self.pop_frame();
                }
            }
            Stmt::If(cond, then_body, else_body) => {
                let c = match self.eval_expr(cond)? {
                    EvalResult::Value(v) => v,
                    EvalResult::Return(v) => return Ok(Some(v)),
                };
                let branch = if is_truthy(&c) { then_body } else {
                    match else_body { Some(eb) => eb, None => return Ok(None) }
                };
                self.push_frame();
                for s in branch {
                    if let Some(r) = self.exec_stmt(s)? { self.pop_frame(); return Ok(Some(r)); }
                }
                self.pop_frame();
                Ok(None)
            }
            Stmt::Assign(name, expr) => {
                if self.const_vars.contains(name) || self.is_const_global(name) {
                    return Err(self.err(stmt.span, format!("Cannot assign to const variable '{}'", name)));
                }
                let val = match self.eval_expr(expr)? {
                    EvalResult::Value(v) => v,
                    EvalResult::Return(v) => return Ok(Some(v)),
                };
                self.mark_moved_expr(expr, &val);
                self.set_var(name, val)?;
                Ok(None)
            }
            Stmt::Destruct(pattern, expr) => {
                let val = match self.eval_expr(expr)? {
                    EvalResult::Value(v) => v,
                    EvalResult::Return(v) => return Ok(Some(v)),
                };
                self.destruct_bind(pattern, val, expr.span)?;
                Ok(None)
            }
        }
    }

    pub fn match_pattern(&self, pattern: &Pattern, value: &Value, bindings: &mut HashMap<String, Value>) -> bool {
        eprintln!("DEBUG match_pattern: value={:?}, bindings_before={:?}", value, bindings.keys());
        match pattern {
            Pattern::Ignore => true,
            Pattern::Ident(name) => { bindings.insert(name.clone(), value.clone()); true }
            Pattern::Rest(name) => { bindings.insert(name.clone(), value.clone()); true }
            Pattern::LitInt(n) => matches!(value, Value::Int(v) if *v == *n),
            Pattern::LitReal(n) => matches!(value, Value::Real(v) if *v == *n),
            Pattern::LitStr(s) => matches!(value, Value::Str(v) if *v == *s),
            Pattern::LitBool(b) => matches!(value, Value::Bool(v) if *v == *b),
            Pattern::ListDestruct(patterns) => {
                match value {
                    Value::List(items) => {
                        let mut i = 0;
                        for pat in patterns {
                            match pat {
                                Pattern::Rest(rname) => {
                                    let rest: Vec<Value> = items[i..].to_vec();
                                    bindings.insert(rname.clone(), Value::List(rest));
                                    i = items.len();
                                }
                                _ => {
                                    if i >= items.len() { return false; }
                                    if !self.match_pattern(pat, &items[i], bindings) { return false; }
                                    i += 1;
                                }
                            }
                        }
                        i == items.len()
                    }
                    _ => false,
                }
            }
            Pattern::Destruct(fields) => {
                match value {
                    Value::Struct(_, map) => {
                        for (fname, pat) in fields {
                            match map.get(fname) {
                                Some(fval) => {
                                    if !self.match_pattern(pat, fval, bindings) { return false; }
                                }
                                None => return false,
                            }
                        }
                        true
                    }
                    Value::Map(map) => {
                        for (fname, pat) in fields {
                            match map.get(fname) {
                                Some(fval) => {
                                    if !self.match_pattern(pat, fval, bindings) { return false; }
                                }
                                None => return false,
                            }
                        }
                        true
                    }
                    _ => false,
                }
            }
            Pattern::Variant(vname, subpatterns) => {
                match value {
                    Value::Variant(evname, fields) if evname == vname => {
                        if fields.len() != subpatterns.len() { return false; }
                        for (field, pat) in fields.iter().zip(subpatterns.iter()) {
                            if !self.match_pattern(pat, field, bindings) { return false; }
                        }
                        true
                    }
                    _ => false,
                }
            }
        }
    }

    pub fn destruct_bind(&mut self, pattern: &Pattern, val: Value, span: Span) -> Result<()> {
        match pattern {
            Pattern::Ident(name) => { self.set_var(name, val)?; Ok(()) }
            Pattern::Rest(name) => { self.set_var(name, val)?; Ok(()) }
            Pattern::Ignore => Ok(()),
            Pattern::LitInt(_) | Pattern::LitReal(_) | Pattern::LitStr(_) | Pattern::LitBool(_) => Ok(()),
            Pattern::ListDestruct(elements) => {
                let items = match val {
                    Value::List(items) => items,
                    _ => return Err(self.err(span, "Cannot destructure non-list value")),
                };
                let mut idx = 0;
                for elem in elements {
                    match elem {
                        Pattern::Rest(name) => {
                            let rest: Vec<Value> = items[idx..].to_vec();
                            self.set_var(name, Value::List(rest))?;
                            idx = items.len();
                        }
                        Pattern::Ignore => { idx += 1; }
                        Pattern::Ident(name) => {
                            let v = items.get(idx).cloned()
                                .ok_or_else(|| self.err(span, format!("Destructure index {} out of bounds", idx)))?;
                            self.set_var(name, v)?;
                            idx += 1;
                        }
                        _ => return Err(self.err(span, "Nested destructuring not yet supported in lists")),
                    }
                }
                Ok(())
            }
            Pattern::Destruct(fields) => {
                let obj = match val {
                    Value::Map(m) => m,
                    Value::Struct(_, fields_map) => fields_map,
                    _ => return Err(self.err(span, "Cannot destructure non-map/struct value")),
                };
                for (name, subpattern) in fields {
                    let v = obj.get(name).cloned().unwrap_or(Value::None_);
                    self.destruct_bind(subpattern, v, span)?;
                }
                Ok(())
            }
            Pattern::Variant(_, subpatterns) => {
                match val {
                    Value::Variant(_, fields) => {
                        for (subpat, field) in subpatterns.iter().zip(fields.into_iter()) {
                            self.destruct_bind(subpat, field, span)?;
                        }
                        Ok(())
                    }
                    _ => Err(self.err(span, "Cannot destructure non-variant value")),
                }
            }
        }
    }
}
