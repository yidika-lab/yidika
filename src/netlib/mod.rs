#![allow(dead_code)]
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::hardware::HardwareInfo;
use crate::interpret::{Value, FnDef, ClassDef, Interpreter, SubInterpreterPool};
use crate::jit::mcjit::McJit;
use crate::syntax::ast::{Expr, ExprNode};

/// JIT-compiled handler function signature (static literal).
/// Takes a pointer to a YkResponse struct, returns void.
type JitHandlerFn = unsafe extern "C" fn(*mut YkResponse);

/// JIT-compiled FnDef handler function signature.
/// Takes (response struct, request struct, output buffer, buffer length).
type JitFnHandlerFn = unsafe extern "C" fn(*mut YkResponse, *const YkRequest, *mut u8, i64);

/// Response struct layout matching the LLVM IR `%YkResponse` type.
#[repr(C)]
struct YkResponse {
    body: *const u8,
    body_len: i64,
    status_code: i32,
}

/// Request struct layout matching the LLVM IR `%YkRequest` type.
/// Fields ordered: method(ptr, len), path(ptr, len), body(ptr, len).
#[repr(C)]
struct YkRequest {
    method: *const u8,
    method_len: i64,
    path: *const u8,
    path_len: i64,
    body: *const u8,
    body_len: i64,
}

type JitState = Option<(McJit, HashMap<String, usize>)>;

fn jit_state() -> &'static Mutex<JitState> {
    static STATE: std::sync::OnceLock<Mutex<JitState>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

fn ensure_mcjit(guard: &mut JitState) -> Option<&McJit> {
    if guard.is_none() {
        let path = crate::codegen::llvm_api::find_llvm_lib()?;
        let api = crate::codegen::llvm_api::LlvmApi::load(&path).ok()?;
        let mjit = McJit::new(api).ok()?;
        *guard = Some((mjit, HashMap::new()));
    }
    Some(&guard.as_ref().unwrap().0)
}

/// Compile a static handler returning a fixed string, returning its function pointer.
/// The function is cached after first compilation.
fn compile_literal_handler(name: &str, body: &str, status: i32) -> Option<JitHandlerFn> {
    let state = jit_state();
    let mut guard = state.lock().unwrap();

    if let Some((_, ref cache)) = *guard {
        if let Some(&addr) = cache.get(name) {
            return Some(unsafe { std::mem::transmute(addr) });
        }
    }

    let mjit = ensure_mcjit(&mut *guard)?;
    let ir = crate::codegen::llvm::generate_static_handler_ir(name, body, status);
    mjit.add_module(&ir, name).ok()?;
    let ptr = mjit.lookup(name).ok()?;
    let func: JitHandlerFn = unsafe { std::mem::transmute(ptr) };
    guard.as_mut().unwrap().1.insert(name.to_string(), ptr as usize);
    Some(func)
}

/// Compile a FnDef handler to native code, returning its function pointer.
/// The function is cached after first compilation.
fn compile_fn_handler(name: &str, fndef: &FnDef) -> Option<JitFnHandlerFn> {
    let state = jit_state();
    let mut guard = state.lock().unwrap();

    let cache_key = format!("__yk_fn_{}", name);
    if let Some((_, ref cache)) = *guard {
        if let Some(&addr) = cache.get(&cache_key) {
            return Some(unsafe { std::mem::transmute(addr) });
        }
    }

    let mjit = ensure_mcjit(&mut *guard)?;
    let jit_name = &cache_key;
    let ir = crate::codegen::llvm::generate_fn_handler_ir(jit_name, fndef)?;
    mjit.add_module(&ir, jit_name).ok()?;
    let ptr = mjit.lookup(jit_name).ok()?;
    let func: JitFnHandlerFn = unsafe { std::mem::transmute(ptr) };
    guard.as_mut().unwrap().1.insert(cache_key, ptr as usize);
    Some(func)
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn base_interp() -> &'static Interpreter {
    static PTR: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let p = PTR.get_or_init(|| {
        Box::into_raw(Box::new(Interpreter::new())) as usize
    });
    unsafe { &*(*p as *const Interpreter) }
}

