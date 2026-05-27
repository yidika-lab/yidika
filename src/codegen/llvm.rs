use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::hardware::HardwareInfo;
use crate::syntax::ast::*;

const RUNTIME_C: &str = r##"
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <math.h>

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

yk_string* yk_string_from_real(double v) {
    char buf[64];
    int n = snprintf(buf, sizeof(buf), "%g", v);
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

typedef struct { double real; double imag; } yk_complex;

void yk_complex_set(yk_complex* c, double r, double i) { c->real = r; c->imag = i; }
double yk_complex_real(yk_complex* c) { return c->real; }
double yk_complex_imag(yk_complex* c) { return c->imag; }
double yk_complex_mod(yk_complex* c) { return sqrt(c->real * c->real + c->imag * c->imag); }
double yk_complex_arg(yk_complex* c) { return atan2(c->imag, c->real); }
void yk_complex_conj(yk_complex* r, yk_complex* c) { r->real = c->real; r->imag = -c->imag; }
void yk_complex_add(yk_complex* r, yk_complex* a, yk_complex* b) { r->real = a->real + b->real; r->imag = a->imag + b->imag; }
void yk_complex_sub(yk_complex* r, yk_complex* a, yk_complex* b) { r->real = a->real - b->real; r->imag = a->imag - b->imag; }
void yk_complex_mul(yk_complex* r, yk_complex* a, yk_complex* b) { r->real = a->real*b->real - a->imag*b->imag; r->imag = a->real*b->imag + a->imag*b->real; }
void yk_complex_div(yk_complex* r, yk_complex* a, yk_complex* b) { double d = b->real*b->real + b->imag*b->imag; r->real = (a->real*b->real + a->imag*b->imag)/d; r->imag = (a->imag*b->real - a->real*b->imag)/d; }
void yk_print_complex(yk_complex* c) { printf("%g + %gi\n", c->real, c->imag); }

yk_string* yk_string_from_bool(bool v) {
    const char* s = v ? "true" : "false";
    yk_string* r = (yk_string*)malloc(sizeof(yk_string));
    int n = (int)strlen(s);
    r->data = (char*)malloc(n + 1);
    memcpy(r->data, s, n + 1);
    r->len = n;
    return r;
}

yk_string* yk_string_from_complex(yk_complex* c) {
    char buf[128];
    int n;
    if (c->imag < 0)
        n = snprintf(buf, sizeof(buf), "%g%gi", c->real, c->imag);
    else
        n = snprintf(buf, sizeof(buf), "%g+%gi", c->real, c->imag);
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = (char*)malloc(n + 1);
    memcpy(s->data, buf, n + 1);
    s->len = n;
    return s;
}
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
    nullable_types: HashSet<String>,
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
            nullable_types: HashSet::new(),
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

    fn type_to_llvm(&mut self, te: &TypeExpr) -> String {
        match te {
            TypeExpr::Int(_) | TypeExpr::Rint(_) => "i64".into(),
            TypeExpr::Real(_) => "double".into(),
            TypeExpr::Complex(_, _) => "%yk_complex".into(),
            TypeExpr::Bool => "i1".into(),
            TypeExpr::Str => "%yk_string".into(),
            TypeExpr::Vector(inner) => {
                let inner_ty = self.type_to_llvm(inner);
                format!("<2 x {}>", inner_ty)
            }
            TypeExpr::Matrix(inner) => {
                let inner_ty = self.type_to_llvm(inner);
                format!("[<2 x {}> x 2]", inner_ty)
            }
            TypeExpr::Named(name) => {
                if self.struct_defs.contains_key(name) {
                    format!("%struct.{}", name)
                } else {
                    "i64".into()
                }
            }
            TypeExpr::Nullable(inner) => {
                let inner_ty = self.type_to_llvm(inner);
                // Use a named struct type for the nullable wrapper
                let name = format!("%__nullable_{}", inner_ty.replace(|c: char| !c.is_alphanumeric(), "_"));
                if !self.nullable_types.contains(&name) {
                    self.nullable_types.insert(name.clone());
                    self.tuple_types_output.push(format!("{} = type {{ {}, i1 }}", name, inner_ty));
                }
                name
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
            Expr::BinOp(l, _op, r) => {
                let lt = self.expr_type_str(l);
                if lt == "%yk_string" { return "%yk_string".into(); }
                let rt = self.expr_type_str(r);
                if lt == "%yk_complex" || rt == "%yk_complex" { "%yk_complex".into() }
                else if lt == "double" || rt == "double" { "double".into() }
                else { lt }
            }
            Expr::UnOp(_, inner) => self.expr_type_str(inner),
            Expr::Call(_, _) => "i64".into(),
            Expr::StructLit(name, _) => format!("%struct.{}", name),
            Expr::TupleLit(items) => {
                let elem_types: Vec<String> = items.iter().map(|i| self.expr_type_str(i)).collect();
                self.get_or_create_tuple_type(&elem_types)
            }
            Expr::LitComplex(_, _) => "%yk_complex".into(),
            Expr::Field(obj, field) => {
                let ot = self.expr_type_str(obj);
                if ot == "%yk_complex" {
                    if field == "conj" { "%yk_complex".into() } else { "double".into() }
                } else if ot == "%yk_string" {
                    "i64".into()
                } else {
                    "i64".into()
                }
            }
            Expr::PostInc(i) | Expr::PostDec(i) => self.expr_type_str(i),
            Expr::Match(_, arms) => arms.first().map(|a| self.expr_type_str(&a.body)).unwrap_or("i64".into()),
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
        self.e_raw("%yk_complex = type { double, double }");
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
                let align_attr = item.decorators.iter()
                    .find_map(|d| d.strip_prefix("align(").and_then(|s| s.strip_suffix(')')))
                    .map(|n| format!(", align {}", n))
                    .unwrap_or_default();
                self.e_raw(&format!("%struct.{} = type {{ {} }}{}", name, field_llvm.join(", "), align_attr));
            }
        }
        self.e_raw("");

        self.e_raw("declare void @yk_print_int(i64)");
        self.e_raw("declare void @yk_print_real(double)");
        self.e_raw("declare void @yk_print_bool(i1)");
        self.e_raw("declare void @yk_print_str_ptr(ptr)");
        self.e_raw("declare ptr @yk_string_from_int(i64)");
        self.e_raw("declare ptr @yk_string_from_real(double)");
        self.e_raw("declare ptr @yk_string_from_bool(i1)");
        self.e_raw("declare ptr @yk_string_from_complex(ptr)");
        self.e_raw("declare ptr @yk_string_concat_ptr(ptr, ptr)");
        self.e_raw("declare i64 @yk_string_len_ptr(ptr)");
        self.e_raw("declare void @yk_complex_set(ptr, double, double)");
        self.e_raw("declare double @yk_complex_real(ptr)");
        self.e_raw("declare double @yk_complex_imag(ptr)");
        self.e_raw("declare double @yk_complex_mod(ptr)");
        self.e_raw("declare double @yk_complex_arg(ptr)");
        self.e_raw("declare void @yk_complex_conj(ptr, ptr)");
        self.e_raw("declare void @yk_complex_add(ptr, ptr, ptr)");
        self.e_raw("declare void @yk_complex_sub(ptr, ptr, ptr)");
        self.e_raw("declare void @yk_complex_mul(ptr, ptr, ptr)");
        self.e_raw("declare void @yk_complex_div(ptr, ptr, ptr)");
        self.e_raw("declare void @yk_print_complex(ptr)");
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
                let val_ty = self.expr_type_str(expr);
                let ptr = match self.var_alloca.get(name) {
                    Some(p) => p.clone(),
                    None => {
                        let p = self.alloca_name(name);
                        self.e(&format!("{} = alloca {}, align 8", p, val_ty));
                        self.var_alloca.insert(name.clone(), p.clone());
                        self.var_types.insert(name.clone(), val_ty.clone());
                        p
                    }
                };
                let ty = self.val_ty(name);
                let (val, val_ty2) = self.compile_expr(expr);
                let val_ty = if val_ty2 != "i64" { val_ty2 } else { val_ty };
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
                } else if obj_ty == "%yk_complex" {
                    // Store complex value to alloca to get a pointer for runtime
                    let ca = self.fresh_label();
                    self.e(&format!("%{} = alloca %yk_complex, align 8", ca));
                    self.e(&format!("store %yk_complex {}, ptr %{}", o, ca));
                    if field == "conj" {
                        let r = self.fresh_label();
                        self.e(&format!("%{} = alloca %yk_complex, align 8", r));
                        self.e(&format!("call void @yk_complex_conj(ptr %{}, ptr %{})", r, ca));
                        let loaded = self.fresh_label();
                        self.e(&format!("%{} = load %yk_complex, ptr %{}", loaded, r));
                        return (self.ssa(&loaded), "%yk_complex".into());
                    }
                    let func = match field.as_str() {
                        "real" => "yk_complex_real",
                        "img" => "yk_complex_imag",
                        "mod" | "norm" => "yk_complex_mod",
                        "arg" => "yk_complex_arg",
                        _ => "yk_complex_real",
                    };
                    self.e(&format!("%{} = call double @{}(ptr %{})", tmp, func, ca));
                    (self.ssa(&tmp), "double".into())
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
            Expr::Match(scrutinee, arms) => {
                let (sv, st) = self.compile_expr(scrutinee);
                let result_ty = arms.first().map(|a| self.expr_type_str(&a.body)).unwrap_or("i64".into());
                let result_ptr = self.fresh_label();
                self.e(&format!("%{} = alloca {}, align 8", result_ptr, result_ty));
                let merge_label = self.fresh_label();
                let arm_labels: Vec<String> = arms.iter().map(|_| self.fresh_label()).collect();
                for (idx, arm) in arms.iter().enumerate() {
                    let match_cond = self.compile_pattern_match(&arm.pattern, &sv, &st);
                    if let Some(ref cond) = match_cond {
                        let next_target = if idx + 1 < arm_labels.len() { &arm_labels[idx + 1] } else { &merge_label };
                        self.e(&format!("br i1 {}, label %{}, label %{}", cond, arm_labels[idx], next_target));
                    } else {
                        self.e(&format!("br label %{}", arm_labels[idx]));
                    }
                    self.e_raw(&format!("{}:", arm_labels[idx]));
                    self.compile_pattern_bind(&arm.pattern, &sv, &st);
                    let (body_val, body_ty) = self.compile_expr(&arm.body);
                    self.e(&format!("store {} {}, ptr %{}", body_ty, body_val, result_ptr));
                    self.e(&format!("br label %{}", merge_label));
                }
                self.e_raw(&format!("{}:", merge_label));
                let result_val = self.fresh_label();
                self.e(&format!("%{} = load {}, ptr %{}", result_val, result_ty, result_ptr));
                (self.ssa(&result_val), result_ty)
            }
            Expr::ForIn(_, _, _) | Expr::While(_, _) | Expr::Loop(_) => ("0".into(), "i64".into()),
            Expr::LitComplex(r, im) => {
                let cptr = self.fresh_label();
                self.e(&format!("%{} = alloca %yk_complex, align 8", cptr));
                let (rv, rt) = self.compile_expr(r);
                let (iv, it) = self.compile_expr(im);
                let rv_conv = if rt == "i64" {
                    let t = self.fresh_label();
                    self.e(&format!("%{} = sitofp i64 {} to double", t, rv));
                    self.ssa(&t)
                } else { rv.clone() };
                let iv_conv = if it == "i64" {
                    let t = self.fresh_label();
                    self.e(&format!("%{} = sitofp i64 {} to double", t, iv));
                    self.ssa(&t)
                } else { iv.clone() };
                self.e(&format!("call void @yk_complex_set(ptr %{}, double {}, double {})", cptr, rv_conv, iv_conv));
                let loaded = self.fresh_label();
                self.e(&format!("%{} = load %yk_complex, ptr %{}", loaded, cptr));
                (self.ssa(&loaded), "%yk_complex".into())
            }
            Expr::VectorLit(items) => {
                if items.is_empty() {
                    ("zeroinitializer".into(), "<2 x double>".into())
                } else {
                    let first = self.compile_expr(&items[0]);
                    let vty = format!("<{} x {}>", items.len(), first.1);
                    let mut result = format!("{} undef", vty);
                    for (i, item) in items.iter().enumerate() {
                        let (val, _) = self.compile_expr(item);
                        let lbl = self.fresh_label();
                        self.e(&format!("%{} = insertelement {} {}, {} {}", lbl, vty, self.ssa(&result), val, i));
                        result = format!("%{}", lbl);
                    }
                    (self.ssa(&result), vty)
                }
            }
            Expr::MatrixLit(rows) => {
                if rows.is_empty() || rows[0].is_empty() {
                    ("zeroinitializer".into(), "[<2 x double> x 0]".into())
                } else {
                    let first = self.compile_expr(&rows[0][0]);
                    let vty = format!("<{} x {}>", rows[0].len(), first.1);
                    let mty = format!("[{} x {}]", vty, rows.len());
                    let mut result = format!("{} undef", mty);
                    for (i, row) in rows.iter().enumerate() {
                        let mut row_val = format!("{} undef", vty);
                        for (j, item) in row.iter().enumerate() {
                            let (val, _) = self.compile_expr(item);
                            let lbl = self.fresh_label();
                            self.e(&format!("%{} = insertelement {} {}, {} {}", lbl, vty, self.ssa(&row_val), val, j));
                            row_val = format!("%{}", lbl);
                        }
                        let lbl2 = self.fresh_label();
                        self.e(&format!("%{} = insertvalue {} {}, {} {}", lbl2, mty, self.ssa(&result), self.ssa(&row_val), i));
                        result = format!("%{}", lbl2);
                    }
                    (self.ssa(&result), mty)
                }
            }
            Expr::PostInc(i) | Expr::PostDec(i) => self.compile_expr(i),
            Expr::SafeCall(obj, field) => {
                let (obj_val, obj_ty) = self.compile_expr(obj);
                // Extract the null flag (i1, second element)
                let null_flag = self.fresh_label();
                self.e(&format!("%{} = extractvalue {} {}, 1", null_flag, obj_ty, obj_val));
                // Check if valid (1 = non-null)
                let is_valid = self.fresh_label();
                self.e(&format!("%{} = icmp eq i1 {}, 1", is_valid, self.ssa(&null_flag)));
                let null_bb = self.fresh_label();
                let valid_bb = self.fresh_label();
                let merge_bb = self.fresh_label();
                self.e(&format!("br i1 {}, label %{}, label %{}", self.ssa(&is_valid), valid_bb, null_bb));
                // Null branch: return zero-initialized nullable
                self.e(&format!("{}:", null_bb));
                let null_result = self.fresh_label();
                // We need the field type - infer from struct definitions
                let inner_obj_ty = obj_ty.trim_start_matches("%__nullable_");
                let field_ty = self.guess_field_type(&inner_obj_ty, field);
                let nullable_field_ty = format!("%__nullable_{}", field_ty.replace(|c: char| !c.is_alphanumeric(), "_"));
                self.e(&format!("%{} = insertvalue {} undef, i1 0, 1", null_result, nullable_field_ty));
                let null_val = self.ssa(&null_result);
                self.e(&format!("br label %{}", merge_bb));
                // Valid branch: extract inner, access field, wrap in nullable
                self.e(&format!("{}:", valid_bb));
                let inner = self.fresh_label();
                self.e(&format!("%{} = extractvalue {} {}, 0", inner, obj_ty, obj_val));
                let inner_val = self.ssa(&inner);
                let (field_val, _) = self.compile_field_access(&inner_val, &inner_obj_ty, field);
                let valid_result = self.fresh_label();
                self.e(&format!("%{} = insertvalue {} undef, {} {}, 0", valid_result, nullable_field_ty, field_ty, field_val));
                let valid_result2 = self.fresh_label();
                self.e(&format!("%{} = insertvalue {} %{}, i1 1, 1", valid_result2, nullable_field_ty, self.ssa(&valid_result)));
                let valid_val = self.ssa(&valid_result2);
                self.e(&format!("br label %{}", merge_bb));
                // Merge
                self.e(&format!("{}:", merge_bb));
                let phi = self.fresh_label();
                self.e(&format!("%{} = phi {} [ %{}, %{} ], [ %{}, %{} ]", phi, nullable_field_ty, null_val, null_bb, valid_val, valid_bb));
                (self.ssa(&phi), nullable_field_ty)
            }
            Expr::Elvis(a, b) => {
                let (a_val, a_ty) = self.compile_expr(a);
                let null_flag = self.fresh_label();
                self.e(&format!("%{} = extractvalue {} {}, 1", null_flag, a_ty, a_val));
                let is_null = self.fresh_label();
                self.e(&format!("%{} = icmp eq i1 {}, 0", is_null, self.ssa(&null_flag)));
                let null_bb = self.fresh_label();
                let nonnull_bb = self.fresh_label();
                let merge_bb = self.fresh_label();
                self.e(&format!("br i1 {}, label %{}, label %{}", self.ssa(&is_null), null_bb, nonnull_bb));
                self.e(&format!("{}:", null_bb));
                let (b_val, b_ty) = self.compile_expr(b);
                self.e(&format!("br label %{}", merge_bb));
                self.e(&format!("{}:", nonnull_bb));
                let inner = self.fresh_label();
                self.e(&format!("%{} = extractvalue {} {}, 0", inner, a_ty, a_val));
                let inner_val = self.ssa(&inner);
                self.e(&format!("br label %{}", merge_bb));
                self.e(&format!("{}:", merge_bb));
                let phi = self.fresh_label();
                self.e(&format!("%{} = phi {} [ %{}, %{} ], [ %{}, %{} ]", phi, b_ty, b_val, null_bb, inner_val, nonnull_bb));
                (self.ssa(&phi), b_ty)
            }
            Expr::Variant(_, _, _) => ("0".into(), "i64".into()), // TODO: enum LLVM
            Expr::Try(_) | Expr::TryCatch(_, _, _) => ("0".into(), "i64".into()),
            Expr::As(_, _) => self.compile_expr(&ExprNode::new(0, crate::diagnostics::span::Span::new(0,0), Expr::LitNull)),
        }
    }

    fn compile_field_access(&mut self, val: &str, ty: &str, field: &str) -> (String, String) {
        // Try struct type
        let struct_name = ty.strip_prefix("%struct.").unwrap_or("");
        if !struct_name.is_empty() {
            if let Some(defs) = self.struct_defs.get(struct_name) {
                if let Some(idx) = defs.iter().position(|(n, _)| n == field) {
                    let fty = defs[idx].1.clone();
                    let ext = self.fresh_label();
                    self.e(&format!("%{} = extractvalue {} {}, {}", ext, ty, val, idx));
                    return (self.ssa(&ext), fty);
                }
            }
        }
        // Fallback: return val as-is
        (val.to_string(), ty.to_string())
    }

    fn guess_field_type(&mut self, ty: &str, field: &str) -> String {
        let struct_name = ty.strip_prefix("%struct.").unwrap_or("");
        if !struct_name.is_empty() {
            if let Some(defs) = self.struct_defs.get(struct_name) {
                if let Some(idx) = defs.iter().position(|(n, _)| n == field) {
                    return defs[idx].1.clone();
                }
            }
        }
        "i64".into()
    }

    fn compile_binop(&mut self, l: &ExprNode, op: &BinOp, r: &ExprNode) -> (String, String) {
        let lt = self.expr_type_str(l);
        let (lc, _) = self.compile_expr(l);
        let (rc, _) = self.compile_expr(r);

        let rt = self.expr_type_str(r);
        let is_float = lt == "double";
        let is_complex = lt == "%yk_complex" || rt == "%yk_complex";
        let (arith_op, cmp_op) = if is_float { ("f", "fcmp") } else { ("", "icmp") };

        let tmp = self.fresh_label();
        if is_complex {
            let func = match op {
                BinOp::Add => "yk_complex_add",
                BinOp::Sub => "yk_complex_sub",
                BinOp::Mul => "yk_complex_mul",
                BinOp::Div => "yk_complex_div",
                _ => "yk_complex_add",
            };
            // Allocate temps on stack and store values to pass pointers to runtime
            let la = self.fresh_label();
            let ra = self.fresh_label();
            self.e(&format!("%{} = alloca %yk_complex, align 8", la));
            self.e(&format!("%{} = alloca %yk_complex, align 8", ra));
            // Left operand
            if lt == "%yk_complex" {
                self.e(&format!("store %yk_complex {}, ptr %{}", lc, la));
            } else {
                let lca = self.fresh_label();
                self.e(&format!("%{} = sitofp {} {} to double", lca, lt, lc));
                self.e(&format!("call void @yk_complex_set(ptr %{}, double %{}, double 0.0)", la, lca));
            }
            // Right operand (check right type)
            if rt == "%yk_complex" {
                self.e(&format!("store %yk_complex {}, ptr %{}", rc, ra));
            } else {
                let rca = self.fresh_label();
                self.e(&format!("%{} = sitofp {} {} to double", rca, rt, rc));
                self.e(&format!("call void @yk_complex_set(ptr %{}, double %{}, double 0.0)", ra, rca));
            }
            self.e(&format!("%{} = alloca %yk_complex, align 8", tmp));
            self.e(&format!("call void @{}(ptr %{}, ptr %{}, ptr %{})", func, tmp, la, ra));
            let loaded = self.fresh_label();
            self.e(&format!("%{} = load %yk_complex, ptr %{}", loaded, tmp));
            return (self.ssa(&loaded), "%yk_complex".into());
        }
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
                                "%yk_complex" => {
                                    self.e(&format!("call void @yk_print_complex(ptr {})", av));
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
                "str" => {
                    if let Some((av, at)) = arg_results.first() {
                        let ptr_ssa = match at.as_str() {
                            "i64" => {
                                let t = self.fresh_label();
                                self.e(&format!("%{} = call ptr @yk_string_from_int(i64 {})", t, av));
                                self.ssa(&t)
                            }
                            "double" => {
                                let t = self.fresh_label();
                                self.e(&format!("%{} = call ptr @yk_string_from_real(double {})", t, av));
                                self.ssa(&t)
                            }
                            "i1" => {
                                let t = self.fresh_label();
                                self.e(&format!("%{} = call ptr @yk_string_from_bool(i1 {})", t, av));
                                self.ssa(&t)
                            }
                            "%yk_complex" => {
                                let ca = self.fresh_label();
                                self.e(&format!("%{} = alloca %yk_complex, align 8", ca));
                                self.e(&format!("store %yk_complex {}, ptr %{}", av, ca));
                                let t = self.fresh_label();
                                self.e(&format!("%{} = call ptr @yk_string_from_complex(ptr %{})", t, ca));
                                self.ssa(&t)
                            }
                            "%yk_string" => {
                                // Already a %yk_string value, alloca and get ptr
                                let ca = self.fresh_label();
                                self.e(&format!("%{} = alloca %yk_string, align 8", ca));
                                self.e(&format!("store %yk_string {}, ptr %{}", av, ca));
                                self.ssa(&ca)
                            }
                            _ => {
                                let t = self.fresh_label();
                                self.e(&format!("%{} = call ptr @yk_string_from_int(i64 {})", t, av));
                                self.ssa(&t)
                            }
                        };
                        let loaded = self.fresh_label();
                        self.e(&format!("%{} = load %yk_string, ptr {}", loaded, ptr_ssa));
                        (self.ssa(&loaded), "%yk_string".into())
                    } else {
                        ("0".into(), "%yk_string".into())
                    }
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

    fn compile_pattern_match(&mut self, pattern: &Pattern, scrutinee_val: &str, scrutinee_ty: &str) -> Option<String> {
        match pattern {
            Pattern::Ignore => None, // always matches
            Pattern::Ident(_) | Pattern::Rest(_) => None, // always matches (bind)
            Pattern::LitInt(n) => {
                if scrutinee_ty == "i64" {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = icmp eq i64 {}, {}", tmp, scrutinee_val, n));
                    Some(self.ssa(&tmp))
                } else {
                    Some("true".into())
                }
            }
            Pattern::LitReal(n) => {
                if scrutinee_ty == "double" {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = fcmp oeq double {}, {:.10}", tmp, scrutinee_val, n));
                    Some(self.ssa(&tmp))
                } else {
                    Some("true".into())
                }
            }
            Pattern::LitBool(b) => {
                let v = if *b { "true" } else { "false" };
                if scrutinee_ty == "i1" {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = icmp eq i1 {}, {}", tmp, scrutinee_val, v));
                    Some(self.ssa(&tmp))
                } else {
                    Some(v.into())
                }
            }
            Pattern::LitStr(_s) => {
                // Compare strings via runtime
                // For now, fallback
                Some("true".into())
            }
            _ => Some("true".into()), // Destruct/ListDestruct fallback
        }
    }

    fn compile_pattern_bind(&mut self, pattern: &Pattern, scrutinee_val: &str, scrutinee_ty: &str) {
        match pattern {
            Pattern::Ident(name) => {
                let ptr = self.alloca_name(name);
                self.var_alloca.insert(name.clone(), ptr.clone());
                self.var_types.insert(name.clone(), scrutinee_ty.to_string());
                self.e(&format!("{} = alloca {}, align 8", ptr, scrutinee_ty));
                self.e(&format!("store {} {}, ptr {}", scrutinee_ty, scrutinee_val, ptr));
            }
            Pattern::Rest(name) => {
                let ptr = self.alloca_name(name);
                self.var_alloca.insert(name.clone(), ptr.clone());
                self.var_types.insert(name.clone(), scrutinee_ty.to_string());
                self.e(&format!("{} = alloca {}, align 8", ptr, scrutinee_ty));
                self.e(&format!("store {} {}, ptr {}", scrutinee_ty, scrutinee_val, ptr));
            }
            Pattern::ListDestruct(patterns) => {
                for (_idx, pat) in patterns.iter().enumerate() {
                    match pat {
                        Pattern::Ident(name) => {
                            let ptr = self.alloca_name(name);
                            self.var_alloca.insert(name.clone(), ptr.clone());
                            self.var_types.insert(name.clone(), "i64".into());
                            self.e(&format!("{} = alloca i64, align 8", ptr));
                            // GEP into the list and load - but lists are i64 values not pointers
                            // For now, store 0 as placeholder
                            self.e(&format!("store i64 0, ptr {}", ptr));
                        }
                        Pattern::Rest(name) => {
                            let ptr = self.alloca_name(name);
                            self.var_alloca.insert(name.clone(), ptr.clone());
                            self.var_types.insert(name.clone(), "i64".into());
                            self.e(&format!("{} = alloca i64, align 8", ptr));
                            self.e(&format!("store i64 0, ptr {}", ptr));
                        }
                        _ => self.compile_pattern_bind(pat, scrutinee_val, scrutinee_ty),
                    }
                }
            }
            Pattern::Destruct(fields) => {
                for (_fname, pat) in fields {
                    self.compile_pattern_bind(pat, scrutinee_val, scrutinee_ty);
                }
            }
            _ => {}
        }
    }
}

pub fn compile_to_llvm(module: &Module) -> String {
    let mut codegen = LlvmCodegen::new();
    codegen.compile_module(module)
}

fn detect_clang() -> Option<String> {
    // Check common LLVM installation paths
    let candidates = [
        r"C:\Program Files\LLVM\bin\clang.exe",
        r"C:\Program Files (x86)\LLVM\bin\clang.exe",
    ];
    for p in &candidates {
        if std::path::Path::new(p).exists() { return Some(p.to_string()); }
    }
    // Check PATH
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p).find_map(|d| {
            let c = d.join("clang.exe");
            if c.exists() { Some(c.to_string_lossy().to_string()) } else { None }
        })
    })
}

