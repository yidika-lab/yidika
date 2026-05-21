use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use chrono::Datelike;
use chrono::Timelike;
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::syntax::ast::*;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Real(f64),
    Bool(bool),
    Str(String),
    Char(char),
    Range(i64, i64),
    List(Vec<Value>),
    Struct(String, HashMap<String, Value>),
    Tuple(Vec<Value>),
    Dict(Vec<(Value, Value)>),
    Set(Vec<Value>),
    Map(HashMap<String, Value>),
    None_,
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Real(r) => write!(f, "{}", r),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Str(s) => write!(f, "{}", s),
            Value::Char(c) => write!(f, "{}", c),
            Value::Range(a, b) => write!(f, "{}..{}", a, b),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Value::Struct(name, fields) => {
                write!(f, "{} {{ ", name)?;
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, " }}")
            }
            Value::Tuple(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                write!(f, ")")
            }
            Value::Dict(pairs) => {
                write!(f, "{{")?;
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::Set(items) => {
                write!(f, "set{{")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                write!(f, "}}")
            }
            Value::Map(m) => {
                write!(f, "{{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "\"{}\": {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::None_ => write!(f, "none"),
        }
    }
}

#[derive(Debug, Clone)]
struct FnDef {
    params: Vec<Param>,
    body: Vec<StmtNode>,
}

pub struct Interpreter {
    globals: HashMap<String, Value>,
    const_vars: HashSet<String>,
    struct_defs: HashMap<String, Vec<String>>,
    functions: HashMap<String, FnDef>,
    builtin_modules: HashMap<String, String>,
    builtin_funcs: HashMap<String, String>,
    std_imported: bool,
    frames: Vec<HashMap<String, Value>>,
    moved_frames: Vec<HashSet<String>>,
    global_moved: HashSet<String>,
    output: String,
    pub tui_mode: bool,
    ffi_libs: HashMap<String, Arc<libloading::Library>>,
}

impl Interpreter {
    pub fn new() -> Self {
        Self {
            globals: HashMap::new(),
            const_vars: HashSet::new(),
            struct_defs: HashMap::new(),
            functions: HashMap::new(),
            builtin_modules: HashMap::new(),
            builtin_funcs: HashMap::new(),
            std_imported: false,
            frames: vec![HashMap::new()],
            moved_frames: vec![HashSet::new()],
            global_moved: HashSet::new(),
            output: String::new(),
            tui_mode: false,
            ffi_libs: HashMap::new(),
        }
    }

    pub fn load_module(&mut self, module: &Module) {
        for import in &module.imports {
            let source = import.source.as_str();
            match source {
                "std" => {
                    self.std_imported = true;
                    for (name, _) in &import.names {
                        if name == "std" {
                            for sub in crate::stdlib::list_submodules() {
                                self.builtin_modules.insert(sub.to_string(), sub.to_string());
                            }
                        } else if crate::stdlib::list_submodules().any(|s| s == name.as_str()) {
                            self.builtin_modules.insert(name.clone(), name.clone());
                        }
                    }
                }
                "io" | "json" | "datetime" | "path" | "base64" | "re" => {
                    for (name, _) in &import.names {
                        self.builtin_modules.insert(name.clone(), source.to_string());
                    }
                }
                "math" | "time" => {
                    for (name, _) in &import.names {
                        self.builtin_funcs.insert(name.clone(), source.to_string());
                    }
                }
                _ => {
                    // Handle FFI imports (rust:, c++:) — register as modules
                    if let Some(lang) = &import.lang {
                        for (name, _) in &import.names {
                            self.builtin_modules.insert(name.clone(), format!("{}:{}", lang, source));
                        }
                    }
                }
            }
        }
        for item in &module.items {
            match &item.value {
                ItemKind::Fn { name, params, body, .. } => {
                    self.functions.insert(name.clone(), FnDef {
                        params: params.clone(),
                        body: body.clone(),
                    });
                }
                ItemKind::Struct { name, fields, .. } => {
                    self.struct_defs.insert(name.clone(), fields.iter().map(|p| p.name.clone()).collect());
                }
                ItemKind::Const { name, value, .. } => {
                    let val = self.eval_expr(value).unwrap_or(Value::None_);
                    self.globals.insert(name.clone(), val);
                    self.const_vars.insert(name.clone());
                }
                _ => {}
            }
        }
    }

    pub fn run_main(&mut self) -> Result<String> {
        const STACK_SIZE: usize = 8 * 1024 * 1024; // 8MB
        let builder = std::thread::Builder::new()
            .name("interpreter".into())
            .stack_size(STACK_SIZE);
        let functions = self.functions.clone();
        let globals = self.globals.clone();
        let struct_defs = self.struct_defs.clone();
        let const_vars = self.const_vars.clone();
        let builtin_modules = self.builtin_modules.clone();
        let builtin_funcs = self.builtin_funcs.clone();
        let tui_mode = self.tui_mode;
        let std_imported = self.std_imported;
        let ffi_libs = self.ffi_libs.clone();
        let result = builder.spawn(move || {
            let mut interp = Interpreter {
                frames: vec![HashMap::new()],
                globals,
                const_vars,
                struct_defs,
                functions,
                builtin_modules,
                builtin_funcs,
                output: String::new(),
                tui_mode,
                std_imported,
                ffi_libs,
                moved_frames: vec![HashSet::new()],
                global_moved: HashSet::new(),
            };
            let fndef = interp.functions.get("main").cloned();
            match fndef {
                Some(fn_def) => {
                    interp.push_frame();
                    let _result = interp.run_fn_body(&fn_def)?;
                    interp.pop_frame();
                    Ok(std::mem::replace(&mut interp.output, String::new()))
                }
                None => Err(error::err(ErrorKind::NameError, Span::new(0, 0),
                    "No 'main' function found")),
            }
        }).map_err(|e| error::err(ErrorKind::Internal, Span::new(0, 0),
            format!("Failed to spawn interpreter thread: {}", e)))?;
        result.join().map_err(|_| error::err(ErrorKind::Internal, Span::new(0, 0),
            "Interpreter thread panicked"))?
    }

    fn is_moved(&self, name: &str) -> bool {
        if self.global_moved.contains(name) { return true; }
        for mf in self.moved_frames.iter().rev() {
            if mf.contains(name) { return true; }
        }
        false
    }

    fn mark_moved_name(&mut self, name: &str) {
        // Check if exists in any frame
        for (i, frame) in self.frames.iter().enumerate().rev() {
            if frame.contains_key(name) {
                self.moved_frames[i].insert(name.to_string());
                return;
            }
        }
        if self.globals.contains_key(name) {
            self.global_moved.insert(name.to_string());
        }
    }

    fn mark_moved_expr(&mut self, expr: &ExprNode, val: &Value) {
        if is_copy_value(val) { return; }
        match &expr.value {
            Expr::Ident(src) => { self.mark_moved_name(src); }
            Expr::Field(obj, _) => {
                if let Expr::Ident(src) = &obj.value { self.mark_moved_name(src); }
            }
            Expr::AsConst(inner) => self.mark_moved_expr(inner, val),
            _ => {}
        }
    }

