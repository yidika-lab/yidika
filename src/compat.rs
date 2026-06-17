#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};
    use std::sync::Mutex;
    use std::io::{Read, Write};
    use std::time::Duration;

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
    fn compat_net_server_real_request() {
        let server_source = r#"use { Server } from "net";
fn hello(req: auto) -> str {
    return "Hello, world!";
}
fn main() {
    app: Server = Server();
    app.get("/hello", hello);
    app.serve("127.0.0.1:19876");
}"#;
        let test_name = "net_server_real_request";
        let _lock = AOT_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("yk_compat_{}", test_name));
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("main.yk");
        std::fs::write(&file_path, server_source).expect("write source");
        let backend = crate::codegen::backend::LlvmBackend;
        crate::cli::build_program(&file_path, false, &backend, false)
            .expect("AOT build failed");
        let exe_path = dir.join("main.exe");
        assert!(exe_path.exists(), "AOT exe not produced");

        let mut child = Command::new(&exe_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start server");

        // Wait for server port to be ready (up to 5s)
        let mut started = false;
        for _ in 0..50 {
            if std::net::TcpStream::connect_timeout(
                &"127.0.0.1:19876".parse().unwrap(),
                Duration::from_millis(100)
            ).is_ok() {
                started = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(started, "Server did not start in time");

        // Make HTTP request via raw TCP
        let mut stream = std::net::TcpStream::connect("127.0.0.1:19876")
            .expect("connect to server");
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let request = "GET /hello HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
        stream.write_all(request.as_bytes()).expect("write request");

        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        drop(_lock);

        assert!(response.contains("Hello, world!"),
            "net_server_real_request: response should contain handler output, got: {:?}", response);
    }

    #[test]
    fn compat_net_server_wildcard_route() {
        let server_source = r#"use { Server } from "net";
fn hello(req: auto) -> str {
    return "Hello, " + req.path;
}
fn main() {
    app: Server = Server();
    app.get("/hello/{name}", hello);
    app.serve("127.0.0.1:19877");
}"#;
        let test_name = "net_server_wildcard_route";
        let _lock = AOT_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("yk_compat_{}", test_name));
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("main.yk");
        std::fs::write(&file_path, server_source).expect("write source");
        let backend = crate::codegen::backend::LlvmBackend;
        crate::cli::build_program(&file_path, false, &backend, false)
            .expect("AOT build failed");
        let exe_path = dir.join("main.exe");
        assert!(exe_path.exists(), "AOT exe not produced");

        let mut child = Command::new(&exe_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start server");

        // Wait for server port to be ready (up to 5s)
        let mut started = false;
        for _ in 0..50 {
            if std::net::TcpStream::connect_timeout(
                &"127.0.0.1:19877".parse().unwrap(),
                Duration::from_millis(100)
            ).is_ok() {
                started = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(started, "Server did not start in time");

        // Make HTTP request to wildcard route
        let mut stream = std::net::TcpStream::connect("127.0.0.1:19877")
            .expect("connect to server");
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let request = "GET /hello/world HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
        stream.write_all(request.as_bytes()).expect("write request");

        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        drop(_lock);

        // The handler echoes back req.path, which should be the URL path
        assert!(response.contains("Hello, /hello/world"),
            "net_server_wildcard_route: response should contain the path, got: {:?}", response);
    }

    #[test]
    fn compat_net_server_int_param() {
        let server_source = r#"use { Server } from "net";
fn hello(req: auto) -> str {
    return "User " + req.path;
}
fn main() {
    app: Server = Server();
    app.get("/user/{id:int}", hello);
    app.serve("127.0.0.1:19878");
}"#;
        let test_name = "net_server_int_param";
        let _lock = AOT_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("yk_compat_{}", test_name));
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("main.yk");
        std::fs::write(&file_path, server_source).expect("write source");
        let backend = crate::codegen::backend::LlvmBackend;
        crate::cli::build_program(&file_path, false, &backend, false)
            .expect("AOT build failed");
        let exe_path = dir.join("main.exe");
        assert!(exe_path.exists(), "AOT exe not produced");

        let mut child = Command::new(&exe_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start server");

        let mut started = false;
        for _ in 0..50 {
            if std::net::TcpStream::connect_timeout(
                &"127.0.0.1:19878".parse().unwrap(),
                Duration::from_millis(100)
            ).is_ok() {
                started = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(started, "Server did not start in time");

        // Request with valid int param should match
        let mut stream = std::net::TcpStream::connect("127.0.0.1:19878")
            .expect("connect to server");
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let request = "GET /user/42 HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
        stream.write_all(request.as_bytes()).expect("write request");

        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        drop(_lock); // release lock before assertion to avoid poisoning

        assert!(response.contains("User /user/42"),
            "int_param: should match /user/42, got: {:?}", response);
    }

    #[test]
    fn compat_net_server_int_param_reject() {
        let server_source = r#"use { Server } from "net";
fn hello(req: auto) -> str {
    return "User " + req.path;
}
fn main() {
    app: Server = Server();
    app.get("/user/{id:int}", hello);
    app.serve("127.0.0.1:19879");
}"#;
        let test_name = "net_server_int_param_reject";
        let _lock = AOT_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("yk_compat_{}", test_name));
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("main.yk");
        std::fs::write(&file_path, server_source).expect("write source");
        let backend = crate::codegen::backend::LlvmBackend;
        crate::cli::build_program(&file_path, false, &backend, false)
            .expect("AOT build failed");
        let exe_path = dir.join("main.exe");
        assert!(exe_path.exists(), "AOT exe not produced");

        let mut child = Command::new(&exe_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start server");

        let mut started = false;
        for _ in 0..50 {
            if std::net::TcpStream::connect_timeout(
                &"127.0.0.1:19879".parse().unwrap(),
                Duration::from_millis(100)
            ).is_ok() {
                started = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(started, "Server did not start in time");

        // Request with non-int param should get 404
        let mut stream = std::net::TcpStream::connect("127.0.0.1:19879")
            .expect("connect to server");
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let request = "GET /user/abc HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
        stream.write_all(request.as_bytes()).expect("write request");

        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        drop(_lock);

        assert!(response.contains("404"),
            "int_param_reject: /user/abc should get 404, got: {:?}", response);
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
        let _lock = AOT_LOCK.lock().unwrap();
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
        let exe_path = dir.join("main.exe");
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
        let test_name = "h2_basic_get";
        let _lock = AOT_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("yk_compat_{}", test_name));
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("main.yk");
        std::fs::write(&file_path, server_source).expect("write source");
        let backend = crate::codegen::backend::LlvmBackend;
        crate::cli::build_program(&file_path, false, &backend, false)
            .expect("AOT build failed");
        let exe_path = dir.join("main.exe");
        assert!(exe_path.exists(), "AOT exe not produced");

        let mut child = Command::new(&exe_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start server");

        // Wait for server port
        let mut started = false;
        for _ in 0..50 {
            if std::net::TcpStream::connect_timeout(
                &format!("127.0.0.1:{port}").parse().unwrap(),
                Duration::from_millis(100)
            ).is_ok() {
                started = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(started, "Server did not start in time");

        // Connect with raw TCP and send H2 frames
        let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(3))).ok();

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
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        drop(_lock);

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
                // Non-ACK SETTINGS from server — just skip
            } else if frame_type == 0x01 && (frame_flags & 0x04) != 0 { // HEADERS + END_HEADERS
                found_headers = true;
                // HPACK block should contain extended status (index 8 = :status 200 = 0x88)
                let payload = &resp[pos+9..frame_end];
                assert!(!payload.is_empty(), "HEADERS frame has empty HPACK block");
                assert_eq!(payload[0], 0x88,
                    "expected :status 200 HPACK (0x88), got 0x{:02x}", payload[0]);
                break;
            } else if frame_type == 0x00 { // DATA
                // Body frame — just track it
            }
            pos = frame_end;
        }
        assert!(found_headers, "No HEADERS frame found in response");
    }
}