pub fn detect_vcvars() -> Option<String> {
    // Try vswhere
    let vswhere = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";
    if std::path::Path::new(vswhere).exists() {
        if let Ok(out) = std::process::Command::new(vswhere)
            .args(["-latest", "-property", "installationPath"])
            .output()
        {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let bat = format!(r"{}\VC\Auxiliary\Build\vcvars64.bat", path);
            if std::path::Path::new(&bat).exists() { return Some(bat); }
        }
    }
    // Fallback: common paths
    let candidates = [
        r"C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
    ];
    for p in &candidates {
        if std::path::Path::new(p).exists() { return Some(p.to_string()); }
    }
    None
}

pub fn compile_to_exe(llvm_ir: &str, output_path: &Path, hw: &HardwareInfo) -> Result<()> {
    // Fast path: use LLVM API in-process if available
    if let Ok(()) = compile_to_exe_fast(llvm_ir, output_path, hw) {
        return Ok(());
    }
    // Fallback: batch script with clang + MSVC
    compile_to_exe_batch(llvm_ir, output_path, hw)
}

/// Fast compilation path using in-process LLVM API (no batch script, no clang).
/// Falls back silently if LLVM-C DLL is not available.
fn compile_to_exe_fast(llvm_ir: &str, output_path: &Path, hw: &HardwareInfo) -> Result<()> {
    let obj_path = output_path.with_extension("obj");
    let exe_path = output_path.with_extension("exe");

    // Try to load LLVM-C and emit object in-process
    let api_path = match crate::codegen::llvm_api::find_llvm_lib() {
        Some(p) => p,
        None => return Err(error::err(ErrorKind::Internal, Span::new(0, 0), "LLVM-C not found")),
    };
    let api = crate::codegen::llvm_api::LlvmApi::load(&api_path)?;
    emit_obj_in_memory(&api, llvm_ir, &obj_path, hw)?;

    // Ensure runtime object is cached
    let runtime_obj = cache_runtime_obj(&obj_path)?;

    // Single link command (no batch script, no clang)
    let vcvars = detect_vcvars()
        .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
            "Visual Studio not found"))?;
    let status = std::process::Command::new("cmd.exe")
        .args(["/c", &format!(r#""{}" x64 >nul 2>&1 && link.exe /nologo "{}" "{}" /OUT:"{}" /defaultlib:libcmt.lib"#,
            vcvars, obj_path.to_string_lossy(), runtime_obj.to_string_lossy(), exe_path.to_string_lossy())])
        .status()
        .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
            format!("link.exe failed: {}", e)))?;
    if !status.success() {
        return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
            format!("link.exe exited with code {:?}", status.code())));
    }

    // Cleanup obj files
    let _ = std::fs::remove_file(&obj_path);
    let _ = std::fs::remove_file(&runtime_obj);
    Ok(())
}

