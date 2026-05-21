pub mod llvm;

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

yk_string yk_string_make(const char* s) {
    int64_t len = (int64_t)strlen(s);
    char* data = (char*)malloc(len + 1);
    memcpy(data, s, len + 1);
    return (yk_string){data, len};
}

yk_string yk_string_from_int(int64_t v) {
    char buf[64];
    int n = snprintf(buf, sizeof(buf), "%lld", (long long)v);
    char* data = (char*)malloc(n + 1);
    memcpy(data, buf, n + 1);
    return (yk_string){data, n};
}

yk_string yk_string_concat(yk_string a, yk_string b) {
    char* data = (char*)malloc(a.len + b.len + 1);
    memcpy(data, a.data, a.len);
    memcpy(data + a.len, b.data, b.len);
    data[a.len + b.len] = '\0';
    return (yk_string){data, a.len + b.len};
}

int64_t yk_string_len(yk_string s) { return s.len; }

void yk_print_int(int64_t v) { printf("%lld\n", (long long)v); }
void yk_print_real(double v) { printf("%g\n", v); }
void yk_print_bool(bool v) { printf("%s\n", v ? "true" : "false"); }
void yk_print_str(yk_string s) { printf("%.*s\n", (int)s.len, s.data); }

#define yk_print_val(v) _Generic((v), \
    int64_t: yk_print_int, \
    double: yk_print_real, \
    bool: yk_print_bool, \
    yk_string: yk_print_str \
)(v)
"##;

pub struct Codegen {
    output: String,
    indent: usize,
    var_types: HashMap<String, String>,
    struct_defs: HashMap<String, Vec<(String, String)>>,
    tuple_type_names: HashMap<String, String>,
    tuple_counter: usize,
    next_label: usize,
}

impl Codegen {
    pub fn new() -> Self {
        Self { output: String::new(), indent: 0, var_types: HashMap::new(), struct_defs: HashMap::new(), tuple_type_names: HashMap::new(), tuple_counter: 0, next_label: 0 }
    }

    fn emit(&mut self, s: &str) {
        use std::fmt::Write;
        writeln!(self.output, "{}{}", "    ".repeat(self.indent), s).unwrap();
    }

    fn emit_raw(&mut self, s: &str) {
        self.output.push_str(s);
        self.output.push('\n');
    }

    fn type_to_c(&self, te: &TypeExpr) -> String {
        match te {
            TypeExpr::Int(_) | TypeExpr::Rint(_) => "int64_t".into(),
            TypeExpr::Real(_) => "double".into(),
            TypeExpr::Bool => "bool".into(),
            TypeExpr::Str => "yk_string".into(),
            TypeExpr::Named(name) => {
                if self.struct_defs.contains_key(name) {
                    name.clone()
                } else {
                    "int64_t".into()
                }
            }
            _ => "int64_t".into(),
        }
    }

    fn expr_type(&mut self, expr: &ExprNode) -> String {
        match &expr.value {
            Expr::LitInt(_) | Expr::LitHex(_) => "int64_t".into(),
            Expr::LitReal(_) => "double".into(),
            Expr::LitBool(_) => "bool".into(),
            Expr::LitStr(_) => "yk_string".into(),
            Expr::LitSymbol(_) => "yk_string".into(),
            Expr::Ident(name) => self.var_types.get(name).cloned().unwrap_or("int64_t".into()),
            Expr::StructLit(name, _) => name.clone(),
            Expr::TupleLit(items) => {
                let elem_types: Vec<String> = items.iter().map(|i| self.expr_type(i)).collect();
                self.get_or_create_tuple_type(&elem_types)
            }
            _ => "int64_t".into(),
        }
    }

    fn fresh_label(&mut self) -> usize { let l = self.next_label; self.next_label += 1; l }

    fn get_or_create_tuple_type(&mut self, elem_types: &[String]) -> String {
        let sig = elem_types.join("_");
        if let Some(name) = self.tuple_type_names.get(&sig) {
            return name.clone();
        }
        let n = self.tuple_counter;
        self.tuple_counter += 1;
        let name = format!("__yk_t{}", n);
        self.tuple_type_names.insert(sig, name.clone());
        let field_decls: Vec<String> = elem_types.iter().enumerate()
            .map(|(i, ty)| format!("{} f{};", ty, i)).collect();
        self.emit_raw(&format!("typedef struct {{ {} }} {};", field_decls.join(" "), name));
        name
    }