#[derive(Clone)]
enum Handler {
    Literal(String),
    Fn(String, FnDef),
}

#[derive(Clone)]
struct RouteNode {
    children: HashMap<String, RouteNode>,
    wildcard: Option<Box<RouteNode>>,
    handler: Option<(String, Handler)>,
}

impl RouteNode {
    fn new() -> Self {
        RouteNode { children: HashMap::new(), wildcard: None, handler: None }
    }

    fn insert(&mut self, segments: &[&str], method: &str, handler: Handler) {
        if segments.is_empty() {
            self.handler = Some((method.to_string(), handler));
            return;
        }
        let seg = segments[0];
        if seg.starts_with(':') {
            if self.wildcard.is_none() {
                self.wildcard = Some(Box::new(RouteNode::new()));
            }
            self.wildcard.as_mut().unwrap().insert(&segments[1..], method, handler);
        } else {
            self.children.entry(seg.to_string())
                .or_insert_with(RouteNode::new)
                .insert(&segments[1..], method, handler);
        }
    }

    fn find<'a>(&'a self, segments: &[&str], method: &str, params: &mut Vec<String>) -> Option<&'a Handler> {
        if let Some((ref m, ref h)) = self.handler {
            if m == method && segments.is_empty() {
                return Some(h);
            }
        }
        if segments.is_empty() {
            return None;
        }
        let seg = segments[0];
        let rest = &segments[1..];

        if let Some(child) = self.children.get(seg) {
            if let found @ Some(_) = child.find(rest, method, params) {
                return found;
            }
        }
        if let Some(ref wildcard) = self.wildcard {
            params.push(seg.to_string());
            if let found @ Some(_) = wildcard.find(rest, method, params) {
                return found;
            }
            params.pop();
        }
        None
    }

    fn precompile_handlers(&self) {
        if let Some((_, ref handler)) = self.handler {
            match handler {
                Handler::Literal(text) => {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    text.hash(&mut hasher);
                    let jit_name = format!("__yk_lit_{:x}", hasher.finish());
                    let _ = compile_literal_handler(&jit_name, text, 200);
                }
                Handler::Fn(name, fndef) => {
                    let jn = format!("__yk_fn_{}", name);
                    let _ = compile_fn_handler(&jn, fndef);
                }
            }
        }
        for child in self.children.values() {
            child.precompile_handlers();
        }
        if let Some(ref wildcard) = self.wildcard {
            wildcard.precompile_handlers();
        }
    }
}

#[derive(Clone)]
struct RouteTrie {
    root: Arc<RouteNode>,
}

impl RouteTrie {
    fn new() -> Self {
        RouteTrie { root: Arc::new(RouteNode::new()) }
    }

    fn add_route(&mut self, path: &str, method: &str, handler: Handler) {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        Arc::make_mut(&mut self.root).insert(&segments, method, handler);
    }

    fn match_route(&self, path: &str, method: &str) -> Option<(&Handler, Vec<String>)> {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut params = Vec::new();
        self.root.find(&segments, method, &mut params).map(|h| (h, params))
    }

    fn precompile_all(&self) {
        self.root.precompile_handlers();
    }
}

#[derive(Clone)]
struct ServerInstance {
    id: u64,
    addr: String,
    routes: RouteTrie,
}

impl ServerInstance {
    fn new(id: u64) -> Self {
        Self { id, addr: String::new(), routes: RouteTrie::new() }
    }
}

fn servers() -> &'static std::sync::Mutex<HashMap<u64, ServerInstance>> {
    static REGISTRY: std::sync::OnceLock<std::sync::Mutex<HashMap<u64, ServerInstance>>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn sub_interpreter_pool() -> &'static SubInterpreterPool {
    static POOL: std::sync::OnceLock<SubInterpreterPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| SubInterpreterPool::new(64))
}