pub fn emit_obj_in_memory(api: &crate::codegen::llvm_api::LlvmApi, llvm_ir: &str, obj_path: &Path, hw: &HardwareInfo) -> Result<()> {
    unsafe {
        if let Some(f) = api.LLVMInitializeX86TargetInfo { f(); }
        if let Some(f) = api.LLVMInitializeX86Target { f(); }
        if let Some(f) = api.LLVMInitializeX86TargetMC { f(); }
        if let Some(f) = api.LLVMInitializeX86AsmPrinter { f(); }

        let ctx = (api.LLVMContextCreate)();
        let ir_str = std::ffi::CString::new(llvm_ir)
            .map_err(|_| error::err(ErrorKind::Internal, Span::new(0, 0), "LLVM IR contains null byte"))?;
        let name = std::ffi::CString::new("yk").unwrap();
        let membuf = (api.LLVMCreateMemoryBufferWithMemoryRange)(ir_str.as_ptr(), llvm_ir.len(), name.as_ptr(), 1);
        let mut module: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut err: *mut i8 = std::ptr::null_mut();
        let parse_rc = (api.LLVMParseIRInContext)(ctx, membuf, &mut module, &mut err);
        if parse_rc != 0 {
            let err_str = api.get_error(err);
            (api.LLVMDisposeMemoryBuffer)(membuf);
            (api.LLVMContextDispose)(ctx);
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0), format!("LLVM IR parse failed: {}", err_str)));
        }
        (api.LLVMDisposeMemoryBuffer)(membuf);

        // AOT O0 — skip optimization passes entirely (fast compilation)
        let triple_c = std::ffi::CString::new(hw.os.triple.as_str()).unwrap();
        let cpu_c = std::ffi::CString::new(hw.cpu.name.as_str()).unwrap();
        let features = hw.cpu.simd.to_llvm_features().join(",");
        let features_c = std::ffi::CString::new(features).unwrap();
        (api.LLVMSetTarget)(module, triple_c.as_ptr());

        let mut target_ref = (api.LLVMGetFirstTarget)();
        if target_ref.is_null() {
            let mut err_target: *mut i8 = std::ptr::null_mut();
            let rc = (api.LLVMGetTargetFromTriple)(triple_c.as_ptr(), &mut target_ref, &mut err_target);
            if rc != 0 || target_ref.is_null() {
                let err_str = if !err_target.is_null() { api.get_error(err_target) } else { "unknown".into() };
                (api.LLVMDisposeModule)(module);
                (api.LLVMContextDispose)(ctx);
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    format!("LLVM: no target for triple '{}' ({})", hw.os.triple, err_str)));
            }
        }
        // AOT uses O0 for fastest compilation
        let tm = (api.LLVMCreateTargetMachine)(target_ref, triple_c.as_ptr(), cpu_c.as_ptr(), features_c.as_ptr(), 0, 2, 0);
        if tm.is_null() {
            (api.LLVMDisposeModule)(module);
            (api.LLVMContextDispose)(ctx);
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0), "LLVM: failed to create target machine"));
        }

        let td = (api.LLVMCreateTargetDataLayout)(tm);
        (api.LLVMSetModuleDataLayout)(module, td);

        // Emit to memory buffer
        let mut membuf_out: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut err2: *mut i8 = std::ptr::null_mut();
        let emit_result = (api.LLVMTargetMachineEmitToMemoryBuffer)(tm, module, 1, &mut err2, &mut membuf_out);
        if emit_result != 0 {
            let err_str = api.get_error(err2);
            (api.LLVMDisposeTargetMachine)(tm);
            (api.LLVMDisposeModule)(module);
            (api.LLVMContextDispose)(ctx);
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0), format!("LLVM emit failed: {}", err_str)));
        }

        // Write memory buffer to file
        let buf_ptr = (api.LLVMGetBufferStart)(membuf_out);
        let buf_size = (api.LLVMGetBufferSize)(membuf_out);
        let obj_data = std::slice::from_raw_parts(buf_ptr as *const u8, buf_size);
        std::fs::write(obj_path, obj_data)
            .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0), format!("Failed to write obj: {}", e)))?;

        (api.LLVMDisposeMemoryBuffer)(membuf_out);
        (api.LLVMDisposeTargetMachine)(tm);
        (api.LLVMDisposeModule)(module);
        (api.LLVMContextDispose)(ctx);
        Ok(())
    }
}

