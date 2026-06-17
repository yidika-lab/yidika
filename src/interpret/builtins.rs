use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use chrono::Datelike;
use chrono::Timelike;
use regex::Regex;
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use super::Value;

static REGEX_CACHE: LazyLock<Mutex<HashMap<String, Regex>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn get_or_compile_regex(pattern: &str) -> Result<Regex> {
    let mut cache = REGEX_CACHE.lock().unwrap();
    if let Some(re) = cache.get(pattern) {
        return Ok(re.clone());
    }
    let re = Regex::new(pattern)
        .map_err(|e| error::err(ErrorKind::Runtime, Span::new(0, 0), format!("regex.match: {}", e)))?;
    cache.insert(pattern.to_string(), re.clone());
    Ok(re)
}

pub fn call_math(field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
    let mut it = args.into_iter();
    match field {
        "cos" | "sin" | "sqrt" | "floor" | "ceil" | "round" => {
            let x = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, format!("math.{}() requires 1 argument", field)))?;
            let x = match x { Value::Int(i) => i as f64, Value::Real(r) => r, _ => return Err(error::err(ErrorKind::Runtime, span, "Expected number")) };
            let r = match field {
                "cos" => x.cos(), "sin" => x.sin(), "sqrt" => x.sqrt(),
                "floor" => x.floor(), "ceil" => x.ceil(), "round" => x.round(),
                _ => unreachable!(),
            };
            Ok(Value::Real(r))
        }
        "abs" => {
            let x = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "math.abs() requires 1 argument"))?;
            match x {
                Value::Int(i) => Ok(Value::Int(i.abs())),
                Value::Real(r) => Ok(Value::Real(r.abs())),
                _ => Err(error::err(ErrorKind::Runtime, span, "math.abs() requires a number")),
            }
        }
        "max" | "min" => {
            let a = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, format!("math.{}() requires 2 arguments", field)))?;
            let b = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, format!("math.{}() requires 2 arguments", field)))?;
            let to_f64 = |v: Value| -> Result<f64> { match v { Value::Int(i) => Ok(i as f64), Value::Real(r) => Ok(r), _ => Err(error::err(ErrorKind::Runtime, span, "Expected number")) } };
            let (af, bf) = (to_f64(a)?, to_f64(b)?);
            let r = match field { "max" => af.max(bf), _ => af.min(bf) };
            Ok(Value::Real(r))
        }
        "pow" => {
            let base = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "math.pow() requires 2 arguments"))?;
            let exp = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "math.pow() requires 2 arguments"))?;
            let to_f64 = |v: Value| -> Result<f64> { match v { Value::Int(i) => Ok(i as f64), Value::Real(r) => Ok(r), _ => Err(error::err(ErrorKind::Runtime, span, "Expected number")) } };
            Ok(Value::Real(to_f64(base)?.powf(to_f64(exp)?)))
        }
        "rand" => {
            let max = it.next();
            let r = if let Some(v) = max {
                let m = match v { Value::Int(i) => i, _ => return Err(error::err(ErrorKind::Runtime, span, "math.rand() requires an integer")) };
                fastrand::i32(0..m as i32) as i64
            } else {
                fastrand::i32(0..i32::MAX) as i64
            };
            Ok(Value::Int(r))
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown math function '{}'", field))),
    }
}

pub fn call_time(field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
    match field {
        "now" => {
            Ok(Value::Str(format!("{}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default().as_secs())))
        }
        "timestamp" => {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default().as_secs() as i64;
            Ok(Value::Int(secs))
        }
        "sleep" => {
            let ms = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "time.sleep() requires 1 argument"))?;
            let ms = match ms { Value::Int(i) => i, _ => return Err(error::err(ErrorKind::Runtime, span, "time.sleep() requires an integer")) };
            std::thread::sleep(std::time::Duration::from_millis(ms as u64));
            Ok(Value::None_)
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown time function '{}'", field))),
    }
}

pub fn call_json(field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
    match field {
        "parse" => {
            let s = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "json.parse() requires 1 argument"))?;
            let s = match s { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "json.parse() requires a string")) };
            let v: serde_json::Value = serde_json::from_str(&s)
                .map_err(|e| error::err(ErrorKind::Runtime, span, format!("json.parse: {}", e)))?;
            Ok(json_to_value(v))
        }
        "stringify" => {
            let mut it2 = args.into_iter();
            let v = it2.next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "json.stringify() requires 1 argument"))?;
            let pretty = it2.next();
            let is_pretty = matches!(pretty, Some(Value::Bool(true)));
            let json_val = value_to_json(&v);
            let s = if is_pretty { serde_json::to_string_pretty(&json_val) } else { serde_json::to_string(&json_val) };
            Ok(Value::Str(s.map_err(|e| error::err(ErrorKind::Runtime, span, format!("json.stringify: {}", e)))?))
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown json function '{}'", field))),
    }
}

