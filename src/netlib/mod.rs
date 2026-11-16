#![allow(dead_code)]
mod hyper_server;
use hyper_server::start_hyper_server;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::hardware::HardwareInfo;
use crate::interpret::{Value, FnDef, ClassDef, Interpreter, SubInterpreterPool};
use crate::jit::mcjit::McJit;
use crate::syntax::ast::{Expr, ExprNode, TypeNode, TypeExpr};

/// JIT-compiled handler function signature (static literal).
/// Takes a pointer to a YkResponse struct, returns void.
type JitHandlerFn = unsafe extern "C" fn(*mut YkResponse);

/// JIT-compiled FnDef handler function signature.
/// Takes (response struct, request struct, output buffer, buffer length).
type JitFnHandlerFn = unsafe extern "C" fn(*mut YkResponse, *const YkRequest, *mut u8, i64);

/// Response struct layout matching the LLVM IR `%YkResponse` type.
#[repr(C)]
pub(crate) struct YkResponse {
    pub(crate) body: *const u8,
    pub(crate) body_len: i64,
    pub(crate) status_code: i32,
}

/// Request struct layout matching the LLVM IR `%YkRequest` type.
/// Fields ordered: method(ptr, len), path(ptr, len), body(ptr, len).
#[repr(C)]
pub(crate) struct YkRequest {
    pub(crate) method: *const u8,
    pub(crate) method_len: i64,
    pub(crate) path: *const u8,
    pub(crate) path_len: i64,
    pub(crate) body: *const u8,
    pub(crate) body_len: i64,
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
pub(crate) fn compile_literal_handler(name: &str, body: &str, status: i32) -> Option<JitHandlerFn> {
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
pub(crate) fn compile_fn_handler(name: &str, fndef: &FnDef) -> Option<JitFnHandlerFn> {
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

type SharedBase = std::sync::Arc<crate::interpret::InterpBase>;

struct BaseSlot(std::sync::RwLock<Option<SharedBase>>);

impl BaseSlot {
    const fn new() -> Self {
        BaseSlot(std::sync::RwLock::new(None))
    }

    fn get(&self) -> SharedBase {
        let current = self.0.read().unwrap_or_else(|p| p.into_inner()).clone();
        current.unwrap_or_else(|| std::sync::Arc::new(crate::interpret::InterpBase::default()))
    }

    fn replace(&self, interp: &Interpreter) {
        let snapshot = std::sync::Arc::new(crate::interpret::InterpBase::from_interp(interp));
        *self.0.write().unwrap_or_else(|p| p.into_inner()) = Some(snapshot);
    }
}

static BASE_INTERP: BaseSlot = BaseSlot::new();

pub(crate) fn base_interp() -> SharedBase {
    BASE_INTERP.get()
}

pub(crate) fn install_base_interp(interp: &Interpreter) {
    BASE_INTERP.replace(interp);
}

#[derive(Clone)]
pub(crate) enum Handler {
    Literal(String),
    Fn(String, FnDef),
}

#[derive(Clone)]
struct RouteNode {
    children: HashMap<String, RouteNode>,
    wildcard: Option<Box<RouteNode>>,
    param_name: Option<String>, // For tracking :param or {param}
    handlers: Vec<(String, Handler)>,
}

impl RouteNode {
    fn new() -> Self {
        RouteNode { children: HashMap::new(), wildcard: None, param_name: None, handlers: Vec::new() }
    }

    fn insert(&mut self, segments: &[&str], method: &str, handler: Handler) {
        if segments.is_empty() {
            self.set_handler(method, handler);
            return;
        }
        let seg = segments[0];
        let (is_param, param_name) = if seg.starts_with(':') {
            (true, seg[1..].to_string())
        } else if seg.starts_with('{') && seg.ends_with('}') {
            (true, seg[1..seg.len()-1].to_string())
        } else {
            (false, String::new())
        };
        
        if is_param {
            if self.wildcard.is_none() {
                let mut node = RouteNode::new();
                node.param_name = Some(param_name);
                self.wildcard = Some(Box::new(node));
            }
            self.wildcard.as_mut().unwrap().insert(&segments[1..], method, handler);
        } else {
            self.children.entry(seg.to_string())
                .or_insert_with(RouteNode::new)
                .insert(&segments[1..], method, handler);
        }
    }

    fn set_handler(&mut self, method: &str, handler: Handler) {
        match self.handlers.iter_mut().find(|(m, _)| m == method) {
            Some(entry) => entry.1 = handler,
            None => self.handlers.push((method.to_string(), handler)),
        }
    }

    fn handler_for(&self, method: &str) -> Option<&Handler> {
        self.handlers.iter().find(|(m, _)| m == method).map(|(_, h)| h)
    }

    fn find<'a>(&'a self, segments: &[&str], method: &str, params: &mut HashMap<String, String>) -> Option<&'a Handler> {
        if segments.is_empty() {
            return self.handler_for(method);
        }
        let seg = segments[0];
        let rest = &segments[1..];

        if let Some(child) = self.children.get(seg) {
            if let found @ Some(_) = child.find(rest, method, params) {
                return found;
            }
        }
        if let Some(ref wildcard) = self.wildcard {
            if let Some(ref name) = wildcard.param_name {
                params.insert(name.clone(), seg.to_string());
            } else {
                params.insert("0".to_string(), seg.to_string()); // Fallback for old :param without name
            }
            if let found @ Some(_) = wildcard.find(rest, method, params) {
                return found;
            }
            if let Some(name) = &wildcard.param_name {
                params.remove(name);
            } else {
                params.remove("0");
            }
        }
        None
    }

    fn precompile_handlers(&self) {
        for (_, handler) in &self.handlers {
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
pub(crate) struct RouteTrie {
    root: Arc<RouteNode>,
}

impl RouteTrie {
    pub(crate) fn new() -> Self {
        RouteTrie { root: Arc::new(RouteNode::new()) }
    }

    pub(crate) fn add_route(&mut self, path: &str, method: &str, handler: Handler) {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        Arc::make_mut(&mut self.root).insert(&segments, method, handler);
    }

    pub(crate) fn match_route(&self, path: &str, method: &str) -> Option<(&Handler, HashMap<String, String>)> {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut params = HashMap::new();
        self.root.find(&segments, method, &mut params).map(|h| (h, params))
    }

    pub(crate) fn precompile_all(&self) {
        self.root.precompile_handlers();
    }
}

#[derive(Clone)]
pub(crate) struct ServerInstance {
    pub(crate) id: u64,
    pub(crate) addr: String,
    pub(crate) routes: RouteTrie,
    pub(crate) cors_enabled: bool,
    pub(crate) cors_origins: Option<Vec<String>>,
    pub(crate) cors_headers: Option<Vec<String>>,
    pub(crate) cors_methods: Option<Vec<String>>,
    pub(crate) static_dir: Option<String>,
    pub(crate) middleware: Vec<Handler>,
    pub(crate) logger: Option<Handler>,
}

impl ServerInstance {
    pub(crate) fn new(id: u64) -> Self {
        Self { 
            id, 
            addr: String::new(), 
            routes: RouteTrie::new(),
            cors_enabled: false,
            cors_origins: None,
            cors_headers: None,
            cors_methods: None,
            static_dir: None,
            middleware: Vec::new(),
            logger: None,
        }
    }
}



pub(crate) fn sub_interpreter_pool() -> &'static SubInterpreterPool {
    static POOL: std::sync::OnceLock<SubInterpreterPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| SubInterpreterPool::new(64))
}

fn servers() -> &'static std::sync::Mutex<HashMap<u64, ServerInstance>> {
    static REGISTRY: std::sync::OnceLock<std::sync::Mutex<HashMap<u64, ServerInstance>>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn udp_sockets() -> &'static std::sync::Mutex<HashMap<u64, std::net::UdpSocket>> {
    static REGISTRY: std::sync::OnceLock<std::sync::Mutex<HashMap<u64, std::net::UdpSocket>>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn tcp_listeners() -> &'static std::sync::Mutex<HashMap<u64, std::net::TcpListener>> {
    static REGISTRY: std::sync::OnceLock<std::sync::Mutex<HashMap<u64, std::net::TcpListener>>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn tcp_streams() -> &'static std::sync::Mutex<HashMap<u64, std::net::TcpStream>> {
    static REGISTRY: std::sync::OnceLock<std::sync::Mutex<HashMap<u64, std::net::TcpStream>>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

pub struct HttpInstance {
    pub id: u64,
    pub last_status: i64,
    pub last_body: String,
    pub default_method: String,
}

impl HttpInstance {
    fn new(id: u64) -> Self {
        HttpInstance { id, last_status: 0, last_body: String::new(), default_method: "GET".into() }
    }
}

pub fn http_instances() -> &'static std::sync::Mutex<HashMap<u64, HttpInstance>> {
    static REGISTRY: std::sync::OnceLock<std::sync::Mutex<HashMap<u64, HttpInstance>>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn ensure_request_class(interp: &mut Interpreter) {
    if !interp.classes.contains_key("Request") {
        Arc::make_mut(&mut interp.classes).insert("Request".into(), ClassDef {
            fields: vec!["method".into(), "path".into(), "body".into(), "headers".into(), "query".into(), "params".into(), "cookies".into()],
            methods: HashMap::new(),
            extends: None,
            constructor: Arc::new(Vec::new()),
            init_body: Arc::new(Vec::new()),
            is_data: true,
            method_cache: std::sync::Mutex::new(HashMap::new()),
            inline_cache: std::sync::Mutex::new(Default::default()),
        });
    }
}

fn ensure_response_class(interp: &mut Interpreter) {
    if !interp.classes.contains_key("Response") {
        Arc::make_mut(&mut interp.classes).insert("Response".into(), ClassDef {
            fields: vec!["status".into(), "body".into(), "headers".into(), "cookies".into()],
            methods: HashMap::new(),
            extends: None,
            constructor: Arc::new(Vec::new()),
            init_body: Arc::new(Vec::new()),
            is_data: true,
            method_cache: std::sync::Mutex::new(HashMap::new()),
            inline_cache: std::sync::Mutex::new(Default::default()),
        });
    }
}

fn ensure_form_class(interp: &mut Interpreter) {
    if !interp.classes.contains_key("Form") {
        Arc::make_mut(&mut interp.classes).insert("Form".into(), ClassDef {
            fields: vec!["fields".into(), "files".into()],
            methods: HashMap::new(),
            extends: None,
            constructor: Arc::new(Vec::new()),
            init_body: Arc::new(Vec::new()),
            is_data: true,
            method_cache: std::sync::Mutex::new(HashMap::new()),
            inline_cache: std::sync::Mutex::new(Default::default()),
        });
    }
}

fn ensure_log_entry_class(interp: &mut Interpreter) {
    if !interp.classes.contains_key("LogEntry") {
        Arc::make_mut(&mut interp.classes).insert("LogEntry".into(), ClassDef {
            fields: vec!["method".into(), "path".into(), "body".into(), "headers".into(), "query".into(), "cookies".into(), "response".into()],
            methods: HashMap::new(),
            extends: None,
            constructor: Arc::new(Vec::new()),
            init_body: Arc::new(Vec::new()),
            is_data: true,
            method_cache: std::sync::Mutex::new(HashMap::new()),
            inline_cache: std::sync::Mutex::new(Default::default()),
        });
    }
}

fn ensure_body_class(interp: &mut Interpreter) {
    if !interp.classes.contains_key("Body") {
        Arc::make_mut(&mut interp.classes).insert("Body".into(), ClassDef {
            fields: vec!["text".into(), "json".into(), "bytes".into()],
            methods: HashMap::new(),
            extends: None,
            constructor: Arc::new(Vec::new()),
            init_body: Arc::new(Vec::new()),
            is_data: true,
            method_cache: std::sync::Mutex::new(HashMap::new()),
            inline_cache: std::sync::Mutex::new(Default::default()),
        });
    }
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
    ensure_request_class(interp);
    ensure_response_class(interp);
    ensure_form_class(interp);
    ensure_body_class(interp);
}

pub(crate) const MAX_HEADER_BYTES: usize = 64 * 1024;
pub(crate) const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn find_head_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|w| w == b"\r\n\r\n")
}