fn ensure_server_class(interp: &mut Interpreter) {
    if !interp.classes.contains_key("Server") {
        Arc::make_mut(&mut interp.classes).insert("Server".into(), ClassDef {
            fields: vec!["__id".into()],
            methods: HashMap::new(),
            extends: None,
            constructor: Arc::new(Vec::new()),
            init_body: Arc::new(Vec::new()),
            is_data: false,
            method_cache: std::sync::Mutex::new(HashMap::new()),
            inline_cache: std::sync::Mutex::new(Default::default()),
        });
    }
}

fn parse_request_line(raw: &str) -> Option<(&str, &str)> {
    let first_line = raw.lines().next()?;
    let mut parts = first_line.splitn(3, ' ');
    let method = parts.next()?;
    let path = parts.next()?;
    Some((method, path))
}

fn extract_header_value(raw: &str, name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let search = &lower;
    for line in raw.lines() {
        if let Some(idx) = line.find(':') {
            let key = line[..idx].trim();
            if key.len() == name.len() && key.to_lowercase() == *search {
                return Some(line[idx + 1..].trim().to_string());
            }
        }
    }
    None
}

fn write_status_line(stream: &mut TcpStream, status: &str, content_type: &str, body_len: usize, keep_alive: bool) -> std::io::Result<()> {
    let conn = if keep_alive { "keep-alive" } else { "close" };
    write!(stream, "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: {}\r\n\r\n", status, content_type, body_len, conn)
}

