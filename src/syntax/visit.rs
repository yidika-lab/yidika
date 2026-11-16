use super::ast::{Expr, ExprNode, Stmt, StmtNode};

pub enum Child<'a> {
    Expr(&'a ExprNode),
    Stmt(&'a StmtNode),
}

pub fn each_child<'a>(expr: &'a ExprNode, f: &mut dyn FnMut(Child<'a>)) {
    let mut e = |x: &'a ExprNode| f(Child::Expr(x));
    match &expr.value {
        Expr::LitInt(_) | Expr::LitHex(_) | Expr::LitReal(_) | Expr::LitStr(_) | Expr::LitChar(_)
        | Expr::LitBool(_) | Expr::LitSymbol(_) | Expr::LitNull | Expr::Ident(_) => {}
        Expr::UnOp(_, a) | Expr::Field(a, _) | Expr::Loop(a) | Expr::PostInc(a) | Expr::PostDec(a)
        | Expr::Await(a) | Expr::Spawn(a) | Expr::ResultOk(a) | Expr::ResultErr(a) | Expr::Try(a)
        | Expr::SafeCall(a, _) | Expr::AsConst(a) | Expr::As(a, _) | Expr::FnLit(_, _, a) | Expr::Closure(_, a) => e(a),
        Expr::BinOp(a, _, b) | Expr::Index(a, b) | Expr::While(a, b) | Expr::Range(a, b)
        | Expr::LitComplex(a, b) | Expr::Elvis(a, b) | Expr::ForIn(_, a, b) => {
            e(a);
            e(b);
        }
        Expr::Call(callee, args) => {
            e(callee);
            args.iter().for_each(e);
        }
        Expr::If(c, t, other) => {
            e(c);
            e(t);
            if let Some(o) = other {
                e(o);
            }
        }
        Expr::Match(scrutinee, arms) => {
            e(scrutinee);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    e(g);
                }
                e(&arm.body);
            }
        }
        Expr::StructLit(_, fields) => fields.iter().for_each(|(_, v)| e(v)),
        Expr::ListLit(items) | Expr::SetLit(items) | Expr::TupleLit(items) | Expr::VectorLit(items)
        | Expr::Variant(_, _, items) => items.iter().for_each(e),
        Expr::MapLit(pairs) => pairs.iter().for_each(|(k, v)| {
            e(k);
            e(v);
        }),
        Expr::MatrixLit(rows) => rows.iter().flatten().for_each(e),
        Expr::Block(stmts) => stmts.iter().for_each(|s| f(Child::Stmt(s))),
        Expr::TryCatch(body, _, handler) => body.iter().chain(handler).for_each(|s| f(Child::Stmt(s))),
    }
}

pub fn each_stmt_child<'a>(stmt: &'a StmtNode, f: &mut dyn FnMut(Child<'a>)) {
    match &stmt.value {
        Stmt::Decl { value, .. } | Stmt::Assign(_, value) | Stmt::Destruct(_, value) | Stmt::Expr(value) => f(Child::Expr(value)),
        Stmt::Return(value) => {
            if let Some(v) = value {
                f(Child::Expr(v));
            }
        }
        Stmt::For(_, iter, body, _) => {
            f(Child::Expr(iter));
            body.iter().for_each(|s| f(Child::Stmt(s)));
        }
        Stmt::While(cond, body) => {
            f(Child::Expr(cond));
            body.iter().for_each(|s| f(Child::Stmt(s)));
        }
        Stmt::Loop(body) => body.iter().for_each(|s| f(Child::Stmt(s))),
        Stmt::If(cond, then_body, else_body) => {
            f(Child::Expr(cond));
            then_body.iter().chain(else_body.iter().flatten()).for_each(|s| f(Child::Stmt(s)));
        }
    }
}