pub(crate) fn decode_head(bytes: &[u8], head_end: usize) -> String {
    String::from_utf8_lossy(&bytes[..head_end]).into_owned()
}

pub(crate) fn decode_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub(crate) fn header_safe(value: &str) -> String {
    value
        .chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .collect()
}

pub(crate) fn content_length(raw: &str) -> Option<usize> {
    extract_header_value(raw, "Content-Length").and_then(|v| v.parse::<usize>().ok())
}

pub(crate) fn parse_request_line(raw: &str) -> Option<(&str, &str)> {
    let first_line = raw.lines().next()?;
    let mut parts = first_line.splitn(3, ' ');
    let method = parts.next()?;
    let path = parts.next()?;
    Some((method, path))
}

pub(crate) fn extract_header_value(raw: &str, name: &str) -> Option<String> {
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

pub(crate) fn extract_all_headers(raw: &str) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    for line in raw.lines() {
        if line.is_empty() { break; } // stop at the empty line before body
        if let Some(idx) = line.find(':') {
            let key = line[..idx].trim().to_lowercase();
            let value = line[idx + 1..].trim().to_string();
            headers.insert(key, value);
        }
    }
    headers
}

pub(crate) fn parse_query_params(path: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Some(query_part) = path.split_once('?').map(|(_, q)| q) {
        for pair in query_part.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                let key = percent_encoding::percent_decode_str(k).decode_utf8_lossy().to_string();
                let value = percent_encoding::percent_decode_str(v).decode_utf8_lossy().to_string();
                params.insert(key, value);
            }
        }
    }
    params
}

pub(crate) fn parse_cookies(headers: &HashMap<String, String>) -> HashMap<String, String> {
    let mut cookies = HashMap::new();
    if let Some(cookie_header) = headers.get("cookie") {
        for pair in cookie_header.split(';') {
            if let Some((k, v)) = pair.split_once('=') {
                let key = k.trim().to_string();
                let value = v.trim().to_string();
                cookies.insert(key, value);
            }
        }
    }
    cookies
}