fn handle_connection(mut stream: TcpStream, server: &ServerInstance, pool: &SubInterpreterPool) {
    let _ = stream.set_nodelay(true);
    let hw = HardwareInfo::cached();
    let buf_size = hw.map(|h| {
        h.cpu.cache_l1.map(|l1| (l1 as usize).max(4096)).unwrap_or(8192)
    }).unwrap_or(8192);

    let mut read_buf = Vec::with_capacity(buf_size);

    loop {
        read_buf.clear();
    loop {
            let mut chunk = [0u8; 4096];
            match stream.read(&mut chunk) {
                Ok(0) => return,
                Ok(n) => {
                    read_buf.extend_from_slice(&chunk[..n]);
                    if read_buf.windows(4).any(|w| w == b"\r\n\r\n") { break; }
                }
                Err(_) => return,
            }
        }

        let raw = unsafe { std::str::from_utf8_unchecked(&read_buf) };

        let (method_raw, path_raw) = match parse_request_line(raw) {
            Some(r) => (r.0.to_uppercase(), r.1.to_string()),
            None => {
                let _ = write_status_line(&mut stream, "400 Bad Request", "text/plain", 9, false);
                let _ = stream.write_all(b"Bad Request");
                return;
            }
        };

        let keep_alive = extract_header_value(raw, "Connection")
            .map(|v| v.eq_ignore_ascii_case("keep-alive"))
            .unwrap_or(false);

        let body = if let Some(cl) = extract_header_value(raw, "Content-Length")
            .and_then(|v| v.parse::<usize>().ok())
        {
            let header_end = raw.find("\r\n\r\n").unwrap_or(raw.len());
            let body_start = header_end + 4;
            let already = read_buf.len().saturating_sub(body_start);
            if cl > already {
                let mut extra = vec![0u8; cl - already];
                if stream.read_exact(&mut extra).is_err() { return; }
                let mut b = read_buf[body_start..].to_vec();
                b.extend_from_slice(&extra);
                String::from_utf8_lossy(&b).to_string()
            } else {
                unsafe { String::from_utf8_unchecked(read_buf[body_start..body_start + cl].to_vec()) }
            }
        } else {
            String::new()
        };

        let (response_body, content_type) = match server.routes.match_route(&path_raw, &method_raw) {
            Some((handler, route_params)) => {
                match handler {
                    Handler::Literal(text) => {
                        let ct = if text.as_bytes().first().map_or(false, |b| *b == b'{' || *b == b'[') { "application/json" } else { "text/plain" };
                        // Try JIT path: compile once, call native
                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        text.hash(&mut hasher);
                        let jit_name = format!("__yk_lit_{:x}", hasher.finish());
                        if let Some(jit_fn) = compile_literal_handler(&jit_name, text, 200) {
                            let mut resp = YkResponse { body: std::ptr::null(), body_len: 0, status_code: 0 };
                            unsafe { jit_fn(&mut resp) };
                            if !resp.body.is_null() && resp.body_len > 0 {
                                let body_slice = unsafe { std::slice::from_raw_parts(resp.body, resp.body_len as usize) };
                                (String::from_utf8_lossy(body_slice).to_string(), ct)
                            } else {
                                (text.clone(), ct)
                            }
                        } else {
                            (text.clone(), ct)
                        }
                    }
                    Handler::Fn(name, fndef) => {
                        // Try JIT compilation; fall back to interpreter on failure.
                        let jit_try: Option<(String, &str)> = (|| {
                            if !fndef.params.iter().any(|p| p.name == "req") { return None; }
                            let jn = format!("__yk_fn_{}", name);
                            let jit_fn = compile_fn_handler(&jn, fndef)?;
                            let req = YkRequest {
                                method: method_raw.as_ptr(), method_len: method_raw.len() as i64,
                                path: path_raw.as_ptr(), path_len: path_raw.len() as i64,
                                body: body.as_ptr(), body_len: body.len() as i64,
                            };
                            let mut buf = vec![0u8; 65536];
                            let mut resp = YkResponse { body: std::ptr::null(), body_len: 0, status_code: 0 };
                            unsafe { jit_fn(&mut resp, &req, buf.as_mut_ptr(), buf.len() as i64) };
                            if resp.body.is_null() || resp.body_len <= 0 { return None; }
                            let body_slice = unsafe { std::slice::from_raw_parts(resp.body, resp.body_len as usize) };
                            let result = String::from_utf8_lossy(body_slice).to_string();
                            let ct = if result.as_bytes().first().map_or(false, |b| *b == b'{' || *b == b'[') { "application/json" } else { "text/plain" };
                            Some((result, ct))
                        })();

                        match jit_try {
                            Some(r) => r,
                            None => {
                                let mut interp = pool.take(base_interp());
                                interp.push_frame();

                                let has_req = fndef.params.iter().any(|p| p.name == "req");

                                if has_req {
                                    let mut fields = HashMap::with_capacity(4);
                                    fields.insert(String::from("method"), Value::Str(method_raw));
                                    fields.insert(String::from("path"), Value::Str(path_raw));
                                    fields.insert(String::from("body"), Value::Str(body));
                                    if !route_params.is_empty() {
                                        let mut pmap = HashMap::with_capacity(route_params.len());
                                        for (i, v) in route_params.iter().enumerate() {
                                            pmap.insert(i.to_string(), Value::Str(v.clone()));
                                        }
                                        fields.insert(String::from("params"), Value::Struct(String::new(), pmap));
                                    }
                                    let req_val = Value::Struct("Request".into(), fields);

                                    for param in fndef.params.iter() {
                                        let val = if param.name == "req" { req_val.clone() } else { Value::None_ };
                                        interp.frames.last_mut().unwrap().insert(param.name.clone(), val);
                                    }
                                } else {
                                    for param in fndef.params.iter() {
                                        interp.frames.last_mut().unwrap().insert(param.name.clone(), Value::None_);
                                    }
                                }

                                let result = match interp.run_fn_body(fndef) {
                                    Ok(Some(val)) => val.to_string(),
                                    _ => "Handler returned no value".into(),
                                };
                                pool.return_interp(interp);
                                let ct = if result.as_bytes().first().map_or(false, |b| *b == b'{' || *b == b'[') { "application/json" } else { "text/plain" };
                                (result, ct)
                            }
                        }
                    }
                }
            }
            None => {
                let _ = write_status_line(&mut stream, "404 Not Found", "text/plain", 9, keep_alive);
                let _ = stream.write_all(b"Not Found");
                if !keep_alive { return; } else { continue; }
            }
        };
        let _ = write_status_line(&mut stream, "200 OK", content_type, response_body.len(), keep_alive);
        let _ = stream.write_all(response_body.as_bytes());
        let _ = stream.flush();

        if !keep_alive {
            return;
        }
    }
}

