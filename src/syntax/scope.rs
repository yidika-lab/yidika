use super::ast::{Expr, ExprNode, Param, Pattern, Stmt, StmtNode};
use super::visit::{each_child, each_stmt_child, Child};

#[derive(Default)]
struct Names {
    used: Vec<String>,
    bound: Vec<String>,
}

impl Names {
    fn use_name(&mut self, name: &str) {
        if !self.used.iter().any(|n| n == name) {
            self.used.push(name.to_string());
        }
    }

    fn bind(&mut self, name: &str) {
        self.bound.push(name.to_string());
    }

    fn free(self) -> Vec<String> {
        let bound = self.bound;
        self.used.into_iter().filter(|n| !bound.contains(n)).collect()
    }
}

pub fn free_names(params: &[Param], body: &ExprNode) -> Vec<String> {
    let mut names = Names::default();
    params.iter().for_each(|p| names.bind(&p.name));
    visit_expr(body, &mut names);
    names.free()
}

pub fn names_captured_in(body: &[StmtNode]) -> Vec<String> {
    let mut captured = Names::default();
    body.iter().for_each(|s| find_lambdas_in_stmt(s, &mut captured));
    captured.used
}

fn find_lambdas_in_stmt(stmt: &StmtNode, captured: &mut Names) {
    each_stmt_child(stmt, &mut |child| find_lambdas(child, captured));
}

fn find_lambdas(child: Child, captured: &mut Names) {
    match child {
        Child::Stmt(s) => find_lambdas_in_stmt(s, captured),
        Child::Expr(e) => match &e.value {
            Expr::FnLit(params, _, body) | Expr::Closure(params, body) => {
                free_names(params, body).iter().for_each(|n| captured.use_name(n));
            }
            _ => each_child(e, &mut |c| find_lambdas(c, captured)),
        },
    }
}

fn visit_expr(expr: &ExprNode, names: &mut Names) {
    match &expr.value {
        Expr::Ident(name) => names.use_name(name),
        Expr::FnLit(params, _, body) | Expr::Closure(params, body) => {
            free_names(params, body).iter().for_each(|n| names.use_name(n));
        }
        Expr::ForIn(var, iter, body) => {
            visit_expr(iter, names);
            names.bind(var);
            visit_expr(body, names);
        }
        Expr::TryCatch(body, var, handler) => {
            body.iter().for_each(|s| visit_stmt(s, names));
            names.bind(var);
            handler.iter().for_each(|s| visit_stmt(s, names));
        }
        Expr::Match(scrutinee, arms) => {
            visit_expr(scrutinee, names);
            for arm in arms {
                bind_pattern(&arm.pattern, names);
                if let Some(g) = &arm.guard {
                    visit_expr(g, names);
                }
                visit_expr(&arm.body, names);
            }
        }
        _ => each_child(expr, &mut |child| visit_child(child, names)),
    }
}

fn visit_child(child: Child, names: &mut Names) {
    match child {
        Child::Expr(e) => visit_expr(e, names),
        Child::Stmt(s) => visit_stmt(s, names),
    }
}

fn visit_stmt(stmt: &StmtNode, names: &mut Names) {
    match &stmt.value {
        Stmt::Decl { name, value, .. } => {
            visit_expr(value, names);
            names.bind(name);
        }
        Stmt::Assign(name, value) => {
            names.use_name(name);
            visit_expr(value, names);
        }
        Stmt::For(var, iter, body, _) => {
            visit_expr(iter, names);
            names.bind(var);
            body.iter().for_each(|s| visit_stmt(s, names));
        }
        Stmt::Destruct(pattern, value) => {
            visit_expr(value, names);
            bind_pattern(pattern, names);
        }
        _ => each_stmt_child(stmt, &mut |child| visit_child(child, names)),
    }
}

fn bind_pattern(pattern: &Pattern, names: &mut Names) {
    match pattern {
        Pattern::Ident(n) | Pattern::Rest(n) => names.bind(n),
        Pattern::Destruct(fields) => fields.iter().for_each(|(_, p)| bind_pattern(p, names)),
        Pattern::ListDestruct(items) | Pattern::Variant(_, items) => items.iter().for_each(|p| bind_pattern(p, names)),
        Pattern::LitInt(_) | Pattern::LitReal(_) | Pattern::LitStr(_) | Pattern::LitBool(_) | Pattern::Ignore => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parser::Parser;

    fn main_body(src: &str) -> Vec<StmtNode> {
        let module = Parser::parse(src).unwrap();
        module.items.into_iter().find_map(|item| match item.value {
            crate::syntax::ast::ItemKind::Fn { name, body, .. } if name == "main" => Some(body),
            _ => None,
        }).unwrap()
    }

    fn lambda_free_names(src: &str) -> Vec<String> {
        let body = main_body(src);
        let mut found = Vec::new();
        for stmt in &body {
            if let Stmt::Decl { value, .. } | Stmt::Assign(_, value) = &stmt.value {
                if let Expr::FnLit(params, _, b) | Expr::Closure(params, b) = &value.value {
                    found = free_names(params, b);
                }
            }
        }
        found
    }

    #[test]
    fn parameters_and_locals_are_not_free() {
        assert_eq!(lambda_free_names("fn main() { k: int = 1; f = (x) => { y: int = x + k; return y; }; }"), vec!["k"]);
    }

    #[test]
    fn assignments_count_as_uses() {
        assert_eq!(lambda_free_names("fn main() { c: int = 0; f = () => { c = c + 1; return c; }; }"), vec!["c"]);
    }

    #[test]
    fn nested_lambdas_pass_their_free_names_up() {
        assert_eq!(lambda_free_names("fn main() { a: int = 1; f = (x) => (y) => x + y + a; }"), vec!["a"]);
    }

    #[test]
    fn captured_names_of_a_function_body() {
        let body = main_body("fn main() { a: int = 1; b: int = 2; f = () => a; g = (b) => b; }");
        assert_eq!(names_captured_in(&body), vec!["a"]);
    }
}
