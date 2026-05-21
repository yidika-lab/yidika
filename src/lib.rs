pub mod cli;
pub mod codegen;
pub mod diagnostics;
pub mod interpret;
pub mod module;
pub mod package;
pub mod semantic;
pub mod stdlib;
pub mod syntax;

#[cfg(test)]
mod tests {
    use crate::semantic::env::Env;
    use crate::semantic::typeck::TypeChecker;
    use crate::syntax::ast;
    use crate::syntax::parser::Parser;

    fn check(source: &str) -> Result<(), String> {
        ast::reset_ids();
        let module = Parser::parse(source).map_err(|e| e.to_string())?;
        let mut env = Env::new();
        let mut checker = TypeChecker::new(&mut env);
        checker.check_module(&module).map_err(|e| e.to_string())
    }

    #[test]
    fn parse_fn() {
        check("fn add(a: int, b: int) -> int { return a + b; }").unwrap();
    }

    // ─── Mutable by default ───────────────────────────

    #[test]
    fn mutable_by_default() {
        check("fn main() { x: int = 10; x = 6; }").unwrap();
    }

    #[test]
    fn mutable_reassign_ok() {
        check("fn main() { counter: int = 0; counter = counter + 1; }").unwrap();
    }

    #[test]
    fn mutable_loop_counter() {
        check("fn main() { sum: int = 0; for (i in 0..10) { sum = sum + i; } }").unwrap();
    }

    // ─── Const = immutable ────────────────────────────

    #[test]
    fn const_decl_inferred() {
        check("fn main() { name: const = \"Alice\"; }").unwrap();
    }

    #[test]
    fn const_as_const_syntax() {
        check("fn main() { name: str = \"Alice\" as const; }").unwrap();
    }

    #[test]
    fn reject_assign_to_const() {
        let r = check("fn main() { x: const = 6; x = 8; }");
        assert!(r.is_err(), "const var reassignment should error");
    }

    #[test]
    fn reject_assign_to_const_explicit() {
        let r = check("fn main() { x: int = 5 as const; x = 10; }");
        assert!(r.is_err(), "as const var reassignment should error");
    }

    // ─── Type strictness ──────────────────────────────

    #[test]
    fn type_strict_int_to_int_ok() {
        check("fn main() { x: int = 10; x = 6; }").unwrap();
    }

    #[test]
    fn reject_redeclaration() {
        let r = check("fn main() { x: int = 1; x: int = 2; }");
        assert!(r.is_err());
    }

    #[test]
    fn reject_unknown_var() {
        let r = check("fn test() -> int { return unknown_var; }");
        assert!(r.is_err());
    }

    // ─── Imports ──────────────────────────────────────

    #[test]
    fn parse_imports() {
        check(r#"
            use {x, y} from "./lib.yk";
            use z from "c++:./header.hpp";
            fn main() {}
        "#).unwrap();
    }

    // ─── Control flow ─────────────────────────────────

    #[test]
    fn parse_if_for_loop() {
        check("
            fn test() -> int {
                x: int = 0;
                if (x > 0) { return 1; } else { return 0; }
                for (i in 0..10) { y: int = i; }
                loop { return 0; }
            }
        ").unwrap();
    }

    // ─── Misc ─────────────────────────────────────────

    #[test]
    fn parse_struct_type_const() {
        check("
            struct Person { name: str; age: int8; }
            type User = int | str;
            const VERSION: int = 1;
        ").unwrap();
    }

    #[test]
    fn type_check_simple() {
        check("
            fn add(a: int, b: int) -> int { return a + b; }
            fn main() { x: int = add(1, 2); }
        ").unwrap();
    }

    #[test]
    fn parse_spawn_async() {
        check("
            async fn work() -> int { return 42; }
            fn main() { task: auto = spawn work(); }
        ").unwrap();
    }

    #[test]
    fn parse_result_ok() {
        check("fn main() { ok: auto = Ok(42); }").unwrap();
    }

    #[test]
    fn parse_empty() {
        check("").unwrap();
    }

    // ─── Full program ─────────────────────────────────

    #[test]
    fn full_program() {
        check("
            const VERSION: int = 1;
            struct Person { name: str; age: int8; }
            type User = int | str;

            fn add(a: int, b: int) -> int { return a + b; }

            async fn fetch(url: str) -> str {
                data: str = await http.get(url);
                return data;
            }

            fn main() {
                x: int = 10;
                x = 6;
                name: const = \"Alice\";
                counter: int = 0;
                counter = counter + 1;
                if (x > 10) { result: int = add(x, 5); } else { result: int = 0; }
                for (i in 0..10) { sq: int = i * i; }
                ok_result: auto = Ok(42);
            }
        ").unwrap();
    }

    // ─── Stdlib std namespace ──────────────────────────

    #[test]
    fn std_import_as_namespace() {
        check("
            use std from \"std\";
            fn main() { result: str = std.fs.read(\"test.txt\"); }
        ").unwrap();
    }

    #[test]
    fn std_import_submodule() {
        check("
            use {fs} from \"std\";
            fn main() { result: str = fs.read(\"test.txt\"); }
        ").unwrap();
    }

    #[test]
    fn std_import_submodule_math() {
        check("
            use {math} from \"std\";
            fn main() { x: real = math.cos(0.0); }
        ").unwrap();
    }

    #[test]
    fn std_import_submodule_json() {
        check("
            use {json} from \"std\";
            fn main() { x: str = json.stringify(42); }
        ").unwrap();
    }

    #[test]
    fn std_import_submodule_re() {
        check("
            use {re} from \"std\";
            fn main() { x: bool = re.match(\"\\\\d+\", \"hi\"); }
        ").unwrap();
    }

    #[test]
    fn std_import_submodule_sys() {
        check("
            use {sys} from \"std\";
            fn main() { x: int = sys.pid(); }
        ").unwrap();
    }

    #[test]
    fn ffi_rust_syntax_parses() {
        check("
            use {gpu} from \"rust:./compute\";
            fn main() {}
        ").unwrap();
    }

    #[test]
    fn std_fs_read_types_ok() {
        check("
            use std from \"std\";
            fn main() { x: str = std.fs.read(\"test.txt\"); }
        ").unwrap();
    }

    #[test]
    fn std_sys_pid_returns_int() {
        check("
            use std from \"std\";
            fn main() { x: int = std.sys.pid(); }
        ").unwrap();
    }

    #[test]
    fn std_json_stringify_returns_str() {
        check("
            use std from \"std\";
            fn main() { x: str = std.json.stringify(42); }
        ").unwrap();
    }
}