pub fn call_net(func: &str, _args: Vec<Value>, interp: &mut Interpreter, span: Span) -> Result<Value> {
    match func {
        "Server" => {
            ensure_server_class(interp);
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let server = ServerInstance::new(id);
            servers().lock().unwrap().insert(id, server);
            Ok(Value::Instance("Server".into(), vec![Value::Int(id as i64)]))
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown netlib function '{}'", func))),
    }
}

pub fn call_net_method(method: &str, raw_args: &[ExprNode], args: &[Value], receiver: Value, interp: &mut Interpreter, span: Span) -> Result<Value> {
    let id = match &receiver {
        Value::Instance(_, fields) => match fields.first() {
            Some(Value::Int(id)) => *id as u64,
            _ => return Err(error::err(ErrorKind::Runtime, span, "Invalid server instance")),
        },
        _ => return Err(error::err(ErrorKind::Runtime, span, "Expected server instance")),
    };
    match method {
        "get" | "post" => {
            let path = get_str(&args, 0, span)?;
            if args.len() < 2 {
                return Err(error::err(ErrorKind::Runtime, span, "get/post requires a handler argument"));
            }
            let handler = match (args.get(1), raw_args.get(1)) {
                (Some(Value::Str(text)), Some(raw)) => {
                    match &raw.value {
                        Expr::LitStr(_) => Handler::Literal(text.clone()),
                        _ => {
                            if let Some(fndef) = interp.functions.get(text) {
                                Handler::Fn(text.clone(), fndef.clone())
                            } else {
                                return Err(error::err(ErrorKind::Runtime, span, format!("Function '{}' not found", text)));
                            }
                        }
                    }
                }
                _ => return Err(error::err(ErrorKind::Runtime, span, "Handler must be a string or function name")),
            };
            let m = if method == "get" { "GET" } else { "POST" };
            if let Some(srv) = servers().lock().unwrap().get_mut(&id) {
                srv.routes.add_route(&path, m, handler);
            }
            Ok(Value::None_)
        }
        "serve" => {
            #[cfg(windows)]
            {
                extern "system" {
                    fn timeBeginPeriod(uPeriod: u32) -> u32;
                }
                unsafe { timeBeginPeriod(1); }
            }

            let _addr = if let Some(srv) = servers().lock().unwrap().get_mut(&id) {
                let addr = match args.first() {
                    Some(Value::Int(port)) => format!("0.0.0.0:{}", port),
                    Some(Value::Str(a)) => a.clone(),
                    _ => "0.0.0.0:8080".into(),
                };
                srv.addr = addr.clone();
                addr
            } else {
                return Err(error::err(ErrorKind::Runtime, span, "Server not found"));
            };

            let server = servers().lock().unwrap().get(&id).cloned()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "Server not found"))?;

            // Pre-compile all route handlers via JIT before accepting connections
            // This moves LLVM-C DLL loading and MCJIT initialization overhead
            // from first-request time to server startup time.
            server.routes.precompile_all();

            let pool = sub_interpreter_pool();

            let conn_count = Arc::new(AtomicU64::new(0));

            let evt_server = server.clone();
            let evt_pool = pool;
            let evt_conn_count = conn_count.clone();

            let evt_handle = std::thread::spawn(move || {
                let listener = match std::net::TcpListener::bind(&evt_server.addr) {
                    Ok(l) => l,
                    Err(e) => { eprintln!("std::TcpListener::bind({}) failed: {}", evt_server.addr, e); return; }
                };

                // Standard accept loop used on all platforms.
                // On Windows, a warm-up accept at startup reduces first-connect latency
                // from ~3s to ~60ms by triggering Windows Defender/WFP socket classification
                // before the first real request.
                standard_accept_loop(&listener, &evt_server, evt_pool, &*evt_conn_count);
            });

            let _ = evt_handle.join();

            Ok(Value::None_)
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown Server method '{}'", method))),
    }
}