/// Cache the pre-compiled runtime C object file to avoid recompiling every time
fn cache_runtime_obj(_output_path: &Path) -> Result<std::path::PathBuf> {
    let cache_dir = std::env::temp_dir().join("yk_cache");
    let _ = std::fs::create_dir_all(&cache_dir);
    let cached_obj = cache_dir.join("yk_rt.obj");

    if !cached_obj.exists() {
        let runtime_c_path = cache_dir.join("yk_rt.c");
        std::fs::write(&runtime_c_path, RUNTIME_C)
            .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                format!("Failed to write runtime C: {}", e)))?;

        let vcvars = detect_vcvars()
            .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0), "Visual Studio not found"))?;
        let rt_c_str = runtime_c_path.to_string_lossy();
        let rt_obj_str = cached_obj.to_string_lossy();
        let status = std::process::Command::new("cmd.exe")
            .args(["/c", &format!(r#""{}" x64 >nul 2>&1 && cl.exe /nologo /std:c11 /c "{}" /Fo:"{}" /utf-8"#,
                vcvars, rt_c_str, rt_obj_str)])
            .status()
            .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                format!("cl.exe failed: {}", e)))?;
        if !status.success() {
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                format!("cl.exe exited with code {:?}", status.code())));
        }
    }
    Ok(cached_obj)
}