pub(crate) struct ProcessedResponse {
    pub status: String,
    pub body: Vec<u8>,
    pub content_type: String,
    pub headers: Vec<String>,
}

const INVALID_STATUS: &str = "500 Internal Server Error";

fn response_status(value: &crate::interpret::Value) -> Option<String> {
    match value {
        crate::interpret::Value::Int(i) => Some(status_line_for_code(*i)),
        crate::interpret::Value::Str(s) => Some(validated_status_text(s)),
        _ => None,
    }
}

fn status_line_for_code(code: i64) -> String {
    if crate::http_status::is_valid(code) {
        format!("{} {}", code, crate::http_status::reason(code))
    } else {
        INVALID_STATUS.to_string()
    }
}

fn validated_status_text(text: &str) -> String {
    let code_is_valid = text
        .get(..3)
        .and_then(|digits| digits.parse::<i64>().ok())
        .is_some_and(crate::http_status::is_valid);
    let rest_is_valid = text.len() == 3 || text.as_bytes().get(3) == Some(&b' ');
    let has_control = text.chars().any(|c| c.is_control());
    if code_is_valid && rest_is_valid && !has_control {
        text.to_string()
    } else {
        INVALID_STATUS.to_string()
    }
}

fn inferred_content_type(body: &str) -> String {
    if body.as_bytes().first().is_some_and(|b| *b == b'{' || *b == b'[') {
        "application/json".to_string()
    } else {
        "text/plain".to_string()
    }
}

fn collect_response_headers(value: &crate::interpret::Value, out: &mut Vec<String>) {
    match value {
        crate::interpret::Value::Struct(_, fields) | crate::interpret::Value::Map(fields) => {
            for (k, v) in fields {
                out.push(format!("{}: {}", k, v));
            }
        }
        _ => {}
    }
}