pub fn call_datetime(field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
    let mut it = args.into_iter();
    match field {
        "now" => Ok(Value::Str(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())),
        "utc" => Ok(Value::Str(chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string())),
        "timestamp" => Ok(Value::Int(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64)),
        "format" => {
            let ts = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "datetime.format() requires 2 arguments"))?;
            let fmt = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "datetime.format() requires 2 arguments"))?;
            let ts = match ts { Value::Int(i) => i, _ => return Err(error::err(ErrorKind::Runtime, span, "datetime.format() timestamp must be an integer")) };
            let fmt = match fmt { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "datetime.format() format must be a string")) };
            let dt = chrono::DateTime::from_timestamp(ts, 0)
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "datetime.format() invalid timestamp"))?;
            let local: chrono::DateTime<chrono::Local> = chrono::DateTime::from(dt);
            Ok(Value::Str(local.format(&fmt).to_string()))
        }
        "parse" => {
            let s = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "datetime.parse() requires 2 arguments"))?;
            let fmt = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "datetime.parse() requires 2 arguments"))?;
            let s = match s { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "datetime.parse() string must be a string")) };
            let fmt = match fmt { Value::Str(f) => f, _ => return Err(error::err(ErrorKind::Runtime, span, "datetime.parse() format must be a string")) };
            let dt = chrono::NaiveDateTime::parse_from_str(&s, &fmt)
                .map_err(|e| error::err(ErrorKind::Runtime, span, format!("datetime.parse: {}", e)))?;
            Ok(Value::Int(dt.and_utc().timestamp()))
        }
        "year" | "month" | "day" | "hour" | "minute" | "second" => {
            let ts = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, format!("datetime.{}() requires 1 argument", field)))?;
            let ts = match ts { Value::Int(i) => i, _ => return Err(error::err(ErrorKind::Runtime, span, "timestamp must be an integer")) };
            let dt = chrono::DateTime::from_timestamp(ts, 0)
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "invalid timestamp"))?;
            let local: chrono::DateTime<chrono::Local> = chrono::DateTime::from(dt);
            let r = match field {
                "year" => local.year() as i64,
                "month" => local.month() as i64,
                "day" => local.day() as i64,
                "hour" => local.hour() as i64,
                "minute" => local.minute() as i64,
                "second" => local.second() as i64,
                _ => unreachable!(),
            };
            Ok(Value::Int(r))
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown datetime function '{}'", field))),
    }
}

pub fn call_path_module(field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
    use std::path::Path;
    match field {
        "join" => {
            let parts: Vec<String> = args.into_iter().map(|v| v.to_string()).collect();
            let p: PathBuf = parts.iter().collect();
            Ok(Value::Str(p.to_string_lossy().to_string().replace('\\', "/")))
        }
        "dirname" => {
            let p = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "path.dirname() requires 1 argument"))?;
            let p = match p { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "path.dirname() requires a string")) };
            match Path::new(&p).parent() {
                Some(parent) => Ok(Value::Str(parent.to_string_lossy().to_string().replace('\\', "/"))),
                None => Ok(Value::Str("".into())),
            }
        }
        "basename" => {
            let p = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "path.basename() requires 1 argument"))?;
            let p = match p { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "path.basename() requires a string")) };
            match Path::new(&p).file_name() {
                Some(name) => Ok(Value::Str(name.to_string_lossy().to_string())),
                None => Ok(Value::Str("".into())),
            }
        }
        "extension" => {
            let p = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "path.extension() requires 1 argument"))?;
            let p = match p { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "path.extension() requires a string")) };
            match Path::new(&p).extension() {
                Some(ext) => Ok(Value::Str(ext.to_string_lossy().to_string())),
                None => Ok(Value::Str("".into())),
            }
        }
        "is_absolute" => {
            let p = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "path.is_absolute() requires 1 argument"))?;
            let p = match p { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "path.is_absolute() requires a string")) };
            Ok(Value::Bool(Path::new(&p).is_absolute()))
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown path function '{}'", field))),
    }
}