    fn get_var(&self, name: &str) -> Result<Value> {
        if self.is_moved(name) {
            return Err(error::err(ErrorKind::NameError, Span::new(0, 0),
                format!("Cannot use variable '{}' after it has been moved", name)));
        }
        for frame in self.frames.iter().rev() {
            if let Some(v) = frame.get(name) { return Ok(v.clone()); }
        }
        self.globals.get(name).cloned()
            .ok_or_else(|| error::err(ErrorKind::NameError, Span::new(0, 0),
                format!("Variable '{}' not found", name)))
    }

    fn is_const_global(&self, name: &str) -> bool {
        self.const_vars.contains(name) && !self.frames.iter().any(|f| f.contains_key(name))
    }

    fn set_var(&mut self, name: &str, val: Value) -> Result<()> {
        for frame in self.frames.iter_mut().rev() {
            if frame.contains_key(name) {
                frame.insert(name.to_string(), val);
                return Ok(());
            }
        }
        if self.globals.contains_key(name) {
            self.globals.insert(name.to_string(), val);
            return Ok(());
        }
        self.frames.last_mut().unwrap().insert(name.to_string(), val);
        Ok(())
    }

    fn push_frame(&mut self) {
        self.frames.push(HashMap::new());
        self.moved_frames.push(HashSet::new());
    }

    fn pop_frame(&mut self) {
        self.frames.pop();
        self.moved_frames.pop();
    }

    fn run_fn_body(&mut self, fndef: &FnDef) -> Result<Option<Value>> {
        for stmt in &fndef.body {
            let r = self.exec_stmt(stmt)?;
            if r.is_some() { return Ok(r); }
        }
        Ok(None)
    }

