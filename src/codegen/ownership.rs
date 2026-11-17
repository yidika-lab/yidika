use std::collections::{HashMap, HashSet};

use crate::syntax::ast::{Expr, ExprNode, ItemKind, Module, Pattern, Stmt, StmtNode};
use crate::syntax::visit::{each_child, each_stmt_child, Child};

const READ_ONLY_CALLS: [&str; 2] = ["print", "len"];

pub const STR_FRESH_TYPES: [&str; 5] = ["i64", "double", "i1", "%yk_complex", "%yk_string"];

pub fn assigned_values(body: &[StmtNode]) -> Vec<&ExprNode> {
    let mut values = Vec::new();
    body.iter().for_each(|s| collect_assigned(s, &mut values));
    values
}

fn collect_assigned<'a>(stmt: &'a StmtNode, values: &mut Vec<&'a ExprNode>) {
    if let Stmt::Decl { value, .. } | Stmt::Assign(_, value) = &stmt.value {
        values.push(value);
    }
    each_stmt_child(stmt, &mut |child| collect_assigned_child(child, values));
}

fn collect_assigned_child<'a>(child: Child<'a>, values: &mut Vec<&'a ExprNode>) {
    match child {
        Child::Stmt(s) => collect_assigned(s, values),
        Child::Expr(e) => each_child(e, &mut |c| collect_assigned_child(c, values)),
    }
}

pub fn names_used_outside(modules: &[&Module], entry: &str) -> Vec<String> {
    let mut names = HashSet::new();
    for item in modules.iter().flat_map(|m| &m.items) {
        if !matches!(&item.value, ItemKind::Fn { name, .. } if name == entry) {
            item_names(&item.value, &mut names);
        }
    }
    let mut names: Vec<String> = names.into_iter().collect();
    names.sort();
    names
}

fn item_names(item: &ItemKind, names: &mut HashSet<String>) {
    match item {
        ItemKind::Fn { body, .. } => body.iter().for_each(|s| stmt_names(s, names)),
        ItemKind::Class { super_args, methods, init_body, .. } => {
            super_args.iter().for_each(|e| expr_names(e, names));
            init_body.iter().for_each(|s| stmt_names(s, names));
            methods.iter().for_each(|m| item_names(m, names));
        }
        ItemKind::Object { methods, init_body, .. } => {
            init_body.iter().for_each(|s| stmt_names(s, names));
            methods.iter().for_each(|m| item_names(m, names));
        }
        ItemKind::Const { name, value, .. } => {
            names.insert(name.clone());
            expr_names(value, names);
        }
        _ => {}
    }
}

fn stmt_names(stmt: &StmtNode, names: &mut HashSet<String>) {
    if let Stmt::Decl { name, .. } | Stmt::Assign(name, _) = &stmt.value {
        names.insert(name.clone());
    }
    each_stmt_child(stmt, &mut |child| child_names(child, names));
}

fn expr_names(expr: &ExprNode, names: &mut HashSet<String>) {
    if let Expr::Ident(name) = &expr.value {
        names.insert(name.clone());
    }
    each_child(expr, &mut |child| child_names(child, names));
}

fn child_names(child: Child, names: &mut HashSet<String>) {
    match child {
        Child::Stmt(s) => stmt_names(s, names),
        Child::Expr(e) => expr_names(e, names),
    }
}

pub fn is_read_only_call(callee: &ExprNode) -> bool {
    matches!(&callee.value, Expr::Ident(f) if READ_ONLY_CALLS.contains(&f.as_str()))
}

pub fn freeable_vars(body: &[StmtNode], fixed: &[String], is_fresh: &dyn Fn(&ExprNode) -> bool) -> HashSet<String> {
    let mut facts = Facts::default_with(is_fresh);
    for name in fixed {
        facts.assigned.insert(name.clone(), false);
    }
    body.iter().for_each(|s| facts.stmt(s, false));
    facts.assigned.iter()
        .filter(|(name, all_fresh)| **all_fresh && facts.uses.get(*name) == facts.reads.get(*name))
        .map(|(name, _)| name.clone())
        .collect()
}

struct Facts<'a> {
    is_fresh: &'a dyn Fn(&ExprNode) -> bool,
    assigned: HashMap<String, bool>,
    uses: HashMap<String, usize>,
    reads: HashMap<String, usize>,
}

impl<'a> Facts<'a> {
    fn default_with(is_fresh: &'a dyn Fn(&ExprNode) -> bool) -> Self {
        Self { is_fresh, assigned: HashMap::new(), uses: HashMap::new(), reads: HashMap::new() }
    }

    fn assign(&mut self, name: &str, fresh: bool) {
        let entry = self.assigned.entry(name.to_string()).or_insert(true);
        *entry = *entry && fresh;
    }

    fn bind_unknown(&mut self, name: &str) {
        self.assign(name, false);
    }