fn guess_content_type(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();
    
    match ext.as_str() {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "txt" => "text/plain",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

pub(crate) fn try_serve_static(static_dir: &str, path: &str) -> Option<ProcessedResponse> {
    let base = std::fs::canonicalize(static_dir).ok()?;
    let relative = path.trim_start_matches('/');
    let candidate = base.join(relative);
    let resolved = std::fs::canonicalize(&candidate).ok()?;

    if !resolved.starts_with(&base) {
        return None;
    }
    if !resolved.is_file() {
        return None;
    }

    let body = std::fs::read(&resolved).ok()?;
    let content_type = guess_content_type(resolved.to_str().unwrap_or(""));
    Some(ProcessedResponse {
        status: "200 OK".to_string(),
        body,
        content_type: content_type.to_string(),
        headers: vec![],
    })
}

fn run_logger(
    logger: &Handler,
    method: &str,
    path: &str,
    body: &str,
    headers: &HashMap<String, String>,
    query: &HashMap<String, String>,
    cookies: &HashMap<String, String>,
    response_status: &str,
    response_body: &str,
    pool: &SubInterpreterPool
) {
    // Create a log entry struct
    let mut log_fields = crate::interpret::ValueMap::with_capacity(8);
    log_fields.insert("method".into(), Value::Str(method.into()));
    log_fields.insert("path".into(), Value::Str(path.into()));
    log_fields.insert("body".into(), Value::Str(body.into()));
    
    let headers_val = Value::Struct("".into(), headers.iter().map(|(k, v)| (k.clone(), Value::Str(v.clone()))).collect());
    log_fields.insert("headers".into(), headers_val);
    
    let query_val = Value::Struct("".into(), query.iter().map(|(k, v)| (k.clone(), Value::Str(v.clone()))).collect());
    log_fields.insert("query".into(), query_val);
    
    let cookies_val = Value::Struct("".into(), cookies.iter().map(|(k, v)| (k.clone(), Value::Str(v.clone()))).collect());
    log_fields.insert("cookies".into(), cookies_val);
    
    // Create proper Response struct
    let mut response_fields = crate::interpret::ValueMap::with_capacity(3);
    response_fields.insert("status".into(), Value::Str(response_status.into()));
    response_fields.insert("body".into(), Value::Str(response_body.into()));
    let response_val = Value::Struct("Response".into(), response_fields);
    log_fields.insert("response".into(), response_val);
    
    let log_entry = Value::Struct("LogEntry".into(), log_fields);
    
    // Now execute logger handler
    match logger {
        Handler::Literal(_text) => {},
        Handler::Fn(_name, fndef) => {
            let mut interp = pool.take(&base_interp());
            ensure_log_entry_class(&mut interp);
            ensure_response_class(&mut interp);
            interp.push_frame();
            
            // Add log_entry as argument
            if let Some(param) = fndef.params.first() {
                interp.frames.last_mut().unwrap().insert(param.name.clone(), log_entry);
            }
            
            let _ = interp.run_fn_body(fndef);
            
            if !interp.output.is_empty() {
                print!("{}", interp.output);
            }
            pool.return_interp(interp);
        }
    }
}

fn execute_handler(
    handler: &Handler,
    method_raw: &str,
    path_raw: &str,
    body: &str,
    headers: &HashMap<String, String>,
    query_params: &HashMap<String, String>,
    cookies: &HashMap<String, String>,
    route_params: Option<&HashMap<String, String>>,
    pool: &SubInterpreterPool
) -> ProcessedResponse {
    match handler {
        Handler::Literal(text) => ProcessedResponse::from_string(text.clone()),
        Handler::Fn(_name, fndef) => {
            let mut interp = pool.take(&base_interp());
            interp.push_frame();

            let mut fields = crate::interpret::ValueMap::with_capacity(8);
            fields.insert(String::from("method"), Value::Str(method_raw.to_string()));
            fields.insert(String::from("path"), Value::Str(path_raw.to_string()));
            fields.insert(String::from("body"), Value::Str(body.to_string()));

            // Add headers
            let headers_map = headers.iter()
                .map(|(k, v)| (k.clone(), Value::Str(v.clone())))
                .collect();
            fields.insert(String::from("headers"), Value::Struct(String::new(), headers_map));

            // Add query params
            let qp_map = query_params.iter()
                .map(|(k, v)| (k.clone(), Value::Str(v.clone())))
                .collect();
            fields.insert(String::from("query"), Value::Struct(String::new(), qp_map));

            // Add cookies
            let cookies_map = cookies.iter()
                .map(|(k, v)| (k.clone(), Value::Str(v.clone())))
                .collect();
            fields.insert(String::from("cookies"), Value::Struct(String::new(), cookies_map));

            // Add route params if present
            if let Some(rp) = route_params {
                let mut pmap = crate::interpret::ValueMap::with_capacity(rp.len());
                for (k, v) in rp.iter() {
                    pmap.insert(k.clone(), Value::Str(v.clone()));
                }
                fields.insert(String::from("params"), Value::Struct(String::new(), pmap));
            }

            let req_val = Value::Struct("Request".into(), fields);
            interp.frames.last_mut().unwrap().insert("req".into(), req_val);
            for param in fndef.params.iter() {
                if param.name != "req" {
                    let maybe_value = route_params.and_then(|rp| rp.get(&param.name))
                        .or_else(|| query_params.get(&param.name))
                        .map(|s| Value::Str(s.clone()));
                    
                    if let Some(v) = maybe_value {
                        let converted = match &param.type_expr {
                            TypeNode { value: TypeExpr::Named(ref n, ..), .. } if n == "int" => {
                                v.to_string().parse::<i64>().ok().map(Value::Int)
                            }
                            TypeNode { value: TypeExpr::Named(ref n, ..), .. } if n == "real" => {
                                v.to_string().parse::<f64>().ok().map(Value::Real)
                            }
                            TypeNode { value: TypeExpr::Named(ref n, ..), .. } if n == "bool" => {
                                match v.to_string().to_lowercase().as_str() {
                                    "true" | "1" => Some(Value::Bool(true)),
                                    "false" | "0" => Some(Value::Bool(false)),
                                    _ => None
                                }
                            }
                            _ => Some(v.clone())
                        };
                        if let Some(cv) = converted {
                            interp.frames.last_mut().unwrap().insert(param.name.clone(), cv);
                        } else {
                            interp.frames.last_mut().unwrap().insert(param.name.clone(), v);
                        }
                    } else {
                        interp.frames.last_mut().unwrap().insert(param.name.clone(), Value::None_);
                    }
                }
            }

            let result = match interp.run_fn_body(fndef) {
                Ok(Some(val)) => ProcessedResponse::from_value(&val),
                Ok(None) => ProcessedResponse::from_string("Handler returned no value".into()),
                Err(e) => {
                    eprintln!("handler error: {}", e.msg);
                    ProcessedResponse {
                        status: "500 Internal Server Error".to_string(),
                        body: b"Internal Server Error".to_vec(),
                        content_type: "text/plain".to_string(),
                        headers: Vec::new(),
                    }
                }
            };
            // Print everything that was printed in the handler!
            if !interp.output.is_empty() {
                print!("{}", interp.output);
            }
            pool.return_interp(interp);
            result
        }
    }
}


impl ProcessedResponse {
    pub fn new() -> Self {
        ProcessedResponse {
            status: "200 OK".to_string(),
            body: Vec::new(),
            content_type: "text/plain".to_string(),
            headers: Vec::new(),
        }
    }

    pub fn from_string(body: String) -> Self {
        let content_type = if body.as_bytes().first().map_or(false, |b| *b == b'{' || *b == b'[') {
            "application/json".to_string()
        } else {
            "text/plain".to_string()
        };
        ProcessedResponse {
            status: "200 OK".to_string(),
            body: body.into_bytes(),
            content_type,
            headers: Vec::new(),
        }
    }

    pub fn from_value(value: &crate::interpret::Value) -> Self {
        match value {
            crate::interpret::Value::Struct(class_name, fields) if class_name == "Response" => {
                let status = fields.get("status")
                    .and_then(response_status)
                    .unwrap_or_else(|| "200 OK".to_string());

                let body = fields.get("body")
                    .map(|v| v.to_string())
                    .unwrap_or_default();

                let content_type = fields.get("content_type")
                    .and_then(|v| match v {
                        crate::interpret::Value::Str(s) => Some(s.clone()),
                        _ => None
                    })
                    .unwrap_or_else(|| inferred_content_type(&body));

                let mut headers = Vec::new();
                if let Some(header_fields) = fields.get("headers") {
                    collect_response_headers(header_fields, &mut headers);
                }

                ProcessedResponse {
                    status,
                    body: body.into_bytes(),
                    content_type,
                    headers,
                }
            }
            crate::interpret::Value::Instance(class_name, fields) if class_name == "Response" => {
                let status = fields.first()
                    .and_then(response_status)
                    .unwrap_or_else(|| "200 OK".to_string());
                let body = fields.get(1).map(|v| v.to_string()).unwrap_or_default();
                let content_type = inferred_content_type(&body);
                let mut headers = Vec::new();
                if let Some(header_fields) = fields.get(2) {
                    collect_response_headers(header_fields, &mut headers);
                }
                ProcessedResponse {
                    status,
                    body: body.into_bytes(),
                    content_type,
                    headers,
                }
            }
            _ => ProcessedResponse::from_string(value.to_string())
        }
    }
}

pub(crate) fn cors_headers(server: &ServerInstance) -> String {
    if !server.cors_enabled {
        return String::new();
    }
    let mut headers = String::new();
    
    // Access-Control-Allow-Origin
    if let Some(ref origins) = server.cors_origins {
        if origins.len() == 1 {
            headers.push_str(&format!("Access-Control-Allow-Origin: {}\r\n", header_safe(&origins[0])));
        } else {
            headers.push_str("Access-Control-Allow-Origin: *\r\n");
        }
    } else {
        headers.push_str("Access-Control-Allow-Origin: *\r\n");
    }

    // Access-Control-Allow-Headers
    if let Some(ref hds) = server.cors_headers {
        headers.push_str(&format!("Access-Control-Allow-Headers: {}\r\n", header_safe(&hds.join(", "))));
    } else {
        headers.push_str("Access-Control-Allow-Headers: Content-Type, Authorization, X-Requested-With\r\n");
    }

    // Access-Control-Allow-Methods
    if let Some(ref meths) = server.cors_methods {
        headers.push_str(&format!("Access-Control-Allow-Methods: {}\r\n", header_safe(&meths.join(", "))));
    } else {
        headers.push_str("Access-Control-Allow-Methods: GET, POST, PUT, DELETE, PATCH, OPTIONS\r\n");
    }

    // Allow credentials (optional, default to false)
    headers.push_str("Access-Control-Allow-Credentials: true\r\n");

    headers
}

pub(crate) async fn write_response_async(
    stream: &mut tokio::net::TcpStream,
    response: &ProcessedResponse,
    keep_alive: bool,
    cors_headers: &str,
) -> std::io::Result<()> {
    let conn = if keep_alive { "keep-alive" } else { "close" };
    use tokio::io::AsyncWriteExt;
    let mut http_response = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: {}\r\n{}",
        header_safe(&response.status),
        header_safe(&response.content_type),
        response.body.len(),
        conn,
        cors_headers
    );
    for header in &response.headers {
        http_response.push_str(&header_safe(header));
        http_response.push_str("\r\n");
    }
    http_response.push_str("\r\n");
    stream.write_all(http_response.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    stream.flush().await
}

pub(crate) async fn write_status_line_async(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    content_type: &str,
    body_len: usize,
    keep_alive: bool,
    cors_headers: &str,
) -> std::io::Result<()> {
    let conn = if keep_alive { "keep-alive" } else { "close" };
    use tokio::io::AsyncWriteExt;
    stream.write_all(
        format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: {}\r\n{}\r\n",
            header_safe(status),
            header_safe(content_type),
            body_len,
            conn,
            cors_headers
        )
        .as_bytes(),
    )
    .await
}

fn write_response(
    stream: &mut TcpStream,
    response: &ProcessedResponse,
    keep_alive: bool,
    cors_headers: &str,
) -> std::io::Result<()> {
    let conn = if keep_alive { "keep-alive" } else { "close" };
    let mut http_response = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: {}\r\n{}",
        header_safe(&response.status),
        header_safe(&response.content_type),
        response.body.len(),
        conn,
        cors_headers
    );
    for header in &response.headers {
        http_response.push_str(&header_safe(header));
        http_response.push_str("\r\n");
    }
    http_response.push_str("\r\n");
    write!(stream, "{}", http_response)?;
    stream.write_all(&response.body)?;
    stream.flush()
}

