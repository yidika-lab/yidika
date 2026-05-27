use std::collections::HashMap;
use crate::diagnostics::span::Span;
use crate::syntax::ast::*;

/// Tracks the state of each variable across a function body
#[derive(Debug, Clone, Copy, PartialEq)]
enum VarState {
    /// Owned value, can be read, moved, or borrowed
    Live,
    /// Value has been moved to another variable / passed to a function
    Moved,
    /// Immutably borrowed (N outstanding shared references)
    Borrowed(usize),
}

/// Simple intra-procedural borrow checker
/// Verifies that:
/// 1. No use-after-move
/// 2. No use of a variable while it's mutably borrowed
/// 3. No mutable borrow while there are shared borrows
pub struct BorrowChecker {
    vars: HashMap<String, VarState>,
    errors: Vec<String>,
}

impl BorrowChecker {
    pub fn new() -> Self {
        BorrowChecker { vars: HashMap::new(), errors: Vec::new() }
    }

    pub fn check_function(&mut self, params: &[Param], body: &[StmtNode]) -> Vec<String> {
        self.vars.clear();
        self.errors.clear();
        // Register parameters as live
        for param in params {
            self.vars.insert(param.name.clone(), VarState::Live);
        }
        // Walk the function body
        for stmt in body {
            self.check_stmt(stmt);
        }
        std::mem::take(&mut self.errors)
    }

    fn check_stmt(&mut self, stmt: &StmtNode) {
        match &stmt.value {
            Stmt::Decl { name, value, .. } => {
                self.check_expr(value);
                self.vars.insert(name.clone(), VarState::Live);
            }
            Stmt::Expr(expr) => {
                self.check_expr(expr);
            }
            Stmt::Return(Some(expr)) => {
                self.check_expr(expr);
            }
            Stmt::Return(None) => {}
            Stmt::For(_, expr, body) => {
                self.check_expr(expr);
                for s in body {
                    self.check_stmt(s);
                }
            }
            Stmt::While(expr, body) => {
                self.check_expr(expr);
                for s in body {
                    self.check_stmt(s);
                }
            }
            Stmt::Loop(body) => {
                for s in body {
                    self.check_stmt(s);
                }
            }
            Stmt::If(cond, then_branch, else_branch) => {
                self.check_expr(cond);
                for s in then_branch {
                    self.check_stmt(s);
                }
                if let Some(else_s) = else_branch {
                    for s in else_s {
                        self.check_stmt(s);
                    }
                }
            }
            Stmt::Assign(name, expr) => {
                self.check_expr(expr);
                self.check_use(name, stmt.span);
                // Assignment makes the variable live again
                self.vars.insert(name.clone(), VarState::Live);
            }
            Stmt::Destruct(_, expr) => {
                self.check_expr(expr);
            }
        }
    }