    pub fn compile_module(&mut self, module: &Module) -> String {
        self.emit_raw(RUNTIME_C);
        self.emit_raw("");

        let mut has_main = false;
        for item in &module.items {
            if let ItemKind::Fn { name, .. } = &item.value {
                if name == "main" { has_main = true; break; }
            }
        }

        for item in &module.items {
            match &item.value {
                ItemKind::Struct { name, fields, .. } => {
                    let mut field_types = Vec::new();
                    for p in fields {
                        let ft = self.type_to_c(&p.type_expr.value);
                        field_types.push((p.name.clone(), ft));
                    }
                    self.struct_defs.insert(name.clone(), field_types.clone());
                    let field_decls: Vec<String> = field_types.iter()
                        .map(|(fname, fty)| format!("{} {};", fty, fname)).collect();
                    self.emit_raw(&format!("typedef struct {{ {} }} {};", field_decls.join(" "), name));
                }
                ItemKind::Fn { name, params, ret_type, body, .. } => {
                    if name != "main" {
                        self.compile_fn(name, params, ret_type, body);
                    }
                }
                ItemKind::Const { name, .. } => {
                    self.var_types.insert(name.clone(), "int64_t".into());
                }
                _ => {}
            }
        }

        if has_main {
            self.emit_raw("int main(int argc, char** argv) {");
            self.indent += 1;
            self.emit("(void)argc; (void)argv;");
            self.compile_fn_body("main", &[], &None, &module.items.iter()
                .find_map(|item| match &item.value {
                    ItemKind::Fn { name, body, .. } if name == "main" => Some(body),
                    _ => None,
                }).unwrap_or(&vec![]));
            self.emit("return 0;");
            self.indent -= 1;
            self.emit_raw("}");
        }

        std::mem::take(&mut self.output)
    }

    fn compile_fn(&mut self, name: &str, params: &[Param], ret_type: &Option<TypeNode>, body: &[StmtNode]) {
        let ret = ret_type.as_ref().map(|t| self.type_to_c(&t.value)).unwrap_or_else(|| "void".into());
        let param_list: Vec<String> = params.iter()
            .map(|p| format!("{} {}", self.type_to_c(&p.type_expr.value), p.name)).collect();
        self.emit_raw(&format!("{} {}({}) {{", ret, name, param_list.join(", ")));
        self.indent += 1;
        self.compile_fn_body(name, params, ret_type, body);
        self.indent -= 1;
        self.emit_raw("}");
        self.emit_raw("");
    }

    fn compile_fn_body(&mut self, _name: &str, params: &[Param], _ret_type: &Option<TypeNode>, body: &[StmtNode]) {
        for p in params {
            self.var_types.insert(p.name.clone(), self.type_to_c(&p.type_expr.value).to_string());
        }
        for stmt in body {
            self.compile_stmt(stmt);
        }
    }

