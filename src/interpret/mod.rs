pub mod builtins;

mod value;
mod class;
mod env;
mod expr;
mod stmt;

pub use value::{Value, EvalResult, is_truthy, is_copy_value, cmp_binop};
pub use class::{FnDef, ClassDef, ObjectDef, Frame, MethodCache, InlineCache};
pub use env::{Interpreter, SubInterpreterPool, fast_hash};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::ast;
    use crate::syntax::parser::Parser;

    fn run(source: &str) -> String {
        ast::reset_ids();
        let module = Parser::parse(source).unwrap();
        let mut interp = Interpreter::new();
        interp.load_module(&module);
        interp.run_main().unwrap()
    }

    fn run_invalid(source: &str) -> String {
        ast::reset_ids();
        let module = Parser::parse(source).unwrap();
        let mut interp = Interpreter::new();
        interp.load_module(&module);
        interp.run_main().unwrap_err().to_string()
    }

    #[test]
    fn empty_main() {
        let out = run("fn main() {}");
        assert_eq!(out, "");
    }

    #[test]
    fn arithmetic() {
        let out = run("fn main() { x: int = 1 + 2 * 3; print(x); }");
        assert_eq!(out, "7\n");
    }

    #[test]
    fn variable_mutation() {
        let out = run("fn main() { x: int = 10; x = x + 5; print(x); }");
        assert_eq!(out, "15\n");
    }

    #[test]
    fn if_else_branch() {
        let out = run("fn main() { x: int = 5; if (x > 3) { print(\"big\"); } else { print(\"small\"); } }");
        assert_eq!(out, "big\n");
    }

    #[test]
    fn if_else_false_branch() {
        let out = run("fn main() { x: int = 1; if (x > 3) { print(\"big\"); } else { print(\"small\"); } }");
        assert_eq!(out, "small\n");
    }

    #[test]
    fn for_loop() {
        let out = run("fn main() { sum: int = 0; for (i in 0..5) { sum = sum + i; } print(sum); }");
        assert_eq!(out, "10\n");
    }

    #[test]
    fn while_loop() {
        let out = run("fn main() { x: int = 3; while (x > 0) { print(x); x = x - 1; } }");
        assert_eq!(out, "3\n2\n1\n");
    }

    #[test]
    fn function_call() {
        let out = run("fn double(x: int) -> int { return x * 2; } fn main() { print(double(21)); }");
        assert_eq!(out, "42\n");
    }

    #[test]
    fn nested_calls() {
        let out = run("fn add(a: int, b: int) -> int { return a + b; } fn main() { print(add(add(1, 2), 3)); }");
        assert_eq!(out, "6\n");
    }

    #[test]
    fn bool_operators() {
        let out = run("fn main() { t: bool = true; f: bool = false; print(t && f); print(t || f); print(!t); }");
        assert_eq!(out, "false\ntrue\nfalse\n");
    }

    #[test]
    fn comparison() {
        let out = run("fn main() { print(1 < 2); print(2 <= 2); print(3 > 4); }");
        assert_eq!(out, "true\ntrue\nfalse\n");
    }

    #[test]
    fn loop_break_through_return() {
        let out = run("fn main() { loop { print(\"once\"); return; } }");
        assert_eq!(out, "once\n");
    }

    #[test]
    fn scope_blocks() {
        let out = run("fn main() { x: int = 1; { x: int = 2; print(x); } print(x); }");
        assert_eq!(out, "2\n1\n");
    }

    #[test]
    fn const_global() {
        let out = run("const PI: int = 3; fn main() { print(PI); }");
        assert_eq!(out, "3\n");
    }

    #[test]
    fn for_with_range() {
        let out = run("fn main() { sum: int = 0; for (i in 1..4) { sum = sum + i; } print(sum); }");
        assert_eq!(out, "6\n");
    }

    #[test]
    fn string_concat() {
        let out = run("fn main() { s: str = \"Hello, \" + \"world!\"; print(s); }");
        assert_eq!(out, "Hello, world!\n");
    }

    #[test]
    fn list_literal() {
        let out = run("fn main() { items: auto = [1, 2, 3]; print(items); }");
        assert_eq!(out, "[1, 2, 3]\n");
    }

    #[test]
    fn list_index() {
        let out = run("fn main() { items: auto = [10, 20, 30]; print(items[1]); }");
        assert_eq!(out, "20\n");
    }

    #[test]
    fn list_index_out_of_bounds() {
        let r = run_invalid("fn main() { items: auto = [1, 2]; x: auto = items[5]; }");
        assert!(r.contains("out of bounds"), "should error: {}", r);
    }

    #[test]
    fn string_index() {
        let out = run("fn main() { s: str = \"hello\"; print(s[0]); }");
        assert_eq!(out, "h\n");
    }

    #[test]
    fn struct_literal_and_field() {
        let out = run("struct Point { x: int; y: int; } fn main() { p: auto = Point { x: 10, y: 20 }; print(p.x); print(p.y); }");
        assert_eq!(out, "10\n20\n");
    }

    #[test]
    fn struct_unknown_field_error() {
        let err = run_invalid("struct A { x: int; } fn main() { p: auto = A { z: 1 }; }");
        assert!(err.contains("has no field"), "should error on unknown field: {}", err);
    }

    #[test]
    fn tuple_literal_and_field() {
        let out = run("fn main() { print((10, 20, 30).0); print((10, 20, 30).1); print((10, 20, 30).2); }");
        assert_eq!(out, "10\n20\n30\n");
    }

    #[test]
    fn tuple_mixed_types() {
        let out = run(r#"fn main() { print((1, "hello", true).1); print((1, "hello", true).2); }"#);
        assert_eq!(out, "hello\ntrue\n");
    }

    #[test]
    fn map_literal() {
        let out = run(r#"fn main() { print(map { 1: "one", 2: "two" }); }"#);
        assert_eq!(out, "{1: one, 2: two}\n");
    }

    #[test]
    fn set_literal() {
        let out = run(r#"fn main() { print(set { 1, 2, 3 }); }"#);
        assert_eq!(out, "set{1, 2, 3}\n");
    }

    #[test]
    fn set_dedup() {
        let out = run(r#"fn main() { print(set { 1, 2, 2, 3, 1 }); }"#);
        assert_eq!(out, "set{1, 2, 3}\n");
        let out = run(r#"fn main() { s: auto = set { 1, 2, 3 }; s.add(2); s.add(4); print(s); }"#);
        assert_eq!(out, "set{1, 2, 3, 4}\n");
    }

    #[test]
    fn set_has() {
        let out = run(r#"fn main() { s: auto = set { 1, 2, 3 }; print(s.has(2)); print(s.has(5)); }"#);
        assert_eq!(out, "true\nfalse\n");
    }

    #[test]
    fn set_add_remove() {
        let out = run(r#"
            fn main() {
                s: auto = set { 1, 2, 3 };
                s.remove(2);
                s.add(4);
                print(s);
            }
        "#);
        assert_eq!(out, "set{1, 3, 4}\n");
    }

    #[test]
    fn set_len() {
        let out = run(r#"fn main() { s: auto = set { 10, 20, 30 }; print(s.len()); }"#);
        assert_eq!(out, "3\n");
    }

    #[test]
    fn set_to_list() {
        let out = run(r#"fn main() { s: auto = set { 1, 2, 3 }; print(s.to_list()); }"#);
        assert_eq!(out, "[1, 2, 3]\n");
    }

    #[test]
    fn set_union_intersection_difference() {
        let out = run(r#"
            fn main() {
                a: auto = set { 1, 2, 3, 4 };
                b: auto = set { 3, 4, 5, 6 };
                print(a.union(b));
                print(a.intersection(b));
                print(a.difference(b));
            }
        "#);
        assert_eq!(out, "set{1, 2, 3, 4, 5, 6}\nset{3, 4}\nset{1, 2}\n");
    }

    #[test]
    fn set_equality() {
        let out = run(r#"fn main() { a: auto = set { 1, 2, 3 }; b: auto = set { 3, 2, 1 }; print(a == b); print(a == set { 1, 2 }); }"#);
        assert_eq!(out, "true\nfalse\n");
    }

    #[test]
    fn fn_literal() {
        let out = run(r#"fn main() { print(fn () -> Int { 42 }); }"#);
        assert_eq!(out, "<fn>\n");
    }

    #[test]
    fn fstring_literal() {
        let out = run(r#"fn main() { name: str = "world"; print(f'hello {name}'); }"#);
        assert_eq!(out, "hello world\n");
    }

    #[test]
    fn fstring_expr() {
        let out = run(r#"fn main() { print(f'sum: {1 + 2}'); }"#);
        assert_eq!(out, "sum: 3\n");
    }

    #[test]
    fn backtick_string() {
        let out = run("fn main() { print(`hello`); }");
        assert_eq!(out, "hello\n");
    }

    #[test]
    fn struct_unknown_type_error() {
        let err = run_invalid("fn main() { p: auto = Bogus { x: 1 }; }");
        assert!(err.contains("Unknown struct"), "should error: {}", err);
    }

    #[test]
    fn reassign_const_local() {
        let err = run_invalid("fn main() { x: const = 5; x = 10; }");
        assert!(err.contains("Cannot assign to const"), "should error: {}", err);
    }

    #[test]
    fn reassign_global_const() {
        let err = run_invalid("const X: int = 42; fn main() { X = 99; }");
        assert!(err.contains("Cannot assign to const"), "should error: {}", err);
    }

    #[test]
    fn builtin_len_string() {
        let out = run("fn main() { print(len(\"hello\")); }");
        assert_eq!(out, "5\n");
    }

    #[test]
    fn builtin_len_list() {
        let out = run("fn main() { print(len([10, 20, 30])); }");
        assert_eq!(out, "3\n");
    }

    #[test]
    fn method_style_len() {
        let out = run("fn main() { s: str = \"abc\"; print(s.len()); }");
        assert_eq!(out, "3\n");
    }

    #[test]
    fn builtin_str() {
        let out = run("fn main() { print(str(42) + \"!\"); }");
        assert_eq!(out, "42!\n");
    }

    #[test]
    fn builtin_str_bool() {
        let out = run("fn main() { print(str(true)); }");
        assert_eq!(out, "true\n");
    }

    #[test]
    fn print_variadic() {
        let out = run(r#"fn main() { print("x =", 42, "y =", 7); }"#);
        assert_eq!(out, "x = 42 y = 7\n");
    }

    #[test]
    fn print_empty() {
        let out = run("fn main() { print(); }");
        assert_eq!(out, "\n");
    }

    #[test]
    fn list_push() {
        let out = run("fn main() { items: auto = [1, 2]; items.push(3); print(items); }");
        assert_eq!(out, "[1, 2, 3]\n");
    }

    #[test]
    fn list_pop() {
        let out = run("fn main() { items: auto = [1, 2, 3]; v: auto = items.pop(); print(v); print(items); }");
        assert_eq!(out, "3\n[1, 2]\n");
    }

    #[test]
    fn list_pop_empty_error() {
        let err = run_invalid("fn main() { items: auto = []; items.pop(); }");
        assert!(err.contains("empty list"), "should error: {}", err);
    }

    #[test]
    fn input_with_prompt() {
        let out = run("fn main() { print(\"skip\"); }");
        assert_eq!(out, "skip\n");
    }

    #[test]
    fn try_operator_ok() {
        let out = run("fn foo() -> Result<int, str> { return Ok(42); } fn main() { x: int = foo()?; print(x); }");
        assert_eq!(out, "42\n");
    }

    #[test]
    fn try_operator_propagates_error() {
        let out = run("fn bar() -> Result<int, str> { return Error(\"fail\"); } fn foo() -> Result<int, str> { x: int = bar()?; return Ok(x); } fn main() { r: auto = foo(); print(r); }");
        assert_eq!(out, "Error(fail)\n");
    }

    #[test]
    fn nullable_elvis_operator() {
        let out = run("fn main() { x: int? = null; y: int = x ?: 42; print(y); }");
        assert_eq!(out, "42\n");
    }

    #[test]
    fn nullable_elvis_non_null() {
        let out = run("fn main() { x: int? = 10; y: int = x ?: 42; print(y); }");
        assert_eq!(out, "10\n");
    }

    #[test]
    fn nullable_safe_call_null() {
        let out = run("struct Foo { val: int; } fn main() { x: Foo? = null; v: int? = x?.val; print(v ?: -1); }");
        assert_eq!(out, "-1\n");
    }

    #[test]
    fn nullable_safe_call_non_null() {
        let out = run("struct Foo { val: int; } fn main() { x: Foo? = Foo { val: 99 }; v: int? = x?.val; print(v ?: -1); }");
        assert_eq!(out, "99\n");
    }

    #[test]
    fn nullable_null_literal_type() {
        let out = run("fn main() { x: int? = null; print(x ?: 0); }");
        assert_eq!(out, "0\n");
    }

    #[test]
    fn interface_dispatch() {
        let out = run("
            interface Drawable {
                fn draw(&self) -> str;
            }
            class Circle implements Drawable {
                x: int;
                fn draw(&self) -> str {
                    return \"circle\";
                }
            }
            class Square implements Drawable {
                side: int;
                fn draw(&self) -> str {
                    return \"square\";
                }
            }
            fn describe(d: Drawable) -> str {
                return d.draw();
            }
            fn main() {
                c: Circle = Circle { x: 10 };
                s: Square = Square { side: 5 };
                print(c.draw());
                print(s.draw());
                print(describe(Circle { x: 1 }));
                print(describe(Square { side: 2 }));
            }
        ");
        assert_eq!(out, "circle\nsquare\ncircle\nsquare\n");
    }

    #[test]
    fn union_type_annotation() {
        let out = run("fn main() { x: int | str = 42; print(x); x = \"hello\"; print(x); }");
        assert_eq!(out, "42\nhello\n");
    }

    #[test]
    fn union_type_param() {
        let out = run("
            fn foo(x: int | str) {
                print(x);
            }
            fn main() {
                foo(42);
                foo(\"hello\");
            }
        ");
        assert_eq!(out, "42\nhello\n");
    }

    #[test]
    fn union_match_int_or_str() {
        let out = run("
            fn describe(x: int | str) -> str {
                return match x {
                    42 => \"answer\",
                    \"hello\" => \"greeting\",
                    _ => \"other\",
                };
            }
            fn main() {
                print(describe(42));
                print(describe(\"hello\"));
            }
        ");
        assert_eq!(out, "answer\ngreeting\n");
    }
}
