use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::sync::Arc;
use crate::hardware::HardwareInfo;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::netlib::{
    ServerInstance, Handler, sub_interpreter_pool, base_interp,
    compile_literal_handler, compile_fn_handler,
    YkResponse, YkRequest,
    parse_request_line, extract_header_value, write_status_line_async
};
use crate::interpret::Value;

pub async fn run_server(server: ServerInstance, addr: String) {
    let server = Arc::new(server);
    let listener = TcpListener::bind(addr.as_str()).await.unwrap();
    
    println!("Server listening on: {}", addr);
    
    // Warm-up for Windows
    if let Ok(warmup) = TcpStream::connect(addr.as_str()).await {
        drop(warmup);
    }
    
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let server = server.clone();
                // Spawn a lightweight task (not an OS thread!)
                tokio::spawn(async move {
                    handle_connection_fast(stream, &server).await;
                });
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }
}

async fn handle_connection_fast(mut stream: TcpStream, server: &ServerInstance) {
    let pool = sub_interpreter_pool();
    let hw = HardwareInfo::cached();
    let buf_size = hw.map(|h| {
        h.cpu.cache_l1.map(|l1| (l1 as usize).max(4096)).unwrap_or(8192)
    }).unwrap_or(8192);

    let mut read_buf = Vec::with_capacity(buf_size);

    loop {
        read_buf.clear();
        loop {
            let mut chunk = [0u8; 4096];
            match stream.read(&mut chunk).await {
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
                let _ = write_status_line_async(&mut stream, "400 Bad Request", "text/plain", 9, false).await;
                let _ = stream.write_all(b"Bad Request").await;
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
                if stream.read_exact(&mut extra).await.is_err() { return; }
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
                        // Try JIT path
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
                        // Try JIT compilation
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
                                // Always inject req
                                interp.frames.last_mut().unwrap().insert("req".into(), req_val);
                                for param in fndef.params.iter() {
                                    if param.name != "req" {
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
                let _ = write_status_line_async(&mut stream, "404 Not Found", "text/plain", 9, keep_alive).await;
                let _ = stream.write_all(b"Not Found").await;
                if !keep_alive { return; } else { continue; }
            }
        };
        let _ = write_status_line_async(&mut stream, "200 OK", content_type, response_body.len(), keep_alive).await;
        let _ = stream.write_all(response_body.as_bytes()).await;
        let _ = stream.flush().await;

        if !keep_alive {
            return;
        }
    }
}

pub(crate) fn start_hyper_server(server: ServerInstance, addr: String) {
    // Configuration Tokio ultra-performante !
    let rt = tokio::runtime::Builder::new_multi_thread()
        // Commence avec 1 thread, auto-scale jusqu'au nombre de cœurs
        .worker_threads(num_cpus::get())
        // Optimisé pour throughput maximum
        .thread_stack_size(2 * 1024 * 1024) // 2MB stack par thread
        .enable_all()
        .build()
        .unwrap();
    
    rt.block_on(async {
        run_server(server, addr).await;
    });
}