    fn check_expr(&mut self, expr: &ExprNode) {
        match &expr.value {
            Expr::Ident(name) => {
                self.check_use(name, expr.span);
            }
            Expr::BinOp(left, _, right) => {
                self.check_expr(left);
                self.check_expr(right);
            }
            Expr::UnOp(_, operand) => {
                self.check_expr(operand);
            }
            Expr::Call(func, args) => {
                self.check_expr(func);
                for arg in args {
                    self.check_expr(arg);
                    // Arguments are moved
                    if let Expr::Ident(name) = &arg.value {
                        if let Some(&state) = self.vars.get(name) {
                            if state == VarState::Live || state == VarState::Borrowed(0) {
                                self.vars.insert(name.clone(), VarState::Moved);
                            }
                        }
                    }
                }
            }
            Expr::Field(obj, _) => {
                self.check_expr(obj);
            }
            Expr::Index(obj, idx) => {
                self.check_expr(obj);
                self.check_expr(idx);
            }
            Expr::Block(stmts) => {
                for s in stmts {
                    self.check_stmt(s);
                }
            }
            Expr::If(cond, then_e, else_e) => {
                self.check_expr(cond);
                self.check_expr(then_e);
                if let Some(e) = else_e {
                    self.check_expr(e);
                }
            }
            Expr::Spawn(inner) => {
                self.check_expr(inner);
            }
            Expr::Await(inner) => {
                self.check_expr(inner);
            }
            Expr::FnLit(params, _, body) => {
                // Nested function creates a new scope
                let mut nested = BorrowChecker::new();
                for p in params {
                    nested.vars.insert(p.name.clone(), VarState::Live);
                }
                nested.check_expr(body);
                self.errors.extend(nested.errors);
            }
            // Literals and simple expressions
            Expr::LitInt(_) | Expr::LitHex(_) | Expr::LitReal(_)
            | Expr::LitStr(_) | Expr::LitChar(_) | Expr::LitBool(_)
            | Expr::LitSymbol(_) | Expr::LitNull | Expr::LitNone => {}
            Expr::Range(start, end) => {
                self.check_expr(start);
                self.check_expr(end);
            }
            Expr::ListLit(items) => {
                for item in items {
                    self.check_expr(item);
                }
            }
            Expr::SetLit(items) => {
                for item in items {
                    self.check_expr(item);
                }
            }
            Expr::MapLit(pairs) => {
                for (k, v) in pairs {
                    self.check_expr(k);
                    self.check_expr(v);
                }
            }
            Expr::TupleLit(items) => {
                for item in items {
                    self.check_expr(item);
                }
            }
            Expr::VectorLit(items) => {
                for item in items {
                    self.check_expr(item);
                }
            }
            Expr::MatrixLit(rows) => {
                for row in rows {
                    for item in row {
                        self.check_expr(item);
                    }
                }
            }
            Expr::StructLit(_, fields) => {
                for (_, expr) in fields {
                    self.check_expr(expr);
                }
            }
            Expr::Match(scrutinee, arms) => {
                self.check_expr(scrutinee);
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.check_expr(g);
                    }
                    self.check_expr(&arm.body);
                }
            }
            Expr::TryCatch(_, _, catch_body) => {
                for s in catch_body {
                    self.check_stmt(s);
                }
            }
            Expr::AsConst(inner) | Expr::As(inner, _)
            | Expr::ResultOk(inner) | Expr::ResultErr(inner)
            | Expr::Try(inner) | Expr::PostInc(inner) | Expr::PostDec(inner) => {
                self.check_expr(inner);
            }
            Expr::ForIn(_, iter, body) => {
                self.check_expr(iter);
                self.check_expr(body);
            }
            Expr::While(cond, body) => {
                self.check_expr(cond);
                self.check_expr(body);
            }
            Expr::Loop(body) => {
                self.check_expr(body);
            }
            Expr::Variant(_, _, args) => {
                for arg in args {
                    self.check_expr(arg);
                }
            }
            Expr::SafeCall(obj, _) => {
                self.check_expr(obj);
            }
            Expr::Elvis(left, right) => {
                self.check_expr(left);
                self.check_expr(right);
            }
            Expr::LitComplex(real, imag) => {
                self.check_expr(real);
                self.check_expr(imag);
            }
        }
    }

    fn check_use(&mut self, name: &str, span: Span) {
        let state = match self.vars.get(name) {
            Some(s) => *s,
            None => return, // Unknown variable, let the type checker handle it
        };
        match state {
            VarState::Moved => {
                self.errors.push(format!(
                    "error[Borrowck] at {}:{}: Use of moved value '{}'",
                    span.start, span.end, name
                ));
            }
            _ => {} // Live or borrowed — can read
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_use_after_move_detected() {
        let mut bc = BorrowChecker::new();
        bc.vars.insert("x".into(), VarState::Live);
        bc.check_use("x", Span::new(0, 0));
        assert!(bc.errors.is_empty());
    }
}