fn write_status_line(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body_len: usize,
    keep_alive: bool,
    cors_headers: &str,
) -> std::io::Result<()> {
    let conn = if keep_alive { "keep-alive" } else { "close" };
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: {}\r\n{}\r\n",
        header_safe(status),
        header_safe(content_type),
        body_len,
        conn,
        cors_headers
    )
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
        let mut headers_too_large = false;
        loop {
            let mut chunk = [0u8; 4096];
            match stream.read(&mut chunk) {
                Ok(0) => return,
                Ok(n) => {
                    read_buf.extend_from_slice(&chunk[..n]);
                    if find_head_end(&read_buf).is_some() { break; }
                    if read_buf.len() > MAX_HEADER_BYTES {
                        headers_too_large = true;
                        break;
                    }
                }
                Err(_) => return,
            }
        }

        let cors = cors_headers(server);

        if headers_too_large {
            let _ = write_status_line(&mut stream, "431 Request Header Fields Too Large", "text/plain", 0, false, &cors);
            return;
        }

        let head_end = match find_head_end(&read_buf) {
            Some(i) => i,
            None => return,
        };
        let raw_head = decode_head(&read_buf, head_end);
        let raw = raw_head.as_str();

        let keep_alive = extract_header_value(raw, "Connection")
            .map(|v| v.eq_ignore_ascii_case("keep-alive"))
            .unwrap_or(false);

        let (method_raw, path_raw) = match parse_request_line(raw) {
            Some(r) => (r.0.to_uppercase(), r.1.to_string()),
            None => {
                let _ = write_status_line(&mut stream, "400 Bad Request", "text/plain", 9, false, &cors);
                let _ = stream.write_all(b"Bad Request");
                return;
            }
        };

        // Handle OPTIONS preflight
        if method_raw == "OPTIONS" {
            let _ = write_status_line(&mut stream, "204 No Content", "text/plain", 0, keep_alive, &cors);
            let _ = stream.flush();
            if !keep_alive { return; } else { continue; }
        }

        let body = match content_length(raw) {
            Some(cl) if cl > MAX_BODY_BYTES => {
                let _ = write_status_line(&mut stream, "413 Content Too Large", "text/plain", 0, false, &cors);
                return;
            }
            Some(cl) => {
                let body_start = head_end + 4;
                let already = read_buf.len().saturating_sub(body_start);
                if cl > already {
                    let mut extra = vec![0u8; cl - already];
                    if stream.read_exact(&mut extra).is_err() { return; }
                    let mut b = read_buf[body_start..].to_vec();
                    b.extend_from_slice(&extra);
                    decode_bytes(&b)
                } else {
                    decode_bytes(&read_buf[body_start..body_start + cl])
                }
            }
            None => String::new(),
        };

        // Parse all headers, query params, and cookies for the legacy sync server too
        let headers = extract_all_headers(raw);
        let query_params = parse_query_params(&path_raw);
        let cookies = parse_cookies(&headers);
        // Extract path without query string for route matching
        let path_for_route = path_raw.split_once('?').map(|(p, _)| p).unwrap_or(&path_raw).to_string();

        let response = match server.routes.match_route(&path_for_route, &method_raw) {
            Some((handler, route_params)) => {
                let mut short_circuit = None;
                for middleware in &server.middleware {
                    let mw_resp = execute_handler(
                        middleware,
                        &method_raw,
                        &path_raw,
                        &body,
                        &headers,
                        &query_params,
                        &cookies,
                        None,
                        pool
                    );
                    let successful = mw_resp.status.starts_with("200")
                        || mw_resp.status.starts_with("204");
                    if !successful {
                        short_circuit = Some(mw_resp);
                        break;
                    }
                }

                short_circuit.unwrap_or_else(|| {
                    execute_handler(
                        handler,
                        &method_raw,
                        &path_raw,
                        &body,
                        &headers,
                        &query_params,
                        &cookies,
                        Some(&route_params),
                        pool
                    )
                })
            }
            None => {
                // Try serving static file if static_dir is set and method is GET
                if method_raw == "GET" {
                    if let Some(ref static_dir) = server.static_dir {
                        if let Some(static_response) = try_serve_static(static_dir, &path_for_route) {
                            static_response
                        } else {
                            ProcessedResponse {
                                status: "404 Not Found".to_string(),
                                body: b"Not Found".to_vec(),
                                content_type: "text/plain".to_string(),
                                headers: Vec::new(),
                            }
                        }
                    } else {
                        ProcessedResponse {
                            status: "404 Not Found".to_string(),
                            body: b"Not Found".to_vec(),
                            content_type: "text/plain".to_string(),
                            headers: Vec::new(),
                        }
                    }
                } else {
                    ProcessedResponse {
                        status: "404 Not Found".to_string(),
                        body: b"Not Found".to_vec(),
                        content_type: "text/plain".to_string(),
                        headers: Vec::new(),
                    }
                }
            }
        };
        // Call logger if set
        if let Some(ref logger) = server.logger {
            run_logger(
                logger,
                &method_raw,
                &path_raw,
                &body,
                &headers,
                &query_params,
                &cookies,
                &response.status,
                &String::from_utf8_lossy(&response.body),
                pool
            );
        }
        let _ = write_response(&mut stream, &response, keep_alive, &cors);

        if !keep_alive {
            return;
        }
    }
}

