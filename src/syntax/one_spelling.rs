#[cfg(test)]
mod tests {
    use crate::syntax::parser::Parser;

    fn parse_error(source: &str) -> String {
        match Parser::parse(source) {
            Ok(_) => String::new(),
            Err(e) => e.to_string(),
        }
    }

    fn check_error(source: &str) -> String {
        let module = match Parser::parse(source) {
            Ok(module) => module,
            Err(e) => return e.to_string(),
        };
        let mut env = crate::semantic::env::Env::new();
        let mut checker = crate::semantic::typeck::TypeChecker::new(&mut env);
        checker.check_module(&module).err().map(|e| e.to_string()).unwrap_or_default()
    }

    #[test]
    fn a_c_style_for_loop_says_what_to_write_instead() {
        assert!(parse_error("for (i = 0; i < 3; i += 1) { print(i); }").contains("for (i in 0..n)"));
    }

    #[test]
    fn println_is_not_a_second_name_for_print() {
        assert!(check_error("println(1);").contains("println"));
    }

    #[test]
    fn none_is_not_a_second_name_for_null() {
        assert!(check_error("x = None;").contains("None"));
    }

    #[test]
    fn infiny_is_not_a_second_name_for_loop() {
        assert!(!parse_error("infiny { print(1); }").is_empty());
    }

    #[test]
    fn a_class_implements_an_interface_with_implements_only() {
        assert!(parse_error("interface I { fn f(&self) -> int; }\nclass C implements I { fn f(&self) -> int { return 1; } }").is_empty());
        assert!(!parse_error("interface I { fn f(&self) -> int; }\nclass C use I { fn f(&self) -> int { return 1; } }").is_empty());
    }

    #[test]
    fn collection_constructors_tuple_and_map_are_gone() {
        assert!(!check_error("t = tuple{1, 2};").is_empty());
        assert!(!check_error("m = map{\"a\": 1};").is_empty());
    
    }

    #[test]
    fn a_constant_is_written_with_const_only() {
        assert!(check_error("const limit = 100;\nconst name: str = \"Ada\";\nprint(limit, name);").is_empty());
        assert!(parse_error("fn main() { x: const<int> = 1; }").contains("const name = value"));
        assert!(!parse_error("fn main() { x: int = 1 as const; }").is_empty());
    }

    #[test]
    fn a_constant_cannot_be_assigned_again() {
        assert!(!check_error("fn main() { const x = 5; x = 6; }").is_empty());
    }

    #[test]
    fn collection_types_are_written_as_in_typescript() {
        assert!(check_error("names: str[] = [\"a\"];\nids: Set<int> = {1, 2};\nprint(names, ids);").is_empty());
        assert!(parse_error("xs: list[int] = [1];").contains("T[]"));
        assert!(parse_error("xs: [int] = [1];").contains("T[]"));
        assert!(parse_error("xs: List<int> = [1];").contains("T[]"));
        assert!(parse_error("s: set[int] = {1};").contains("Set<T>"));
        assert!(parse_error("fn f(m: map[str, int]) { }").contains("Map<K, V>"));
    }
}
