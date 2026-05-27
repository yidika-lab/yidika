use std::io::{Read,Write};
fn main() {
    let listener = std::net::TcpListener::bind("0.0.0.0:8083").unwrap();
    loop {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap();
        let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\nHello Rust!\n";
        stream.write_all(resp).unwrap();
    }
}