    fn compile_stmt(&mut self, stmt: &StmtNode) {
        match &stmt.value {
            Stmt::Decl { name, type_expr, value, is_const: _ } => {
                let ty = match type_expr {
                    Some(te) => self.type_to_c(&te.value).to_string(),
                    None => self.expr_type(value),
                };
                self.var_types.insert(name.clone(), ty.clone());
                let val = self.compile_expr(value);
                self.emit(&format!("{} {} = {};", ty, name, val));
            }
            Stmt::Assign(name, expr) => {
                let val = self.compile_expr(expr);
                self.emit(&format!("{} = {};", name, val));
            }
            Stmt::Expr(e) => {
                let code = self.compile_expr(e);
                if !code.is_empty() && code != "0" {
                    self.emit(&format!("{};", code));
                }
            }
            Stmt::Return(e) => {
                match e {
                    Some(ex) => { let val = self.compile_expr(ex); self.emit(&format!("return {};", val)); }
                    None => self.emit("return;"),
                }
            }
            Stmt::If(cond, then_body, else_body) => {
                let cond_code = self.compile_expr(cond);
                self.emit(&format!("if ({}) {{", cond_code));
                self.indent += 1;
                for s in then_body { self.compile_stmt(s); }
                self.indent -= 1;
                if let Some(eb) = else_body {
                    if eb.is_empty() { self.emit("}"); }
                    else {
                        self.emit("} else {");
                        self.indent += 1;
                        for s in eb { self.compile_stmt(s); }
                        self.indent -= 1;
                        self.emit("}");
                    }
                } else {
                    self.emit("}");
                }
            }
            Stmt::While(cond, body) => {
                let cond_code = self.compile_expr(cond);
                self.emit(&format!("while ({}) {{", cond_code));
                self.indent += 1;
                for s in body { self.compile_stmt(s); }
                self.indent -= 1;
                self.emit("}");
            }
            Stmt::For(var, iter, body) => {
                let lbl = self.fresh_label();
                self.var_types.insert(var.clone(), "int64_t".into());
                let iter_code = self.compile_expr(iter);
                self.emit(&format!("{{"));
                self.indent += 1;
                self.emit(&format!("int64_t yk_start_{} = {};", lbl, iter_code));
                self.emit(&format!("int64_t yk_end_{} = {};", lbl, iter_code));
                self.emit(&format!("for (int64_t {} = yk_start_{}; {} < yk_end_{}; {}++) {{", var, lbl, var, lbl, var));
                self.indent += 1;
                for s in body { self.compile_stmt(s); }
                self.indent -= 1;
                self.emit("}");
                self.indent -= 1;
                self.emit("}");
            }
            Stmt::Loop(body) => {
                self.emit("for (;;) {");
                self.indent += 1;
                for s in body { self.compile_stmt(s); }
                self.indent -= 1;
                self.emit("}");
            }
            Stmt::Destruct(_, expr) => {
                let val = self.compile_expr(expr);
                self.emit(&format!("(void){};", val));
            }
        }
    }

    fn compile_expr(&mut self, expr: &ExprNode) -> String {
        match &expr.value {
            Expr::LitInt(n) => n.to_string(),
            Expr::LitHex(n) => format!("(int64_t)0x{}", n),
            Expr::LitReal(n) => {
                let s = n.to_string();
                if s.contains('.') || s.contains('e') || s.contains('E') { s }
                else { format!("{}.0", s) }
            }
            Expr::LitBool(true) => "true".into(),
            Expr::LitBool(false) => "false".into(),
            Expr::LitStr(s) => format!("yk_string_make(\"{}\")", self.esc_str(s)),
            Expr::LitChar(c) => format!("(int64_t){}", *c as i64),
            Expr::LitNull | Expr::LitNone => "0".into(),
            Expr::LitSymbol(s) => format!("yk_string_make(\":{}\")", self.esc_str(s)),
            Expr::Ident(name) => name.clone(),
            Expr::BinOp(l, op, r) => self.compile_binop(l, op, r),
            Expr::UnOp(op, inner) => {
                let i = self.compile_expr(inner);
                match op { UnOp::Neg => format!("(-{})", i), UnOp::Not => format!("(!{})", i) }
            }
            Expr::Call(callee, args) => self.compile_call(callee, args),
            Expr::Field(obj, field) => {
                format!("({}.{})", self.compile_expr(obj), field)
            }
            Expr::Index(obj, index) => {
                format!("({}[{}])", self.compile_expr(obj), self.compile_expr(index))
            }
            Expr::Range(_l, r) => {
                format!("({})", self.compile_expr(r))
            }
            Expr::Block(stmts) => {
                let lbl = self.fresh_label();
                let mut out = String::new();
                out.push_str(&format!("({{\nint64_t yk_block_{}_ret = 0;\n", lbl));
                let old = self.indent;
                self.indent = 0;
                for s in stmts {
                    match &s.value {
                        Stmt::Return(e) => {
                            out.push_str(&format!("yk_block_{}_ret = {}; goto yk_block_{}_end;\n", lbl,
                                e.as_ref().map(|ex| self.compile_expr(ex)).unwrap_or_else(|| "0".into()), lbl));
                        }
                        _ => {
                            let old_out = std::mem::take(&mut self.output);
                            self.compile_stmt(s);
                            out.push_str(&std::mem::take(&mut self.output));
                            self.output = old_out;
                        }
                    }
                }
                out.push_str(&format!("yk_block_{}_end:;\n", lbl));
                out.push_str(&format!("yk_block_{}_ret;\n}})", lbl));
                self.indent = old;
                out
            }
            Expr::AsConst(inner) => self.compile_expr(inner),
            Expr::If(cond, then_e, else_e) => {
                let c = self.compile_expr(cond);
                let t = self.compile_expr(then_e);
                let e = else_e.as_ref().map(|ex| self.compile_expr(ex)).unwrap_or_else(|| "0".into());
                format!("({} ? {} : {})", c, t, e)
            }
            Expr::ListLit(items) => {
                if items.is_empty() { "0".into() }
                else if items.len() == 1 { self.compile_expr(&items[0]) }
                else {
                    let codes: Vec<String> = items.iter().map(|item| self.compile_expr(item)).collect();
                    format!("({})", codes.join(", "))
                }
            }
            Expr::StructLit(name, fields) => {
                let field_codes: Vec<String> = fields.iter()
                    .map(|(fname, fexpr)| format!(".{} = {}", fname, self.compile_expr(fexpr))).collect();
                format!("({}){{ {} }}", name, field_codes.join(", "))
            }
            Expr::TupleLit(items) => {
                let elem_types: Vec<String> = items.iter().map(|i| self.expr_type(i)).collect();
                let ty = self.get_or_create_tuple_type(&elem_types);
                let val_codes: Vec<String> = items.iter().map(|i| self.compile_expr(i)).collect();
                format!("({}){{ {} }}", ty, val_codes.join(", "))
            }
            Expr::MapLit(pairs) => {
                let codes: Vec<String> = pairs.iter()
                    .flat_map(|(k, v)| [self.compile_expr(k), self.compile_expr(v)])
                    .collect();
                if codes.is_empty() { "0".into() } else { format!("({})", codes.join(", ")) }
            }
            Expr::SetLit(items) => {
                let codes: Vec<String> = items.iter().map(|i| self.compile_expr(i)).collect();
                if codes.is_empty() { "0".into() } else { format!("({})", codes.join(", ")) }
            }
            Expr::FnLit(_, _, body) => self.compile_expr(body),
            Expr::Await(inner) | Expr::Spawn(inner) => self.compile_expr(inner),
            Expr::ResultOk(inner) | Expr::ResultErr(inner) => self.compile_expr(inner),
            Expr::Match(_, _) | Expr::ForIn(_, _, _) | Expr::While(_, _) | Expr::Loop(_) => "0".into(),
        }
    }

