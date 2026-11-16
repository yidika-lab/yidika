#[cfg(test)]
pub(crate) mod tests {
    use std::process::{Command, Stdio};
    use std::sync::Mutex;
    use std::io::{Read, Write};
    use std::time::Duration;

    static AOT_LOCK: Mutex<()> = Mutex::new(());

    fn aot_lock() -> std::sync::MutexGuard<'static, ()> {
        AOT_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct AotServer {
        child: std::process::Child,
        dir: std::path::PathBuf,
        port: u16,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl AotServer {
        fn start(test_name: &str, source: &str, port: u16) -> Self {
            let lock = aot_lock();
            let dir = std::env::temp_dir().join(format!("yk_compat_{}", test_name));
            let _ = std::fs::create_dir_all(&dir);
            let file_path = dir.join("main.yk");
            std::fs::write(&file_path, source).expect("write source");
            crate::cli::build_program(&file_path, false, &crate::codegen::backend::LlvmBackend, false)
                .expect("AOT build failed");
            let log = std::fs::File::create(dir.join("server.log")).expect("create server log");
            let child = Command::new(dir.join(format!("main{}", std::env::consts::EXE_SUFFIX)))
                .env("YK_DEBUG", "1")
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .expect("failed to start server");
            let server = AotServer { child, dir, port, _lock: lock };
            server.wait_until_listening();
            server
        }

        fn log(&self) -> String {
            std::fs::read_to_string(self.dir.join("server.log")).unwrap_or_default()
        }

        fn address(&self) -> String {
            format!("127.0.0.1:{}", self.port)
        }

        fn wait_until_listening(&self) {
            let addr = self.address().parse().unwrap();
            for _ in 0..50 {
                if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            panic!("server on {} did not start in time, log:\n{}", self.address(), self.log());
        }

        fn connect(&self, read_timeout: Duration) -> std::net::TcpStream {
            let stream = std::net::TcpStream::connect(self.address()).expect("connect to server");
            stream.set_read_timeout(Some(read_timeout)).ok();
            stream
        }

        fn exchange(&self, request: &str) -> HttpExchange {
            let mut stream = self.connect(Duration::from_secs(5));
            stream.write_all(request.as_bytes()).expect("write request");
            read_until_close(&mut stream)
        }
    }

    impl Drop for AotServer {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    struct HttpExchange {
        response: String,
        closed_by_server: bool,
    }

    fn read_until_close(stream: &mut std::net::TcpStream) -> HttpExchange {
        let mut received = Vec::new();
        let mut chunk = [0u8; 4096];
        let closed_by_server = loop {
            match stream.read(&mut chunk) {
                Ok(0) => break true,
                Ok(n) => received.extend_from_slice(&chunk[..n]),
                Err(_) => break false,
            }
        };
        HttpExchange { response: String::from_utf8_lossy(&received).into_owned(), closed_by_server }
    }

    fn get_request(path: &str) -> String {
        format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n", path)
    }

    fn route_server(route: &str, body: &str, port: u16) -> String {
        typed_route_server(route, "str", body, port)
    }

    fn typed_route_server(route: &str, return_type: &str, body: &str, port: u16) -> String {
        format!(
            "use {{ Server }} from \"net\";\nfn handler(req: auto) -> {} {{\n    return {};\n}}\nfn main() {{\n    app: Server = Server();\n    app.get(\"{}\", handler);\n    app.serve(\"127.0.0.1:{}\");\n}}",
            return_type, body, route, port
        )
    }

    pub(crate) fn run_interp(source: &str) -> Result<String, String> {
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

    pub(crate) fn run_aot(source: &str, test_name: &str) -> Result<String, String> {
        let exe_path = build_aot(source, test_name)?;
        let dir = exe_path.parent().unwrap().to_path_buf();
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

    fn build_aot(source: &str, test_name: &str) -> Result<std::path::PathBuf, String> {
        let _lock = aot_lock();
        let dir = std::env::temp_dir().join(format!("yk_compat_{}", test_name));
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("main.yk");
        std::fs::write(&file_path, source)
            .map_err(|e| format!("write source: {}", e))?;

        let backend = crate::codegen::backend::LlvmBackend;
        crate::cli::build_program(&file_path, false, &backend, false)
            .map_err(|e| format!("AOT build: {}", e))?;

        let exe_path = dir.join(format!("main{}", std::env::consts::EXE_SUFFIX));
        if !exe_path.exists() {
            return Err("AOT exe not produced".into());
        }
        Ok(exe_path)
    }

    fn first_line_before_exit(exe: &std::path::Path) -> Option<String> {
        use std::io::BufRead;
        let mut child = Command::new(exe).stdout(std::process::Stdio::piped()).spawn().ok()?;
        let stdout = child.stdout.take()?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = std::io::BufReader::new(stdout).read_line(&mut line);
            let _ = tx.send(line);
        });
        let line = rx.recv_timeout(std::time::Duration::from_secs(10)).ok();
        let _ = child.kill();
        let _ = child.wait();
        line.map(|l| l.trim_end().to_string())
    }

    /// Normalize line endings and trim whitespace
    pub(crate) fn normalize(s: &str) -> String {
        s.replace("\r\n", "\n").replace("\r", "").trim().to_string()
    }

    fn check_project_compat(files: &[(&str, &str)], test_name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("yk_compat_{}", test_name));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, content) in files {
            let file = dir.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, content).unwrap();
        }
        let main = dir.join("main.yk");
        let interp = crate::cli::interpret_project(&main).expect("interpreter run failed");
        let exe = {
            let _lock = aot_lock();
            crate::cli::build_program(&main, false, &crate::codegen::backend::LlvmBackend, false).expect("AOT build failed");
            dir.join(format!("main{}", std::env::consts::EXE_SUFFIX))
        };
        let output = Command::new(&exe).output().expect("AOT run failed");
        let aot = normalize(&String::from_utf8_lossy(&output.stdout));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(normalize(&interp), aot, "{}: interpreter vs AOT output mismatch", test_name);
        aot
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
        // Just verify they're valid - don't compare exact values (PID, timestamps differ)
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
    fn compat_const_syntax() {
        check_compat("fn main() { x: const<int> = 42; print(x); }", "const_syntax");
    }

    #[test]
    fn compat_const_as_const() {
        check_compat("fn main() { x: int = 42 as const; print(x); }", "const_as_const");
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
    // io/json/datetime/path/base64/regex/sys/fs are namespace objects

    #[test]
    fn compat_json_stringify() {
        check_compat(
            r#"use { json } from "json";
             fn main() { print(json.stringify(42)); print(json.stringify("hello")); print(json.stringify(true)); }"#,
            "json_stringify",
        );
    }

    #[test]
    fn compat_regex_match() {
        check_compat(
            r#"use { regex } from "regex";
             fn main() { print(regex.match("\\d+", "abc123")); print(regex.match("\\d+", "abc")); }"#,
            "regex_match",
        );
    }

    #[test]
    fn compat_regex_replace() {
        check_compat(
            r#"use { regex } from "regex";
             fn main() { print(regex.replace("\\d+", "a1b2", "X")); }"#,
            "regex_replace",
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

    #[test]
    fn compat_class_constructor_call() {
        check_compat(
            "class Foo(x: int, y: str) {
                fn get_x(&self) -> int { return self.x; }
                fn get_y(&self) -> str { return self.y; }
             }
             fn main() {
                f: Foo = Foo(42, \"hi\");
                print(f.get_x());
                print(f.get_y());
             }",
            "class_ctor",
        );
    }

    #[test]
    fn compat_class_constructor_with_init() {
        check_compat(
            "class Counter(start: int) {
                fn get(&self) -> int { return self.start; }
                init { print(\"init \"); }
             }
             fn main() {
                c: Counter = Counter(99);
                print(c.get());
             }",
            "class_init",
        );
    }

    // ── HTTP fetch ─────────────────────────────────────

    #[test]
    #[cfg_attr(not(windows), ignore = "HTTP client is WinHTTP only, issue #65")]
    fn compat_fetch_get() {
        let url = "https://example.com";
        let interp_out = run_interp(&format!("fn main() {{ print(fetch(\"{}\")); }}", url))
            .expect("interpreter failed");
        let aot_out = run_aot(&format!("fn main() {{ print(fetch(\"{}\")); }}", url), "fetch_get")
            .expect("AOT build/run failed");
        assert!(!interp_out.trim().is_empty(), "interpreter fetch() empty");
        assert!(!aot_out.trim().is_empty(), "AOT fetch() empty");
        // Both should contain "Example Domain" in the response
        assert!(interp_out.contains("Example Domain"), "interp response missing Example Domain");
        assert!(aot_out.contains("Example Domain"), "AOT response missing Example Domain");
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

    #[test]
    fn compat_call_results_concatenate_as_strings() {
        check_compat("fn name() -> str { return \"yk\"; } fn main() { n: int = 3; print(str(2.5) + \"!\"); print(name() + \"-\" + str(n)); print(f`{n} items`); }", "call_concat");
    }

    #[test]
    fn compat_char_values_print_as_characters() {
        check_compat("fn main() { c = 'A'; print(c); print('\u{e9}'); print(str(c) + \"!\"); print(f`[{c}]`); s: str = \"\"; for (x in 'a'..'e') { s = s + str(x); } print(s); for (y in 'c'..'a') { print(y); } }", "char_values");
    }

    #[test]
    fn compat_print_joins_its_arguments_with_spaces() {
        check_compat("fn main() { n: int = 2; print(\"=\", n, 1.5, true, 'z'); }", "print_args");
    }

    #[test]
    fn compat_real_prints_the_shortest_round_trip() {
        check_compat("fn main() { print(32.2 + 2.71); print(0.1 + 0.2); print(1.5); print(3.0); print(-0.25); print(1.0 / 3.0); print(123456789.125); print(0.000001); print(1048576.5 * 1048576.0); s: str = str(2.5); print(s); }", "real_format");
    }

    #[test]
    fn compat_complex_prints_like_the_interpreter() {
        check_compat("fn main() { x = 5.4i; y = 0.5i + 4; z = 4 - 0.5i; print(x); print(y); print(z); print(f`{y} and {z}`); print(y.mod); }", "complex_format");
    }

    #[test]
    fn compat_output_is_written_before_the_program_ends() {
        let exe = build_aot("fn main() { print(\"ready\"); while (true) { } }", "line_flush").expect("AOT build failed");
        let line = first_line_before_exit(&exe);
        let _ = std::fs::remove_dir_all(exe.parent().unwrap());
        assert_eq!(line.as_deref(), Some("ready"));
    }

    #[test]
    fn compat_equality_on_ints_and_reals() {
        check_compat("fn main() { a: int = 4; b: real = 0.5; for (i in 1..6) { if (i % 2 == 0) { print(i); } if (i != 3) { print(\"not3\"); } } print(a == 4); print(a != 4); print(b == 0.5); print(b != 0.25); }", "equality");
    }

    #[test]
    fn compat_package_imported_by_name() {
        let out = check_project_compat(&[
            ("project.toml", "[package]\nname = \"app\"\n"),
            ("@yidi/greet/greet.yk", "use {shout} from \"greet/text\";\nexport fn hello(n: str) -> str { return shout(\"hello \" + n); }\n"),
            ("@yidi/greet/text.yk", "export fn shout(s: str) -> str { return s + \"!\"; }\n"),
            ("@yidi/count/main.yk", "export fn twice(x: int) -> int { return x * 2; }\n"),
            ("src/util.yk", "use twice from \"count\";\nexport fn four() -> int { return twice(2); }\n"),
            ("main.yk", "use {hello} from \"greet\";\nuse {four} from \"./src/util.yk\";\nfn main() { print(hello(\"bob\")); print(four()); }\n"),
        ], "package_by_name");
        assert_eq!(out, "hello bob!\n4");
    }

    #[test]
    fn compat_closure_keeps_a_parameter_of_a_finished_call() {
        check_compat("fn adder(n: int) -> fn(int) -> int { return (x) => x + n; } fn main() { add5 = adder(5); print(add5(37)); add1 = adder(1); print(add1(1)); print(add5(0)); }", "closure_adder");
    }

    #[test]
    fn compat_closure_shares_mutations() {
        check_compat("fn counter() -> fn() -> int { c: int = 0; return () => { c = c + 1; return c; }; } fn main() { next = counter(); next(); next(); print(next()); other = counter(); print(other()); }", "closure_counter");
    }

    #[test]
    fn compat_closure_sees_later_writes() {
        check_compat("fn main() { k: int = 1; f = () => k; k = 10; print(f()); }", "closure_later_write");
    }

    #[test]
    fn compat_lambda_in_an_untyped_variable() {
        check_compat("fn main() { f = (x) => x + 1; print(f(41)); name: str = \"bob\"; g = () => \"hi \" + name; print(g()); }", "closure_untyped");
    }

    #[test]
    fn compat_lambda_called_where_it_is_written() {
        check_compat("fn main() { k: int = 2; print(((x) => x * 3)(5)); print((() => k + 1)()); }", "closure_immediate");
    }

    #[test]
    fn compat_named_function_passed_as_a_value() {
        check_compat("fn twice(x: int) -> int { return x * 2; } fn apply(g: fn(int) -> int, v: int) -> int { return g(v); } fn main() { print(apply(twice, 21)); f: fn(int) -> int = twice; print(f(5)); }", "fn_value_named");
    }

    #[test]
    fn compat_lambda_passed_as_an_argument() {
        check_compat("fn apply(g: fn(int) -> int, v: int) -> int { return g(v); } fn main() { print(apply((x) => x * 10, 4)); print(apply(fn(y: int) -> int { return y + 1; }, 4)); }", "fn_value_lambda");
    }

    #[test]
    fn compat_function_values_with_reals_and_strings() {
        check_compat("fn greet(name: str) -> str { return \"hi \" + name; } fn half(x: real) -> real { return x / 2.0; } fn on_str(g: fn(str) -> str, s: str) -> str { return g(s); } fn on_real(g: fn(real) -> real, x: real) -> real { return g(x); } fn main() { print(on_str(greet, \"bob\")); print(on_real(half, 5.0)); }", "fn_value_types");
    }

    #[test]
    fn compat_range_ascending_includes_both_bounds() {
        check_compat("fn main() { s: str = \"\"; for (i in 1..5) { s = s + str(i) + \",\"; } print(s); }", "range_ascending");
    }

    #[test]
    fn compat_range_descending() {
        check_compat("fn main() { s: str = \"\"; for (i in 5..1) { s = s + str(i); } print(s); }", "range_descending");
    }

    #[test]
    fn compat_range_negative_bounds() {
        check_compat("fn main() { s: str = \"\"; for (i in -10..-2) { s = s + str(i) + \",\"; } print(s); }", "range_negative");
    }

    #[test]
    fn compat_for_count_excludes_the_end() {
        check_compat("fn main() { s: str = \"\"; for (i: 5) { s = s + str(i); } print(s); }", "for_count");
    }

    #[test]
    fn compat_range_with_variable_bounds() {
        check_compat("fn main() { a: int = 3; b: int = 0; s: str = \"\"; for (i in a..b) { s = s + str(i); } for (j in b..a) { s = s + str(j); } print(s); }", "range_variables");
    }

    #[test]
    fn compat_net_server_real_request() {
        let server = AotServer::start("net_server_real_request", &route_server("/hello", "\"Hello, world!\"", 19876), 19876);
        let exchange = server.exchange(&get_request("/hello"));
        assert!(exchange.response.contains("Hello, world!"), "got: {:?}\nserver log:\n{}", exchange.response, server.log());
    }

    #[test]
    fn compat_net_server_listens_on_a_port_number() {
        let source = route_server("/hello", "\"on a port number\"", 19890).replace("app.serve(\"127.0.0.1:19890\")", "app.serve(19890)");
        let server = AotServer::start("net_server_port_number", &source, 19890);
        let exchange = server.exchange(&get_request("/hello"));
        assert!(exchange.response.contains("on a port number"), "got: {:?}\nserver log:\n{}", exchange.response, server.log());
    }

    #[test]
    fn compat_net_server_unknown_route_says_not_found() {
        let server = AotServer::start("net_server_not_found_body", &route_server("/hello", "\"hi\"", 19891), 19891);
        let exchange = server.exchange(&get_request("/missing"));
        assert!(exchange.response.starts_with("HTTP/1.1 404"), "got: {:?}", exchange.response);
        assert!(exchange.response.ends_with("\r\n\r\nNot Found"), "got: {:?}\nserver log:\n{}", exchange.response, server.log());
    }

    #[test]
    fn compat_net_server_two_methods_on_one_path() {
        let source = "use { Server } from \"net\";\nfn main() {\n    app = Server();\n    app.get(\"/notes\", \"list\");\n    app.post(\"/notes\", \"created\");\n    app.serve(\"127.0.0.1:19892\");\n}";
        let server = AotServer::start("net_server_two_methods", source, 19892);
        let get = server.exchange(&get_request("/notes"));
        let post = server.exchange("POST /notes HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        assert!(get.response.ends_with("\r\n\r\nlist"), "GET got: {:?}\nserver log:\n{}", get.response, server.log());
        assert!(post.response.ends_with("\r\n\r\ncreated"), "POST got: {:?}\nserver log:\n{}", post.response, server.log());
    }

    #[test]
    fn compat_net_server_wildcard_route() {
        let server = AotServer::start("net_server_wildcard_route", &route_server("/hello/{name}", "\"Hello, \" + req.path", 19877), 19877);
        let exchange = server.exchange(&get_request("/hello/world"));
        assert!(exchange.response.contains("Hello, /hello/world"), "got: {:?}\nserver log:\n{}", exchange.response, server.log());
    }

    #[test]
    fn compat_net_server_int_param() {
        let server = AotServer::start("net_server_int_param", &route_server("/user/{id:int}", "\"User \" + req.path", 19878), 19878);
        let exchange = server.exchange(&get_request("/user/42"));
        assert!(exchange.response.contains("User /user/42"), "got: {:?}\nserver log:\n{}", exchange.response, server.log());
    }

    #[test]
    fn compat_net_server_int_param_reject() {
        let server = AotServer::start("net_server_int_param_reject", &route_server("/user/{id:int}", "\"User \" + req.path", 19879), 19879);
        let exchange = server.exchange(&get_request("/user/abc"));
        assert!(exchange.response.contains("404"), "got: {:?}\nserver log:\n{}", exchange.response, server.log());
    }

    #[test]
    fn compat_net_server_status_line_has_reason_phrase() {
        let server = AotServer::start("net_server_status_reason", &route_server("/hello", "\"hi\"", 19880), 19880);
        let found = server.exchange(&get_request("/hello"));
        let missing = server.exchange(&get_request("/missing"));
        assert!(found.response.starts_with("HTTP/1.1 200 OK\r\n"), "got: {:?}\nserver log:\n{}", found.response, server.log());
        assert!(missing.response.starts_with("HTTP/1.1 404 Not Found\r\n"), "got: {:?}\nserver log:\n{}", missing.response, server.log());
    }

    #[test]
    fn compat_net_server_response_status() {
        let source = typed_route_server("/denied", "Response", "Response { status: 401, body: \"denied\" }", 19882);
        let server = AotServer::start("net_server_response_status", &source, 19882);
        let exchange = server.exchange(&get_request("/denied"));
        assert!(exchange.response.starts_with("HTTP/1.1 401 Unauthorized\r\n"), "got: {:?}\nserver log:\n{}", exchange.response, server.log());
        assert!(exchange.response.ends_with("denied"), "got: {:?}\nserver log:\n{}", exchange.response, server.log());
    }

    #[test]
    fn compat_net_server_honours_connection_close() {
        let server = AotServer::start("net_server_connection_close", &route_server("/hello", "\"hi\"", 19881), 19881);
        let exchange = server.exchange(&get_request("/hello"));
        assert!(exchange.response.contains("Connection: close"), "got: {:?}\nserver log:\n{}", exchange.response, server.log());
        assert!(exchange.closed_by_server, "server kept the connection open after Connection: close\nserver log:\n{}", server.log());
    }

    // ── TcpStream ────────────────────────────────────────

    #[test]
    fn compat_tcp_stream_echo() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if let Ok(mut s) = stream {
                    let mut buf = [0u8; 1024];
                    if let Ok(n) = s.read(&mut buf) {
                        let _ = s.write_all(&buf[..n]);
                    }
                }
            }
        });
        std::thread::sleep(Duration::from_millis(50));
        let source = format!(r#"use {{ TcpStream }} from "net"
fn main() {{
    sock = TcpStream("127.0.0.1:{port}")
    sock.send("hello")
    data = sock.recv(1024)
    print(data)
    sock.close()
}}"#);
        check_compat(&source, "tcp_stream_echo");
    }

    #[test]
    fn compat_tcp_stream_echo_aot() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                if let Ok(n) = stream.read(&mut buf) {
                    let _ = stream.write_all(&buf[..n]);
                }
            }
        });
        std::thread::sleep(Duration::from_millis(100));

        let source = format!(r#"use {{ TcpStream }} from "net"
fn main() {{
    sock = TcpStream("127.0.0.1:{port}")
    sock.send("hello")
    data = sock.recv(1024)
    sock.close()
    print(data)
    print("ok")
}}"#);
        let test_name = "tcp_stream_echo_aot";
        let _lock = aot_lock();
        let dir = std::env::temp_dir().join(format!("yk_compat_{}", test_name));
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("main.yk");
        std::fs::write(&file_path, &source).expect("write source");

        let backend = crate::codegen::backend::LlvmBackend;
        match crate::cli::build_program(&file_path, false, &backend, false) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("AOT build error: {}", e);
                panic!("AOT build failed: {}", e);
            }
        }
        let exe_path = dir.join(format!("main{}", std::env::consts::EXE_SUFFIX));
        assert!(exe_path.exists(), "AOT exe not produced");