fn compile_to_exe_batch(llvm_ir: &str, output_path: &Path, hw: &HardwareInfo) -> Result<()> {
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

    let vcvars = detect_vcvars()
        .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
            "Visual Studio 2019/2022/2025 not found. Install Build Tools or set PATH manually.".to_string()))?;
    let clang = detect_clang()
        .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
            "clang.exe not found. Install LLVM from https://llvm.org or add it to PATH.".to_string()))?;

    let opt_level = if crate::hardware::memory::is_low_memory(&hw.memory) { "O2" }
        else { "O3" };
    let target = &hw.os.triple;
    let march = &hw.cpu.name;
    let simd_features = hw.cpu.simd.to_llvm_features().join(",");
    let target_features = if simd_features.is_empty() { String::new() }
        else { format!(" -target-feature {}", simd_features) };

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

:: Compile LLVM IR to object file (hardware-adaptive)
"{}" -c "{}" -o "{}" -target {} -{} -march={}{}
if errorlevel 1 exit /b 1

:: Compile runtime C to object file
cl.exe /nologo /std:c11 /c "{}" /Fo:"{}" /utf-8
if errorlevel 1 exit /b 1

:: Link objects into executable
link.exe /nologo "{}" "{}" /OUT:"{}" /defaultlib:libcmt.lib
exit /b %errorlevel%
"#, vcvars, clang, ll_str, obj_str, target, opt_level, march, target_features, rt_c_str, rt_obj_str, obj_str, rt_obj_str, exe_str);

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