pub fn call_net(func: &str, args: Vec<Value>, interp: &mut Interpreter, span: Span) -> Result<Value> {
    match func {
        "Server" => {
            ensure_server_class(interp);
            install_base_interp(interp);
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let server = ServerInstance::new(id);
            servers().lock().unwrap().insert(id, server);
            Ok(Value::Instance("Server".into(), vec![Value::Int(id as i64)]))
        }
        "UdpSocket" => {
            let addr = match args.first() {
                Some(Value::Str(a)) => a.clone(),
                _ => return Err(error::err(ErrorKind::Runtime, span, "UdpSocket(addr): expected string argument")),
            };
            let socket = UdpSocket::bind(&addr)
                .map_err(|e| error::err(ErrorKind::Runtime, span, format!("UdpSocket bind failed: {}", e)))?;
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            udp_sockets().lock().unwrap().insert(id, socket);
            Ok(Value::Instance("UdpSocket".into(), vec![Value::Int(id as i64)]))
        }
        "TcpStream" => {
            let addr = match args.first() {
                Some(Value::Str(a)) => a.clone(),
                _ => return Err(error::err(ErrorKind::Runtime, span, "TcpStream(addr): expected string argument")),
            };
            let stream = TcpStream::connect(&addr)
                .map_err(|e| error::err(ErrorKind::Runtime, span, format!("TcpStream connect failed: {}", e)))?;
            let _ = stream.set_nodelay(true);
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            tcp_streams().lock().unwrap().insert(id, stream);
            Ok(Value::Instance("TcpStream".into(), vec![Value::Int(id as i64)]))
        }
        "lookup" => {
            let host = match args.first() {
                Some(Value::Str(a)) => a.clone(),
                _ => return Err(error::err(ErrorKind::Runtime, span, "lookup(host): expected string argument")),
            };
            let ip = match std::net::ToSocketAddrs::to_socket_addrs(&host.as_str()) {
                Ok(mut addrs) => addrs.find_map(|a| Some(a.ip().to_string())).unwrap_or_default(),
                Err(e) => return Err(error::err(ErrorKind::Runtime, span, format!("DNS lookup failed: {}", e))),
            };
            Ok(Value::Str(ip))
        }
        "HTTP" => {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            http_instances().lock().unwrap().insert(id, HttpInstance::new(id));
            Ok(Value::Instance("HTTP".into(), vec![Value::Int(id as i64)]))
        }
        "TcpListener" => {
            let addr = match args.first() {
                Some(Value::Str(a)) => a.clone(),
                _ => return Err(error::err(ErrorKind::Runtime, span, "TcpListener(addr): expected string argument")),
            };
            let listener = std::net::TcpListener::bind(&addr)
                .map_err(|e| error::err(ErrorKind::Runtime, span, format!("TcpListener bind failed: {}", e)))?;
            listener.set_nonblocking(true).ok();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            tcp_listeners().lock().unwrap().insert(id, listener);
            Ok(Value::Instance("TcpListener".into(), vec![Value::Int(id as i64)]))
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown netlib function '{}'", func))),
    }
}

pub fn call_net_method(method: &str, raw_args: &[ExprNode], args: &[Value], receiver: Value, interp: &mut Interpreter, span: Span) -> Result<Value> {
    let (type_name, id) = match &receiver {
        Value::Instance(tn, fields) => match fields.first() {
            Some(Value::Int(id)) => (tn.clone(), *id as u64),
            _ => return Err(error::err(ErrorKind::Runtime, span, "Invalid instance")),
        },
        _ => return Err(error::err(ErrorKind::Runtime, span, "Expected instance")),
    };
    match type_name.as_str() {
        "Server" => match method {
            "cors" => {
                if let Some(srv) = servers().lock().unwrap().get_mut(&id) {
                    srv.cors_enabled = true;
                    // Parse optional arguments: origins, headers, methods
                    if args.len() >= 1 {
                        if let Value::List(ref orgs) = &args[0] {
                            let origins: Vec<String> = orgs.iter()
                                .filter_map(|v| if let Value::Str(s) = v { Some(s.clone()) } else { None })
                                .collect();
                            if !origins.is_empty() {
                                srv.cors_origins = Some(origins);
                            }
                        }
                    }
                    if args.len() >= 2 {
                        if let Value::List(ref hdrs) = &args[1] {
                            let headers: Vec<String> = hdrs.iter()
                                .filter_map(|v| if let Value::Str(s) = v { Some(s.clone()) } else { None })
                                .collect();
                            if !headers.is_empty() {
                                srv.cors_headers = Some(headers);
                            }
                        }
                    }
                    if args.len() >= 3 {
                        if let Value::List(ref meths) = &args[2] {
                            let methods: Vec<String> = meths.iter()
                                .filter_map(|v| if let Value::Str(s) = v { Some(s.clone()) } else { None })
                                .collect();
                            if !methods.is_empty() {
                                srv.cors_methods = Some(methods);
                            }
                        }
                    }
                }
                Ok(Value::None_)
            }
            "static" => {
                let dir = get_str(&args, 0, span)?;
                if let Some(srv) = servers().lock().unwrap().get_mut(&id) {
                    srv.static_dir = Some(dir.clone());
                }
                Ok(Value::None_)
            }
            "logger" => {
                if args.len() < 1 {
                    return Err(error::err(ErrorKind::Runtime, span, "logger requires a handler argument"));
                }
                let handler = match (args.get(0), raw_args.get(0)) {
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
                    (Some(Value::Fn(fndef)), _) => {
                        Handler::Fn(format!("__logger_fn_{}", fndef.body.len()), fndef.clone())
                    }
                    _ => return Err(error::err(ErrorKind::Runtime, span, "Handler must be a string, function name, fn(){} or ()=>{")),
                };
                if let Some(srv) = servers().lock().unwrap().get_mut(&id) {
                    srv.logger = Some(handler);
                }
                Ok(Value::None_)
            }
            "use" => {
                if args.len() < 1 {
                    return Err(error::err(ErrorKind::Runtime, span, "use requires a handler argument"));
                }
                let handler = match (args.get(0), raw_args.get(0)) {
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
                    (Some(Value::Fn(fndef)), _) => {
                        Handler::Fn(format!("__handler_fn_{}", fndef.body.len()), fndef.clone())
                    }
                    _ => return Err(error::err(ErrorKind::Runtime, span, "Handler must be a string, function name, fn(){} or ()=>{}")),
                };
                if let Some(srv) = servers().lock().unwrap().get_mut(&id) {
                    srv.middleware.push(handler);
                }
                Ok(Value::None_)
            }
            "get" | "post" | "put" | "delete" | "patch" | "ws" => {
                let path = get_str(&args, 0, span)?;
                if args.len() < 2 {
                    return Err(error::err(ErrorKind::Runtime, span, "route requires a handler argument"));
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
                    (Some(Value::Fn(fndef)), _) => {
                        Handler::Fn(format!("__handler_fn_{}", fndef.body.len()), fndef.clone())
                    }
                    _ => return Err(error::err(ErrorKind::Runtime, span, "Handler must be a string, function name, fn(){} or ()=>{")),
                };
                let m = match method {
                    "get" => "GET",
                    "post" => "POST",
                    "put" => "PUT",
                    "delete" => "DELETE",
                    "patch" => "PATCH",
                    "ws" => "WS",
                    _ => "GET",
                };
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
                        _ => format!("0.0.0.0:{}", crate::stdlib::DEFAULT_SERVER_PORT),
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

                // Use Hyper Server (Tokio) for maximum performance
                start_hyper_server(server, _addr);

                Ok(Value::None_)
            }
            _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown Server method '{}'", method))),
        },
        "TcpStream" => {
            let mut streams = tcp_streams().lock().unwrap();
            let stream = streams.get_mut(&id)
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "TcpStream not found (already closed?)"))?;
            match method {
                "send" => {
                    let data = get_str(args, 0, span)?;
                    let written = stream.write(data.as_bytes())
                        .map_err(|e| error::err(ErrorKind::Runtime, span, format!("TcpStream.send: {}", e)))?;
                    Ok(Value::Int(written as i64))
                }
                "recv" => {
                    let n = match args.first() {
                        Some(Value::Int(n)) => *n as usize,
                        _ => 4096,
                    };
                    let mut buf = vec![0u8; n];
                    match stream.read(&mut buf) {
                        Ok(0) => Ok(Value::Str(String::new())),
                        Ok(read) => {
                            buf.truncate(read);
                            Ok(Value::Str(String::from_utf8_lossy(&buf).to_string()))
                        }
                        Err(e) => Err(error::err(ErrorKind::Runtime, span, format!("TcpStream.recv: {}", e))),
                    }
                }
                "close" => {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    drop(streams.remove(&id));
                    Ok(Value::None_)
                }
                _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown TcpStream method '{}'", method))),
            }
        }
        "TcpListener" => {
            let mut listeners = tcp_listeners().lock().unwrap();
            let listener = listeners.get_mut(&id)
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "TcpListener not found"))?;
            match method {
                "accept" => {
                    listener.set_nonblocking(false).ok();
                    let (stream, _) = listener.accept()
                        .map_err(|e| error::err(ErrorKind::Runtime, span, format!("TcpListener.accept: {}", e)))?;
                    let _ = stream.set_nodelay(true);
                    let stream_id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                    tcp_streams().lock().unwrap().insert(stream_id, stream);
                    Ok(Value::Instance("TcpStream".into(), vec![Value::Int(stream_id as i64)]))
                }
                "close" => {
                    drop(listeners.remove(&id));
                    Ok(Value::None_)
                }
                _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown TcpListener method '{}'", method))),
            }
        }
        "UdpSocket" => {
            let mut sockets = udp_sockets().lock().unwrap();
            let socket = sockets.get_mut(&id)
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "UdpSocket not found"))?;
            match method {
                "send_to" => {
                    let data = get_str(args, 0, span)?;
                    let addr = get_str(args, 1, span)?;
                    let sent = socket.send_to(data.as_bytes(), &addr)
                        .map_err(|e| error::err(ErrorKind::Runtime, span, format!("UdpSocket.send_to: {}", e)))?;
                    Ok(Value::Int(sent as i64))
                }
                "recv_from" => {
                    let n = match args.first() {
                        Some(Value::Int(n)) => *n as usize,
                        _ => 4096,
                    };
                    let mut buf = vec![0u8; n];
                    match socket.recv_from(&mut buf) {
                        Ok((read, src)) => {
                            buf.truncate(read);
                            let data = String::from_utf8_lossy(&buf).to_string();
                            let addr = src.to_string();
                            let mut fields = crate::interpret::ValueMap::new();
                            fields.insert("data".into(), Value::Str(data));
                            fields.insert("addr".into(), Value::Str(addr));
                            Ok(Value::Struct("recv_result".into(), fields))
                        }
                        Err(e) => Err(error::err(ErrorKind::Runtime, span, format!("UdpSocket.recv_from: {}", e))),
                    }
                }
                "close" => {
                    drop(sockets.remove(&id));
                    Ok(Value::None_)
                }
                _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown UdpSocket method '{}'", method))),
            }
        }
        "HTTP" => {
            let mut instances = http_instances().lock().unwrap();
            let http = instances.get_mut(&id)
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "HTTP instance not found"))?;
            match method {
                "get" => {
                    let url = get_str(args, 0, span)?;
                    let req = ureq::http::Request::builder()
                        .method("GET")
                        .uri(&url)
                        .body(String::new())
                        .map_err(|e| error::err(ErrorKind::Runtime, span, format!("HTTP build: {}", e)))?;
                    let mut response = ureq::run(req)
                        .map_err(|e| error::err(ErrorKind::Runtime, span, format!("HTTP GET: {}", e)))?;
                    http.last_status = response.status().as_u16() as i64;
                    http.last_body = response.body_mut().read_to_string()
                        .map_err(|e| error::err(ErrorKind::Runtime, span, format!("HTTP read: {}", e)))?;
                    Ok(Value::Int(http.last_status))
                }
                "post" | "put" | "delete" | "head" | "options" | "patch" => {
                    let url = get_str(args, 0, span)?;
                    let body = if method == "post" || method == "put" || method == "patch" {
                        args.get(1).map(|v| v.to_string()).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    let http_method: ureq::http::Method = method.to_uppercase().parse()
                        .map_err(|e| error::err(ErrorKind::Runtime, span, format!("bad HTTP method: {}", e)))?;
                    let req = ureq::http::Request::builder()
                        .method(http_method)
                        .uri(&url)
                        .body(body)
                        .map_err(|e| error::err(ErrorKind::Runtime, span, format!("HTTP build: {}", e)))?;
                    let mut response = ureq::run(req)
                        .map_err(|e| error::err(ErrorKind::Runtime, span, format!("HTTP {}: {}", &method, e)))?;
                    http.last_status = response.status().as_u16() as i64;
                    http.last_body = response.body_mut().read_to_string()
                        .map_err(|e| error::err(ErrorKind::Runtime, span, format!("HTTP read: {}", e)))?;
                    Ok(Value::Int(http.last_status))
                }
                "status" => Ok(Value::Int(http.last_status)),
                "body" => Ok(Value::Str(http.last_body.clone())),
                "method" => Ok(Value::Str(http.default_method.clone())),
                _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown HTTP method '{}'", method))),
            }
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown instance type '{}'", type_name))),
    }
}

