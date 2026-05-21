use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::syntax::ast::*;

const RUNTIME_C: &str = r##"
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

typedef struct { char* data; int64_t len; } yk_string;

void yk_string_init_ptr(yk_string* s, const char* data, int64_t len) {
    s->data = (char*)data;
    s->len = len;
}

yk_string* yk_string_from_int(int64_t v) {
    char buf[64];
    int n = snprintf(buf, sizeof(buf), "%lld", (long long)v);
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = (char*)malloc(n + 1);
    memcpy(s->data, buf, n + 1);
    s->len = n;
    return s;
}

yk_string* yk_string_concat_ptr(yk_string* a, yk_string* b) {
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = (char*)malloc(a->len + b->len + 1);
    memcpy(s->data, a->data, a->len);
    memcpy(s->data + a->len, b->data, b->len);
    s->data[a->len + b->len] = '\0';
    s->len = a->len + b->len;
    return s;
}

int64_t yk_string_len_ptr(yk_string* s) { return s->len; }

void yk_print_int(int64_t v) { printf("%lld\n", (long long)v); }
void yk_print_real(double v) { printf("%g\n", v); }
void yk_print_bool(bool v) { printf("%s\n", v ? "true" : "false"); }
void yk_print_str_ptr(yk_string* s) { printf("%.*s\n", (int)s->len, s->data); }
"##;

pub struct LlvmCodegen {
    output: String,
    indent: usize,
    var_types: HashMap<String, String>,
    var_alloca: HashMap<String, String>,
    struct_defs: HashMap<String, Vec<(String, String)>>,
    tuple_type_names: HashMap<String, String>,
    tuple_types_output: Vec<String>,
    tuple_counter: usize,
    label_counter: usize,
    in_block: bool,
    string_constants: String,
}