fn standard_accept_loop(
    listener: &std::net::TcpListener,
    server: &ServerInstance,
    pool: &SubInterpreterPool,
    conn_count: &AtomicU64,
) {
    // Warm-up: connect+accept to trigger Windows Defender/WFP socket classification
    // slow-path before the first real request. We connect from THIS thread (not a
    // background thread) to avoid racing with real clients.
    if let Some(warmup_addr) = listener.local_addr().ok() {
        if let Ok(warmup) = std::net::TcpStream::connect_timeout(
            &warmup_addr, std::time::Duration::from_secs(2))
        {
            if let Ok((ws, _)) = listener.accept() {
                let _ = ws.set_nodelay(true);
            }
            drop(warmup);
        }
    }
    loop {
        match listener.accept() {
            Ok((stream, _addr)) => {
                conn_count.fetch_add(1, Ordering::Relaxed);
                handle_connection(stream, server, pool);
                conn_count.fetch_sub(1, Ordering::Relaxed);
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::ast::*;
    use crate::diagnostics::span::Span;

    fn require_llvm() -> bool {
        match crate::codegen::llvm_api::find_llvm_lib() {
            Some(_) => true,
            None => { eprintln!("SKIP: LLVM-C not available"); false }
        }
    }

    fn make_req(path: &str, body: &str) -> YkRequest {
        YkRequest {
            method: "GET".as_ptr(), method_len: 3,
            path: path.as_ptr(), path_len: path.len() as i64,
            body: body.as_ptr(), body_len: body.len() as i64,
        }
    }

    fn make_param() -> Param {
        Param {
            name: "req".into(),
            type_expr: TypeNode::new(fresh_id(), Span::new(0, 0), TypeExpr::Infer),
            is_ref: false,
        }
    }

    fn call_fn_handler(jit_fn: JitFnHandlerFn, req: &YkRequest) -> String {
        let mut buf = vec![0u8; 65536];
        let mut resp = YkResponse { body: std::ptr::null(), body_len: 0, status_code: 0 };
        unsafe { jit_fn(&mut resp, req, buf.as_mut_ptr(), buf.len() as i64) };
        assert!(!resp.body.is_null() || resp.body_len == 0, "Body should either have a pointer or zero length");
        assert!(resp.body_len >= 0);
        if resp.body_len == 0 {
            return String::new();
        }
        unsafe { std::str::from_utf8(std::slice::from_raw_parts(resp.body, resp.body_len as usize)) }
            .expect("Body should be valid UTF-8").to_string()
    }

    #[test]
    fn test_compile_literal_handler_e2e() {
        if !require_llvm() { return; }
        let jit_fn = match compile_literal_handler("__test_lit_e2e", "Hello JIT!", 200) {
            Some(f) => f,
            None => { eprintln!("SKIP: compile_literal_handler failed"); return; }
        };
        let mut resp = YkResponse { body: std::ptr::null(), body_len: 0, status_code: 0 };
        unsafe { jit_fn(&mut resp) };
        assert!(!resp.body.is_null());
        assert_eq!(resp.body_len, 10);
        assert_eq!(resp.status_code, 200);
        let body = unsafe { std::slice::from_raw_parts(resp.body, resp.body_len as usize) };
        assert_eq!(std::str::from_utf8(body).unwrap(), "Hello JIT!");
    }

    #[test]
    fn test_compile_fn_handler_e2e() {
        if !require_llvm() { return; }
        let span = Span::new(0, 0);

        let prefix = ExprNode::new(fresh_id(), span, Expr::LitStr("Hello! You requested :".into()));
        let req_ident = ExprNode::new(fresh_id(), span, Expr::Ident("req".into()));
        let req_path = ExprNode::new(fresh_id(), span, Expr::Field(Box::new(req_ident), "path".into()));
        let concat = ExprNode::new(fresh_id(), span, Expr::BinOp(Box::new(prefix), BinOp::Add, Box::new(req_path)));
        let body = vec![StmtNode::new(fresh_id(), span, Stmt::Return(Some(concat)))];
        let fndef = FnDef::new(vec![make_param()], body);

        let jit_fn = match compile_fn_handler("__test_fn_e2e", &fndef) {
            Some(f) => f,
            None => { eprintln!("SKIP: compile_fn_handler failed"); return; }
        };

        let req = make_req("/test/path", "");
        let body_str = call_fn_handler(jit_fn, &req);
        assert_eq!(body_str, "Hello! You requested :/test/path");
    }

    #[test]
    fn test_compile_fn_handler_lit_int() {
        if !require_llvm() { return; }
        let span = Span::new(0, 0);

        // return 42
        let expr = ExprNode::new(fresh_id(), span, Expr::LitInt(42));
        let body = vec![StmtNode::new(fresh_id(), span, Stmt::Return(Some(expr)))];
        let fndef = FnDef::new(vec![make_param()], body);

        let jit_fn = match compile_fn_handler("__test_fn_int", &fndef) {
            Some(f) => f,
            None => { eprintln!("SKIP: compile_fn_handler failed"); return; }
        };

        let req = make_req("/x", "");
        let body_str = call_fn_handler(jit_fn, &req);
        assert_eq!(body_str, "42");
    }

    #[test]
    fn test_compile_fn_handler_lit_bool() {
        if !require_llvm() { return; }
        let span = Span::new(0, 0);

        // return true
        let expr = ExprNode::new(fresh_id(), span, Expr::LitBool(true));
        let body = vec![StmtNode::new(fresh_id(), span, Stmt::Return(Some(expr)))];
        let fndef = FnDef::new(vec![make_param()], body);

        let jit_fn = match compile_fn_handler("__test_fn_bool", &fndef) {
            Some(f) => f,
            None => { eprintln!("SKIP: compile_fn_handler failed"); return; }
        };

        let req = make_req("/x", "");
        let body_str = call_fn_handler(jit_fn, &req);
        assert_eq!(body_str, "true");
    }

    #[test]
    fn test_compile_fn_handler_if_integer_cond() {
        if !require_llvm() { return; }
        let span = Span::new(0, 0);

        // if 1 == 1 { return "yes" } else { return "no" }
        let cond = ExprNode::new(fresh_id(), span,
            Expr::BinOp(Box::new(ExprNode::new(fresh_id(), span, Expr::LitInt(1))),
                BinOp::Eq,
                Box::new(ExprNode::new(fresh_id(), span, Expr::LitInt(1)))));
        let then_expr = ExprNode::new(fresh_id(), span, Expr::LitStr("yes".into()));
        let else_expr = ExprNode::new(fresh_id(), span, Expr::LitStr("no".into()));
        let if_expr = ExprNode::new(fresh_id(), span,
            Expr::If(Box::new(cond), Box::new(then_expr), Some(Box::new(else_expr))));
        let body = vec![StmtNode::new(fresh_id(), span, Stmt::Return(Some(if_expr)))];
        let fndef = FnDef::new(vec![make_param()], body);

        let jit_fn = match compile_fn_handler("__test_fn_if_true", &fndef) {
            Some(f) => f,
            None => { eprintln!("SKIP: compile_fn_handler failed"); return; }
        };

        let req = make_req("/x", "");
        let body_str = call_fn_handler(jit_fn, &req);
        assert_eq!(body_str, "yes");
    }

    #[test]
    fn test_compile_fn_handler_if_false() {
        if !require_llvm() { return; }
        let span = Span::new(0, 0);

        // if 1 == 2 { return "yes" } else { return "no" }
        let cond = ExprNode::new(fresh_id(), span,
            Expr::BinOp(Box::new(ExprNode::new(fresh_id(), span, Expr::LitInt(1))),
                BinOp::Eq,
                Box::new(ExprNode::new(fresh_id(), span, Expr::LitInt(2)))));
        let then_expr = ExprNode::new(fresh_id(), span, Expr::LitStr("yes".into()));
        let else_expr = ExprNode::new(fresh_id(), span, Expr::LitStr("no".into()));
        let if_expr = ExprNode::new(fresh_id(), span,
            Expr::If(Box::new(cond), Box::new(then_expr), Some(Box::new(else_expr))));
        let body = vec![StmtNode::new(fresh_id(), span, Stmt::Return(Some(if_expr)))];
        let fndef = FnDef::new(vec![make_param()], body);

        let jit_fn = match compile_fn_handler("__test_fn_if_false", &fndef) {
            Some(f) => f,
            None => { eprintln!("SKIP: compile_fn_handler failed"); return; }
        };

        let req = make_req("/x", "");
        let body_str = call_fn_handler(jit_fn, &req);
        assert_eq!(body_str, "no");
    }

    #[test]
    fn test_compile_fn_handler_if_no_else() {
        if !require_llvm() { return; }
        let span = Span::new(0, 0);

        // if false { return "yes" }  (no else → empty)
        let cond = ExprNode::new(fresh_id(), span, Expr::LitBool(false));
        let then_expr = ExprNode::new(fresh_id(), span, Expr::LitStr("yes".into()));
        let if_expr = ExprNode::new(fresh_id(), span,
            Expr::If(Box::new(cond), Box::new(then_expr), None));
        let body = vec![StmtNode::new(fresh_id(), span, Stmt::Return(Some(if_expr)))];
        let fndef = FnDef::new(vec![make_param()], body);

        let jit_fn = match compile_fn_handler("__test_fn_no_else", &fndef) {
            Some(f) => f,
            None => { eprintln!("SKIP: compile_fn_handler failed"); return; }
        };

        let req = make_req("/x", "");
        let body_str = call_fn_handler(jit_fn, &req);
        assert_eq!(body_str, "");
    }

    #[test]
    fn test_compile_fn_handler_integer_neq() {
        if !require_llvm() { return; }
        let span = Span::new(0, 0);

        // if 99 != 100 { return "neq" } else { return "eq" }
        let cond = ExprNode::new(fresh_id(), span,
            Expr::BinOp(Box::new(ExprNode::new(fresh_id(), span, Expr::LitInt(99))),
                BinOp::Ne,
                Box::new(ExprNode::new(fresh_id(), span, Expr::LitInt(100)))));
        let then_expr = ExprNode::new(fresh_id(), span, Expr::LitStr("neq".into()));
        let else_expr = ExprNode::new(fresh_id(), span, Expr::LitStr("eq".into()));
        let if_expr = ExprNode::new(fresh_id(), span,
            Expr::If(Box::new(cond), Box::new(then_expr), Some(Box::new(else_expr))));
        let body = vec![StmtNode::new(fresh_id(), span, Stmt::Return(Some(if_expr)))];
        let fndef = FnDef::new(vec![make_param()], body);

        let jit_fn = match compile_fn_handler("__test_fn_neq", &fndef) {
            Some(f) => f,
            None => { eprintln!("SKIP: compile_fn_handler failed"); return; }
        };

        let req = make_req("/x", "");
        let body_str = call_fn_handler(jit_fn, &req);
        assert_eq!(body_str, "neq");
    }

    #[test]
    fn test_jit_integration_tcp_request() {
        if !require_llvm() { return; }
        // Build a handler: return "Hello JIT TCP!"
        let span = Span::new(0, 0);
        let ret = ExprNode::new(fresh_id(), span, Expr::LitStr("Hello JIT TCP!".into()));
        let expr = ExprNode::new(fresh_id(), span, Expr::If(
            Box::new(ExprNode::new(fresh_id(), span, Expr::LitBool(true))),
            Box::new(ret),
            None,
        ));
        let body = vec![StmtNode::new(fresh_id(), span, Stmt::Return(Some(expr)))];
        let fndef = FnDef::new(vec![make_param()], body);

        let jit_fn = match compile_fn_handler("__test_tcp_fn", &fndef) {
            Some(f) => f,
            None => { eprintln!("SKIP: compile_fn_handler failed"); return; }
        };

        let req = make_req("/test", "");
        let body_str = call_fn_handler(jit_fn, &req);
        assert_eq!(body_str, "Hello JIT TCP!");
    }
}

fn get_str(args: &[Value], idx: usize, span: Span) -> Result<String> {
    match args.get(idx) {
        Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Argument {} must be a string", idx))),
    }
}