pub fn call_base64(field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
    match field {
        "encode" => {
            let s = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "base64.encode() requires 1 argument"))?;
            let s = match s { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "base64.encode() requires a string")) };
            Ok(Value::Str(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, s.as_bytes())))
        }
        "decode" => {
            let s = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "base64.decode() requires 1 argument"))?;
            let s = match s { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "base64.decode() requires a string")) };
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &s)
                .map_err(|e| error::err(ErrorKind::Runtime, span, format!("base64.decode: {}", e)))?;
            Ok(Value::Str(String::from_utf8_lossy(&bytes).to_string()))
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown base64 function '{}'", field))),
    }
}

pub fn call_regex(field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
    let mut it = args.into_iter();
    match field {
        "match" => {
            let pattern = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "regex.match() requires 2 arguments"))?;
            let text = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "regex.match() requires 2 arguments"))?;
            let pattern = match pattern { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "regex.match() pattern must be a string")) };
            let text = match text { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "regex.match() text must be a string")) };
            let re = get_or_compile_regex(&pattern)?;
            Ok(Value::Bool(re.is_match(&text)))
        }
        "find" => {
            let pattern = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "regex.find() requires 2 arguments"))?;
            let text = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "regex.find() requires 2 arguments"))?;
            let pattern = match pattern { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "regex.find() pattern must be a string")) };
            let text = match text { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "regex.find() text must be a string")) };
            let re = get_or_compile_regex(&pattern)?;
            let matches: Vec<Value> = re.find_iter(&text).map(|m| Value::Str(m.as_str().to_string())).collect();
            Ok(Value::List(matches))
        }
        "replace" => {
            let pattern = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "regex.replace() requires 3 arguments"))?;
            let text = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "regex.replace() requires 3 arguments"))?;
            let replacement = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "regex.replace() requires 3 arguments"))?;
            let pattern = match pattern { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "regex.replace() pattern must be a string")) };
            let text = match text { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "regex.replace() text must be a string")) };
            let replacement = match replacement { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "regex.replace() replacement must be a string")) };
            let re = get_or_compile_regex(&pattern)?;
            Ok(Value::Str(re.replace_all(&text, replacement).to_string()))
        }
        "split" => {
            let pattern = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "regex.split() requires 2 arguments"))?;
            let text = it.next().ok_or_else(|| error::err(ErrorKind::Runtime, span, "regex.split() requires 2 arguments"))?;
            let pattern = match pattern { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "regex.split() pattern must be a string")) };
            let text = match text { Value::Str(s) => s, _ => return Err(error::err(ErrorKind::Runtime, span, "regex.split() text must be a string")) };
            let re = get_or_compile_regex(&pattern)?;
            let parts: Vec<Value> = re.split(&text).map(|p| Value::Str(p.to_string())).collect();
            Ok(Value::List(parts))
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown regex function '{}'", field))),
    }
}