fn standard_accept_loop(
    listener: &std::net::TcpListener,
    server: &ServerInstance,
    pool: &'static SubInterpreterPool,
    conn_count: Arc<AtomicU64>,
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
    let server = server.clone();
    loop {
        match listener.accept() {
            Ok((stream, _addr)) => {
                let server = server.clone();
                let conn_count = conn_count.clone();
                conn_count.fetch_add(1, Ordering::Relaxed);
                std::thread::spawn(move || {
                    handle_connection(stream, &server, pool);
                    conn_count.fetch_sub(1, Ordering::Relaxed);
                });
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
    use std::net::TcpListener;

    fn literal_of(trie: &RouteTrie, path: &str, method: &str) -> Option<String> {
        match trie.match_route(path, method) {
            Some((Handler::Literal(text), _)) => Some(text.clone()),
            _ => None,
        }
    }

    #[test]
    fn two_methods_on_one_path_keep_their_own_handler() {
        let mut trie = RouteTrie::new();
        trie.add_route("/notes", "GET", Handler::Literal("list".into()));
        trie.add_route("/notes", "POST", Handler::Literal("created".into()));
        assert_eq!(literal_of(&trie, "/notes", "GET").as_deref(), Some("list"));
        assert_eq!(literal_of(&trie, "/notes", "POST").as_deref(), Some("created"));
        assert_eq!(literal_of(&trie, "/notes", "DELETE"), None);
    }

    #[test]
    fn registering_a_method_again_replaces_its_handler() {
        let mut trie = RouteTrie::new();
        trie.add_route("/notes/{id}", "GET", Handler::Literal("old".into()));
        trie.add_route("/notes/{id}", "GET", Handler::Literal("new".into()));
        assert_eq!(literal_of(&trie, "/notes/7", "GET").as_deref(), Some("new"));
    }

    fn start_echo_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                if let Ok(n) = stream.read(&mut buf) {
                    let _ = stream.write_all(&buf[..n]);
                }
            }
        });
        port
    }

    fn require_llvm() -> bool {
        match crate::codegen::llvm_api::find_llvm_lib() {
            Some(_) => true,
            None => { eprintln!("SKIP: LLVM-C not available"); false }
        }
    }

    #[test]
    fn response_instance_drives_status() {
        let mut headers = crate::interpret::ValueMap::new();
        headers.insert("X-Test".to_string(), crate::interpret::Value::Str("1".into()));
        let value = crate::interpret::Value::Instance(
            "Response".into(),
            vec![
                crate::interpret::Value::Int(401),
                crate::interpret::Value::Str("denied".into()),
                crate::interpret::Value::Map(headers),
                crate::interpret::Value::Map(crate::interpret::ValueMap::new()),
            ],
        );
        let response = ProcessedResponse::from_value(&value);
        assert_eq!(response.status, "401 Unauthorized");
        assert_eq!(response.body, b"denied");
        assert_eq!(response.headers, vec!["X-Test: 1".to_string()]);
    }

    #[test]
    fn header_values_cannot_inject_lines() {
        assert_eq!(header_safe("text/plain\r\nX-Evil: 1"), "text/plain  X-Evil: 1");
        assert_eq!(header_safe("ok\n"), "ok ");
    }

    #[test]
    fn head_end_is_detected_incrementally() {
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\nHost: x\r\n\r\nbody"), Some(23));
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\nHost: x\r\n"), None);
    }

    #[test]
    fn content_length_reads_the_header() {
        assert_eq!(content_length("POST / HTTP/1.1\r\nContent-Length: 42\r\n\r\n"), Some(42));
        assert_eq!(content_length("POST / HTTP/1.1\r\n\r\n"), None);
        assert_eq!(content_length("POST / HTTP/1.1\r\nContent-Length: nope\r\n\r\n"), None);
    }

    #[test]
    fn static_serving_keeps_binary_files_intact() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("yk_static_bin_{}_{}", std::process::id(), unique));
        std::fs::create_dir_all(&base).unwrap();
        let png_header = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xFF, 0x00];
        std::fs::write(base.join("logo.png"), &png_header).unwrap();

        let served = try_serve_static(base.to_str().unwrap(), "/logo.png").expect("binary file should be served");
        assert_eq!(served.body, png_header);
        assert_eq!(served.content_type, "image/png");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn static_serving_rejects_traversal() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("yk_static_{}_{}", std::process::id(), unique));
        let base = root.join("public");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("index.html"), "hello").unwrap();
        std::fs::write(root.join("secret.txt"), "secret").unwrap();

        let base_str = base.to_str().unwrap();
        let served = try_serve_static(base_str, "/index.html").expect("index should be served");
        assert_eq!(served.body, b"hello");
        assert!(try_serve_static(base_str, "/../secret.txt").is_none());
        assert!(try_serve_static(base_str, "/%2e%2e/secret.txt").is_none());
        assert!(try_serve_static(base_str, "/missing.html").is_none());

        let _ = std::fs::remove_dir_all(&root);
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

        // if false { return "yes" }  (no else -> empty)
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

    #[test]
    fn test_tcp_stream_echo_interp() {
        let port = start_echo_server();
        let mut interp = Interpreter::new();
        interp.tui_mode = false;

        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        tcp_streams().lock().unwrap().insert(id, stream);

        let sock_val = Value::Instance("TcpStream".into(), vec![Value::Int(id as i64)]);

        // send "hello"
        let _ = call_net_method("send", &[], &[Value::Str("hello".into())], sock_val.clone(), &mut interp, Span::new(0, 0)).unwrap();

        // recv 1024 bytes
        let result = call_net_method("recv", &[], &[Value::Int(1024)], sock_val.clone(), &mut interp, Span::new(0, 0)).unwrap();
        assert_eq!(result, Value::Str("hello".into()), "echo should match");

        // close
        let _ = call_net_method("close", &[], &[], sock_val, &mut interp, Span::new(0, 0)).unwrap();
    }

    #[test]
    fn status_codes_map_to_their_reason_phrases() {
        for (code, expected) in [
            (200, "200 OK"),
            (204, "204 No Content"),
            (404, "404 Not Found"),
            (418, "418 I'm a Teapot"),
            (422, "422 Unprocessable Content"),
            (451, "451 Unavailable For Legal Reasons"),
            (503, "503 Service Unavailable"),
        ] {
            let mut fields = crate::interpret::ValueMap::new();
            fields.insert("status".to_string(), Value::Int(code));
            let resp = ProcessedResponse::from_value(&Value::Struct("Response".into(), fields));
            assert_eq!(resp.status, expected, "status line for {code}");
        }
    }

    #[test]
    fn base_slot_keeps_the_latest_program() {
        let slot = BaseSlot::new();
        let mut first = Interpreter::new();
        first.source_dir = std::path::PathBuf::from("first");
        slot.replace(&first);
        let mut second = Interpreter::new();
        second.source_dir = std::path::PathBuf::from("second");
        slot.replace(&second);
        assert_eq!(slot.get().source_dir, std::path::PathBuf::from("second"));
    }

    fn status_of(value: Value) -> String {
        let mut fields = crate::interpret::ValueMap::new();
        fields.insert("status".to_string(), value);
        ProcessedResponse::from_value(&Value::Struct("Response".into(), fields)).status
    }

    #[test]
    fn out_of_range_status_codes_become_500() {
        for code in [0, -1, 99, 600, 99999] {
            assert_eq!(status_of(Value::Int(code)), "500 Internal Server Error", "code {code}");
        }
    }

    #[test]
    fn string_status_with_crlf_is_rejected() {
        let injected = "200 OK\r\nSet-Cookie: session=stolen";
        assert_eq!(status_of(Value::Str(injected.into())), "500 Internal Server Error");
    }

    #[test]
    fn malformed_string_status_is_rejected() {
        for text in ["", "OK", "20", "2000 OK", "200OK", "abc Teapot", "700 Custom"] {
            assert_eq!(status_of(Value::Str(text.into())), "500 Internal Server Error", "{text:?}");
        }
    }

    #[test]
    fn well_formed_string_status_is_kept() {
        assert_eq!(status_of(Value::Str("201 Created".into())), "201 Created");
        assert_eq!(status_of(Value::Str("299 Custom Reason".into())), "299 Custom Reason");
        assert_eq!(status_of(Value::Str("204".into())), "204");
    }

    #[test]
    fn unknown_status_codes_are_not_labelled_ok() {
        let mut fields = crate::interpret::ValueMap::new();
        fields.insert("status".to_string(), Value::Int(599));
        let resp = ProcessedResponse::from_value(&Value::Struct("Response".into(), fields));
        assert_eq!(resp.status, "599 Unknown Status Code");
    }
}

fn get_str(args: &[Value], idx: usize, span: Span) -> Result<String> {
    match args.get(idx) {
        Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Argument {} must be a string", idx))),
    }
}
