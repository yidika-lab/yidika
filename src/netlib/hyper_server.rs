use tokio::net::{TcpListener, TcpStream};
use tokio::io::AsyncReadExt;
use std::sync::Arc;
use crate::hardware::HardwareInfo;

use crate::netlib::{
    ServerInstance, sub_interpreter_pool,
    parse_request_line, extract_header_value, write_response_async,
    extract_all_headers, parse_query_params, parse_cookies,
    cors_headers, ProcessedResponse, try_serve_static, execute_handler, run_logger,
    MAX_HEADER_BYTES, MAX_BODY_BYTES, find_head_end, decode_head, decode_bytes, content_length
};

pub async fn run_server(server: ServerInstance, addr: String) {
    let server = Arc::new(server);
    let listener = match TcpListener::bind(addr.as_str()).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("Cannot bind {}: {}", addr, e);
            return;
        }
    };
    
    println!("Server listening on: {}", addr);
    
    // Warm-up for Windows
    if let Ok(warmup) = TcpStream::connect(addr.as_str()).await {
        drop(warmup);
    }
    
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let server = server.clone();
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
        let mut headers_too_large = false;
        loop {
            let mut chunk = [0u8; 4096];
            match stream.read(&mut chunk).await {
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
            let _ = write_response_async(&mut stream, &ProcessedResponse { status: "431 Request Header Fields Too Large".to_string(), body: String::new(), content_type: "text/plain".to_string(), headers: vec![] }, false, &cors).await;
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
                let _ = write_response_async(&mut stream, &ProcessedResponse { status: "400 Bad Request".to_string(), body: "Bad Request".to_string(), content_type: "text/plain".to_string(), headers: vec![] }, false, &cors).await;
                return;
            }
        };

        let body = match content_length(raw) {
            Some(cl) if cl > MAX_BODY_BYTES => {
                let _ = write_response_async(&mut stream, &ProcessedResponse { status: "413 Content Too Large".to_string(), body: String::new(), content_type: "text/plain".to_string(), headers: vec![] }, false, &cors).await;
                return;
            }
            Some(cl) => {
                let body_start = head_end + 4;
                let already = read_buf.len().saturating_sub(body_start);
                if cl > already {
                    let mut extra = vec![0u8; cl - already];
                    if stream.read_exact(&mut extra).await.is_err() { return; }
                    let mut b = read_buf[body_start..].to_vec();
                    b.extend_from_slice(&extra);
                    decode_bytes(&b)
                } else {
                    decode_bytes(&read_buf[body_start..body_start + cl])
                }
            }
            None => String::new(),
        };

        // Handle OPTIONS preflight
        if method_raw == "OPTIONS" {
            let _ = write_response_async(&mut stream, &ProcessedResponse { status: "204 No Content".to_string(), body: String::new(), content_type: "text/plain".to_string(), headers: vec![] }, keep_alive, &cors).await;
            if !keep_alive { return; } else { continue; }
        };

        // Parse all headers, query params, and cookies
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
                                body: "Not Found".to_string(),
                                content_type: "text/plain".to_string(),
                                headers: Vec::new(),
                            }
                        }
                    } else {
                        ProcessedResponse {
                            status: "404 Not Found".to_string(),
                            body: "Not Found".to_string(),
                            content_type: "text/plain".to_string(),
                            headers: Vec::new(),
                        }
                    }
                } else {
                    ProcessedResponse {
                        status: "404 Not Found".to_string(),
                        body: "Not Found".to_string(),
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
                &response.body,
                pool
            );
        }
        let _ = write_response_async(&mut stream, &response, keep_alive, &cors).await;

        if !keep_alive {
            return;
        }
    }
}

pub(crate) fn start_hyper_server(server: ServerInstance, addr: String) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(num_cpus::get())
        .thread_stack_size(2 * 1024 * 1024)
        .enable_all()
        .build()
        .unwrap();
    
    rt.block_on(async {
        run_server(server, addr).await;
    });
}