pub fn call_ffi(name: &str, field: &str, args: Vec<Value>, span: Span, ffi_path: &str, ffi_libs: &HashMap<String, Arc<libloading::Library>>) -> Result<Value> {
    let lib_name = format!("yk_ffi_{}", ffi_path.replace('/', "_").replace('.', ""));
    let lib = match ffi_libs.get(&lib_name) {
        Some(l) => l.clone(),
        None => return Err(error::err(ErrorKind::Runtime, span, format!("FFI library '{}' not loaded", lib_name))),
    };
    let func_name = format!("yk_{}_{}", name, field);

    let func: libloading::Symbol<unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64> = unsafe {
        match lib.get(func_name.as_bytes()) {
            Ok(f) => f,
            Err(_) => return Err(error::err(ErrorKind::Runtime, span, format!("FFI function '{}' not found in {}", func_name, lib_name))),
        }
    };
    let fp = *func;

    let raw_args: Vec<i64> = args.iter().map(|v| match v {
        Value::Int(i) => *i,
        Value::Real(f) => f.to_bits() as i64,
        Value::Bool(b) => if *b { 1 } else { 0 },
        Value::Str(s) => std::ffi::CString::new(s.as_str())
            .map(|cs| cs.into_raw() as i64)
            .unwrap_or(0),
        Value::None_ => 0,
        _ => 0,
    }).collect();

    let result: i64 = match raw_args.len() {
        0 => unsafe {
            let f: unsafe extern "C" fn() -> i64 = std::mem::transmute(fp);
            f()
        }
        1 => unsafe {
            let f: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute(fp);
            f(raw_args[0])
        }
        2 => unsafe {
            let f: unsafe extern "C" fn(i64, i64) -> i64 = std::mem::transmute(fp);
            f(raw_args[0], raw_args[1])
        }
        3 => unsafe {
            let f: unsafe extern "C" fn(i64, i64, i64) -> i64 = std::mem::transmute(fp);
            f(raw_args[0], raw_args[1], raw_args[2])
        }
        4 => unsafe {
            let f: unsafe extern "C" fn(i64, i64, i64, i64) -> i64 = std::mem::transmute(fp);
            f(raw_args[0], raw_args[1], raw_args[2], raw_args[3])
        }
        5 => unsafe {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(fp);
            f(raw_args[0], raw_args[1], raw_args[2], raw_args[3], raw_args[4])
        }
        6 => unsafe {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(fp);
            f(raw_args[0], raw_args[1], raw_args[2], raw_args[3], raw_args[4], raw_args[5])
        }
        7 => unsafe {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(fp);
            f(raw_args[0], raw_args[1], raw_args[2], raw_args[3], raw_args[4], raw_args[5], raw_args[6])
        }
        8 => unsafe {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(fp);
            f(raw_args[0], raw_args[1], raw_args[2], raw_args[3], raw_args[4], raw_args[5], raw_args[6], raw_args[7])
        }
        9 => unsafe {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(fp);
            f(raw_args[0], raw_args[1], raw_args[2], raw_args[3], raw_args[4], raw_args[5], raw_args[6], raw_args[7], raw_args[8])
        }
        10 => unsafe {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(fp);
            f(raw_args[0], raw_args[1], raw_args[2], raw_args[3], raw_args[4], raw_args[5], raw_args[6], raw_args[7], raw_args[8], raw_args[9])
        }
        11 => unsafe {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(fp);
            f(raw_args[0], raw_args[1], raw_args[2], raw_args[3], raw_args[4], raw_args[5], raw_args[6], raw_args[7], raw_args[8], raw_args[9], raw_args[10])
        }
        12 => unsafe {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(fp);
            f(raw_args[0], raw_args[1], raw_args[2], raw_args[3], raw_args[4], raw_args[5], raw_args[6], raw_args[7], raw_args[8], raw_args[9], raw_args[10], raw_args[11])
        }
        _ => return Err(error::err(ErrorKind::Runtime, span, format!("FFI: too many arguments (max 12, got {})", raw_args.len()))),
    };
    Ok(Value::Int(result))
}