    fn exec_stmt(&mut self, stmt: &StmtNode) -> Result<Option<Value>> {
        match &stmt.value {
            Stmt::Decl { name, value, is_const, .. } => {
                let val = self.eval_expr(value)?;
                self.mark_moved_expr(value, &val);
                if *is_const || matches!(&value.value, Expr::AsConst(_)) {
                    self.const_vars.insert(name.clone());
                }
                self.frames.last_mut().unwrap().insert(name.clone(), val);
                Ok(None)
            }
            Stmt::Expr(e) => {
                self.eval_expr(e)?;
                Ok(None)
            }
            Stmt::Return(e) => {
                let val = match e {
                    Some(x) => {
                        let v = self.eval_expr(x)?;
                        self.mark_moved_expr(x, &v);
                        v
                    }
                    None => Value::None_,
                };
                return Ok(Some(val));
            }
            Stmt::For(var, iter, body) => {
                let iter_val = self.eval_expr(iter)?;
                let (start, end) = match iter_val {
                    Value::Range(s, e) => (s, e),
                    Value::Int(end) => (0, end),
                    _ => return Err(self.err(stmt.span, "For-in requires a range or integer")),
                };
                for i in start..end {
                    self.push_frame();
                    self.frames.last_mut().unwrap().insert(var.clone(), Value::Int(i));
                    for s in body {
                        if let Some(r) = self.exec_stmt(s)? { self.pop_frame(); return Ok(Some(r)); }
                    }
                    self.pop_frame();
                }
                Ok(None)
            }
            Stmt::While(cond, body) => {
                loop {
                    let c = self.eval_expr(cond)?;
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
                let c = self.eval_expr(cond)?;
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
                let val = self.eval_expr(expr)?;
                self.mark_moved_expr(expr, &val);
                self.set_var(name, val)?;
                Ok(None)
            }
            Stmt::Destruct(_, expr) => {
                self.eval_expr(expr)?;
                Ok(None)
            }
        }
    }

    fn eval_expr(&mut self, expr: &ExprNode) -> Result<Value> {
        match &expr.value {
            Expr::LitInt(i) => Ok(Value::Int(*i)),
            Expr::LitHex(h) => Ok(Value::Int(*h)),
            Expr::LitReal(f) => Ok(Value::Real(*f)),
            Expr::LitStr(s) => Ok(Value::Str(s.clone())),
            Expr::LitChar(c) => Ok(Value::Char(*c)),
            Expr::LitBool(b) => Ok(Value::Bool(*b)),
            Expr::LitNull | Expr::LitNone => Ok(Value::None_),
            Expr::LitSymbol(s) => Ok(Value::Str(format!(":{}", s))),
            Expr::Ident(name) => {
                self.get_var(name)
            }
            Expr::BinOp(l, op, r) => {
                let lv = self.eval_expr(l)?;
                let rv = self.eval_expr(r)?;
                self.eval_binop(lv, *op, rv, expr.span)
            }
            Expr::UnOp(op, inner) => {
                let v = self.eval_expr(inner)?;
                match op {
                    UnOp::Neg => match v {
                        Value::Int(i) => Ok(Value::Int(-i)),
                        Value::Real(r) => Ok(Value::Real(-r)),
                        _ => Err(self.err(expr.span, "Cannot negate")),
                    },
                    UnOp::Not => match v {
                        Value::Bool(b) => Ok(Value::Bool(!b)),
                        _ => Err(self.err(expr.span, "Cannot boolean not")),
                    },
                }
            }
            Expr::Call(callee, args) => {
                let arg_vals: Result<Vec<Value>> = args.iter().map(|a| self.eval_expr(a)).collect();
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
                                Ok(Value::None_)
                            }
                            "len" => {
                                let v = arg_vals.into_iter().next()
                                    .ok_or_else(|| self.err(expr.span, "len() requires 1 argument"))?;
                                match v {
                                    Value::Str(s) => Ok(Value::Int(s.chars().count() as i64)),
                                    Value::List(l) => Ok(Value::Int(l.len() as i64)),
                                    _ => Err(self.err(expr.span, "len() requires a string or list")),
                                }
                            }
                            "str" => {
                                let v = arg_vals.into_iter().next()
                                    .ok_or_else(|| self.err(expr.span, "str() requires 1 argument"))?;
                                Ok(Value::Str(v.to_string()))
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
                                Ok(Value::Str(line))
                            }
                            _ => {
                                // Check if this is a stdlib function imported directly (e.g. cos from math)
                                if let Some(module) = self.builtin_funcs.get(name).cloned() {
                                    let span = expr.span;
                                    match module.as_str() {
                                        "math" => self.call_math(name, arg_vals, span),
                                        "time" => self.call_time(name, arg_vals, span),
                                        _ => Err(self.err(span, format!("Unknown function '{}'", name))),
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
                                            Ok(result.unwrap_or(Value::None_))
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
                                            "read" | "write" | "append" | "remove" | "exists" | "list" | "is_dir" | "is_file" => self.call_fs(&func, arg_vals, expr.span),
                                            _ => self.call_sys(&func, arg_vals, expr.span),
                                        }
                                    }
                                    "json" => self.call_json(&func, arg_vals, expr.span),
                                    "datetime" => self.call_datetime(&func, arg_vals, expr.span),
                                    "path" => self.call_path_module(&func, arg_vals, expr.span),
                                    "base64" => self.call_base64(&func, arg_vals, expr.span),
                                    "re" => self.call_re(&func, arg_vals, expr.span),
                                    "math" => self.call_math(&func, arg_vals, expr.span),
                                    "time" => self.call_time(&func, arg_vals, expr.span),
                                    ffi if ffi.starts_with("rust:") || ffi.starts_with("c++:") => {
                                        let ffi_path = &ffi[ffi.find(':').unwrap() + 1..];
                                        self.call_ffi(&name, &func, arg_vals, expr.span, ffi_path)
                                    }
                                    _ => Err(self.err(expr.span, format!("Unknown module '{}'", name))),
                                }
                            }
                            None => {
                                let var_name = match &obj.value { Expr::Ident(n) => Some(n.clone()), _ => None };
                                let mut receiver = self.eval_expr(obj)?;
                                match field.as_str() {
                                    "push" => match &mut receiver {
                                        Value::List(items) => {
                                            if let Some(val) = arg_vals.into_iter().next() {
                                                items.push(val);
                                            }
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(Value::None_)
                                        }
                                        _ => Err(self.err(expr.span, "push requires a list")),
                                    },
                                    "pop" => match &mut receiver {
                                        Value::List(items) => {
                                            let result = items.pop()
                                                .ok_or_else(|| self.err(expr.span, "pop from empty list"))?;
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(result)
                                        }
                                        _ => Err(self.err(expr.span, "pop requires a list")),
                                    },
                                    "sort" => match &mut receiver {
                                        Value::List(items) => {
                                            items.sort_by(|a, b| a.to_string().cmp(&b.to_string()));
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(Value::None_)
                                        }
                                        _ => Err(self.err(expr.span, "sort requires a list")),
                                    },
                                    "reverse" => match &mut receiver {
                                        Value::List(items) => {
                                            items.reverse();
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(Value::None_)
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
                                            Ok(Value::None_)
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
                                            Ok(Value::None_)
                                        }
                                        _ => Err(self.err(expr.span, "remove requires a list")),
                                    },
                                    "clear" => match &mut receiver {
                                        Value::List(items) => {
                                            items.clear();
                                            if let Some(ref n) = var_name { self.set_var(n, receiver)?; }
                                            Ok(Value::None_)
                                        }
                                        _ => Err(self.err(expr.span, "clear requires a list")),
                                    },
                                    "len" => match &receiver {
                                        Value::Str(s) => Ok(Value::Int(s.chars().count() as i64)),
                                        Value::List(l) => Ok(Value::Int(l.len() as i64)),
                                        Value::Map(m) => Ok(Value::Int(m.len() as i64)),
                                        _ => Err(self.err(expr.span, "len requires a string, list, or map")),
                                    },
                                    "split" => match &receiver {
                                        Value::Str(s) => {
                                            let delim = match arg_vals.into_iter().next() {
                                                Some(Value::Str(d)) => d,
                                                Some(_) => return Err(self.err(expr.span, "split requires a string delimiter")),
                                                None => "".into(),
                                            };
                                            Ok(Value::List(s.split(&delim).map(|p| Value::Str(p.to_string())).collect()))
                                        }
                                        _ => Err(self.err(expr.span, "split requires a string")),
                                    },
                                    "contains" => match &receiver {
                                        Value::Str(s) => {
                                            let sub = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "contains requires 1 argument"))?;
                                            match sub {
                                                Value::Str(p) => Ok(Value::Bool(s.contains(&p))),
                                                _ => Err(self.err(expr.span, "contains requires a string argument")),
                                            }
                                        }
                                        _ => Err(self.err(expr.span, "contains requires a string")),
                                    },
                                    "trim" => match &receiver {
                                        Value::Str(s) => Ok(Value::Str(s.trim().to_string())),
                                        _ => Err(self.err(expr.span, "trim requires a string")),
                                    },
                                    "toUpper" => match &receiver {
                                        Value::Str(s) => Ok(Value::Str(s.to_uppercase())),
                                        _ => Err(self.err(expr.span, "toUpper requires a string")),
                                    },
                                    "toLower" => match &receiver {
                                        Value::Str(s) => Ok(Value::Str(s.to_lowercase())),
                                        _ => Err(self.err(expr.span, "toLower requires a string")),
                                    },
                                    "replace" => match &receiver {
                                        Value::Str(s) => {
                                            let mut it = arg_vals.into_iter();
                                            let from = it.next().ok_or_else(|| self.err(expr.span, "replace requires 2 arguments"))?;
                                            let to = it.next().ok_or_else(|| self.err(expr.span, "replace requires 2 arguments"))?;
                                            match (from, to) {
                                                (Value::Str(f), Value::Str(t)) => Ok(Value::Str(s.replace(&f, &t))),
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
                                                Value::Int(i) => Ok(Value::Str(s.repeat(i as usize))),
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
                                                Value::Str(p) => Ok(Value::Bool(s.starts_with(&p))),
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
                                                Value::Str(p) => Ok(Value::Bool(s.ends_with(&p))),
                                                _ => Err(self.err(expr.span, "endsWith requires a string argument")),
                                            }
                                        }
                                        _ => Err(self.err(expr.span, "endsWith requires a string")),
                                    },
                                    "isalpha" => match &receiver {
                                        Value::Str(s) => Ok(Value::Bool(s.chars().all(|c| c.is_alphabetic()))),
                                        _ => Err(self.err(expr.span, "isalpha requires a string")),
                                    },
                                    "isdigit" => match &receiver {
                                        Value::Str(s) => Ok(Value::Bool(s.chars().all(|c| c.is_ascii_digit()))),
                                        _ => Err(self.err(expr.span, "isdigit requires a string")),
                                    },
                                    "isalnum" => match &receiver {
                                        Value::Str(s) => Ok(Value::Bool(s.chars().all(|c| c.is_alphanumeric()))),
                                        _ => Err(self.err(expr.span, "isalnum requires a string")),
                                    },
                                    "isupper" => match &receiver {
                                        Value::Str(s) => Ok(Value::Bool(s.chars().any(|c| c.is_uppercase()))),
                                        _ => Err(self.err(expr.span, "isupper requires a string")),
                                    },
                                    "islower" => match &receiver {
                                        Value::Str(s) => Ok(Value::Bool(s.chars().any(|c| c.is_lowercase()))),
                                        _ => Err(self.err(expr.span, "islower requires a string")),
                                    },
                                    "isspace" => match &receiver {
                                        Value::Str(s) => Ok(Value::Bool(s.chars().all(|c| c.is_whitespace()))),
                                        _ => Err(self.err(expr.span, "isspace requires a string")),
                                    },
                                    "strip" => match &receiver {
                                        Value::Str(s) => Ok(Value::Str(s.trim().to_string())),
                                        _ => Err(self.err(expr.span, "strip requires a string")),
                                    },
                                    "lstrip" => match &receiver {
                                        Value::Str(s) => Ok(Value::Str(s.trim_start().to_string())),
                                        _ => Err(self.err(expr.span, "lstrip requires a string")),
                                    },
                                    "rstrip" => match &receiver {
                                        Value::Str(s) => Ok(Value::Str(s.trim_end().to_string())),
                                        _ => Err(self.err(expr.span, "rstrip requires a string")),
                                    },
                                    "capitalize" => match &receiver {
                                        Value::Str(s) => {
                                            let mut chars = s.chars();
                                            let first = chars.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                                            let rest = chars.as_str().to_lowercase();
                                            Ok(Value::Str(first + &rest))
                                        }
                                        _ => Err(self.err(expr.span, "capitalize requires a string")),
                                    },
                                    "swapcase" => match &receiver {
                                        Value::Str(s) => {
                                            Ok(Value::Str(s.chars().map(|c| {
                                                if c.is_uppercase() { c.to_lowercase().next().unwrap_or(c) }
                                                else { c.to_uppercase().next().unwrap_or(c) }
                                            }).collect()))
                                        }
                                        _ => Err(self.err(expr.span, "swapcase requires a string")),
                                    },
                                    "count" => match &receiver {
                                        Value::Str(s) => {
                                            let sub = arg_vals.into_iter().next()
                                                .ok_or_else(|| self.err(expr.span, "count requires 1 argument"))?;
                                            match sub {
                                                Value::Str(p) => Ok(Value::Int(s.matches(&p).count() as i64)),
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
                                                Value::Str(p) => s.find(&p).map(|i| Value::Int(i as i64))
                                                    .ok_or_else(|| self.err(expr.span, "substring not found")),
                                                _ => Err(self.err(expr.span, "index requires a string argument")),
                                            }
                                        }
                                        _ => Err(self.err(expr.span, "index requires a string")),
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
                Ok(result)
            }
            Expr::If(cond, then_expr, else_expr) => {
                let c = self.eval_expr(cond)?;
                if is_truthy(&c) {
                    self.eval_expr(then_expr)
                } else if let Some(e) = else_expr {
                    self.eval_expr(e)
                } else {
                    Ok(Value::None_)
                }
            }
            Expr::Range(start, end) => {
                let s = self.eval_expr(start)?;
                let e = self.eval_expr(end)?;
                match (s, e) {
                    (Value::Int(a), Value::Int(b)) => Ok(Value::Range(a, b)),
                    _ => Err(self.err(expr.span, "Range bounds must be integers")),
                }
            }
            Expr::ListLit(items) => {
                let vals: Result<Vec<Value>> = items.iter().map(|i| self.eval_expr(i)).collect();
                Ok(Value::List(vals?))
            }
            Expr::Index(obj, idx_expr) => {
                let obj_val = self.eval_expr(obj)?;
                let idx = self.eval_expr(idx_expr)?;
                match (&obj_val, idx) {
                    (Value::List(items), Value::Int(i)) => {
                        let i = if i < 0 { (items.len() as i64 + i) as usize } else { i as usize };
                        items.get(i).cloned()
                            .ok_or_else(|| self.err(expr.span, format!("Index {} out of bounds for list of length {}", i, items.len())))
                    }
                    (Value::Str(s), Value::Int(i)) => {
                        let i = if i < 0 { (s.len() as i64 + i) as usize } else { i as usize };
                        s.chars().nth(i).map(|c| Value::Str(c.to_string()))
                            .ok_or_else(|| self.err(expr.span, format!("Index {} out of bounds for string of length {}", i, s.len())))
                    }
                    _ => Err(self.err(expr.span, "Cannot index non-indexable value")),
                }
            }
            Expr::Field(obj, field) => {
                let obj_val = self.eval_expr(obj)?;
                match obj_val {
                    Value::Struct(_, fields) => {
                        fields.get(field).cloned()
                            .ok_or_else(|| self.err(expr.span, format!("Struct has no field '{}'", field)))
                    }
                    Value::Tuple(items) => {
                        let idx: usize = field.parse()
                            .map_err(|_| self.err(expr.span, format!("Invalid tuple index '{}'", field)))?;
                        items.get(idx).cloned()
                            .ok_or_else(|| self.err(expr.span, format!("Tuple index {} out of bounds", idx)))
                    }
                    _ => Err(self.err(expr.span, "Cannot access field on non-struct value")),
                }
            }
            Expr::StructLit(name, field_exprs) => {
                let def_fields = self.struct_defs.get(name).cloned()
                    .ok_or_else(|| self.err(expr.span, format!("Unknown struct '{}'", name)))?;
                let mut fields = HashMap::new();
                for (fname, fexpr) in field_exprs {
                    if !def_fields.contains(fname) {
                        return Err(self.err(expr.span, format!("Struct '{}' has no field '{}'", name, fname)));
                    }
                    let val = self.eval_expr(fexpr)?;
                    fields.insert(fname.clone(), val);
                }
                Ok(Value::Struct(name.clone(), fields))
            }
            Expr::TupleLit(items) => {
                let vals: Result<Vec<Value>> = items.iter().map(|i| self.eval_expr(i)).collect();
                Ok(Value::Tuple(vals?))
            }
            Expr::MapLit(pairs) => {
                let mut dict = Vec::new();
                for (k_expr, v_expr) in pairs {
                    let k = self.eval_expr(k_expr)?;
                    let v = self.eval_expr(v_expr)?;
                    dict.push((k, v));
                }
                Ok(Value::Dict(dict))
            }
            Expr::SetLit(items) => {
                let mut set = Vec::new();
                for item in items {
                    set.push(self.eval_expr(item)?);
                }
                Ok(Value::Set(set))
            }
            Expr::FnLit(_, _, body) => {
                self.eval_expr(body)
            }
            Expr::ResultOk(i) | Expr::ResultErr(i) | Expr::Spawn(i) | Expr::AsConst(i) => {
                self.eval_expr(i)
            }
            _ => Err(self.err(expr.span, "Expression not supported in interpreter")),
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

    fn eval_binop(&self, lv: Value, op: BinOp, rv: Value, span: Span) -> Result<Value> {
        match op {
            BinOp::Add => match (&lv, &rv) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
                (Value::Real(a), Value::Real(b)) => Ok(Value::Real(a + b)),
                (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{}{}", a, b))),
                _ => Err(self.err(span, "Type mismatch in addition")),
            },
            BinOp::Sub | BinOp::Mul | BinOp::Div => {
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
                    _ => Err(self.err(span, "Type mismatch in arithmetic")),
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

    fn call_math(&self, field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
        let mut it = args.into_iter();
        match field {
            "cos" | "sin" | "sqrt" | "floor" | "ceil" | "round" => {
                let x = it.next().ok_or_else(|| self.err(span, format!("math.{}() requires 1 argument", field)))?;
                let x = match x { Value::Int(i) => i as f64, Value::Real(r) => r, _ => return Err(self.err(span, "Expected number")) };
                let r = match field {
                    "cos" => x.cos(), "sin" => x.sin(), "sqrt" => x.sqrt(),
                    "floor" => x.floor(), "ceil" => x.ceil(), "round" => x.round(),
                    _ => unreachable!(),
                };
                Ok(Value::Real(r))
            }
            "abs" => {
                let x = it.next().ok_or_else(|| self.err(span, "math.abs() requires 1 argument"))?;
                match x {
                    Value::Int(i) => Ok(Value::Int(i.abs())),
                    Value::Real(r) => Ok(Value::Real(r.abs())),
                    _ => Err(self.err(span, "math.abs() requires a number")),
                }
            }
            "max" | "min" => {
                let a = it.next().ok_or_else(|| self.err(span, format!("math.{}() requires 2 arguments", field)))?;
                let b = it.next().ok_or_else(|| self.err(span, format!("math.{}() requires 2 arguments", field)))?;
                let to_f64 = |v: Value| -> Result<f64> { match v { Value::Int(i) => Ok(i as f64), Value::Real(r) => Ok(r), _ => Err(self.err(span, "Expected number")) } };
                let (af, bf) = (to_f64(a)?, to_f64(b)?);
                let r = match field { "max" => af.max(bf), _ => af.min(bf) };
                Ok(Value::Real(r))
            }
            "pow" => {
                let base = it.next().ok_or_else(|| self.err(span, "math.pow() requires 2 arguments"))?;
                let exp = it.next().ok_or_else(|| self.err(span, "math.pow() requires 2 arguments"))?;
                let to_f64 = |v: Value| -> Result<f64> { match v { Value::Int(i) => Ok(i as f64), Value::Real(r) => Ok(r), _ => Err(self.err(span, "Expected number")) } };
                Ok(Value::Real(to_f64(base)?.powf(to_f64(exp)?)))
            }
            "rand" => {
                let max = it.next();
                let r = if let Some(v) = max {
                    let m = match v { Value::Int(i) => i, _ => return Err(self.err(span, "math.rand() requires an integer")) };
                    fastrand::i32(0..m as i32) as i64
                } else {
                    fastrand::i32(0..i32::MAX) as i64
                };
                Ok(Value::Int(r))
            }
            _ => Err(self.err(span, format!("Unknown math function '{}'", field))),
        }
    }

    fn call_time(&self, field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
        match field {
            "now" => {
                Ok(Value::Str(format!("{}", std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default().as_secs())))
            }
            "sleep" => {
                let ms = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "time.sleep() requires 1 argument"))?;
                let ms = match ms { Value::Int(i) => i, _ => return Err(self.err(span, "time.sleep() requires an integer")) };
                std::thread::sleep(std::time::Duration::from_millis(ms as u64));
                Ok(Value::None_)
            }
            _ => Err(self.err(span, format!("Unknown time function '{}'", field))),
        }
    }

    fn call_json(&self, field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
        match field {
            "parse" => {
                let s = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "json.parse() requires 1 argument"))?;
                let s = match s { Value::Str(s) => s, _ => return Err(self.err(span, "json.parse() requires a string")) };
                let v: serde_json::Value = serde_json::from_str(&s)
                    .map_err(|e| self.err(span, format!("json.parse: {}", e)))?;
                Ok(json_to_value(v))
            }
            "stringify" => {
                let mut it2 = args.into_iter();
                let v = it2.next()
                    .ok_or_else(|| self.err(span, "json.stringify() requires 1 argument"))?;
                let pretty = it2.next();
                let is_pretty = matches!(pretty, Some(Value::Bool(true)));
                let json_val = value_to_json(&v);
                let s = if is_pretty { serde_json::to_string_pretty(&json_val) } else { serde_json::to_string(&json_val) };
                Ok(Value::Str(s.map_err(|e| self.err(span, format!("json.stringify: {}", e)))?))
            }
            _ => Err(self.err(span, format!("Unknown json function '{}'", field))),
        }
    }

    fn call_datetime(&self, field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
        let mut it = args.into_iter();
        match field {
            "now" => Ok(Value::Str(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())),
            "utc" => Ok(Value::Str(chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string())),
            "timestamp" => Ok(Value::Int(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64)),
            "format" => {
                let ts = it.next().ok_or_else(|| self.err(span, "datetime.format() requires 2 arguments"))?;
                let fmt = it.next().ok_or_else(|| self.err(span, "datetime.format() requires 2 arguments"))?;
                let ts = match ts { Value::Int(i) => i, _ => return Err(self.err(span, "datetime.format() timestamp must be an integer")) };
                let fmt = match fmt { Value::Str(s) => s, _ => return Err(self.err(span, "datetime.format() format must be a string")) };
                let dt = chrono::DateTime::from_timestamp(ts, 0)
                    .ok_or_else(|| self.err(span, "datetime.format() invalid timestamp"))?;
                let local: chrono::DateTime<chrono::Local> = chrono::DateTime::from(dt);
                Ok(Value::Str(local.format(&fmt).to_string()))
            }
            "parse" => {
                let s = it.next().ok_or_else(|| self.err(span, "datetime.parse() requires 2 arguments"))?;
                let fmt = it.next().ok_or_else(|| self.err(span, "datetime.parse() requires 2 arguments"))?;
                let s = match s { Value::Str(s) => s, _ => return Err(self.err(span, "datetime.parse() string must be a string")) };
                let fmt = match fmt { Value::Str(f) => f, _ => return Err(self.err(span, "datetime.parse() format must be a string")) };
                let dt = chrono::NaiveDateTime::parse_from_str(&s, &fmt)
                    .map_err(|e| self.err(span, format!("datetime.parse: {}", e)))?;
                Ok(Value::Int(dt.and_utc().timestamp()))
            }
            "year" | "month" | "day" | "hour" | "minute" | "second" => {
                let ts = it.next().ok_or_else(|| self.err(span, format!("datetime.{}() requires 1 argument", field)))?;
                let ts = match ts { Value::Int(i) => i, _ => return Err(self.err(span, "timestamp must be an integer")) };
                let dt = chrono::DateTime::from_timestamp(ts, 0)
                    .ok_or_else(|| self.err(span, "invalid timestamp"))?;
                let local: chrono::DateTime<chrono::Local> = chrono::DateTime::from(dt);
                let r = match field {
                    "year" => local.year() as i64,
                    "month" => local.month() as i64,
                    "day" => local.day() as i64,
                    "hour" => local.hour() as i64,
                    "minute" => local.minute() as i64,
                    "second" => local.second() as i64,
                    _ => unreachable!(),
                };
                Ok(Value::Int(r))
            }
            _ => Err(self.err(span, format!("Unknown datetime function '{}'", field))),
        }
    }

    fn call_path_module(&self, field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
        use std::path::Path;
        match field {
            "join" => {
                let parts: Vec<String> = args.into_iter().map(|v| v.to_string()).collect();
                let p: PathBuf = parts.iter().collect();
                Ok(Value::Str(p.to_string_lossy().to_string().replace('\\', "/")))
            }
            "dirname" => {
                let p = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "path.dirname() requires 1 argument"))?;
                let p = match p { Value::Str(s) => s, _ => return Err(self.err(span, "path.dirname() requires a string")) };
                match Path::new(&p).parent() {
                    Some(parent) => Ok(Value::Str(parent.to_string_lossy().to_string().replace('\\', "/"))),
                    None => Ok(Value::Str("".into())),
                }
            }
            "basename" => {
                let p = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "path.basename() requires 1 argument"))?;
                let p = match p { Value::Str(s) => s, _ => return Err(self.err(span, "path.basename() requires a string")) };
                match Path::new(&p).file_name() {
                    Some(name) => Ok(Value::Str(name.to_string_lossy().to_string())),
                    None => Ok(Value::Str("".into())),
                }
            }
            "extension" => {
                let p = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "path.extension() requires 1 argument"))?;
                let p = match p { Value::Str(s) => s, _ => return Err(self.err(span, "path.extension() requires a string")) };
                match Path::new(&p).extension() {
                    Some(ext) => Ok(Value::Str(ext.to_string_lossy().to_string())),
                    None => Ok(Value::Str("".into())),
                }
            }
            "is_absolute" => {
                let p = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "path.is_absolute() requires 1 argument"))?;
                let p = match p { Value::Str(s) => s, _ => return Err(self.err(span, "path.is_absolute() requires a string")) };
                Ok(Value::Bool(Path::new(&p).is_absolute()))
            }
            _ => Err(self.err(span, format!("Unknown path function '{}'", field))),
        }
    }

    fn call_base64(&self, field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
        match field {
            "encode" => {
                let s = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "base64.encode() requires 1 argument"))?;
                let s = match s { Value::Str(s) => s, _ => return Err(self.err(span, "base64.encode() requires a string")) };
                Ok(Value::Str(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, s.as_bytes())))
            }
            "decode" => {
                let s = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "base64.decode() requires 1 argument"))?;
                let s = match s { Value::Str(s) => s, _ => return Err(self.err(span, "base64.decode() requires a string")) };
                let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &s)
                    .map_err(|e| self.err(span, format!("base64.decode: {}", e)))?;
                Ok(Value::Str(String::from_utf8_lossy(&bytes).to_string()))
            }
            _ => Err(self.err(span, format!("Unknown base64 function '{}'", field))),
        }
    }

    fn call_re(&self, field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
        let mut it = args.into_iter();
        match field {
            "match" => {
                let pattern = it.next().ok_or_else(|| self.err(span, "re.match() requires 2 arguments"))?;
                let text = it.next().ok_or_else(|| self.err(span, "re.match() requires 2 arguments"))?;
                let pattern = match pattern { Value::Str(s) => s, _ => return Err(self.err(span, "re.match() pattern must be a string")) };
                let text = match text { Value::Str(s) => s, _ => return Err(self.err(span, "re.match() text must be a string")) };
                let re = regex::Regex::new(&pattern)
                    .map_err(|e| self.err(span, format!("re.match: {}", e)))?;
                Ok(Value::Bool(re.is_match(&text)))
            }
            "find" => {
                let pattern = it.next().ok_or_else(|| self.err(span, "re.find() requires 2 arguments"))?;
                let text = it.next().ok_or_else(|| self.err(span, "re.find() requires 2 arguments"))?;
                let pattern = match pattern { Value::Str(s) => s, _ => return Err(self.err(span, "re.find() pattern must be a string")) };
                let text = match text { Value::Str(s) => s, _ => return Err(self.err(span, "re.find() text must be a string")) };
                let re = regex::Regex::new(&pattern)
                    .map_err(|e| self.err(span, format!("re.find: {}", e)))?;
                let matches: Vec<Value> = re.find_iter(&text).map(|m| Value::Str(m.as_str().to_string())).collect();
                Ok(Value::List(matches))
            }
            "replace" => {
                let pattern = it.next().ok_or_else(|| self.err(span, "re.replace() requires 3 arguments"))?;
                let text = it.next().ok_or_else(|| self.err(span, "re.replace() requires 3 arguments"))?;
                let replacement = it.next().ok_or_else(|| self.err(span, "re.replace() requires 3 arguments"))?;
                let pattern = match pattern { Value::Str(s) => s, _ => return Err(self.err(span, "re.replace() pattern must be a string")) };
                let text = match text { Value::Str(s) => s, _ => return Err(self.err(span, "re.replace() text must be a string")) };
                let replacement = match replacement { Value::Str(s) => s, _ => return Err(self.err(span, "re.replace() replacement must be a string")) };
                let re = regex::Regex::new(&pattern)
                    .map_err(|e| self.err(span, format!("re.replace: {}", e)))?;
                Ok(Value::Str(re.replace_all(&text, replacement).to_string()))
            }
            "split" => {
                let pattern = it.next().ok_or_else(|| self.err(span, "re.split() requires 2 arguments"))?;
                let text = it.next().ok_or_else(|| self.err(span, "re.split() requires 2 arguments"))?;
                let pattern = match pattern { Value::Str(s) => s, _ => return Err(self.err(span, "re.split() pattern must be a string")) };
                let text = match text { Value::Str(s) => s, _ => return Err(self.err(span, "re.split() text must be a string")) };
                let re = regex::Regex::new(&pattern)
                    .map_err(|e| self.err(span, format!("re.split: {}", e)))?;
                let parts: Vec<Value> = re.split(&text).map(|p| Value::Str(p.to_string())).collect();
                Ok(Value::List(parts))
            }
            _ => Err(self.err(span, format!("Unknown re function '{}'", field))),
        }
    }

    fn call_ffi(&self, name: &str, field: &str, args: Vec<Value>, span: Span, ffi_path: &str) -> Result<Value> {
        let lib_name = format!("yk_ffi_{}", ffi_path.replace('/', "_").replace('.', ""));
        let lib = match self.ffi_libs.get(&lib_name) {
            Some(l) => l.clone(),
            None => return Err(self.err(span, format!("FFI library '{}' not loaded", lib_name))),
        };
        let func_name = format!("yk_{}_{}", name, field);
        let func: libloading::Symbol<unsafe extern "C" fn(i64) -> i64> = unsafe {
            match lib.get(func_name.as_bytes()) {
                Ok(f) => f,
                Err(_) => return Err(self.err(span, format!("FFI function '{}' not found in {}", func_name, lib_name))),
            }
        };
        let arg = args.first().map(|v| match v { Value::Int(i) => *i, _ => 0 }).unwrap_or(0);
        let result = unsafe { func(arg) };
        Ok(Value::Int(result))
    }

    fn call_fs(&self, field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
        match field {
            "read" => {
                let path = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "fs.read() requires 1 argument"))?;
                let path = match path {
                    Value::Str(s) => s,
                    _ => return Err(self.err(span, "fs.read() requires a string path")),
                };
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| self.err(span, format!("fs.read: {}", e)))?;
                Ok(Value::Str(content))
            }
            "write" => {
                let mut it = args.into_iter();
                let path = it.next()
                    .ok_or_else(|| self.err(span, "fs.write() requires 2 arguments"))?;
                let content = it.next()
                    .ok_or_else(|| self.err(span, "fs.write() requires 2 arguments"))?;
                let path = match path {
                    Value::Str(s) => s,
                    _ => return Err(self.err(span, "fs.write() path must be a string")),
                };
                let content = content.to_string();
                std::fs::write(&path, &content)
                    .map_err(|e| self.err(span, format!("fs.write: {}", e)))?;
                Ok(Value::None_)
            }
            "append" => {
                let mut it = args.into_iter();
                let path = it.next()
                    .ok_or_else(|| self.err(span, "fs.append() requires 2 arguments"))?;
                let content = it.next()
                    .ok_or_else(|| self.err(span, "fs.append() requires 2 arguments"))?;
                let path = match path {
                    Value::Str(s) => s,
                    _ => return Err(self.err(span, "fs.append() path must be a string")),
                };
                let content = content.to_string();
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .append(true).create(true).open(&path)
                    .map_err(|e| self.err(span, format!("fs.append: {}", e)))?;
                write!(file, "{}", content)
                    .map_err(|e| self.err(span, format!("fs.append: {}", e)))?;
                Ok(Value::None_)
            }
            "remove" => {
                let path = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "fs.remove() requires 1 argument"))?;
                let path = match path {
                    Value::Str(s) => s,
                    _ => return Err(self.err(span, "fs.remove() requires a string path")),
                };
                std::fs::remove_file(&path)
                    .map_err(|e| self.err(span, format!("fs.remove: {}", e)))?;
                Ok(Value::None_)
            }
            "exists" => {
                let path = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "fs.exists() requires 1 argument"))?;
                let path = match path {
                    Value::Str(s) => s,
                    _ => return Err(self.err(span, "fs.exists() requires a string path")),
                };
                Ok(Value::Bool(std::path::Path::new(&path).exists()))
            }
            "list" => {
                let dir = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "fs.list() requires 1 argument"))?;
                let dir = match dir {
                    Value::Str(s) => s,
                    _ => return Err(self.err(span, "fs.list() requires a string path")),
                };
                let entries = std::fs::read_dir(&dir)
                    .map_err(|e| self.err(span, format!("fs.list: {}", e)))?;
                let mut items = Vec::new();
                for entry in entries {
                    let entry = entry.map_err(|e| self.err(span, format!("fs.list: {}", e)))?;
                    items.push(Value::Str(entry.file_name().to_string_lossy().to_string()));
                }
                Ok(Value::List(items))
            }
            "is_dir" => {
                let path = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "fs.is_dir() requires 1 argument"))?;
                let path = match path {
                    Value::Str(s) => s,
                    _ => return Err(self.err(span, "fs.is_dir() requires a string path")),
                };
                Ok(Value::Bool(std::path::Path::new(&path).is_dir()))
            }
            "is_file" => {
                let path = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "fs.is_file() requires 1 argument"))?;
                let path = match path {
                    Value::Str(s) => s,
                    _ => return Err(self.err(span, "fs.is_file() requires a string path")),
                };
                Ok(Value::Bool(std::path::Path::new(&path).is_file()))
            }
            _ => Err(self.err(span, format!("Unknown fs function '{}'", field))),
        }
    }

    fn call_sys(&self, field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
        match field {
            "env" => {
                let name = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "sys.env() requires 1 argument"))?;
                let name = match name {
                    Value::Str(s) => s,
                    _ => return Err(self.err(span, "sys.env() requires a string name")),
                };
                match std::env::var(&name) {
                    Ok(val) => Ok(Value::Str(val)),
                    Err(_) => Ok(Value::None_),
                }
            }
            "args" => {
                let collected: Vec<Value> = std::env::args().map(Value::Str).collect();
                Ok(Value::List(collected))
            }
            "exit" => {
                let code = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "sys.exit() requires 1 argument"))?;
                let code = match code {
                    Value::Int(i) => i as i32,
                    _ => return Err(self.err(span, "sys.exit() requires an integer code")),
                };
                std::process::exit(code);
            }
            "cwd" => {
                match std::env::current_dir() {
                    Ok(p) => Ok(Value::Str(p.to_string_lossy().to_string())),
                    Err(e) => Err(self.err(span, format!("sys.cwd: {}", e))),
                }
            }
            "pid" => {
                Ok(Value::Int(std::process::id() as i64))
            }
            "platform" => {
                Ok(Value::Str(std::env::consts::OS.to_string()))
            }
            "sleep" => {
                let ms = args.into_iter().next()
                    .ok_or_else(|| self.err(span, "sys.sleep() requires 1 argument"))?;
                let ms = match ms {
                    Value::Int(i) => i,
                    _ => return Err(self.err(span, "sys.sleep() requires an integer")),
                };
                std::thread::sleep(std::time::Duration::from_millis(ms as u64));
                Ok(Value::None_)
            }
            _ => Err(self.err(span, format!("Unknown sys function '{}'", field))),
        }
    }

    fn err(&self, span: Span, msg: impl Into<String>) -> error::YkError {
        error::err(ErrorKind::Runtime, span, msg)
    }
}