/// Generate LLVM IR for a handler function that returns a static string.
///
/// The generated function has C ABI: `void handler_name(struct YkResponse* resp)`
/// and writes the response body, length, and status code into the struct.
///
/// # Arguments
/// * `handler_name` - The name of the LLVM function (also used as JIT symbol)
/// * `response_body` - The static response body string
/// * `status_code` - HTTP status code (e.g. 200)
pub fn generate_static_handler_ir(handler_name: &str, response_body: &str, status_code: i32) -> String {
    let body_len = response_body.len();
    let escaped_body = response_body
        .replace('\\', "\\\\")
        .replace('"', "\\22")
        .replace('\n', "\\0A")
        .replace('\r', "\\0D")
        .replace('\t', "\\09");
    let cstr_name = format!("@__yk_cstr_{}", handler_name);

    format!(
        r#"; JIT-compiled handler for '{handler_name}'
target triple = "x86_64-pc-windows-msvc"

{global_str} = private unnamed_addr constant [{array_len} x i8] c"{escaped_body}\00"

define void @{handler_name}(ptr %resp) {{
entry:
  ; Write body pointer at byte offset 0
  %bp = getelementptr i8, ptr %resp, i32 0
  store ptr {global_str}, ptr %bp
  ; Write body_len at byte offset 8
  %lp = getelementptr i8, ptr %resp, i32 8
  store i64 {body_len}, ptr %lp
  ; Write status_code at byte offset 16
  %sp = getelementptr i8, ptr %resp, i32 16
  store i32 {status_code}, ptr %sp
  ret void
}}
"#,
        handler_name = handler_name,
        global_str = cstr_name,
        escaped_body = escaped_body,
        array_len = body_len + 1,
        body_len = body_len,
        status_code = status_code,
    )
}

/// Generate LLVM IR for a handler function from an AST FnDef body.
///
/// The generated function has C ABI:
///   void @handler_name(ptr %resp, ptr %req, ptr %buf, i64 %buf_len)
///
/// It evaluates the FnDef body (supporting string literals, req field access,
/// and string concatenation of literals + req fields) and writes the result
/// into the caller-provided %buf buffer, updating %resp.
///
/// Handler parameter `req` maps to the %req struct pointer. The struct layout:
///   offsets: method(0 ptr, 8 len), path(16 ptr, 24 len), body(32 ptr, 40 len)
pub fn generate_fn_handler_ir(handler_name: &str, fndef: &crate::interpret::FnDef) -> Option<String> {
    let body = &fndef.body;
    let ret_expr = find_return_expr(body)?;
    let mut gen = FnIrGen::new();
    Some(gen.gen_fn(handler_name, ret_expr))
}

