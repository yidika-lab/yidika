pub mod env;
pub mod typeck;

#[cfg(test)]
mod tests {
    use crate::semantic::env::Env;
    use crate::semantic::typeck::TypeChecker;
    use crate::syntax::ast;
    use crate::syntax::parser::Parser;

    fn typed(source: &str) -> Result<(), String> {
        ast::reset_ids();
        let module = Parser::parse(source).map_err(|e| e.to_string())?;
        let mut env = Env::new();
        let mut checker = TypeChecker::new(&mut env);
        checker.check_module(&module).map_err(|e| e.to_string())
    }

    #[test]
    fn display_line_on_error() {
        let r = typed("fn test() -> int { return unknown; }");
        assert!(r.is_err());
        let err = r.unwrap_err();
        assert!(err.contains("'unknown'"), "error should name the variable");
    }

    #[test]
    fn env_imported_fn_visible() {
        ast::reset_ids();
        let source_a = "export fn greet(name: str) -> str { return name; }";
        let source_b = "use {greet} from \"./a.yk\"; fn main() { msg: str = greet(\"Hi\"); }";

        let module_a = Parser::parse(source_a).unwrap();
        let module_b = Parser::parse(source_b).unwrap();

        let mut env = Env::new();
        let mut checker_a = TypeChecker::new(&mut env);
        checker_a.check_module(&module_a).unwrap();

        let mut checker_b = TypeChecker::new(&mut env);
        let result = checker_b.check_module(&module_b);
        assert!(result.is_ok(), "greet should be visible via env: {:?}", result);
    }

    #[test]
    fn env_missing_import_is_error() {
        ast::reset_ids();
        let source = "use {nonexistent} from \"./lib.yk\"; fn main() { nonexistent(); }";
        let module = Parser::parse(source).unwrap();
        let mut env = Env::new();
        let mut checker = TypeChecker::new(&mut env);
        let result = checker.check_module(&module);
        assert!(result.is_err(), "non-exported symbol should error");
    }

    #[test]
    fn env_type_alias_across_modules() {
        ast::reset_ids();
        let source_a = "export type UserId = int;";
        let source_b = "use {UserId} from \"./types.yk\"; fn main() { id: UserId = 42; }";

        let module_a = Parser::parse(source_a).unwrap();
        let module_b = Parser::parse(source_b).unwrap();

        let mut env = Env::new();
        let mut checker_a = TypeChecker::new(&mut env);
        checker_a.check_module(&module_a).unwrap();

        let mut checker_b = TypeChecker::new(&mut env);
        let result = checker_b.check_module(&module_b);
        assert!(result.is_ok(), "UserId should be visible: {:?}", result);
    }
}
