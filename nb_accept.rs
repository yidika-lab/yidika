#![allow(non_snake_case, dead_code)]
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::windows::io::{AsRawSocket, FromRawSocket};

type SOCKET = u64;
type c_int = i32;
const INVALID_SOCKET: u64 = !0u64;

extern "system" {
    fn accept(s: SOCKET, addr: *mut u8, addrlen: *mut c_int) -> SOCKET;
    fn ntohs(netshort: u16) -> u16;
    fn select(nfds: c_int, readfds: *mut u8, writefds: *mut u8, exceptfds: *mut u8, timeout: *mut u8) -> c_int;
    fn FD_SET(fd: SOCKET, set: *mut u8);
    fn FD_ZERO(set: *mut u8);
    fn FD_ISSET(fd: SOCKET, set: *mut u8) -> c_int;
}

fn main() {
    let listener = TcpListener::bind("0.0.0.0:8086").unwrap();
    listener.set_nonblocking(true).unwrap();
    let sock = listener.as_raw_socket();

    loop {
        let mut readfds = [0u8; 128]; // fd_set structure
        unsafe { FD_ZERO(readfds.as_mut_ptr()) };
        unsafe { FD_SET(sock, readfds.as_mut_ptr()) };

        let mut tv = [0u8; 8]; // timeval: 2 sec timeout
        // tv_sec = 2, tv_usec = 0
        tv[0] = 2;

        let ret = unsafe { select(0, readfds.as_mut_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), tv.as_mut_ptr()) };
        if ret <= 0 { continue; }

        let mut storage: [u8; 128] = unsafe { std::mem::zeroed() };
        let mut addrlen: c_int = 128;
        let result = unsafe { accept(sock, storage.as_mut_ptr(), &mut addrlen) };
        if result == INVALID_SOCKET { continue; }

        let mut stream = unsafe { std::net::TcpStream::from_raw_socket(result) };
        let _ = stream.set_nodelay(true);
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\nHello Rust!\n";
        let _ = stream.write_all(resp);
    }
}