fn json_to_value(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::None_,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() { Value::Int(i) }
            else if let Some(f) = n.as_f64() { Value::Real(f) }
            else { Value::None_ }
        }
        serde_json::Value::String(s) => Value::Str(s),
        serde_json::Value::Array(arr) => Value::List(arr.into_iter().map(json_to_value).collect()),
        serde_json::Value::Object(obj) => {
            Value::Map(obj.into_iter().map(|(k, v)| (k, json_to_value(v))).collect())
        }
    }
}

fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Int(i) => serde_json::Value::Number((*i).into()),
        Value::Real(f) => serde_json::Value::Number(serde_json::Number::from_f64(*f).unwrap_or(serde_json::Number::from(0))),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::List(items) => serde_json::Value::Array(items.iter().map(value_to_json).collect()),
        Value::Tuple(items) => serde_json::Value::Array(items.iter().map(value_to_json).collect()),
        Value::Dict(pairs) => serde_json::Value::Array(pairs.iter().map(|(k, v)| {
            serde_json::Value::Array(vec![value_to_json(k), value_to_json(v)])
        }).collect()),
        Value::Set(items) => serde_json::Value::Array(items.iter().map(value_to_json).collect()),
        Value::Map(m) => serde_json::Value::Object(m.iter().map(|(k, v)| (k.clone(), value_to_json(v))).collect()),
        Value::Struct(_, fields) => serde_json::Value::Object(fields.iter().map(|(k, v)| (k.clone(), value_to_json(v))).collect()),
        Value::None_ => serde_json::Value::Null,
        Value::Char(c) => serde_json::Value::String(c.to_string()),
        Value::Range(_, _) => serde_json::Value::Null,
    }
}

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Int(i) => *i != 0,
        Value::Real(f) => *f != 0.0,
        Value::None_ => false,
        _ => true,
    }
}