fn find_return_expr(body: &[StmtNode]) -> Option<&ExprNode> {
    for stmt in body.iter().rev() {
        match &stmt.value {
            Stmt::Return(Some(e)) => return Some(e),
            Stmt::Return(None) => return None,
            Stmt::Expr(e) => return Some(e),
            _ => {}
        }
    }
    None
}

struct FnIrGen {
    label: usize,
    output: String,
    string_constants: String,
}

impl FnIrGen {
    fn new() -> Self {
        FnIrGen { label: 0, output: String::new(), string_constants: String::new() }
    }

    fn fresh(&mut self) -> String {
        let n = self.label;
        self.label += 1;
        format!("%l{}", n)
    }

    fn e(&mut self, s: &str) {
        use std::fmt::Write;
        writeln!(self.output, "  {}", s).unwrap();
    }

    fn gen_fn(&mut self, handler_name: &str, ret_expr: &ExprNode) -> String {
        self.output.clear();
        self.string_constants.clear();

        // Build the full IR in correct order: target triple, types, constants, function
        let mut ir = String::new();
        ir.push_str(&format!("; JIT-compiled FnDef handler for '{}'\n", handler_name));
        ir.push_str("target triple = \"x86_64-pc-windows-msvc\"\n");
        ir.push_str("\n");
        ir.push_str("%YkResponse = type { ptr, i64, i32 }\n");
        ir.push_str("\n");

        // Generate function body into self.output & string constants
        self.output.push_str(&format!(
            "define void @{}(ptr %resp, ptr %req, ptr %buf, i64 %buf_len) {{\n",
            handler_name
        ));
        self.output.push_str("entry:\n");

        let (ptr_val, len_val) = self.gen_expr(ret_expr);

        self.e(&format!("store ptr {}, ptr %resp", ptr_val));
        self.e(&format!("%lp = getelementptr i8, ptr %resp, i32 8"));
        self.e(&format!("store i64 {}, ptr %lp", len_val));
        self.e(&format!("%sp = getelementptr i8, ptr %resp, i32 16"));
        self.e("store i32 200, ptr %sp");
        self.e("ret void");
        self.output.push_str("}\n");

        // Assemble: constants then function body
        ir.push_str(&self.string_constants);
        ir.push('\n');
        ir.push_str(&self.output);
        ir
    }

    fn gen_str_constant(&mut self, s: &str) -> (String, String) {
        let idx = self.label;
        self.label += 1;
        let cstr_name = format!("@__yk_cstr_{}", idx);
        let escaped = s.replace('\\', "\\\\").replace('"', "\\22").replace('\n', "\\0A").replace('\r', "\\0D");
        let arr_len = escaped.len() + 1;
        use std::fmt::Write;
        writeln!(self.string_constants, "{cstr_name} = private unnamed_addr constant [{arr_len} x i8] c\"{escaped}\\00\"").unwrap();
        let ptr = self.fresh();
        self.e(&format!("{ptr} = getelementptr inbounds [{arr_len} x i8], ptr {cstr_name}, i64 0, i64 0"));
        let len_val = s.len().to_string();
        (ptr, len_val)
    }

    fn gen_int64(&mut self, expr: &ExprNode) -> String {
        match &expr.value {
            Expr::LitInt(n) => n.to_string(),
            _ => "0".into(),
        }
    }

    fn gen_condition(&mut self, expr: &ExprNode) -> String {
        match &expr.value {
            Expr::LitBool(b) => {
                if *b { "true".into() } else { "false".into() }
            }
            Expr::BinOp(l, BinOp::Eq, r) => {
                let lhs = self.gen_int64(l);
                let rhs = self.gen_int64(r);
                let r = self.fresh();
                self.e(&format!("{r} = icmp eq i64 {lhs}, {rhs}"));
                r
            }
            Expr::BinOp(l, BinOp::Ne, r) => {
                let lhs = self.gen_int64(l);
                let rhs = self.gen_int64(r);
                let r = self.fresh();
                self.e(&format!("{r} = icmp ne i64 {lhs}, {rhs}"));
                r
            }
            _ => "true".into(),
        }
    }

