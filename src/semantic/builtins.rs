use crate::syntax::ast::TypeExpr;

pub const GLOBAL_BUILTINS: [&str; 5] = ["print", "len", "str", "input", "fetch"];

pub fn return_type(name: &str) -> Option<TypeExpr> {
    match name {
        "print" => Some(TypeExpr::None_),
        "len" => Some(TypeExpr::Int(0)),
        "str" | "input" | "fetch" => Some(TypeExpr::Str),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_global_builtin_has_a_return_type() {
        for name in GLOBAL_BUILTINS {
            assert!(return_type(name).is_some(), "{} has no return type", name);
        }
    }

    #[test]
    fn names_without_an_implementation_are_not_builtins() {
        for name in ["typeof", "int", "real", "bool"] {
            assert!(!GLOBAL_BUILTINS.contains(&name));
            assert!(return_type(name).is_none());
        }
    }
}
