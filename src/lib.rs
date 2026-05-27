pub mod cli;
pub mod codegen;
pub mod diagnostics;
pub mod hardware;
pub mod interpret;
pub mod jit;
pub mod memory;
pub mod module;
pub mod netlib;
pub mod package;
pub mod runtime;
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
        checker.check_module(&module).map_err(|e| e.to_string())?;

        // Borrow check all function bodies
        for item in &module.items {
            if let ast::ItemKind::Fn { ref params, ref body, .. } = item.value {
                let errors = crate::semantic::borrowck::BorrowChecker::new().check_function(params, body);
                if !errors.is_empty() {
                    return Err(errors.join("\n"));
                }
            }
        }

        Ok(())
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

    #[test]
    fn as_operator_type_conversion() {
        check("fn main() { x: int = 3.14 as int; }").unwrap();  // real -> int
        check("fn main() { x: int = -5 as int; }").unwrap();     // rint -> int (no parens needed now)
        check("fn main() { x: str = 42 as str; }").unwrap();     // int -> str
        check("fn main() { x: str = 3.14 as str; }").unwrap();   // real -> str
        check("fn main() { x: str = true as str; }").unwrap();   // bool -> str
        check("fn main() { x: int = -42 as int; print(x); }").unwrap();
    }

    #[test]
    fn as_operator_rejects_invalid() {
        assert!(check("fn main() { x: real = 42 as real; }").is_err());
        assert!(check("fn main() { x: bool = 1 as bool; }").is_err());
        assert!(check("fn main() { x: int = true as int; }").is_err());
        assert!(check("fn main() { x: real = false as real; }").is_err());
        assert!(check("fn main() { x: bool = 0.0 as bool; }").is_err());
        assert!(check("fn main() { x: str = \"hi\" as int; }").is_err());
    }

    #[test]
    fn generic_fn_call() {
        check("fn first<T>(list: List<T>) -> T { return list[0]; } fn main() { x: int = first([1, 2, 3]); }").unwrap();
        check("fn pair<T, U>(a: T, b: U) -> str { return a as str + b as str; } fn main() { x: str = pair(42, true); }").unwrap();
    }

    #[test]
    fn generic_fn_body_checked() {
        assert!(check("fn bad<T>(x: T) -> int { let y: int = x; return y; } fn main() { bad(\"hello\"); }").is_err());
        assert!(check("fn bad2<T>(x: T) -> int { return x; } fn main() { bad2(\"hello\"); }").is_err());
        check("fn id<T>(x: T) -> T { return x; } fn main() { x: int = id(42); y: str = id(\"hi\"); }").unwrap();
    }

    #[test]
    fn generic_struct() {
        check("struct Pair<T, U> { first: T; second: U; } fn main() { p: Pair<int, str> = Pair { first: 42, second: \"hi\" }; }").unwrap();
        check("struct Pair<T, U> { first: T; second: U; } fn main() { p: Pair<str, bool> = Pair { first: \"a\", second: true }; }").unwrap();
    }

    #[test]
    fn generic_struct_field_access() {
        check("struct Pair<T, U> { first: T; second: U; } fn main() { p: Pair<int, str> = Pair { first: 42, second: \"hi\" }; print(p.first); }").unwrap();
    }

    #[test]
    fn generic_struct_infer() {
        check("struct Pair<T, U> { first: T; second: U; } fn main() { p = Pair { first: 42, second: \"hi\" }; }").unwrap();
    }

    #[test]
    fn generic_struct_wrong_field_type() {
        assert!(check("struct Pair<T, U> { first: T; second: U; } fn main() { p: Pair<int, str> = Pair { first: \"bad\", second: \"hi\" }; }").is_err());
    }

    #[test]
    fn generic_struct_unknown_field() {
        assert!(check("struct Pair<T, U> { first: T; second: U; } fn main() { p: Pair<int, str> = Pair { first: 42, second: \"hi\", third: 1 }; }").is_err());
    }

    #[test]
    fn class_method() {
        check("class Counter { val: int; fn get(self) -> int { return self.val; } fn inc(self) { self.val = self.val + 1; } } fn main() { c: Counter = Counter { val: 0 }; print(c.get()); }").unwrap();
    }

    #[test]
    fn class_method_arg() {
        check("class Adder { val: int; fn add(self, x: int) -> int { return self.val + x; } } fn main() { a: Adder = Adder { val: 10 }; print(a.add(5)); }").unwrap();
    }

    #[test]
    fn class_method_type_mismatch() {
        assert!(check("class Adder { val: int; fn add(self, x: int) -> int { return self.val + x; } } fn main() { a: Adder = Adder { val: 10 }; a.add(\"str\"); }").is_err());
    }

    #[test]
    fn class_unknown_method() {
        assert!(check("class Adder { val: int; fn add(self, x: int) -> int { return self.val + x; } } fn main() { a: Adder = Adder { val: 10 }; a.bad(); }").is_err());
    }

    #[test]
    fn generic_class_method() {
        check("class Box<T> { val: T; fn get(self) -> T { return self.val; } } fn main() { b: Box<int> = Box { val: 42 }; print(b.get()); }").unwrap();
    }

    #[test]
    fn generic_class_method_arg() {
        check("class Box<T> { val: T; fn set(self, v: T) { self.val = v; } } fn main() { b: Box<int> = Box { val: 0 }; b.set(99); }").unwrap();
    }

    #[test]
    fn generic_class_str() {
        check("class Box<T> { val: T; label: str; fn get(self) -> T { return self.val; } } fn main() { b: Box<str> = Box { val: \"hi\", label: \"x\" }; print(b.get()); }").unwrap();
    }

    #[test]
    fn ref_self() {
        check("class C { val: int; fn get(&self) -> int { return self.val; } fn inc(&self) { self.val = self.val + 1; } } fn main() { c: C = C { val: 0 }; c.inc(); print(c.get()); }").unwrap();
    }

    #[test]
    fn ref_self_generic() {
        check("class Box<T> { val: T; fn get(&self) -> T { return self.val; } fn set(&self, v: T) { self.val = v; } } fn main() { b: Box<int> = Box { val: 0 }; b.set(42); print(b.get()); }").unwrap();
    }

    #[test]
    fn nested_generic_struct() {
        check("struct Pair<T, U> { first: T; second: U; } fn main() { p: Pair<Pair<int, str>, bool> = Pair { first: Pair { first: 42, second: \"hi\" }, second: true }; print(p.first.first); }").unwrap();
    }

    #[test]
    fn nested_generic_class_in_struct() {
        check("class Box<T> { val: T; fn get(&self) -> T { return self.val; } fn set(&self, v: T) { self.val = v; } } struct Pair<A, B> { first: A; second: B; } fn main() { p: Pair<Box<int>, str> = Pair { first: Box { val: 99 }, second: \"items\" }; print(p.first.get()); p.first.set(42); print(p.first.get()); }").unwrap();
    }

    #[test]
    fn nested_generic_swap() {
        check("class Box<T> { val: T; fn get(&self) -> T { return self.val; } } class Pair<A, B> { first: A; second: B; fn swap(&self) { tmp: auto = self.first; self.first = self.second; self.second = tmp; } } fn main() { p: Pair<Box<int>, Box<int>> = Pair { first: Box { val: 10 }, second: Box { val: 20 } }; p.swap(); print(p.first.get()); print(p.second.get()); }").unwrap();
    }

    #[test]
    fn auto_type_keyword() {
        check("fn main() { x: auto = 42; print(x); }").unwrap();
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

    #[test]
    fn interface_method_dispatch() {
        check("
            interface Drawable {
                fn draw(&self) -> str;
            }
            class Circle implements Drawable {
                x: int;
                fn draw(&self) -> str {
                    return \"drawing circle\";
                }
            }
            fn describe(d: Drawable) -> str {
                return d.draw();
            }
            fn main() {
                d: Drawable = Circle { x: 10 };
                print(d.draw());
                print(describe(Circle { x: 20 }));
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

    // ─── Enum tests ────────────────────────────────────

    #[test]
    fn enum_basic() {
        check("
            enum Color { Red; Green; Blue; }
            fn main() { c: Color = Color::Red; }
        ").unwrap();
    }

    #[test]
    fn enum_with_payload() {
        check("
            enum Option { Some(x: int); None; }
            fn main() { o: Option = Option::Some(42); }
        ").unwrap();
    }

    #[test]
    fn enum_match() {
        check("
            enum Color { Red; Green; Blue; }
            fn main() { c: Color = Color::Red; match c { Red => 1 }; }
        ").unwrap();
    }

    // ─── Object tests ──────────────────────────────────

    #[test]
    fn object_basic() {
        check("
            object Logger {
                fn log(msg: str) { print(msg); }
            }
            fn main() { Logger.log(\"hello\"); }
        ").unwrap();
    }

    #[test]
    fn object_with_fields() {
        check("
            object Config {
                debug: bool;
                version: str;
                init { debug = true; }
            }
            fn main() { x: bool = Config.debug; }
        ").unwrap();
    }
}