    fn stmt(&mut self, stmt: &StmtNode, in_closure: bool) {
        match &stmt.value {
            Stmt::Decl { name, value, .. } | Stmt::Assign(name, value) => {
                let fresh = !in_closure && (self.is_fresh)(value);
                self.assign(name, fresh);
            }
            Stmt::Destruct(pattern, _) => self.pattern_names(pattern),
            Stmt::For(name, _, _, _) => self.bind_unknown(name),
            _ => {}
        }
        each_stmt_child(stmt, &mut |child| self.child(child, in_closure));
    }

    fn child(&mut self, child: Child, in_closure: bool) {
        match child {
            Child::Expr(e) => self.expr(e, in_closure),
            Child::Stmt(s) => self.stmt(s, in_closure),
        }
    }

    fn expr(&mut self, expr: &ExprNode, in_closure: bool) {
        match &expr.value {
            Expr::Ident(name) => *self.uses.entry(name.clone()).or_default() += 1,
            Expr::ForIn(name, _, _) => self.bind_unknown(name),
            Expr::Match(_, arms) => arms.iter().for_each(|arm| self.pattern_names(&arm.pattern)),
            Expr::FnLit(params, _, _) | Expr::Closure(params, _) => params.iter().for_each(|p| self.bind_unknown(&p.name)),
            _ => {}
        }
        if !in_closure {
            self.count_reads(expr);
        }
        let nested = in_closure || matches!(expr.value, Expr::FnLit(..) | Expr::Closure(..));
        each_child(expr, &mut |child| self.child(child, nested));
    }

    fn count_reads(&mut self, expr: &ExprNode) {
        let read_args: Vec<&ExprNode> = match &expr.value {
            Expr::BinOp(a, _, b) => vec![a, b],
            Expr::Call(callee, args) if is_read_only_call(callee) => args.iter().collect(),
            _ => Vec::new(),
        };
        for arg in read_args {
            if let Expr::Ident(name) = &arg.value {
                *self.reads.entry(name.clone()).or_default() += 1;
            }
        }
    }

    fn pattern_names(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Ident(n) | Pattern::Rest(n) => self.bind_unknown(n),
            Pattern::Destruct(fields) => fields.iter().for_each(|(_, p)| self.pattern_names(p)),
            Pattern::ListDestruct(items) | Pattern::Variant(_, items) => items.iter().for_each(|p| self.pattern_names(p)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(src: &str) -> Vec<StmtNode> {
        let module = crate::syntax::parser::Parser::parse(&format!("fn main() {{ {} }}", src)).unwrap();
        match &module.items[0].value {
            crate::syntax::ast::ItemKind::Fn { body, .. } => body.clone(),
            _ => unreachable!(),
        }
    }

    fn fresh_calls(e: &ExprNode) -> bool {
        matches!(&e.value, Expr::Call(c, _) if matches!(&c.value, Expr::Ident(f) if f == "make"))
    }

    fn freeable(src: &str) -> Vec<String> {
        let mut names: Vec<String> = freeable_vars(&body(src), &[], &fresh_calls).into_iter().collect();
        names.sort();
        names
    }

    #[test]
    fn a_fresh_value_only_read_by_print_and_len_is_freeable() {
        assert_eq!(freeable("s = make(1); print(s); n = len(s); s = make(2); print(s + \"!\");"), vec!["s"]);
    }

    #[test]
    fn a_value_that_is_not_known_to_be_fresh_is_kept() {
        assert!(freeable("s = other(1); print(s);").is_empty());
        assert!(freeable("s = make(1); s = other(2); print(s);").is_empty());
    }

    #[test]
    fn a_value_that_escapes_is_kept() {
        assert!(freeable("s = make(1); keep(s);").is_empty());
        assert!(freeable("s = make(1); t = s;").is_empty());
        assert!(freeable("s = make(1); xs = [s];").is_empty());
        assert!(freeable("s = make(1); xs.push(s);").is_empty());
    }

    #[test]
    fn a_value_used_inside_a_closure_is_kept() {
        assert!(freeable("s = make(1); f = () => print(s);").is_empty());
        assert!(freeable("f = () => { s = make(1); print(s); };").is_empty());
    }

    #[test]
    fn a_name_bound_by_a_loop_or_a_pattern_is_kept() {
        assert!(freeable("s = make(1); print(s); for (s in xs) { print(s); }").is_empty());
        assert!(freeable("s = make(1); print(s); match (v) { s => print(s), }").is_empty());
    }

    #[test]
    fn a_global_mentioned_by_another_function_is_reported() {
        let module = crate::syntax::parser::Parser::parse("s = \"a\";\nfn f() { print(s); }\nclass C { fn m(self) { t = 1; } }\nprint(s);").unwrap();
        let names = names_used_outside(&[&module], "main");
        assert!(names.contains(&"s".to_string()));
        assert!(names.contains(&"t".to_string()));
    }

    #[test]
    fn every_assigned_value_is_collected_even_inside_loops() {
        let b = body("s = make(1); while (true) { s = make(2); if (x) { t = make(3); } }");
        assert_eq!(assigned_values(&b).len(), 3);
    }

    #[test]
    fn parameters_are_never_freed() {
        let names = freeable_vars(&body("p = make(1); print(p);"), &["p".to_string()], &fresh_calls);
        assert!(names.is_empty());
    }
}