    fn gen_expr(&mut self, expr: &ExprNode) -> (String, String) {
        match &expr.value {
            Expr::LitStr(s) => self.gen_str_constant(s),
            Expr::LitInt(n) => self.gen_str_constant(&n.to_string()),
            Expr::LitBool(b) => self.gen_str_constant(if *b { "true" } else { "false" }),
            Expr::LitChar(c) => self.gen_str_constant(&c.to_string()),
            Expr::LitReal(v) => self.gen_str_constant(&format!("{}", v)),
            Expr::LitHex(n) => self.gen_str_constant(&format!("{}", n)),
            Expr::If(cond, then_expr, else_expr) => {
                let cond_i1 = self.gen_condition(cond);
                let (then_ptr, then_len) = self.gen_expr(then_expr);
                let (else_ptr, else_len) = if let Some(ee) = else_expr {
                    self.gen_expr(ee)
                } else {
                    let empty = self.fresh();
                    self.e(&format!("{empty} = getelementptr i8, ptr %buf, i64 0"));
                    (empty, "0".into())
                };
                let ptr = self.fresh();
                self.e(&format!("{ptr} = select i1 {cond_i1}, ptr {then_ptr}, ptr {else_ptr}"));
                let len = self.fresh();
                self.e(&format!("{len} = select i1 {cond_i1}, i64 {then_len}, i64 {else_len}"));
                (ptr, len)
            }
            Expr::Field(obj, field) => {
                if let Expr::Ident(name) = &obj.value {
                    if name == "req" {
                        let (field_offset_ptr, field_offset_len) = match field.as_str() {
                            "method" => (0i32, 8i32),
                            "path" => (16, 24),
                            "body" => (32, 40),
                            _ => (32, 40),
                        };
                        let ptr_label = self.fresh();
                        let len_label = self.fresh();
                        self.e(&format!("; Load req.{}", field));
                        self.e(&format!("{ptr_label} = getelementptr i8, ptr %req, i32 {field_offset_ptr}"));
                        let ptr_v = self.fresh();
                        self.e(&format!("{ptr_v} = load ptr, ptr {ptr_label}"));
                        self.e(&format!("{len_label} = getelementptr i8, ptr %req, i32 {field_offset_len}"));
                        let len_v = self.fresh();
                        self.e(&format!("{len_v} = load i64, ptr {len_label}"));
                        (ptr_v, len_v)
                    } else {
                        let empty_ptr = self.fresh();
                        self.e(&format!("{empty_ptr} = getelementptr i8, ptr %buf, i64 0"));
                        (empty_ptr, "0".into())
                    }
                } else {
                    let empty_ptr = self.fresh();
                    self.e(&format!("{empty_ptr} = getelementptr i8, ptr %buf, i64 0"));
                    (empty_ptr, "0".into())
                }
            }
            Expr::Ident(name) => {
                if name == "req" {
                    self.gen_expr(
                        &ExprNode::new(0, Span::new(0, 0),
                            Expr::Field(Box::new(ExprNode::new(0, Span::new(0, 0), Expr::Ident("req".into()))), "body".into())),
                    )
                } else {
                    let empty_ptr = self.fresh();
                    self.e(&format!("{empty_ptr} = getelementptr i8, ptr %buf, i64 0"));
                    (empty_ptr, "0".into())
                }
            }
            Expr::BinOp(l, op, r) if *op == BinOp::Add => {
                let (l_ptr, l_len) = self.gen_expr(l);
                let (r_ptr, r_len) = self.gen_expr(r);

                let total = self.fresh();
                self.e(&format!("{total} = add i64 {l_len}, {r_len}")); // unused, replaced by clamped

                // Clamp left to buf_len
                let l_clamped = self.fresh();
                self.e(&format!("{l_clamped} = icmp ugt i64 {l_len}, %buf_len"));
                let l_actual = self.fresh();
                self.e(&format!("{l_actual} = select i1 {l_clamped}, i64 %buf_len, i64 {l_len}"));
                self.e(&format!("call void @llvm.memcpy.p0.p0.i64(ptr %buf, ptr {l_ptr}, i64 {l_actual}, i1 false)"));

                let r_offset = self.fresh();
                self.e(&format!("{r_offset} = getelementptr i8, ptr %buf, i64 {l_actual}"));
                let r_remaining = self.fresh();
                self.e(&format!("{r_remaining} = sub i64 %buf_len, {l_actual}"));
                let r_clamped = self.fresh();
                self.e(&format!("{r_clamped} = icmp ugt i64 {r_len}, {r_remaining}"));
                let r_actual = self.fresh();
                self.e(&format!("{r_actual} = select i1 {r_clamped}, i64 {r_remaining}, i64 {r_len}"));
                self.e(&format!("call void @llvm.memcpy.p0.p0.i64(ptr {r_offset}, ptr {r_ptr}, i64 {r_actual}, i1 false)"));

                let actual_len = self.fresh();
                self.e(&format!("{actual_len} = add i64 {l_actual}, {r_actual}"));

                ("%buf".into(), actual_len)
            }
            Expr::Block(stmts) => {
                if let Some(ret) = find_return_expr(stmts) {
                    self.gen_expr(ret)
                } else {
                    let empty_ptr = self.fresh();
                    self.e(&format!("{empty_ptr} = getelementptr i8, ptr %buf, i64 0"));
                    (empty_ptr, "0".into())
                }
            }
            _ => {
                let empty_ptr = self.fresh();
                self.e(&format!("{empty_ptr} = getelementptr i8, ptr %buf, i64 0"));
                (empty_ptr, "0".into())
            }
        }
    }
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

    #[test]
    fn test_generate_fn_handler_ir_literal() {
        // Manually construct a FnDef that returns a string literal
        use crate::syntax::ast::{Param, TypeNode, Stmt};
        use crate::interpret::FnDef;
        let param = Param {
            name: "req".into(),
            type_expr: TypeNode::new(fresh_id(), Span::new(0, 0), TypeExpr::Infer),
            is_ref: false,
        };
        let body = vec![
            StmtNode::new(fresh_id(), Span::new(0, 0), Stmt::Return(Some(
                ExprNode::new(fresh_id(), Span::new(0, 0), Expr::LitStr("Hello World".into()))
            ))),
        ];
        let fndef = FnDef::new(vec![param], body);
        let ir = generate_fn_handler_ir("test_fn_handler", &fndef)
            .expect("Should generate IR");
        assert!(ir.contains("define void @test_fn_handler"));
        assert!(ir.contains("target triple = \"x86_64-pc-windows-msvc\""));
        assert!(ir.contains("%YkResponse = type { ptr, i64, i32 }"));
        assert!(ir.contains("store i32 200"));
        assert!(ir.contains("Hello World"));
    }

    #[test]
    fn test_generate_fn_handler_ir_req_body() {
        use crate::syntax::ast::{Param, TypeNode, Stmt};
        use crate::interpret::FnDef;

        // Build: return req.body
        let param = Param {
            name: "req".into(),
            type_expr: TypeNode::new(fresh_id(), Span::new(0, 0), TypeExpr::Infer),
            is_ref: false,
        };
        let req_ident = ExprNode::new(fresh_id(), Span::new(0, 0), Expr::Ident("req".into()));
        let body_field = ExprNode::new(fresh_id(), Span::new(0, 0),
            Expr::Field(Box::new(req_ident), "body".into()));
        let body = vec![
            StmtNode::new(fresh_id(), Span::new(0, 0), Stmt::Return(Some(body_field))),
        ];
        let fndef = FnDef::new(vec![param], body);
        let ir = generate_fn_handler_ir("test_req_body", &fndef)
            .expect("Should generate IR");
        assert!(ir.contains("define void @test_req_body"));
        assert!(ir.contains("Load req.body"));
        assert!(ir.contains("getelementptr i8, ptr %req, i32 32")); // body ptr offset
        assert!(ir.contains("getelementptr i8, ptr %req, i32 40")); // body len offset
    }

    #[test]
    fn test_generate_fn_handler_ir_concat() {
        use crate::syntax::ast::{Param, TypeNode, Stmt};
        use crate::interpret::FnDef;

        // Build: return "Prefix: " + req.body
        let param = Param {
            name: "req".into(),
            type_expr: TypeNode::new(fresh_id(), Span::new(0, 0), TypeExpr::Infer),
            is_ref: false,
        };
        let prefix = ExprNode::new(fresh_id(), Span::new(0, 0), Expr::LitStr("Prefix: ".into()));
        let req_ident = ExprNode::new(fresh_id(), Span::new(0, 0), Expr::Ident("req".into()));
        let req_body = ExprNode::new(fresh_id(), Span::new(0, 0),
            Expr::Field(Box::new(req_ident), "body".into()));
        let concat = ExprNode::new(fresh_id(), Span::new(0, 0),
            Expr::BinOp(Box::new(prefix), BinOp::Add, Box::new(req_body)));
        let body = vec![
            StmtNode::new(fresh_id(), Span::new(0, 0), Stmt::Return(Some(concat))),
        ];
        let fndef = FnDef::new(vec![param], body);
        let ir = generate_fn_handler_ir("test_concat", &fndef)
            .expect("Should generate IR");
        assert!(ir.contains("define void @test_concat"));
        assert!(ir.contains("call void @llvm.memcpy.p0.p0.i64"));
        assert!(ir.contains("Prefix:"));
    }
}
