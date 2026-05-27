#![allow(non_snake_case, non_camel_case_types, dead_code)]
use std::io::{Read,Write};
use std::net::TcpListener;
use std::os::windows::io::{AsRawSocket, FromRawSocket};

type SOCKET = u64;
type c_int = i32;
const INVALID_SOCKET: u64 = !0u64;
const AF_INET: c_int = 2;
const SOCKADDR_IN_SIZE: usize = 16;

extern "system" {
    fn accept(s: SOCKET, addr: *mut u8, addrlen: *mut c_int) -> SOCKET;
    fn ntohs(netshort: u16) -> u16;
}

fn raw_accept(listener: &TcpListener) -> std::io::Result<(std::net::TcpStream, std::net::SocketAddr)> {
    let sock = listener.as_raw_socket();
    let mut storage: [u8; 128] = unsafe { std::mem::zeroed() };
    let mut addrlen: c_int = 128 as c_int;
    
    let result = unsafe { accept(sock, storage.as_mut_ptr(), &mut addrlen) };
    if result == INVALID_SOCKET {
        return Err(std::io::Error::last_os_error());
    }
    
    let stream = unsafe { std::net::TcpStream::from_raw_socket(result) };
    let port = unsafe { ntohs(u16::from_ne_bytes([storage[2], storage[3]])) };
    let ip_bytes = [storage[4], storage[5], storage[6], storage[7]];
    let addr = std::net::SocketAddr::new(
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3])),
        port,
    );
    Ok((stream, addr))
}

fn main() {
    let listener = TcpListener::bind("0.0.0.0:8085").unwrap();
    loop {
        let (mut stream, _) = raw_accept(&listener).unwrap();
        let mut buf = [0u8; 4096];
        let _n = stream.read(&mut buf).unwrap();
        let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\nHello Rust!\n";
        stream.write_all(resp).unwrap();
    }
}
