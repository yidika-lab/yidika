#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::Mutex;

    static AOT_LOCK: Mutex<()> = Mutex::new(());

    fn run_interp(source: &str) -> Result<String, String> {
        crate::syntax::ast::reset_ids();
        let module = crate::syntax::parser::Parser::parse(source)
            .map_err(|e| e.to_string())?;
        let mut env = crate::semantic::env::Env::new();
        let mut checker = crate::semantic::typeck::TypeChecker::new(&mut env);
        checker.check_module(&module).map_err(|e| e.to_string())?;

        let mut interp = crate::interpret::Interpreter::new();
        interp.tui_mode = false;
        interp.load_module(&module);
        interp.run_main().map_err(|e| e.msg)
    }

    fn run_aot(source: &str, test_name: &str) -> Result<String, String> {
        let _lock = AOT_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("yk_compat_{}", test_name));
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("main.yk");
        std::fs::write(&file_path, source)
            .map_err(|e| format!("write source: {}", e))?;

        let backend = crate::codegen::backend::LlvmBackend;
        crate::cli::build_program(&file_path, false, &backend, false)
            .map_err(|e| format!("AOT build: {}", e))?;

        let exe_path = dir.join("main.exe");
        if !exe_path.exists() {
            return Err("AOT exe not produced".into());
        }
        let output = Command::new(&exe_path)
            .output()
            .map_err(|e| format!("run exe: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("exe exit {:?}: {}", output.status.code(), stderr));
        }

        let _ = std::fs::remove_dir_all(&dir);
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Normalize line endings and trim whitespace
    fn normalize(s: &str) -> String {
        s.replace("\r\n", "\n").replace("\r", "").trim().to_string()
    }

    fn check_compat(source: &str, test_name: &str) {
        let interp_out = run_interp(source).expect("interpreter failed");
        let aot_out = run_aot(source, test_name).expect("AOT build/run failed");
        assert_eq!(
            normalize(&interp_out),
            normalize(&aot_out),
            "compat_{}: interpreter vs AOT output mismatch", test_name
        );
    }

    fn check_compat_approx(source: &str, test_name: &str) {
        let interp_out = run_interp(source).expect("interpreter failed");
        let aot_out = run_aot(source, test_name).expect("AOT build/run failed");
        let interp_norm = normalize(&interp_out);
        let aot_norm = normalize(&aot_out);
        // Both should be non-empty and same type (both numeric)
        assert!(!interp_norm.is_empty(), "compat_{}: interp empty", test_name);
        assert!(!aot_norm.is_empty(), "compat_{}: aot empty", test_name);
        // Both should parse as numbers (approximate comparison)
        let interp_num: i64 = interp_norm.parse().expect("compat_{}: interp not a number");
        let aot_num: i64 = aot_norm.parse().expect("compat_{}: aot not a number");
        // Just verify they're valid — don't compare exact values (PID, timestamps differ)
        assert!(interp_num > 0, "compat_{}: interp value must be positive", test_name);
        assert!(aot_num > 0, "compat_{}: aot value must be positive", test_name);
    }

    // ── Basic print variants ──────────────────────────

    #[test]
    fn compat_print_hello() {
        check_compat("fn main() { print(\"hello\"); }", "print_hello");
    }

    #[test]
    fn compat_print_int() {
        check_compat("fn main() { print(42); }", "print_int");
    }

    #[test]
    fn compat_print_real() {
        check_compat("fn main() { print(3.14); }", "print_real");
    }

    #[test]
    fn compat_print_bool() {
        check_compat("fn main() { print(true); }", "print_bool");
    }

    #[test]
    fn compat_print_str() {
        check_compat("fn main() { print(\"hello world\"); }", "print_str");
    }

    #[test]
    fn compat_arithmetic_int() {
        check_compat("fn main() { print(1 + 2 * 3); }", "arithmetic_int");
    }

    #[test]
    fn compat_fn_call() {
        check_compat(
            "fn double(x: int) -> int { return x * 2; }
             fn main() { print(double(5)); }",
            "fn_call",
        );
    }

    #[test]
    fn compat_struct_field() {
        check_compat(
            "struct Point { x: int, y: int }
             fn main() { p: Point = Point { x: 10, y: 20 }; print(p.x); }",
            "struct_field",
        );
    }

    #[test]
    fn compat_if_else() {
        check_compat(
            "fn main() { x: int = 5; if (x > 3) { print(1); } else { print(0); } }",
            "if_else",
        );
    }

    #[test]
    fn compat_while_loop() {
        check_compat(
            "fn main() { i: int = 0; while (i < 3) { print(i); i = i + 1; } }",
            "while_loop",
        );
    }

    #[test]
    fn compat_for_loop() {
        check_compat(
            "fn main() { for (i in 0..3) { print(i); } }",
            "for_loop",
        );
    }

    #[test]
    fn compat_double_print() {
        check_compat(
            "fn main() { print(1); print(2); print(3); }",
            "double_print",
        );
    }

    #[test]
    fn compat_mixed_print() {
        check_compat(
            r#"fn main() { print(1); print("hello"); print(true); }"#,
            "mixed_print",
        );
    }

    #[test]
    fn compat_string_concat() {
        check_compat(
            r#"fn main() { s: str = "hello " + "world"; print(s); }"#,
            "string_concat",
        );
    }

    // ── Std modules via direct function calls ─────────
    // math/time/net functions are imported directly, not as namespace objects

    #[test]
    fn compat_math_module() {
        check_compat(
            "use { sqrt } from \"math\";
             fn main() { print(sqrt(16.0)); }",
            "math_module",
        );
    }

    #[test]
    fn compat_time_module() {
        check_compat_approx(
            "use { timestamp } from \"time\";
             fn main() { print(timestamp()); }",
            "time_module",
        );
    }

    // ── Std modules via namespace calls ───────────────
    // io/json/datetime/path/base64/re/sys/fs are namespace objects

    #[test]
    fn compat_json_stringify() {
        check_compat(
            r#"use { json } from "json";
             fn main() { print(json.stringify(42)); print(json.stringify("hello")); print(json.stringify(true)); }"#,
            "json_stringify",
        );
    }

    #[test]
    fn compat_re_match() {
        check_compat(
            r#"use { re } from "re";
             fn main() { print(re.match("\\d+", "abc123")); print(re.match("\\d+", "abc")); }"#,
            "re_match",
        );
    }

    #[test]
    fn compat_re_replace() {
        check_compat(
            r#"use { re } from "re";
             fn main() { print(re.replace("\\d+", "a1b2", "X")); }"#,
            "re_replace",
        );
    }

    #[test]
    fn compat_sys_module() {
        check_compat_approx(
            "use { sys } from \"std\";
             fn main() { print(sys.pid()); }",
            "sys_module",
        );
    }

    #[test]
    fn compat_base64_module() {
        check_compat(
            "use { base64 } from \"base64\";
             fn main() { print(base64.encode(\"hello\")); }",
            "base64_module",
        );
    }

    #[test]
    fn compat_path_module() {
        check_compat(
            "use { path } from \"path\";
             fn main() { print(path.is_absolute(\"/tmp\")); }",
            "path_module",
        );
    }

    #[test]
    fn compat_datetime_now() {
        let interp_out = run_interp(
            "use { datetime } from \"datetime\";
             fn main() { print(datetime.now()); }"
        ).expect("interpreter failed");
        let aot_out = run_aot(
            "use { datetime } from \"datetime\";
             fn main() { print(datetime.now()); }",
            "datetime_now"
        ).expect("AOT build/run failed");
        // Both should produce a datetime string; compare format not exact value
        assert!(!interp_out.trim().is_empty(), "interpreter datetime.now() empty");
        assert!(!aot_out.trim().is_empty(), "AOT datetime.now() empty");
        // Same format: "YYYY-MM-DD HH:MM:SS"
        assert_eq!(interp_out.trim().len(), aot_out.trim().len(),
            "datetime_now: output length mismatch (different times OK but format differs)");
    }

    // ── Classes ───────────────────────────────────────

    #[test]
    fn compat_class_field() {
        check_compat(
            "class Dog { name: str; }
             fn main() { d: Dog = Dog { name: \"Rex\" }; print(d.name); }",
            "class_field",
        );
    }

    #[test]
    fn compat_class_method() {
        check_compat(
            "class Dog { name: str; fn speak(self) -> str { return \"woof\"; } }
             fn main() { d: Dog = Dog { name: \"Rex\" }; print(d.speak()); }",
            "class_method",
        );
    }

    #[test]
    fn compat_generic_class() {
        check_compat(
            "class Box<T> { val: T; fn get(self) -> T { return self.val; } }
             fn main() { b: Box<int> = Box { val: 42 }; print(b.get()); }",
            "generic_class",
        );
    }

    // ── HTTP fetch ─────────────────────────────────────

    #[test]
    fn compat_fetch_get() {
        let url = "https://httpbin.org/get";
        let interp_out = run_interp(&format!("fn main() {{ print(fetch(\"{}\")); }}", url))
            .expect("interpreter failed");
        let aot_out = run_aot(&format!("fn main() {{ print(fetch(\"{}\")); }}", url), "fetch_get")
            .expect("AOT build/run failed");
        assert!(!interp_out.trim().is_empty(), "interpreter fetch() empty");
        assert!(!aot_out.trim().is_empty(), "AOT fetch() empty");
        // Both should contain "httpbin" in the response
        assert!(interp_out.contains("httpbin"), "interp response missing httpbin");
        assert!(aot_out.contains("httpbin"), "AOT response missing httpbin");
    }

    // ── try/? ───────────────────────────────────────────

    #[test]
    fn compat_try_ok() {
        check_compat(
            "fn foo() -> Result<int, str> { return Ok(42); }
             fn main() { x: int = foo()?; print(x); }",
            "try_ok",
        );
    }

    #[test]
    fn compat_try_error() {
        check_compat(
            "fn bar() -> Result<int, str> { return Error(\"fail\"); }
             fn foo() -> Result<int, str> { x: int = bar()?; return Ok(x); }
             fn main() { r: auto = foo(); print(r); }",
            "try_error",
        );
    }

    // ── List methods ────────────────────────────────────

    #[test]
    fn compat_list_push_pop() {
        check_compat(
            "fn main() { items: auto = []; items.push(1); items.push(2); items.push(3); print(items.pop()); print(items.pop()); }",
            "list_push_pop",
        );
    }

    #[test]
    fn compat_list_len() {
        check_compat(
            "fn main() { items: auto = [1, 2, 3]; print(len(items)); }",
            "list_len",
        );
    }

    #[test]
    fn compat_list_sort() {
        check_compat(
            "fn main() { items: auto = [3, 1, 2]; items.sort(); print(items.pop()); print(items.pop()); print(items.pop()); }",
            "list_sort",
        );
    }

    #[test]
    fn compat_list_reverse() {
        check_compat(
            "fn main() { items: auto = [1, 2, 3]; items.reverse(); print(items.pop()); }",
            "list_reverse",
        );
    }

    #[test]
    fn compat_list_insert_remove() {
        check_compat(
            "fn main() { items: auto = [1, 2, 3]; items.insert(0, 9); items.remove(1); print(len(items)); print(items.pop()); }",
            "list_insert_remove",
        );
    }

    #[test]
    fn compat_list_clear() {
        check_compat(
            "fn main() { items: auto = [1, 2, 3]; items.clear(); print(len(items)); }",
            "list_clear",
        );
    }

    #[test]
    fn compat_list_index() {
        check_compat(
            "fn main() { items: auto = [10, 20, 30]; print(items[1]); }",
            "list_index",
        );
    }

    // ── net Server ───────────────────────────────────────

    #[test]
    fn compat_net_server_import() {
        check_compat(
            "use { Server } from \"net\"
             fn main() { print(\"net ok\"); }",
            "net_server_import",
        );
    }

    // ── Nullable / Safe-call / Elvis ─────────────────────

    #[test]
    fn compat_nullable_elvis_null() {
        check_compat(
            "fn main() { x: int? = null; y: int = x ?: 42; print(y); }",
            "nullable_elvis_null",
        );
    }

    #[test]
    fn compat_nullable_elvis_non_null() {
        check_compat(
            "fn main() { x: int? = 10; y: int = x ?: 42; print(y); }",
            "nullable_elvis_non_null",
        );
    }

    #[test]
    fn compat_nullable_safe_call_null() {
        check_compat(
            "struct Foo { val: int; }
             fn main() { x: Foo? = null; v: int? = x?.val; print(v ?: -1); }",
            "nullable_safe_call_null",
        );
    }

    #[test]
    fn compat_nullable_safe_call_non_null() {
        check_compat(
            "struct Foo { val: int; }
             fn main() { x: Foo? = Foo { val: 99 }; v: int? = x?.val; print(v ?: -1); }",
            "nullable_safe_call_non_null",
        );
    }

    #[test]
    fn compat_nullable_null_literal() {
        check_compat(
            "fn main() { x: int? = null; print(x ?: 0); }",
            "nullable_null_literal",
        );
    }
}
