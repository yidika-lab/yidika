use std::collections::HashSet;

use crate::syntax::ast::{Expr, ExprNode, Stmt, StmtNode};
use crate::syntax::visit::{each_child, each_stmt_child, Child};

pub struct HandlerParts<'a> {
    pub prelude: Vec<StmtNode>,
    pub response: &'a ExprNode,
}

pub fn split(body: &[StmtNode]) -> Option<HandlerParts<'_>> {
    let (last, before) = body.split_last()?;
    let response = match &last.value {
        Stmt::Return(Some(e)) | Stmt::Expr(e) => e,
        _ => return None,
    };
    let mut prelude = before.to_vec();
    if let Expr::Block(stmts) = &response.value {
        let inner = split(stmts)?;
        prelude.extend(inner.prelude);
        return Some(HandlerParts { prelude, response: inner.response });
    }
    Some(HandlerParts { prelude, response })
}

pub fn unsupported_reason(parts: &HandlerParts) -> Option<&'static str> {
    if !crate::codegen::ownership::returned_values(&parts.prelude).is_empty() {
        return Some("a route handler with an early return");
    }
    let locals = declared_names(&parts.prelude);
    if mentioned_names(parts.response).iter().any(|n| locals.contains(n)) {
        return Some("a route handler whose response uses a local variable");
    }
    None
}

fn declared_names(body: &[StmtNode]) -> HashSet<String> {
    let mut names = HashSet::new();
    body.iter().for_each(|s| collect_declared(s, &mut names));
    names
}

fn collect_declared(stmt: &StmtNode, names: &mut HashSet<String>) {
    if let Stmt::Decl { name, .. } | Stmt::Assign(name, _) = &stmt.value {
        names.insert(name.clone());
    }
    each_stmt_child(stmt, &mut |child| {
        if let Child::Stmt(s) = child {
            collect_declared(s, names);
        }
    });
}

fn mentioned_names(expr: &ExprNode) -> HashSet<String> {
    let mut names = HashSet::new();
    collect_mentioned(expr, &mut names);
    names
}

fn collect_mentioned(expr: &ExprNode, names: &mut HashSet<String>) {
    if let Expr::Ident(name) = &expr.value {
        names.insert(name.clone());
    }
    each_child(expr, &mut |child| match child {
        Child::Expr(e) => collect_mentioned(e, names),
        Child::Stmt(s) => each_stmt_child(s, &mut |c| {
            if let Child::Expr(e) = c {
                collect_mentioned(e, names);
            }
        }),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler_body(src: &str) -> Vec<StmtNode> {
        let module = crate::syntax::parser::Parser::parse(&format!("fn h(req: auto) -> auto {{ {} }}", src)).unwrap();
        match &module.items[0].value {
            crate::syntax::ast::ItemKind::Fn { body, .. } => body.clone(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn the_statements_before_the_response_form_the_prelude() {
        let body = handler_body("print(\"hit\"); x = 1; return \"ok\";");
        let parts = split(&body).unwrap();
        assert_eq!(parts.prelude.len(), 2);
        assert!(matches!(parts.response.value, Expr::LitStr(_)));
        assert!(unsupported_reason(&parts).is_none());
    }

    #[test]
    fn a_closure_block_is_flattened() {
        let module = crate::syntax::parser::Parser::parse("fn main() { f = () => { print(\"a\"); return \"b\"; }; }").unwrap();
        let crate::syntax::ast::ItemKind::Fn { body, .. } = &module.items[0].value else { unreachable!() };
        let (Stmt::Assign(_, value) | Stmt::Decl { value, .. }) = &body[0].value else { unreachable!() };
        let Expr::Closure(_, closure_body) = &value.value else { unreachable!() };
        let wrapped = vec![StmtNode::new(0, crate::diagnostics::span::Span::new(0, 0), Stmt::Return(Some((**closure_body).clone())))];
        let parts = split(&wrapped).unwrap();
        assert_eq!(parts.prelude.len(), 1);
        assert!(matches!(parts.response.value, Expr::LitStr(_)));
    }

    #[test]
    fn a_response_built_from_a_local_is_reported() {
        let body = handler_body("name = \"x\"; return \"hi \" + name;");
        assert!(unsupported_reason(&split(&body).unwrap()).is_some());
    }

    #[test]
    fn an_early_return_is_reported() {
        let body = handler_body("if (req.path == \"/a\") { return \"a\"; } return \"b\";");
        assert!(unsupported_reason(&split(&body).unwrap()).is_some());
    }

    #[test]
    fn a_response_that_only_reads_the_request_is_fine() {
        let body = handler_body("print(\"x\"); return \"p\" + req.path;");
        assert!(unsupported_reason(&split(&body).unwrap()).is_none());
    }
}