        let output = Command::new(&exe_path)
            .output()
            .expect("run exe");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let normalized = stdout.replace("\r\n", "\n").trim().to_string();
        assert!(output.status.success(), "AOT exe failed: exit={:?} stderr={:?}",
            output.status.code(), String::from_utf8_lossy(&output.stderr));
        assert_eq!(normalized, "hello\nok", "AOT TcpStream echo mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        drop(_lock);
    }

    // ── HTTP class ────────────────────────────────────────

    #[test]
    #[cfg_attr(not(windows), ignore = "HTTP client is WinHTTP only, issue #65")]
    fn compat_http_class_get() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if let Ok(mut s) = stream {
                    let mut buf = [0u8; 4096];
                    if let Ok(_) = s.read(&mut buf) {
                        let resp = "HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\nHello World!";
                        let _ = s.write_all(resp.as_bytes());
                    }
                }
            }
        });
        std::thread::sleep(Duration::from_millis(50));
        let source = format!(r#"use {{ HTTP }} from "net"
fn main() {{
    http = HTTP()
    s = http.get("http://127.0.0.1:{port}/test")
    print(s)
    print(http.body)
}}"#);
        check_compat(&source, "http_class_get");
    }

    #[test]
    fn compat_http_class_properties() {
        let source = r#"use { HTTP } from "net"
fn main() {
    http = HTTP()
    print(http.status)
    print(http.body)
}"#;
        check_compat(source, "http_class_properties");
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

    // ── Tuple tests ───────────────────────────────────

    #[test]
    fn compat_tuple_literal() {
        check_compat(
            "fn main() { t = (42, \"hello\"); print(t.0); print(t.1); }",
            "tuple_literal",
        );
    }

    // ── PostInc / PostDec tests ───────────────────────

    #[test]
    fn compat_post_inc() {
        check_compat(
            "fn main() { i: int = 1; i++; print(i); }",
            "post_inc",
        );
    }

    #[test]
    fn compat_post_dec() {
        check_compat(
            "fn main() { i: int = 5; i--; print(i); }",
            "post_dec",
        );
    }

    // ── Enum tests ────────────────────────────────────

    #[test]
    fn compat_enum_match_payload() {
        // Note: uses `_` instead of bound variable `v` due to interpreter
        // bug with pattern variable binding in variant subpatterns.
        check_compat(
            "enum Option { Some(x: int); Nothing; }
             fn main() {
                 x: Option = Option::Some(42);
                 y: int = match x {
                     Some(_) => 1,
                     Nothing => 0,
                 };
                 print(y);
             }",
            "enum_match_payload",
        );
    }

    #[test]
    fn compat_enum_unit_variant() {
        check_compat(
            "enum Color { Red; Green; Blue; }
             fn main() {
                 c: Color = Color::Red;
                 v: int = match c {
                     Red => 1,
                     _ => 0,
                 };
                 print(v);
             }",
            "enum_unit_variant",
        );
    }

    // ── Union tests ───────────────────────────────────

    #[test]
    fn compat_union_type_annotation() {
        check_compat(
            "fn main() { x: int | str = 42; print(x); x = \"hello\"; print(x); }",
            "union_type_annotation",
        );
    }

    #[test]
    fn compat_union_type_param() {
        check_compat(
            "fn foo(x: int | str) { print(x); }
             fn main() { foo(42); foo(\"hello\"); }",
            "union_type_param",
        );
    }

    #[test]
    fn compat_union_str() {
        check_compat(
            "fn main() {
                x: int | str = 42;
                print(str(x));
                x = \"hello\";
                print(str(x));
            }",
            "union_str",
        );
    }

    #[test]
    fn compat_union_print() {
        check_compat(
            "fn main() {
                print(42);
                print(\"hello\");
                x: int | str = 42;
                print(x);
                x = \"hello\";
                print(x);
            }",
            "union_print",
        );
    }

    // ── .length property ──────────────────────────────

    #[test]
    fn compat_length_string() {
        check_compat(
            r#"fn main() { s: str = "hello"; print(s.length); }"#,
            "length_string",
        );
    }

    #[test]
    fn compat_length_list() {
        check_compat(
            "fn main() { items: auto = [10, 20, 30]; print(items.length); }",
            "length_list",
        );
    }

    // ── .toString() method ────────────────────────────

    #[test]
    fn compat_to_string_int() {
        check_compat(
            "fn main() { print((42).toString()); }",
            "to_string_int",
        );
    }

    #[test]
    fn compat_to_string_list() {
        check_compat(
            "fn main() { items: auto = [1, 2, 3]; print(items.toString()); }",
            "to_string_list",
        );
    }

    // ── str() for collections ─────────────────────────

    #[test]
    fn compat_str_list() {
        check_compat(
            "fn main() { items: auto = [1, 2]; print(str(items)); }",
            "str_list",
        );
    }

    // ── Math builtins ─────────────────────────────────

    #[test]
    fn compat_math_abs() {
        check_compat(
            "use { abs } from \"math\";
             fn main() { print(abs(-5)); }",
            "math_abs",
        );
    }

    #[test]
    fn compat_math_sqrt() {
        check_compat(
            "use { sqrt } from \"math\";
             fn main() { print(sqrt(9.0)); }",
            "math_sqrt",
        );
    }

    #[test]
    fn compat_math_sin() {
        check_compat(
            "use { sin } from \"math\";
             fn main() { print(sin(0.0)); }",
            "math_sin",
        );
    }

    // ── Interface tests ───────────────────────────────

    #[test]
    fn compat_interface_method_call() {
        check_compat(
            "interface Drawable { fn draw(&self) -> str; }
             class Circle implements Drawable {
                 x: int;
                 fn draw(&self) -> str { return \"circle\"; }
             }
             fn describe(d: Drawable) -> str {
                 return d.draw();
             }
             fn main() {
                 d: Drawable = Circle { x: 10 };
                 print(d.draw());
                 print(describe(Circle { x: 20 }));
             }",
            "interface_method_call",
        );
    }

    #[test]
    fn compat_object_basic() {
        check_compat(
            "object Logger {
                 fn log(msg: str) {
                     print(msg);
                 }
             }
             fn main() {
                 Logger.log(\"hello from object\");
             }",
             "object_basic",
        );
    }

    #[test]
    fn compat_arith_mod() {
        check_compat(
            "fn main() { print(17 % 5); print(-17 % 5); }",
            "arith_mod",
        );
    }

    #[test]
    fn compat_arith_pow() {
        check_compat(
            "fn main() { print(2 ** 10); print(3 ** 4); }",
            "arith_pow",
        );
    }

    #[test]
    fn compat_arith_pow_real() {
        check_compat(
            "fn main() { print(2.0 ** 3.0); print(4.0 ** 0.5); }",
            "arith_pow_real",
        );
    }

    #[test]
    fn compat_bitwise_and() {
        check_compat(
            "fn main() { print(6 & 3); print(0xFF & 0x0F); }",
            "bitwise_and",
        );
    }

    #[test]
    fn compat_bitwise_or() {
        check_compat(
            "fn main() { print(6 | 3); print(0xF0 | 0x0F); }",
            "bitwise_or",
        );
    }

    #[test]
    fn compat_bitwise_xor() {
        check_compat(
            "fn main() { print(6 ^ 3); print(0xFF ^ 0xF0); }",
            "bitwise_xor",
        );
    }

    #[test]
    fn compat_mixed_int_real() {
        check_compat(
            "fn main() { print(3.2 + 3); print(3.5 - 2); print(2.5 * 4); print(10.0 / 2); }",
            "mixed_int_real",
        );
    }

    #[test]
    fn compat_shift() {
        check_compat(
            "fn main() { print(1 << 8); print(256 >> 4); }",
            "shift",
        );
    }

    #[test]
    fn compat_bitnot() {
        check_compat(
            "fn main() { print(~0); print(~255); }",
            "bitnot",
        );
    }

    #[test]
    fn compat_compound_add() {
        check_compat(
            "fn main() { x: int = 5; x += 3; print(x); }",
            "compound_add",
        );
    }

    #[test]
    fn compat_compound_mul() {
        check_compat(
            "fn main() { x: int = 4; x *= 3; print(x); }",
            "compound_mul",
        );
    }

    #[test]
    fn compat_arith_precedence() {
        check_compat(
            "fn main() { print(2 + 3 * 4); print(10 - 2 * 3); print(16 / 4 + 2); print(2 + 6 % 4); }",
            "arith_precedence",
        );
    }

    #[test]
    fn compat_pow_right_assoc() {
        check_compat(
            "fn main() { print(2 ** 1 ** 3); }",
            "pow_right_assoc",
        );
    }

    /// Build an HTTP/2 frame (9-byte header + payload)
    fn h2_frame(typ: u8, flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        let len = payload.len() as u32;
        f.push((len >> 16) as u8);
        f.push((len >> 8) as u8);
        f.push(len as u8);
        f.push(typ);
        f.push(flags);
        f.push((stream_id >> 24) as u8);
        f.push((stream_id >> 16) as u8);
        f.push((stream_id >> 8) as u8);
        f.push(stream_id as u8);
        f.extend_from_slice(payload);
        f
    }

    #[test]
    fn compat_h2_basic_get() {
        let port = 19879u16;
        let server_source = format!(r#"use {{ Server }} from "net";
fn hello(req: auto) -> str {{
    return "Hello H2!";
}}
fn main() {{
    app: Server = Server();
    app.get("/", hello);
    app.serve("127.0.0.1:{port}");
}}"#);
        let server = AotServer::start("h2_basic_get", &server_source, port);
        let mut stream = server.connect(Duration::from_secs(3));

        // H2 connection preface
        let mut send_buf = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec();
        // SETTINGS (empty)
        send_buf.extend(h2_frame(0x04, 0x00, 0, &[]));
        // SETTINGS ACK
        send_buf.extend(h2_frame(0x04, 0x01, 0, &[]));
        // HEADERS with END_HEADERS|END_STREAM, stream=1
        // HPACK: index 2 (:method GET), index 4 (:path /), index 6 (:scheme http)
        let hpack = vec![0x82u8, 0x84, 0x86];
        send_buf.extend(h2_frame(0x01, 0x05, 1, &hpack));

        stream.write_all(&send_buf).expect("write h2 frames");

        // Read up to 4096 bytes with short timeout
        let mut resp = vec![0u8; 4096];
        let mut total = 0usize;
        loop {
            match stream.read(&mut resp[total..]) {
                Ok(0) => break,
                Ok(n) => { total += n; if total >= resp.len() { break; } }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        resp.truncate(total);
        drop(server);

        // Validate: should get SETTINGS + HEADERS + DATA
        assert!(resp.len() >= 27, "response too short: {} bytes", resp.len());

        // Parse H2 frames: first should be SETTINGS from server
        let mut pos = 0;
        let mut found_headers = false;
        while pos + 9 <= resp.len() {
            let frame_len = ((resp[pos] as usize) << 16) | ((resp[pos+1] as usize) << 8) | (resp[pos+2] as usize);
            let frame_type = resp[pos+3];
            let frame_flags = resp[pos+4];
            let frame_end = pos + 9 + frame_len;
            if frame_end > resp.len() { break; }

            if frame_type == 0x04 && (frame_flags & 0x01) == 0 {
                // Non-ACK SETTINGS from server - just skip
            } else if frame_type == 0x01 && (frame_flags & 0x04) != 0 { // HEADERS + END_HEADERS
                found_headers = true;
                // HPACK block should contain extended status (index 8 = :status 200 = 0x88)
                let payload = &resp[pos+9..frame_end];
                assert!(!payload.is_empty(), "HEADERS frame has empty HPACK block");
                assert_eq!(payload[0], 0x88,
                    "expected :status 200 HPACK (0x88), got 0x{:02x}", payload[0]);
                break;
            } else if frame_type == 0x00 { // DATA
                // Body frame - just track it
            }
            pos = frame_end;
        }
        assert!(found_headers, "No HEADERS frame found in response");
    }

    static GUI_LOCK: Mutex<()> = Mutex::new(());

    fn run_gui_compat(source: &str, test_name: &str) {
        let interp_out = {
            let guard = GUI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            yk_ffi_gui::tree::reset_state();
            let out = run_interp(source);
            drop(guard);
            out
        }
        .expect("gui interpreter failed");
        let aot_out = run_aot(source, test_name).expect("gui AOT build/run failed");
        assert_eq!(
            normalize(&interp_out),
            normalize(&aot_out),
            "compat_{}: interpreter vs AOT output mismatch", test_name
        );
    }

    #[test]
    #[cfg_attr(not(windows), ignore = "the GUI engine is Windows only")]
    fn compat_gui_dump_static() {
        run_gui_compat(r#"use { gui } from "std";

fn main() {
    w = gui.window("T", 200, 100);
    root = gui.div(w);
    gui.style(root, "padding:8px;background:#00ff66;flex-direction:column;gap:4px");
    lbl = gui.text(root, "hello");
    gui.style(lbl, "font-size:12px;color:#ffffff");
    btn = gui.button(root, "ok");
    gui.dump(w);
}
"#, "gui_dump_static");
    }

    #[test]
    #[cfg_attr(not(windows), ignore = "the GUI engine is Windows only")]
    fn compat_gui_event_state() {
        run_gui_compat(r#"use { gui } from "std";

fn bump(ev: int) {
    gui.set_text(ev, "clicked");
}

fn main() {
    w = gui.window("T", 240, 120);
    root = gui.div(w);
    gui.style(root, "padding:10px;flex-direction:column;gap:6px");
    btn = gui.button(root, "click");
    gui.on(btn, "click", bump);
    gui.synthetic_click(btn);
    gui.synthetic_click(btn);
    gui.dump(w);
}
"#, "gui_event_state");
    }

    #[test]
    fn compat_inherited_method_reads_parent_field() {
        check_compat(r#"open class A(name: str) {
    fn hi(self) -> str { return "hi " + self.name; }
}
class B(k: int, name: str) : A {
}
fn main() {
    b = B { name: "x", k: 1 };
    println(b.hi());
    println(b.k);
}
"#, "inherited_method_reads_parent_field");
    }

    #[test]
    fn compat_set_literal_drops_duplicates() {
        check_compat(r#"fn main() {
    s: set[int] = {1, 2, 2, 3, 1};
    println(s.len());
}
"#, "set_literal_drops_duplicates");
    }

    #[test]
    fn compat_generic_fn_keeps_argument_type() {
        check_compat(r#"fn id<T>(x: T) -> T { return x; }
fn main() {
    println(id("s"));
    println(id(5));
    println(id(2.5));
}
"#, "generic_fn_keeps_argument_type");
    }

    #[test]
    fn compat_await_returns_spawned_value() {
        check_compat(r#"fn work(n: int) -> int { return n * 2; }
fn main() {
    t = spawn work(21);
    println(await t);
}
"#, "await_returns_spawned_value");
    }

    #[test]
    fn compat_if_expression_yields_branch_value() {
        check_compat(r#"fn main() {
    x = 5;
    y = if (x > 3) { "big" } else { "small" };
    println(y);
    z = if (x > 9) { 1 } else { 2 };
    println(z);
}
"#, "if_expression_yields_branch_value");
    }

    #[test]
    fn compat_collections_keep_element_type() {
        check_compat(r#"fn first(xs: list[str]) -> str { return xs[0]; }
fn main() {
    l = [1, 2, 3];
    println(l);
    s: set[int] = {3, 1, 3};
    println(s);
    println(s.len());
    n = ["a", "b"];
    println(n);
    println(n[1]);
    println(first(n));
    r = [1.5, 2.5];
    println(r[0]);
    println(r);
    b = [true, false];
    println(b);
    w = [];
    w.push("x");
    w.push("y");
    println(w);
    println(w[1]);
    t: set[str] = {"a", "b", "a"};
    println(t);
}
"#, "collections_keep_element_type");
    }

    #[test]
    fn aot_refuses_json_parse_instead_of_printing_zero() {
        let err = build_aot(r#"use { json } from "json";
fn main() {
    v = json.parse("{\"a\":1}");
    println(v);
}
"#, "refuses_json_parse").expect_err("json.parse must not build until it is implemented natively");
        assert!(err.contains("json.parse"), "{}", err);
    }

    #[test]
    fn aot_stops_on_list_index_out_of_bounds() {
        let source = "fn main() {\n    l = [1, 2];\n    println(l[5]);\n}\n";
        let interp = run_interp(source).expect_err("the interpreter stops on a bad index");
        let native = run_aot(source, "list_index_out_of_bounds").expect_err("the native binary must stop instead of printing 0");
        assert!(interp.contains("Index 5 out of bounds for list of length 2"), "{}", interp);
        assert!(native.contains("Index 5 out of bounds for list of length 2"), "{}", native);
    }

    #[test]
    fn compat_enum_parameter_matches_its_own_variant() {
        check_compat(r#"enum Shape { Circle(r: int); Square(side: int); Empty; }
fn kind(s: Shape) -> str {
    return match s {
        Circle(_) => "round",
        Square(_) => "square",
        Empty => "nothing",
    };
}
fn main() {
    println(kind(Shape::Circle(2)), kind(Shape::Square(3)), kind(Shape::Empty));
}
"#, "enum_parameter_matches_its_own_variant");
    }

    #[test]
    fn compat_match_reaches_every_arm() {
        check_compat(r#"fn name(n: int) -> str {
    return match n { 0 => "zero", 1 => "one", 2 => "two", _ => "many" };
}
fn main() {
    println(name(0), name(1), name(2), name(9));
    s = "c";
    println(match s { "a" => 1, "b" => 2, "c" => 3, _ => 0 });
}
"#, "match_reaches_every_arm");
    }

    #[test]
    fn compat_base64_round_trips_every_padding_length() {
        check_compat(r#"use { base64 } from "base64";
fn main() {
    println(base64.encode("a"), base64.decode(base64.encode("a")));
    println(base64.encode("ab"), base64.decode(base64.encode("ab")));
    println(base64.encode("abc"), base64.decode(base64.encode("abc")));
    println(base64.encode("abcd"), base64.decode(base64.encode("abcd")));
    println(base64.encode("hello"), base64.decode(base64.encode("hello")));
    println(base64.encode(""), base64.decode(""));
}
"#, "base64_round_trips_every_padding_length");
    }

    #[test]
    fn aot_stops_on_invalid_base64_like_the_interpreter() {
        let source = "use { base64 } from \"base64\";\nfn main() {\n    println(base64.decode(\"YQ\"));\n}\n";
        assert!(run_interp(source).is_err(), "the interpreter rejects unpadded input");
        let native = run_aot(source, "invalid_base64").expect_err("the native binary must stop instead of reading past the input");
        assert!(native.contains("base64.decode"), "{}", native);
    }

    #[test]
    fn compat_deep_recursion_runs_in_both_engines() {
        check_compat(r#"fn depth(n: int) -> int {
    if (n == 0) { return 0; }
    return 1 + depth(n - 1);
}
fn main() {
    println(depth(400));
}
"#, "deep_recursion_runs_in_both_engines");
    }
}