    fn compile_binop(&mut self, l: &ExprNode, op: &BinOp, r: &ExprNode) -> String {
        let lt = self.expr_type(l);
        let lc = self.compile_expr(l);
        let rc = self.compile_expr(r);
        match op {
            BinOp::Add => {
                if lt == "yk_string" {
                    format!("yk_string_concat({}, {})", lc, rc)
                } else {
                    format!("({} + {})", lc, rc)
                }
            }
            BinOp::Sub => format!("({} - {})", lc, rc),
            BinOp::Mul => format!("({} * {})", lc, rc),
            BinOp::Div => format!("({} / {})", lc, rc),
            BinOp::Eq => format!("({} == {})", lc, rc),
            BinOp::Ne => format!("({} != {})", lc, rc),
            BinOp::Lt => format!("({} < {})", lc, rc),
            BinOp::Gt => format!("({} > {})", lc, rc),
            BinOp::Le => format!("({} <= {})", lc, rc),
            BinOp::Ge => format!("({} >= {})", lc, rc),
            BinOp::And => format!("({} && {})", lc, rc),
            BinOp::Or => format!("({} || {})", lc, rc),
            BinOp::Assign => format!("({} = {})", lc, rc),
        }
    }

    fn compile_call(&mut self, callee: &ExprNode, args: &[ExprNode]) -> String {
        let arg_codes: Vec<String> = args.iter().map(|a| self.compile_expr(a)).collect();
        match &callee.value {
            Expr::Ident(name) => match name.as_str() {
                "print" | "println" => {
                    if arg_codes.is_empty() {
                        "printf(\"\\n\")".into()
                    } else {
                        let mut out = String::new();
                        for ac in &arg_codes {
                            out.push_str(&format!("yk_print_val({})", ac));
                        }
                        out
                    }
                }
                "len" => {
                    if let Some(arg) = args.first() {
                        if self.expr_type(arg) == "yk_string" {
                            format!("yk_string_len({})", arg_codes[0])
                        } else {
                            format!("(int64_t){}", arg_codes[0])
                        }
                    } else { "0".into() }
                }
                _ => format!("{}({})", name, arg_codes.join(", ")),
            },
            Expr::Field(obj, field) => {
                let obj_code = self.compile_expr(obj);
                format!("({}.{})", obj_code, field)
            }
            _ => "0".into(),
        }
    }