pub fn call_fs(field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
    match field {
        "read" => {
            let path = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "fs.read() requires 1 argument"))?;
            let path = match path {
                Value::Str(s) => s,
                _ => return Err(error::err(ErrorKind::Runtime, span, "fs.read() requires a string path")),
            };
            let content = std::fs::read_to_string(&path)
                .map_err(|e| error::err(ErrorKind::Runtime, span, format!("fs.read: {}", e)))?;
            Ok(Value::Str(content))
        }
        "write" => {
            let mut it = args.into_iter();
            let path = it.next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "fs.write() requires 2 arguments"))?;
            let content = it.next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "fs.write() requires 2 arguments"))?;
            let path = match path {
                Value::Str(s) => s,
                _ => return Err(error::err(ErrorKind::Runtime, span, "fs.write() path must be a string")),
            };
            let content = content.to_string();
            std::fs::write(&path, &content)
                .map_err(|e| error::err(ErrorKind::Runtime, span, format!("fs.write: {}", e)))?;
            Ok(Value::None_)
        }
        "append" => {
            let mut it = args.into_iter();
            let path = it.next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "fs.append() requires 2 arguments"))?;
            let content = it.next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "fs.append() requires 2 arguments"))?;
            let path = match path {
                Value::Str(s) => s,
                _ => return Err(error::err(ErrorKind::Runtime, span, "fs.append() path must be a string")),
            };
            let content = content.to_string();
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true).create(true).open(&path)
                .map_err(|e| error::err(ErrorKind::Runtime, span, format!("fs.append: {}", e)))?;
            write!(file, "{}", content)
                .map_err(|e| error::err(ErrorKind::Runtime, span, format!("fs.append: {}", e)))?;
            Ok(Value::None_)
        }
        "remove" => {
            let path = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "fs.remove() requires 1 argument"))?;
            let path = match path {
                Value::Str(s) => s,
                _ => return Err(error::err(ErrorKind::Runtime, span, "fs.remove() requires a string path")),
            };
            std::fs::remove_file(&path)
                .map_err(|e| error::err(ErrorKind::Runtime, span, format!("fs.remove: {}", e)))?;
            Ok(Value::None_)
        }
        "exists" => {
            let path = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "fs.exists() requires 1 argument"))?;
            let path = match path {
                Value::Str(s) => s,
                _ => return Err(error::err(ErrorKind::Runtime, span, "fs.exists() requires a string path")),
            };
            Ok(Value::Bool(std::path::Path::new(&path).exists()))
        }
        "list" => {
            let dir = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "fs.list() requires 1 argument"))?;
            let dir = match dir {
                Value::Str(s) => s,
                _ => return Err(error::err(ErrorKind::Runtime, span, "fs.list() requires a string path")),
            };
            let entries = std::fs::read_dir(&dir)
                .map_err(|e| error::err(ErrorKind::Runtime, span, format!("fs.list: {}", e)))?;
            let mut items = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|e| error::err(ErrorKind::Runtime, span, format!("fs.list: {}", e)))?;
                items.push(Value::Str(entry.file_name().to_string_lossy().to_string()));
            }
            Ok(Value::List(items))
        }
        "is_dir" => {
            let path = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "fs.is_dir() requires 1 argument"))?;
            let path = match path {
                Value::Str(s) => s,
                _ => return Err(error::err(ErrorKind::Runtime, span, "fs.is_dir() requires a string path")),
            };
            Ok(Value::Bool(std::path::Path::new(&path).is_dir()))
        }
        "is_file" => {
            let path = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "fs.is_file() requires 1 argument"))?;
            let path = match path {
                Value::Str(s) => s,
                _ => return Err(error::err(ErrorKind::Runtime, span, "fs.is_file() requires a string path")),
            };
            Ok(Value::Bool(std::path::Path::new(&path).is_file()))
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown fs function '{}'", field))),
    }
}

pub fn call_sys(field: &str, args: Vec<Value>, span: Span) -> Result<Value> {
    match field {
        "env" => {
            let name = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "sys.env() requires 1 argument"))?;
            let name = match name {
                Value::Str(s) => s,
                _ => return Err(error::err(ErrorKind::Runtime, span, "sys.env() requires a string name")),
            };
            match std::env::var(&name) {
                Ok(val) => Ok(Value::Str(val)),
                Err(_) => Ok(Value::None_),
            }
        }
        "args" => {
            let collected: Vec<Value> = std::env::args().map(Value::Str).collect();
            Ok(Value::List(collected))
        }
        "exit" => {
            let code = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "sys.exit() requires 1 argument"))?;
            let code = match code {
                Value::Int(i) => i as i32,
                _ => return Err(error::err(ErrorKind::Runtime, span, "sys.exit() requires an integer code")),
            };
            std::process::exit(code);
        }
        "cwd" => {
            match std::env::current_dir() {
                Ok(p) => Ok(Value::Str(p.to_string_lossy().to_string())),
                Err(e) => Err(error::err(ErrorKind::Runtime, span, format!("sys.cwd: {}", e))),
            }
        }
        "pid" => {
            Ok(Value::Int(std::process::id() as i64))
        }
        "platform" => {
            Ok(Value::Str(std::env::consts::OS.to_string()))
        }
        "sleep" => {
            let ms = args.into_iter().next()
                .ok_or_else(|| error::err(ErrorKind::Runtime, span, "sys.sleep() requires 1 argument"))?;
            let ms = match ms {
                Value::Int(i) => i,
                _ => return Err(error::err(ErrorKind::Runtime, span, "sys.sleep() requires an integer")),
            };
            std::thread::sleep(std::time::Duration::from_millis(ms as u64));
            Ok(Value::None_)
        }
        _ => Err(error::err(ErrorKind::Runtime, span, format!("Unknown sys function '{}'", field))),
    }
}