fn is_copy_value(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Real(_) | Value::Bool(_) | Value::Char(_) | Value::None_)
}

fn cmp_binop<F: Fn(f64, f64) -> bool>(a: &Value, b: &Value, cmp: F, span: Span) -> Result<Value> {
    let to_f64 = |v: &Value| -> Result<f64> {
        match v {
            Value::Int(i) => Ok(*i as f64),
            Value::Real(r) => Ok(*r),
            _ => Err(error::err(ErrorKind::TypeError, span, "Cannot compare")),
        }
    };
    let av = to_f64(a)?;
    let bv = to_f64(b)?;
    Ok(Value::Bool(cmp(av, bv)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::ast;
    use crate::syntax::parser::Parser;

    fn run(source: &str) -> String {
        ast::reset_ids();
        let module = Parser::parse(source).unwrap();
        let mut interp = Interpreter::new();
        interp.load_module(&module);
        interp.run_main().unwrap()
    }

    fn run_invalid(source: &str) -> String {
        ast::reset_ids();
        let module = Parser::parse(source).unwrap();
        let mut interp = Interpreter::new();
        interp.load_module(&module);
        interp.run_main().unwrap_err().to_string()
    }

    #[test]
    fn empty_main() {
        let out = run("fn main() {}");
        assert_eq!(out, "");
    }

    #[test]
    fn arithmetic() {
        let out = run("fn main() { x: int = 1 + 2 * 3; print(x); }");
        assert_eq!(out, "7\n");
    }

    #[test]
    fn variable_mutation() {
        let out = run("fn main() { x: int = 10; x = x + 5; print(x); }");
        assert_eq!(out, "15\n");
    }

    #[test]
    fn if_else_branch() {
        let out = run("fn main() { x: int = 5; if (x > 3) { print(\"big\"); } else { print(\"small\"); } }");
        assert_eq!(out, "big\n");
    }

    #[test]
    fn if_else_false_branch() {
        let out = run("fn main() { x: int = 1; if (x > 3) { print(\"big\"); } else { print(\"small\"); } }");
        assert_eq!(out, "small\n");
    }

    #[test]
    fn for_loop() {
        let out = run("fn main() { sum: int = 0; for (i in 0..5) { sum = sum + i; } print(sum); }");
        assert_eq!(out, "10\n");
    }

    #[test]
    fn while_loop() {
        let out = run("fn main() { x: int = 3; while (x > 0) { print(x); x = x - 1; } }");
        assert_eq!(out, "3\n2\n1\n");
    }

    #[test]
    fn function_call() {
        let out = run("fn double(x: int) -> int { return x * 2; } fn main() { print(double(21)); }");
        assert_eq!(out, "42\n");
    }

    #[test]
    fn nested_calls() {
        let out = run("fn add(a: int, b: int) -> int { return a + b; } fn main() { print(add(add(1, 2), 3)); }");
        assert_eq!(out, "6\n");
    }

    #[test]
    fn bool_operators() {
        let out = run("fn main() { t: bool = true; f: bool = false; print(t && f); print(t || f); print(!t); }");
        assert_eq!(out, "false\ntrue\nfalse\n");
    }

    #[test]
    fn comparison() {
        let out = run("fn main() { print(1 < 2); print(2 <= 2); print(3 > 4); }");
        assert_eq!(out, "true\ntrue\nfalse\n");
    }

    #[test]
    fn loop_break_through_return() {
        let out = run("fn main() { loop { print(\"once\"); return; } }");
        assert_eq!(out, "once\n");
    }

    #[test]
    fn scope_blocks() {
        let out = run("fn main() { x: int = 1; { x: int = 2; print(x); } print(x); }");
        assert_eq!(out, "2\n1\n");
    }

    #[test]
    fn const_global() {
        let out = run("const PI: int = 3; fn main() { print(PI); }");
        assert_eq!(out, "3\n");
    }

    #[test]
    fn for_with_range() {
        let out = run("fn main() { sum: int = 0; for (i in 1..4) { sum = sum + i; } print(sum); }");
        assert_eq!(out, "6\n");
    }

    #[test]
    fn string_concat() {
        let out = run("fn main() { s: str = \"Hello, \" + \"world!\"; print(s); }");
        assert_eq!(out, "Hello, world!\n");
    }

    #[test]
    fn list_literal() {
        let out = run("fn main() { items: auto = [1, 2, 3]; print(items); }");
        assert_eq!(out, "[1, 2, 3]\n");
    }

    #[test]
    fn list_index() {
        let out = run("fn main() { items: auto = [10, 20, 30]; print(items[1]); }");
        assert_eq!(out, "20\n");
    }

    #[test]
    fn list_index_out_of_bounds() {
        let r = run_invalid("fn main() { items: auto = [1, 2]; x: auto = items[5]; }");
        assert!(r.contains("out of bounds"), "should error: {}", r);
    }

    #[test]
    fn string_index() {
        let out = run("fn main() { s: str = \"hello\"; print(s[0]); }");
        assert_eq!(out, "h\n");
    }

    #[test]
    fn struct_literal_and_field() {
        let out = run("struct Point { x: int; y: int; } fn main() { p: auto = Point { x: 10, y: 20 }; print(p.x); print(p.y); }");
        assert_eq!(out, "10\n20\n");
    }

    #[test]
    fn struct_unknown_field_error() {
        let err = run_invalid("struct A { x: int; } fn main() { p: auto = A { z: 1 }; }");
        assert!(err.contains("has no field"), "should error on unknown field: {}", err);
    }

    #[test]
    fn tuple_literal_and_field() {
        let out = run("fn main() { print((10, 20, 30).0); print((10, 20, 30).1); print((10, 20, 30).2); }");
        assert_eq!(out, "10\n20\n30\n");
    }

    #[test]
    fn tuple_mixed_types() {
        let out = run(r#"fn main() { print((1, "hello", true).1); print((1, "hello", true).2); }"#);
        assert_eq!(out, "hello\ntrue\n");
    }

    #[test]
    fn map_literal() {
        let out = run(r#"fn main() { print(map { 1: "one", 2: "two" }); }"#);
        assert_eq!(out, "{1: one, 2: two}\n");
    }

    #[test]
    fn set_literal() {
        let out = run(r#"fn main() { print(set { 1, 2, 3 }); }"#);
        assert_eq!(out, "set{1, 2, 3}\n");
    }

    #[test]
    fn fn_literal() {
        let out = run(r#"fn main() { print(fn () -> Int { 42 }); }"#);
        assert_eq!(out, "42\n");
    }

    #[test]
    fn fstring_literal() {
        let out = run(r#"fn main() { name: str = "world"; print(f'hello {name}'); }"#);
        assert_eq!(out, "hello world\n");
    }

    #[test]
    fn fstring_expr() {
        let out = run(r#"fn main() { print(f'sum: {1 + 2}'); }"#);
        assert_eq!(out, "sum: 3\n");
    }

    #[test]
    fn backtick_string() {
        let out = run("fn main() { print(`hello`); }");
        assert_eq!(out, "hello\n");
    }

    #[test]
    fn struct_unknown_type_error() {
        let err = run_invalid("fn main() { p: auto = Bogus { x: 1 }; }");
        assert!(err.contains("Unknown struct"), "should error: {}", err);
    }

    #[test]
    fn reassign_const_local() {
        let err = run_invalid("fn main() { x: const = 5; x = 10; }");
        assert!(err.contains("Cannot assign to const"), "should error: {}", err);
    }

    #[test]
    fn reassign_global_const() {
        let err = run_invalid("const X: int = 42; fn main() { X = 99; }");
        assert!(err.contains("Cannot assign to const"), "should error: {}", err);
    }

    #[test]
    fn builtin_len_string() {
        let out = run("fn main() { print(len(\"hello\")); }");
        assert_eq!(out, "5\n");
    }

    #[test]
    fn builtin_len_list() {
        let out = run("fn main() { print(len([10, 20, 30])); }");
        assert_eq!(out, "3\n");
    }

    #[test]
    fn method_style_len() {
        let out = run("fn main() { s: str = \"abc\"; print(s.len()); }");
        assert_eq!(out, "3\n");
    }

    #[test]
    fn builtin_str() {
        let out = run("fn main() { print(str(42) + \"!\"); }");
        assert_eq!(out, "42!\n");
    }

    #[test]
    fn builtin_str_bool() {
        let out = run("fn main() { print(str(true)); }");
        assert_eq!(out, "true\n");
    }

    #[test]
    fn print_variadic() {
        let out = run(r#"fn main() { print("x =", 42, "y =", 7); }"#);
        assert_eq!(out, "x = 42 y = 7\n");
    }

    #[test]
    fn print_empty() {
        let out = run("fn main() { print(); }");
        assert_eq!(out, "\n");
    }

    #[test]
    fn list_push() {
        let out = run("fn main() { items: auto = [1, 2]; items.push(3); print(items); }");
        assert_eq!(out, "[1, 2, 3]\n");
    }

    #[test]
    fn list_pop() {
        let out = run("fn main() { items: auto = [1, 2, 3]; v: auto = items.pop(); print(v); print(items); }");
        assert_eq!(out, "3\n[1, 2]\n");
    }

    #[test]
    fn list_pop_empty_error() {
        let err = run_invalid("fn main() { items: auto = []; items.pop(); }");
        assert!(err.contains("empty list"), "should error: {}", err);
    }

    #[test]
    fn input_with_prompt() {
        // input() is tested implicitly; this only checks that calling it parses
        let out = run("fn main() { print(\"skip\"); }");
        assert_eq!(out, "skip\n");
    }
}