impl LlvmCodegen {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            indent: 0,
            var_types: HashMap::new(),
            var_alloca: HashMap::new(),
            struct_defs: HashMap::new(),
            tuple_type_names: HashMap::new(),
            tuple_types_output: Vec::new(),
            tuple_counter: 0,
            label_counter: 0,
            in_block: false,
            string_constants: String::new(),
        }
    }

    fn e(&mut self, s: &str) {
        use std::fmt::Write;
        writeln!(self.output, "{}{}", "  ".repeat(self.indent), s).unwrap();
    }

    fn e_raw(&mut self, s: &str) {
        self.output.push_str(s);
        self.output.push('\n');
    }

    fn fresh_label(&mut self) -> String {
        let n = self.label_counter;
        self.label_counter += 1;
        format!("yk_{}", n)
    }

    fn ssa(&self, raw: &str) -> String {
        format!("%{}", raw)
    }

    fn make_string_slot(&mut self, s: &str) -> String {
        let lbl = self.fresh_label();
        let escaped = s.replace('\\', "\\\\").replace('\n', "\\0A").replace('"', "\\22");
        use std::fmt::Write;
        writeln!(self.string_constants, "@{} = private unnamed_addr constant [{} x i8] c\"{}\\00\", align 1", lbl, escaped.len() + 1, escaped).unwrap();

        let ptr = self.fresh_label();
        self.e(&format!("%{} = getelementptr inbounds [{} x i8], ptr @{}, i64 0, i64 0", ptr, escaped.len() + 1, lbl));
        let tmp = self.fresh_label();
        self.e(&format!("%{} = insertvalue %yk_string undef, ptr %{}, 0", tmp, ptr));
        let tmp2 = self.fresh_label();
        self.e(&format!("%{} = insertvalue %yk_string %{}, i64 {}, 1", tmp2, tmp, escaped.len()));
        self.ssa(&tmp2)
    }

    fn string_to_ptr(&mut self, val: &str) -> String {
        let slot = self.fresh_label();
        self.e(&format!("%{} = alloca %yk_string, align 8", slot));
        self.e(&format!("store %yk_string {}, ptr %{}", val, slot));
        self.ssa(&slot)
    }

    fn type_to_llvm(&self, te: &TypeExpr) -> String {
        match te {
            TypeExpr::Int(_) | TypeExpr::Rint(_) => "i64".into(),
            TypeExpr::Real(_) => "double".into(),
            TypeExpr::Bool => "i1".into(),
            TypeExpr::Str => "%yk_string".into(),
            TypeExpr::Named(name) => {
                if self.struct_defs.contains_key(name) {
                    format!("%struct.{}", name)
                } else {
                    "i64".into()
                }
            }
            _ => "i64".into(),
        }
    }

    fn get_or_create_tuple_type(&mut self, elem_types: &[String]) -> String {
        let sig = elem_types.join("_");
        if let Some(name) = self.tuple_type_names.get(&sig) {
            return name.clone();
        }
        let n = self.tuple_counter;
        self.tuple_counter += 1;
        let name = format!("%struct.__yk_t{}", n);
        self.tuple_type_names.insert(sig, name.clone());
        self.tuple_types_output.push(format!("{} = type {{ {} }}", name, elem_types.join(", ")));
        name
    }

    fn expr_type_str(&mut self, expr: &ExprNode) -> String {
        match &expr.value {
            Expr::LitInt(_) | Expr::LitHex(_) => "i64".into(),
            Expr::LitReal(_) => "double".into(),
            Expr::LitBool(_) => "i1".into(),
            Expr::LitStr(_) => "%yk_string".into(),
            Expr::LitSymbol(_) => "%yk_string".into(),
            Expr::Ident(name) => self.var_types.get(name).cloned().unwrap_or("i64".into()),
            Expr::BinOp(l, _, _) => {
                let lt = self.expr_type_str(l);
                if lt == "%yk_string" { "%yk_string".into() } else { lt }
            }
            Expr::UnOp(_, inner) => self.expr_type_str(inner),
            Expr::Call(_, _) => "i64".into(),
            Expr::StructLit(name, _) => format!("%struct.{}", name),
            Expr::TupleLit(items) => {
                let elem_types: Vec<String> = items.iter().map(|i| self.expr_type_str(i)).collect();
                self.get_or_create_tuple_type(&elem_types)
            }
            _ => "i64".into(),
        }
    }

    fn val_ty(&self, name: &str) -> String {
        self.var_types.get(name).cloned().unwrap_or("i64".into())
    }

    fn alloca_name(&self, var: &str) -> String {
        format!("%{}.ptr", var.replace('.', "_"))
    }

    fn value_name(&mut self, var: &str) -> String {
        let n = self.label_counter;
        self.label_counter += 1;
        format!("%{}.val_{}", var.replace('.', "_"), n)
    }

    pub fn compile_module(&mut self, module: &Module) -> String {
        self.e_raw("; LLVM IR generated by yidi");
        self.e_raw("target datalayout = \"e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128\"");
        self.e_raw("target triple = \"x86_64-pc-windows-msvc\"");
        self.e_raw("");

        self.e_raw("%yk_string = type { ptr, i64 }");
        self.e_raw("");

        for item in &module.items {
            if let ItemKind::Struct { name, fields, .. } = &item.value {
                let mut field_types = Vec::new();
                let mut field_llvm = Vec::new();
                for p in fields {
                    let ft = self.type_to_llvm(&p.type_expr.value);
                    field_llvm.push(ft.clone());
                    field_types.push((p.name.clone(), ft));
                }
                self.struct_defs.insert(name.clone(), field_types);
                self.e_raw(&format!("%struct.{} = type {{ {} }}", name, field_llvm.join(", ")));
            }
        }
        self.e_raw("");

        self.e_raw("declare void @yk_print_int(i64)");
        self.e_raw("declare void @yk_print_real(double)");
        self.e_raw("declare void @yk_print_bool(i1)");
        self.e_raw("declare void @yk_print_str_ptr(ptr)");
        self.e_raw("declare ptr @yk_string_from_int(i64)");
        self.e_raw("declare ptr @yk_string_concat_ptr(ptr, ptr)");
        self.e_raw("declare i64 @yk_string_len_ptr(ptr)");
        self.e_raw("");

        let has_main = module.items.iter().any(|item| matches!(&item.value, ItemKind::Fn { name, .. } if name == "main"));

        for item in &module.items {
            if let ItemKind::Fn { name, .. } = &item.value {
                if name != "main" {
                    self.compile_fn(item);
                }
            }
        }

        if has_main {
            let saved_types = self.var_types.clone();
            let saved_alloca = self.var_alloca.clone();
            let main_item = module.items.iter().find(|item| matches!(&item.value, ItemKind::Fn { name, .. } if name == "main")).unwrap();
            if let ItemKind::Fn { params, body, .. } = &main_item.value {
                self.e_raw("define i32 @main(i32 %argc, ptr %argv) {");
                self.indent += 1;
                let entry_ptr = self.fresh_label();
                self.e(&format!("%{} = alloca i32, align 4", entry_ptr));
                self.e(&format!("store i32 %argc, ptr %{}", entry_ptr));
                self.var_types.insert("argc".into(), "i32".into());
                self.var_alloca.insert("argc".into(), format!("%{}", entry_ptr));
                for p in params {
                    let ty = self.type_to_llvm(&p.type_expr.value);
                    let ptr = self.alloca_name(&p.name);
                    self.var_types.insert(p.name.clone(), ty);
                    self.var_alloca.insert(p.name.clone(), ptr.clone());
                }
                self.compile_fn_body(body);
                self.e("ret i32 0");
                self.indent -= 1;
                self.e_raw("}");
            }
            self.var_types = saved_types;
            self.var_alloca = saved_alloca;
        }

        let mut output = std::mem::take(&mut self.output);
        output.push_str(&self.string_constants);

        let type_defs = std::mem::take(&mut self.tuple_types_output);
        if !type_defs.is_empty() {
            let mut prefix = String::new();
            for td in &type_defs {
                prefix.push_str(td);
                prefix.push('\n');
            }
            prefix.push('\n');
            // Insert tuple type defs after %yk_string and before the rest
            if let Some(pos) = output.find("%yk_string = type { ptr, i64 }") {
                let after = pos + "%yk_string = type { ptr, i64 }".len();
                output.insert_str(after, &format!("\n{}", prefix));
            }
        }

        output
    }

    fn compile_fn(&mut self, item: &ItemNode) {
        let saved_types = self.var_types.clone();
        let saved_alloca = self.var_alloca.clone();
        if let ItemKind::Fn { name, params, ret_type, body, .. } = &item.value {
            let ret = ret_type.as_ref().map(|t| self.type_to_llvm(&t.value)).unwrap_or_else(|| "void".into());
            let param_types: Vec<String> = params.iter().map(|p| self.type_to_llvm(&p.type_expr.value)).collect();
            let param_list = param_types.join(", ");
            self.e_raw(&format!("define {} @{}({}) {{", ret, name, param_list));
            self.indent += 1;

            for (i, p) in params.iter().enumerate() {
                let ty = self.type_to_llvm(&p.type_expr.value);
                let ptr = self.alloca_name(&p.name);
                self.var_types.insert(p.name.clone(), ty.clone());
                self.var_alloca.insert(p.name.clone(), ptr.clone());
                self.e(&format!("{} = alloca {}, align 8", ptr, ty));
                self.e(&format!("store {} %{}, ptr {}", ty, i, ptr));
            }

            self.compile_fn_body(body);

            if ret == "void" {
                self.e("ret void");
            } else {
                self.e(&format!("ret {} 0", ret));
            }

            self.indent -= 1;
            self.e_raw("}");
            self.e_raw("");
        }
        self.var_types = saved_types;
        self.var_alloca = saved_alloca;
    }

    fn compile_fn_body(&mut self, body: &[StmtNode]) {
        for stmt in body {
            self.compile_stmt(stmt);
        }
    }

    fn compile_stmt(&mut self, stmt: &StmtNode) {
        match &stmt.value {
            Stmt::Decl { name, type_expr, value, .. } => {
                let ty = match type_expr {
                    Some(te) => self.type_to_llvm(&te.value),
                    None => self.expr_type_str(value),
                };
                let ptr = self.alloca_name(name);
                self.var_types.insert(name.clone(), ty.clone());
                self.var_alloca.insert(name.clone(), ptr.clone());
                self.e(&format!("{} = alloca {}, align 8", ptr, ty));

                let (val, val_ty) = self.compile_expr(value);

                if ty == "i1" && val_ty == "i64" {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = icmp ne i64 {}, 0", tmp, val));
                    self.e(&format!("store i1 %{}, ptr {}", tmp, ptr));
                } else if ty == "i64" && val_ty == "i1" {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = zext i1 {} to i64", tmp, val));
                    self.e(&format!("store i64 %{}, ptr {}", tmp, ptr));
                } else if ty != val_ty && ty == "i64" && val_ty.starts_with("i") {
                    let bits: u32 = val_ty[1..].parse().unwrap_or(1);
                    let tmp = self.fresh_label();
                    if bits <= 1 {
                        self.e(&format!("%{} = zext {} {} to i64", tmp, val_ty, val));
                    } else {
                        self.e(&format!("%{} = sext {} {} to i64", tmp, val_ty, val));
                    }
                    self.e(&format!("store i64 %{}, ptr {}", tmp, ptr));
                } else {
                    self.e(&format!("store {} {}, ptr {}", val_ty, val, ptr));
                }
            }
            Stmt::Assign(name, expr) => {
                let ptr = match self.var_alloca.get(name) {
                    Some(p) => p.clone(),
                    None => {
                        let p = self.alloca_name(name);
                        self.e(&format!("{} = alloca i64, align 8", p));
                        self.var_alloca.insert(name.clone(), p.clone());
                        p
                    }
                };
                let ty = self.val_ty(name);
                let (val, val_ty) = self.compile_expr(expr);
                if !ty.is_empty() && ty != val_ty {
                    if ty == "i1" && val_ty == "i64" {
                        let tmp = self.fresh_label();
                        self.e(&format!("%{} = icmp ne i64 {}, 0", tmp, val));
                        self.e(&format!("store i1 %{}, ptr {}", tmp, ptr));
                    } else {
                        self.e(&format!("store {} {}, ptr {}", val_ty, val, ptr));
                    }
                } else {
                    self.e(&format!("store {} {}, ptr {}", val_ty, val, ptr));
                }
            }
            Stmt::Expr(e) => {
                self.compile_expr(e);
            }
            Stmt::Return(e) => {
                match e {
                    Some(ex) => {
                        let (val, val_ty) = self.compile_expr(ex);
                        self.e(&format!("ret {} {}", val_ty, val));
                    }
                    None => self.e("ret void"),
                }
            }
            Stmt::If(cond, then_body, else_body) => {
                let then_label = self.fresh_label();
                let else_label = self.fresh_label();
                let merge_label = self.fresh_label();

                let (cond_val, _) = self.compile_expr(cond);

                self.e(&format!("br i1 {}, label %{}, label %{}", cond_val, then_label, else_label));
                self.e_raw(&format!("{}:", then_label));
                self.indent += 1;
                for s in then_body { self.compile_stmt(s); }
                self.e(&format!("br label %{}", merge_label));
                self.indent -= 1;

                self.e_raw(&format!("{}:", else_label));
                self.indent += 1;
                if let Some(eb) = else_body {
                    for s in eb { self.compile_stmt(s); }
                }
                self.e(&format!("br label %{}", merge_label));
                self.indent -= 1;

                self.e_raw(&format!("{}:", merge_label));
            }
            Stmt::While(cond, body) => {
                let head_label = self.fresh_label();
                let body_label = self.fresh_label();
                let exit_label = self.fresh_label();

                self.e(&format!("br label %{}", head_label));
                self.e_raw(&format!("{}:", head_label));
                let (cond_val, _) = self.compile_expr(cond);
                self.e(&format!("br i1 {}, label %{}, label %{}", cond_val, body_label, exit_label));

                self.e_raw(&format!("{}:", body_label));
                self.indent += 1;
                for s in body { self.compile_stmt(s); }
                self.e(&format!("br label %{}", head_label));
                self.indent -= 1;

                self.e_raw(&format!("{}:", exit_label));
            }
            Stmt::For(var, iter, body) => {
                let init_label = self.fresh_label();
                let cond_label = self.fresh_label();
                let body_label = self.fresh_label();
                let exit_label = self.fresh_label();

                let (end_val, _) = self.compile_expr(iter);

                self.e(&format!("br label %{}", init_label));
                self.e_raw(&format!("{}:", init_label));

                let ptr = self.alloca_name(var);
                self.var_types.insert(var.clone(), "i64".into());
                self.var_alloca.insert(var.clone(), ptr.clone());
                self.e(&format!("{} = alloca i64, align 8", ptr));
                self.e(&format!("store i64 0, ptr {}", ptr));

                self.e(&format!("br label %{}", cond_label));
                self.e_raw(&format!("{}:", cond_label));
                let v = self.value_name(var);
                self.e(&format!("{} = load i64, ptr {}", v, ptr));
                self.e(&format!("%cmp_{} = icmp slt i64 {}, {}", var, v, end_val));
                self.e(&format!("br i1 %cmp_{}, label %{}, label %{}", var, body_label, exit_label));

                self.e_raw(&format!("{}:", body_label));
                self.indent += 1;
                for s in body { self.compile_stmt(s); }
                let next_v = self.fresh_label();
                self.e(&format!("%{} = add i64 {}, 1", next_v, v));
                self.e(&format!("store i64 %{}, ptr {}", next_v, ptr));
                self.e(&format!("br label %{}", cond_label));
                self.indent -= 1;

                self.e_raw(&format!("{}:", exit_label));
            }
            Stmt::Loop(body) => {
                let loop_label = self.fresh_label();
                self.e(&format!("br label %{}", loop_label));
                self.e_raw(&format!("{}:", loop_label));
                self.indent += 1;
                for s in body { self.compile_stmt(s); }
                self.e(&format!("br label %{}", loop_label));
                self.indent -= 1;
            }
            Stmt::Destruct(_, expr) => {
                self.compile_expr(expr);
            }
        }
    }

    fn compile_expr(&mut self, expr: &ExprNode) -> (String, String) {
        match &expr.value {
            Expr::LitInt(n) => (n.to_string(), "i64".into()),
            Expr::LitHex(n) => (n.to_string(), "i64".into()),
            Expr::LitReal(n) => {
                let s = n.to_string();
                if s.contains('.') || s.contains('e') || s.contains('E') { (s, "double".into()) }
                else { (format!("{}.0", s), "double".into()) }
            }
            Expr::LitBool(true) => ("true".into(), "i1".into()),
            Expr::LitBool(false) => ("false".into(), "i1".into()),
            Expr::LitStr(s) => (self.make_string_slot(s), "%yk_string".into()),
            Expr::LitChar(c) => (format!("{}", *c as i64), "i64".into()),
            Expr::LitNull | Expr::LitNone => ("0".into(), "i64".into()),
            Expr::LitSymbol(s) => (self.make_string_slot(&format!(":{}", s)), "%yk_string".into()),
            Expr::Ident(name) => {
                let ptr_opt = self.var_alloca.get(name).cloned();
                let ty = self.val_ty(name);
                if let Some(ptr) = ptr_opt {
                    let val_name = self.value_name(name);
                    self.e(&format!("{} = load {}, ptr {}", val_name, ty, ptr));
                    (val_name, ty)
                } else {
                    (format!("%{}", name), ty)
                }
            }
            Expr::BinOp(l, op, r) => self.compile_binop(l, op, r),
            Expr::UnOp(op, inner) => {
                let (i, ty) = self.compile_expr(inner);
                let tmp = self.fresh_label();
                match op {
                    UnOp::Neg => {
                        if ty == "double" {
                            self.e(&format!("%{} = fsub double -0.0, {}", tmp, i));
                        } else {
                            self.e(&format!("%{} = sub {} 0, {}", tmp, ty, i));
                        }
                        (self.ssa(&tmp), ty)
                    }
                    UnOp::Not => {
                        if ty == "i1" {
                            self.e(&format!("%{} = xor i1 {}, true", tmp, i));
                        } else {
                            self.e(&format!("%{} = icmp eq i64 {}, 0", tmp, i));
                        }
                        (self.ssa(&tmp), "i1".into())
                    }
                }
            }
            Expr::Call(callee, args) => self.compile_call(callee, args),
            Expr::Field(obj, field) => {
                let (o, obj_ty) = self.compile_expr(obj);
                let tmp = self.fresh_label();
                if obj_ty == "%yk_string" {
                    let idx = if field == "data" { "0" } else { "1" };
                    self.e(&format!("%{} = extractvalue %yk_string {}, {}", tmp, o, idx));
                    if idx == "0" { (self.ssa(&tmp), "ptr".into()) } else { (self.ssa(&tmp), "i64".into()) }
                } else if let Some(struct_name) = obj_ty.strip_prefix("%struct.") {
                    let index: Option<usize> = if self.tuple_type_names.values().any(|n| n == &obj_ty) {
                        field.parse().ok()
                    } else {
                        self.struct_defs.get(struct_name).and_then(|def_fields| {
                            def_fields.iter().enumerate().find(|(_, (n, _))| n == field).map(|(i, _)| i)
                        })
                    };
                    if let Some(idx) = index {
                        self.e(&format!("%{} = extractvalue {} {}, {}", tmp, obj_ty, o, idx));
                        (self.ssa(&tmp), "i64".into())
                    } else {
                        (self.ssa(&tmp), "i64".into())
                    }
                } else {
                    (self.ssa(&tmp), "i64".into())
                }
            }
            Expr::Index(obj, index) => {
                let (o, _) = self.compile_expr(obj);
                let (i, _) = self.compile_expr(index);
                let tmp = self.fresh_label();
                self.e(&format!("%{} = getelementptr inbounds i64, ptr {}, i64 {}", tmp, o, i));
                let tmp2 = self.fresh_label();
                self.e(&format!("%{} = load i64, ptr %{}", tmp2, tmp));
                (self.ssa(&tmp2), "i64".into())
            }
            Expr::Range(l, r) => {
                let (_lv, _) = self.compile_expr(l);
                let (rv, _) = self.compile_expr(r);
                (rv, "i64".into())
            }
            Expr::Block(stmts) => {
                let ret_ptr = self.fresh_label();
                self.e(&format!("%{} = alloca i64, align 8", ret_ptr));
                self.e(&format!("store i64 0, ptr %{}", ret_ptr));

                let old_in_block = self.in_block;
                self.in_block = true;

                for s in stmts {
                    match &s.value {
                        Stmt::Return(e) => {
                            let (val, ty) = match e {
                                Some(ex) => self.compile_expr(ex),
                                None => ("0".into(), "i64".into()),
                            };
                            self.e(&format!("store {} {}, ptr %{}", ty, val, ret_ptr));
                            let end_lbl = self.fresh_label();
                            self.e(&format!("br label %{}", end_lbl));
                            self.e_raw(&format!("{}:", end_lbl));
                        }
                        _ => self.compile_stmt(s),
                    }
                }

                self.in_block = old_in_block;

                let load_lbl = self.fresh_label();
                self.e(&format!("%{} = load i64, ptr %{}", load_lbl, ret_ptr));
                (self.ssa(&load_lbl), "i64".into())
            }
            Expr::AsConst(inner) => self.compile_expr(inner),
            Expr::If(cond, then_e, else_e) => {
                let then_label = self.fresh_label();
                let else_label = self.fresh_label();
                let merge_label = self.fresh_label();

                let (cond_val, _) = self.compile_expr(cond);
                self.e(&format!("br i1 {}, label %{}, label %{}", cond_val, then_label, else_label));

                self.e_raw(&format!("{}:", then_label));
                self.indent += 1;
                let (t_val, t_ty) = self.compile_expr(then_e);
                self.e(&format!("br label %{}", merge_label));
                self.indent -= 1;

                self.e_raw(&format!("{}:", else_label));
                self.indent += 1;
                let (e_val, _e_ty) = match else_e {
                    Some(ex) => self.compile_expr(ex),
                    None => ("0".into(), "i64".into()),
                };
                self.e(&format!("br label %{}", merge_label));
                self.indent -= 1;

                self.e_raw(&format!("{}:", merge_label));
                let result = self.fresh_label();
                self.e(&format!("%{} = phi {} [ {}, %{} ], [ {}, %{} ]", result, t_ty, t_val, then_label, e_val, else_label));
                (self.ssa(&result), t_ty)
            }
            Expr::ListLit(items) => {
                if items.is_empty() { ("0".into(), "i64".into()) }
                else {
                    let mut result = ("0".into(), "i64".into());
                    for item in items {
                        result = self.compile_expr(item);
                    }
                    result
                }
            }
            Expr::StructLit(name, fields) => {
                let struct_ty = format!("%struct.{}", name);
                let mut agg = "undef".to_string();
                let defs = self.struct_defs.get(name).cloned();
                if let Some(def_fields) = defs {
                    for (idx, (fname, _fty)) in def_fields.iter().enumerate() {
                        if let Some((_, fexpr)) = fields.iter().find(|(n, _)| n == fname) {
                            let (fval, fty) = self.compile_expr(fexpr);
                            let tmp = self.fresh_label();
                            self.e(&format!("%{} = insertvalue {} {}, {} {}, {}", tmp, struct_ty, agg, fty, fval, idx));
                            agg = self.ssa(&tmp);
                        }
                    }
                }
                (agg, struct_ty)
            }
            Expr::TupleLit(items) => {
                let elem_types: Vec<String> = items.iter().map(|i| self.expr_type_str(i)).collect();
                let ty = self.get_or_create_tuple_type(&elem_types);
                let mut agg = "undef".to_string();
                for (idx, item) in items.iter().enumerate() {
                    let (fval, fty) = self.compile_expr(item);
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = insertvalue {} {}, {} {}, {}", tmp, ty, agg, fty, fval, idx));
                    agg = self.ssa(&tmp);
                }
                (agg, ty)
            }
            Expr::MapLit(pairs) => {
                if pairs.is_empty() { ("0".into(), "i64".into()) }
                else {
                    let mut result = ("0".into(), "i64".into());
                    for (k, v) in pairs {
                        self.compile_expr(k);
                        result = self.compile_expr(v);
                    }
                    result
                }
            }
            Expr::SetLit(items) => {
                if items.is_empty() { ("0".into(), "i64".into()) }
                else {
                    let mut result = ("0".into(), "i64".into());
                    for item in items {
                        result = self.compile_expr(item);
                    }
                    result
                }
            }
            Expr::FnLit(_, _, body) => self.compile_expr(body),
            Expr::Await(inner) | Expr::Spawn(inner) => self.compile_expr(inner),
            Expr::ResultOk(inner) | Expr::ResultErr(inner) => self.compile_expr(inner),
            Expr::Match(_, _) | Expr::ForIn(_, _, _) | Expr::While(_, _) | Expr::Loop(_) => ("0".into(), "i64".into()),
        }
    }

    fn compile_binop(&mut self, l: &ExprNode, op: &BinOp, r: &ExprNode) -> (String, String) {
        let lt = self.expr_type_str(l);
        let (lc, _) = self.compile_expr(l);
        let (rc, _) = self.compile_expr(r);

        let is_float = lt == "double";
        let (arith_op, cmp_op) = if is_float { ("f", "fcmp") } else { ("", "icmp") };

        let tmp = self.fresh_label();
        match op {
            BinOp::Add => {
                if lt == "%yk_string" {
                    let ptr_a = self.string_to_ptr(&lc);
                    let ptr_b = self.string_to_ptr(&rc);
                    self.e(&format!("%{} = call ptr @yk_string_concat_ptr(ptr {}, ptr {})", tmp, ptr_a, ptr_b));
                    let ptr_result = self.ssa(&tmp);
                    let loaded = self.fresh_label();
                    self.e(&format!("%{} = load %yk_string, ptr {}", loaded, ptr_result));
                    (self.ssa(&loaded), "%yk_string".into())
                } else {
                    self.e(&format!("%{} = {}add {} {}, {}", tmp, arith_op, lt, lc, rc));
                    (self.ssa(&tmp), lt)
                }
            }
            BinOp::Sub => {
                self.e(&format!("%{} = {}sub {} {}, {}", tmp, arith_op, lt, lc, rc));
                (self.ssa(&tmp), lt)
            }
            BinOp::Mul => {
                self.e(&format!("%{} = {}mul {} {}, {}", tmp, arith_op, lt, lc, rc));
                (self.ssa(&tmp), lt)
            }
            BinOp::Div => {
                if is_float {
                    self.e(&format!("%{} = fdiv {} {}, {}", tmp, lt, lc, rc));
                } else {
                    self.e(&format!("%{} = sdiv {} {}, {}", tmp, lt, lc, rc));
                }
                (self.ssa(&tmp), lt)
            }
            BinOp::Eq => {
                self.e(&format!("%{} = {} oeq {} {}, {}", tmp, cmp_op, lt, lc, rc));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Ne => {
                self.e(&format!("%{} = {} one {} {}, {}", tmp, cmp_op, lt, lc, rc));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Lt => {
                let cond = if is_float { "olt" } else { "slt" };
                self.e(&format!("%{} = {} {} {} {}, {}", tmp, cmp_op, cond, lt, lc, rc));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Gt => {
                let cond = if is_float { "ogt" } else { "sgt" };
                self.e(&format!("%{} = {} {} {} {}, {}", tmp, cmp_op, cond, lt, lc, rc));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Le => {
                let cond = if is_float { "ole" } else { "sle" };
                self.e(&format!("%{} = {} {} {} {}, {}", tmp, cmp_op, cond, lt, lc, rc));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Ge => {
                let cond = if is_float { "oge" } else { "sge" };
                self.e(&format!("%{} = {} {} {} {}, {}", tmp, cmp_op, cond, lt, lc, rc));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::And => {
                let z1 = self.fresh_label();
                let z2 = self.fresh_label();
                self.e(&format!("%{} = icmp ne {} {}, 0", z1, lt, lc));
                self.e(&format!("%{} = icmp ne {} {}, 0", z2, lt, rc));
                self.e(&format!("%{} = and i1 %{}, %{}", tmp, z1, z2));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Or => {
                let z1 = self.fresh_label();
                let z2 = self.fresh_label();
                self.e(&format!("%{} = icmp ne {} {}, 0", z1, lt, lc));
                self.e(&format!("%{} = icmp ne {} {}, 0", z2, lt, rc));
                self.e(&format!("%{} = or i1 %{}, %{}", tmp, z1, z2));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Assign => {
                // For compound assignment, emit store
                self.e(&format!("store {} {}, ptr {}", lt, rc, lc));
                (rc, lt)
            }
        }
    }

    fn compile_call(&mut self, callee: &ExprNode, args: &[ExprNode]) -> (String, String) {
        let arg_results: Vec<(String, String)> = args.iter().map(|a| self.compile_expr(a)).collect();

        match &callee.value {
            Expr::Ident(name) => match name.as_str() {
                "print" | "println" => {
                    if arg_results.is_empty() {
                        ("0".into(), "void".into())
                    } else {
                        for (av, at) in &arg_results {
                            match at.as_str() {
                                "i64" => self.e(&format!("call void @yk_print_int(i64 {})", av)),
                                "double" => self.e(&format!("call void @yk_print_real(double {})", av)),
                                "i1" => self.e(&format!("call void @yk_print_bool(i1 {})", av)),
                                "%yk_string" => {
                                    let p = self.string_to_ptr(av);
                                    self.e(&format!("call void @yk_print_str_ptr(ptr {})", p));
                                }
                                _ => self.e(&format!("call void @yk_print_int(i64 {})", av)),
                            }
                        }
                        ("0".into(), "void".into())
                    }
                }
                "len" => {
                    if let Some((av, at)) = arg_results.first() {
                        if at == "%yk_string" {
                            let p = self.string_to_ptr(av);
                            let tmp = self.fresh_label();
                            self.e(&format!("%{} = call i64 @yk_string_len_ptr(ptr {})", tmp, p));
                            (self.ssa(&tmp), "i64".into())
                        } else {
                            ("0".into(), "i64".into())
                        }
                    } else { ("0".into(), "i64".into()) }
                }
                _ => {
                    let tmp = self.fresh_label();
                    let args_str: Vec<String> = arg_results.iter().map(|(v, t)| format!("{} {}", t, v)).collect();
                    self.e(&format!("%{} = call i64 @{}({})", tmp, name, args_str.join(", ")));
                    (self.ssa(&tmp), "i64".into())
                }
            },
            Expr::Field(obj, _field) => {
                let (o, _) = self.compile_expr(obj);
                let tmp = self.fresh_label();
                let args_str: Vec<String> = arg_results.iter().map(|(v, t)| format!("{} {}", t, v)).collect();
                self.e(&format!("%{} = call i64 @{}({})", tmp, o, args_str.join(", ")));
                (self.ssa(&tmp), "i64".into())
            }
            _ => ("0".into(), "i64".into()),
        }
    }
}

pub fn compile_to_llvm(module: &Module) -> String {
    let mut codegen = LlvmCodegen::new();
    codegen.compile_module(module)
}

pub fn compile_to_exe(llvm_ir: &str, output_path: &Path) -> Result<()> {
    let ll_path = output_path.with_extension("ll");
    std::fs::write(&ll_path, llvm_ir)
        .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
            format!("Failed to write {}: {}", ll_path.display(), e)))?;

    let obj_path = output_path.with_extension("obj");
    let exe_path = output_path.with_extension("exe");

    let runtime_dir = output_path.parent().unwrap_or(Path::new("."));
    let runtime_c_path = runtime_dir.join("yk_rt.c");
    std::fs::write(&runtime_c_path, RUNTIME_C)
        .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
            format!("Failed to write {}: {}", runtime_c_path.display(), e)))?;
    let runtime_obj_path = runtime_dir.join("yk_rt.obj");

    let vcvars = r"C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat";

    let bat_dir = std::env::temp_dir();
    let bat_path = bat_dir.join("yk_build.bat");
    let exe_str = exe_path.to_string_lossy();
    let ll_str = ll_path.to_string_lossy();
    let obj_str = obj_path.to_string_lossy();
    let rt_c_str = runtime_c_path.to_string_lossy();
    let rt_obj_str = runtime_obj_path.to_string_lossy();

    let bat_content = format!(
        r#"@echo off
call "{}" x64 >nul 2>&1
if errorlevel 1 exit /b 1

:: Compile LLVM IR to object file (with optimization)
clang.exe -c "{}" -o "{}" -target x86_64-pc-windows-msvc -O3
if errorlevel 1 exit /b 1

:: Compile runtime C to object file
cl.exe /nologo /std:c11 /c "{}" /Fo:"{}" /utf-8
if errorlevel 1 exit /b 1

:: Link objects into executable
link.exe /nologo "{}" "{}" /OUT:"{}" /defaultlib:libcmt.lib
exit /b %errorlevel%
"#, vcvars, ll_str, obj_str, rt_c_str, rt_obj_str, obj_str, rt_obj_str, exe_str);

    std::fs::write(&bat_path, bat_content)
        .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
            format!("Failed to write build script: {}", e)))?;

    let result = Command::new("cmd.exe")
        .args(["/c", &bat_path.to_string_lossy()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| {
            let _ = std::fs::remove_file(&bat_path);
            error::err(ErrorKind::Io, Span::new(0, 0),
                format!("Failed to invoke build: {}", e))
        })?;

    let _ = std::fs::remove_file(&bat_path);

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        let exit_code = result.status.code().unwrap_or(-1);
        return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
            format!("Compilation failed (exit={}):\nSTDOUT:\n{}\nSTDERR:\n{}",
                exit_code, stdout, stderr)));
    }

    let _ = std::fs::remove_file(&ll_path);
    let _ = std::fs::remove_file(&runtime_c_path);
    let _ = std::fs::remove_file(&runtime_obj_path);
    let _ = std::fs::remove_file(&obj_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parser::Parser;
    use crate::syntax::ast;

    #[test]
    fn test_struct_lit_and_field() {
        ast::reset_ids();
        let module = Parser::parse("struct Point { x: int, y: int } fn main() { p: Point = Point { x: 1, y: 2 }; print(p.x); }").unwrap();
        let llvm = compile_to_llvm(&module);
        assert!(llvm.contains("%struct.Point = type { i64, i64 }"));
        assert!(llvm.contains("insertvalue %struct.Point undef, i64 1, 0"));
        assert!(llvm.contains("insertvalue %struct.Point %"));
        assert!(llvm.contains("extractvalue %struct.Point"));
    }

    #[test]
    fn test_tuple_lit() {
        ast::reset_ids();
        let module = Parser::parse("fn main() { print((10, 20, 30).0); }").unwrap();
        let llvm = compile_to_llvm(&module);
        eprintln!("LLVM OUTPUT:\n{}", llvm);
        assert!(llvm.contains("i64, i64, i64"));
        assert!(llvm.contains("insertvalue"));
    }
}