pub fn json_to_value(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::None_,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() { Value::Int(i) }
            else if let Some(f) = n.as_f64() { Value::Real(f) }
            else { Value::None_ }
        }
        serde_json::Value::String(s) => Value::Str(s),
        serde_json::Value::Array(arr) => Value::List(arr.into_iter().map(json_to_value).collect()),
        serde_json::Value::Object(obj) => {
            Value::Map(obj.into_iter().map(|(k, v)| (k, json_to_value(v))).collect())
        }
    }
}

pub fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Int(i) => serde_json::Value::Number((*i).into()),
        Value::Real(f) => serde_json::Value::Number(serde_json::Number::from_f64(*f).unwrap_or(serde_json::Number::from(0))),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::List(items) => serde_json::Value::Array(items.iter().map(value_to_json).collect()),
        Value::Tuple(items) => serde_json::Value::Array(items.iter().map(value_to_json).collect()),
        Value::Dict(pairs) => serde_json::Value::Array(pairs.iter().map(|(k, v)| {
            serde_json::Value::Array(vec![value_to_json(k), value_to_json(v)])
        }).collect()),
        Value::Set(items) => serde_json::Value::Array(items.iter().map(value_to_json).collect()),
        Value::Map(m) => serde_json::Value::Object(m.iter().map(|(k, v)| (k.clone(), value_to_json(v))).collect()),
        Value::Struct(_, fields) => serde_json::Value::Object(fields.iter().map(|(k, v)| (k.clone(), value_to_json(v))).collect()),
        Value::None_ => serde_json::Value::Null,
        Value::Null => serde_json::Value::Null,
        Value::Char(c) => serde_json::Value::String(c.to_string()),
        Value::Range(_, _, _) => serde_json::Value::Null,
        Value::Complex(r, i) => serde_json::Value::Array(vec![
            serde_json::Value::Number(serde_json::Number::from_f64(*r).unwrap_or(serde_json::Number::from(0))),
            serde_json::Value::Number(serde_json::Number::from_f64(*i).unwrap_or(serde_json::Number::from(0))),
        ]),
        Value::Instance(_, _) => serde_json::Value::Null,
        Value::Result(_, v) => value_to_json(v),
        Value::Variant(name, fields) => {
            let mut m = serde_json::Map::new();
            m.insert("variant".to_string(), serde_json::Value::String(name.clone()));
            m.insert("fields".to_string(), serde_json::Value::Array(fields.iter().map(value_to_json).collect()));
            serde_json::Value::Object(m)
        }
        Value::Fn(_) => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_math_cos() {
        let r = call_math("cos", vec![Value::Int(0)], Span::new(0, 0)).unwrap();
        assert_eq!(r, Value::Real(1.0));
    }

    #[test]
    fn call_math_sin() {
        let r = call_math("sin", vec![Value::Int(0)], Span::new(0, 0)).unwrap();
        assert_eq!(r, Value::Real(0.0));
    }

    #[test]
    fn call_math_abs_int() {
        let r = call_math("abs", vec![Value::Int(-5)], Span::new(0, 0)).unwrap();
        assert_eq!(r, Value::Int(5));
    }

    #[test]
    fn call_math_abs_real() {
        let r = call_math("abs", vec![Value::Real(-3.5)], Span::new(0, 0)).unwrap();
        assert_eq!(r, Value::Real(3.5));
    }

    #[test]
    fn call_math_rand() {
        let r = call_math("rand", vec![], Span::new(0, 0)).unwrap();
        assert!(matches!(r, Value::Int(_)));
    }

    #[test]
    fn call_time_now() {
        let r = call_time("now", vec![], Span::new(0, 0)).unwrap();
        assert!(matches!(r, Value::Str(_)));
    }

    #[test]
    fn call_json_parse() {
        let r = call_json("parse", vec![Value::Str(r#"{"a":1}"#.into())], Span::new(0, 0)).unwrap();
        assert!(matches!(r, Value::Map(_)));
    }

    #[test]
    fn call_json_stringify() {
        let r = call_json("stringify", vec![Value::Int(42)], Span::new(0, 0)).unwrap();
        assert_eq!(r, Value::Str("42".into()));
    }

    #[test]
    fn call_datetime_now() {
        let r = call_datetime("now", vec![], Span::new(0, 0)).unwrap();
        assert!(matches!(r, Value::Str(_)));
    }

    #[test]
    fn call_path_join() {
        let r = call_path_module("join", vec![Value::Str("a".into()), Value::Str("b".into())], Span::new(0, 0)).unwrap();
        assert_eq!(r, Value::Str("a/b".into()));
    }

    #[test]
    fn call_base64_encode() {
        let r = call_base64("encode", vec![Value::Str("hello".into())], Span::new(0, 0)).unwrap();
        assert_eq!(r, Value::Str("aGVsbG8=".into()));
    }

    #[test]
    fn call_base64_decode() {
        let r = call_base64("decode", vec![Value::Str("aGVsbG8=".into())], Span::new(0, 0)).unwrap();
        assert_eq!(r, Value::Str("hello".into()));
    }

    #[test]
    fn call_regex_match() {
        let r = call_regex("match", vec![Value::Str(r"\d+".into()), Value::Str("abc123".into())], Span::new(0, 0)).unwrap();
        assert_eq!(r, Value::Bool(true));
    }

    #[test]
    fn call_regex_no_match() {
        let r = call_regex("match", vec![Value::Str(r"\d+".into()), Value::Str("abc".into())], Span::new(0, 0)).unwrap();
        assert_eq!(r, Value::Bool(false));
    }

    #[test]
    fn call_regex_find() {
        let r = call_regex("find", vec![Value::Str(r"\d+".into()), Value::Str("a1b2c3".into())], Span::new(0, 0)).unwrap();
        assert!(matches!(r, Value::List(_)));
    }

    #[test]
    fn call_regex_replace() {
        let r = call_regex("replace", vec![Value::Str(r"\d+".into()), Value::Str("a1b2".into()), Value::Str("X".into())], Span::new(0, 0)).unwrap();
        assert_eq!(r, Value::Str("aXbX".into()));
    }

    #[test]
    fn call_regex_split() {
        let r = call_regex("split", vec![Value::Str(r",".into()), Value::Str("a,b,c".into())], Span::new(0, 0)).unwrap();
        assert_eq!(r, Value::List(vec![Value::Str("a".into()), Value::Str("b".into()), Value::Str("c".into())]));
    }

    #[test]
    fn call_fs_unknown_error() {
        let r = call_fs("nonexistent", vec![], Span::new(0, 0));
        assert!(r.is_err());
    }

    #[test]
    fn call_sys_platform() {
        let r = call_sys("platform", vec![], Span::new(0, 0)).unwrap();
        assert!(matches!(r, Value::Str(_)));
    }

    #[test]
    fn call_sys_pid() {
        let r = call_sys("pid", vec![], Span::new(0, 0)).unwrap();
        assert!(matches!(r, Value::Int(_)));
    }

    #[test]
    fn call_sys_unknown_error() {
        let r = call_sys("nonexistent", vec![], Span::new(0, 0));
        assert!(r.is_err());
    }

    #[test]
    fn json_value_roundtrip() {
        let v = Value::Map(HashMap::from([
            ("name".into(), Value::Str("test".into())),
            ("count".into(), Value::Int(42)),
        ]));
        let json = value_to_json(&v);
        let back = json_to_value(json);
        assert_eq!(v, back);
    }
}
