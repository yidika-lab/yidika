use crate::diagnostics::span::Span;
use crate::syntax::ast::*;

pub const ENTRY_FN: &str = "main";
pub const USER_MAIN: &str = "__yk_user_main";

pub struct Entry {
    pub items: Vec<ItemNode>,
    pub globals: Vec<String>,
}

pub fn with_top_level(items: Vec<ItemNode>, body: Vec<StmtNode>) -> Entry {
    if body.is_empty() {
        return Entry { items, globals: Vec::new() };
    }
    let globals = declared_names(&body);
    let mut items: Vec<ItemNode> = items.into_iter().map(rename_user_main).collect();
    let calls_user_main = items.iter().any(|item| is_fn_named(item, USER_MAIN));
    let mut entry_body = body;
    if calls_user_main {
        entry_body.push(call_stmt(USER_MAIN));
    }
    items.insert(0, entry_fn(entry_body));
    Entry { items, globals }
}

fn declared_names(body: &[StmtNode]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for stmt in body {
        if let Stmt::Decl { name, .. } | Stmt::Assign(name, _) = &stmt.value {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
    }
    names
}

fn is_fn_named(item: &ItemNode, wanted: &str) -> bool {
    matches!(&item.value, ItemKind::Fn { name, .. } if name == wanted)
}

fn rename_user_main(mut item: ItemNode) -> ItemNode {
    if let ItemKind::Fn { name, .. } = &mut item.value {
        if name == ENTRY_FN {
            *name = USER_MAIN.to_string();
        }
    }
    item
}

fn call_stmt(callee: &str) -> StmtNode {
    let span = Span::new(0, 0);
    let target = ExprNode::new(fresh_id(), span, Expr::Ident(callee.to_string()));
    let call = ExprNode::new(fresh_id(), span, Expr::Call(Box::new(target), Vec::new()));
    StmtNode::new(fresh_id(), span, Stmt::Expr(call))
}

fn entry_fn(body: Vec<StmtNode>) -> ItemNode {
    ItemNode::new(fresh_id(), Span::new(0, 0), ItemKind::Fn {
        name: ENTRY_FN.to_string(),
        params: Vec::new(),
        ret_type: None,
        body,
        is_async: false,
        generics: Vec::new(),
        is_open: false,
        is_override: false,
        is_final: false,
        is_abstract_method: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parser::Parser;

    fn fn_names(module: &Module) -> Vec<String> {
        module.items.iter().filter_map(|item| match &item.value {
            ItemKind::Fn { name, .. } => Some(name.clone()),
            _ => None,
        }).collect()
    }

    #[test]
    fn a_program_without_top_level_code_is_unchanged() {
        let module = Parser::parse("fn main() { print(1); }").unwrap();
        assert_eq!(fn_names(&module), vec!["main"]);
        assert!(module.globals.is_empty());
    }

    #[test]
    fn top_level_code_becomes_the_entry_function() {
        let module = Parser::parse("x = 1;\nfn f() -> int { return x; }\nprint(f());").unwrap();
        assert_eq!(fn_names(&module), vec!["main", "f"]);
        assert_eq!(module.globals, vec!["x"]);
    }

    #[test]
    fn a_user_main_runs_after_top_level_code() {
        let module = Parser::parse("y: int = 2;\nfn main() { print(y); }").unwrap();
        assert_eq!(fn_names(&module), vec!["main", USER_MAIN]);
        let ItemKind::Fn { body, .. } = &module.items[0].value else { panic!("entry is not a function") };
        assert!(matches!(&body.last().unwrap().value, Stmt::Expr(_)));
    }
}