    fn esc_str(&self, s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
            .replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
    }
}

pub fn compile_to_c(module: &Module) -> String {
    let mut codegen = Codegen::new();
    codegen.compile_module(module)
}

pub fn compile_to_exe(c_code: &str, output_path: &Path) -> Result<()> {
    let c_path = output_path.with_extension("c");
    std::fs::write(&c_path, c_code)
        .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
            format!("Failed to write {}: {}", c_path.display(), e)))?;

    let exe_path = output_path.with_extension("exe");
    let c_str = c_path.to_string_lossy();
    let exe_str = exe_path.to_string_lossy();

    let vcvars = r"C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat";

    // Create a temp batch file that sets up VS env and runs cl.exe
    let bat_dir = std::env::temp_dir();
    let bat_path = bat_dir.join("yk_build.bat");
    let bat_content = format!(
        r#"@echo off
call "{}" x64 >nul 2>&1
if errorlevel 1 exit /b 1
cl.exe /nologo /std:c11 /O2 /utf-8 "{}" /Fe:"{}" /link /defaultlib:libcmt.lib
exit /b %errorlevel%
"#, vcvars, c_str, exe_str);
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
                format!("Failed to invoke MSVC: {}", e))
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

    let _ = std::fs::remove_file(&c_path);
    let _ = std::fs::remove_file(&bat_dir.join("test.obj"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parser::Parser;
    use crate::syntax::ast;

    #[test]
    fn test_simple_addition() {
        ast::reset_ids();
        let module = Parser::parse("fn main() { x: int = 5; print(x); }").unwrap();
        let c = compile_to_c(&module);
        assert!(c.contains("int64_t x = 5;"));
        assert!(c.contains("yk_print_val(x)"));
        assert!(c.contains("main"));
    }

    #[test]
    fn test_if_else() {
        ast::reset_ids();
        let module = Parser::parse("fn main() { x: int = 5; if (x > 3) { print(1); } else { print(0); } }").unwrap();
        let c = compile_to_c(&module);
        assert!(c.contains("if ("));
        assert!(c.contains("else"));
    }

    #[test]
    fn test_while_loop() {
        ast::reset_ids();
        let module = Parser::parse("fn main() { x: int = 3; while (x > 0) { x = x - 1; } }").unwrap();
        let c = compile_to_c(&module);
        assert!(c.contains("while ("));
        assert!(c.contains("x = (x - 1)"));
    }

    #[test]
    fn test_for_loop() {
        ast::reset_ids();
        let module = Parser::parse("fn main() { sum: int = 0; for (i in 0..5) { sum = sum + i; } print(sum); }").unwrap();
        let c = compile_to_c(&module);
        assert!(c.contains("for (int64_t i ="));
    }

    #[test]
    fn test_fn_call() {
        ast::reset_ids();
        let module = Parser::parse("fn add(x: int) -> int { return x * 2; } fn main() { print(add(5)); }").unwrap();
        let c = compile_to_c(&module);
        assert!(c.contains("int64_t add(int64_t x)"));
        assert!(c.contains("return (x * 2)"));
        assert!(c.contains("add(5)"));
    }

    #[test]
    fn test_struct_lit_and_field() {
        ast::reset_ids();
        let module = Parser::parse("struct Point { x: int, y: int } fn main() { p: Point = Point { x: 1, y: 2 }; print(p.x); }").unwrap();
        let c = compile_to_c(&module);
        assert!(c.contains("typedef struct { int64_t x; int64_t y; } Point;"));
        assert!(c.contains("Point p = (Point){ .x = 1, .y = 2 };"));
        assert!(c.contains("(p.x)"));
    }

    #[test]
    fn test_tuple_lit() {
        ast::reset_ids();
        let module = Parser::parse("fn main() { print((10, 20, 30).0); }").unwrap();
        let c = compile_to_c(&module);
        eprintln!("C OUTPUT:\n{}", c);
        assert!(c.contains("__yk_t0"));
        assert!(c.contains("10, 20, 30"));
    }
}
