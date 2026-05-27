#![allow(non_snake_case, dead_code)]
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::windows::io::{AsRawSocket, FromRawSocket};
type SOCKET = u64;
const INVALID_SOCKET: u64 = !0u64;
const AF_INET: i32 = 2;
const SOCK_STREAM: i32 = 1;
const IPPROTO_TCP: i32 = 6;
const SOL_SOCKET: i32 = 0xFFFF;
const SO_UPDATE_ACCEPT_CONTEXT: i32 = 0x700B;
const WSA_FLAG_OVERLAPPED: u32 = 0x01;

extern "system" {
    fn WSASocketA(af: i32, t: i32, proto: i32, info: *const u8, g: u32, flags: u32) -> SOCKET;
    fn closesocket(s: SOCKET) -> i32;
    fn setsockopt(s: SOCKET, level: i32, name: i32, val: *const u8, len: i32) -> i32;
    fn LoadLibraryA(name: *const u8) -> isize;
    fn GetProcAddress(h: isize, name: *const u8) -> *const std::ffi::c_void;
    fn WSAGetLastError() -> i32;
    fn CreateEventA(attr: *const u8, manual: i32, init: i32, name: *const u8) -> isize;
    fn WaitForSingleObject(h: isize, ms: u32) -> u32;
    fn WSACloseEvent(h: isize) -> i32;
}

type AcceptExFn = unsafe extern "system" fn(
    listen: SOCKET, accept: SOCKET, buf: *mut u8,
    recvlen: u32, localaddrlen: u32, remoteaddrlen: u32,
    bytes: *mut u32, olap: *mut u8,
) -> i32;

fn main() {
    let listener = TcpListener::bind("0.0.0.0:8087").unwrap();
    listener.set_nonblocking(true).unwrap();
    let listen_sock = listener.as_raw_socket();

    let hmod = unsafe { LoadLibraryA("mswsock.dll\0".as_ptr()) };
    let fnptr = unsafe { GetProcAddress(hmod, "AcceptEx\0".as_ptr()) };
    let AcceptEx: AcceptExFn = unsafe { std::mem::transmute(fnptr) };

    // Pre-create socket
    let s = unsafe { WSASocketA(AF_INET, SOCK_STREAM, IPPROTO_TCP, std::ptr::null(), 0, WSA_FLAG_OVERLAPPED) };
    if s == INVALID_SOCKET { eprintln!("WSASocketA failed"); return; }

    // Create event for overlapped
    let evt = unsafe { CreateEventA(std::ptr::null(), 1, 0, std::ptr::null()) };
    if evt == 0 { eprintln!("CreateEvent failed"); return; }

    #[repr(C)]
    struct OVERLAPPED { Internal: usize, InternalHigh: usize, Offset: u32, OffsetHigh: u32, hEvent: isize }
    let mut olap = OVERLAPPED { Internal: 0, InternalHigh: 0, Offset: 0, OffsetHigh: 0, hEvent: evt };
    let mut buf = [0u8; 256];
    let mut bytes: u32 = 0;

    let rc = unsafe {
        AcceptEx(listen_sock, s, buf.as_mut_ptr(), 0, 32, 32, &mut bytes, &mut olap as *mut _ as *mut u8)
    };
    if rc == 0 && unsafe { WSAGetLastError() } != 997 {
        eprintln!("AcceptEx failed: {}", unsafe { WSAGetLastError() });
        return;
    }

    listener.set_nonblocking(false).unwrap();
    eprintln!("Waiting for connection on 8087...");

    let wait = unsafe { WaitForSingleObject(evt, 5000) };
    if wait != 0 { eprintln!("Wait failed: {}", wait); return; }

    unsafe {
        let so_rc = setsockopt(s, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT,
            &listen_sock as *const _ as *const u8, std::mem::size_of::<SOCKET>() as i32);
        if so_rc != 0 { eprintln!("setsockopt update failed: {}", WSAGetLastError()); return; }
    }

    let mut stream = unsafe { std::net::TcpStream::from_raw_socket(s) };

    // Try to read
    let mut readbuf = [0u8; 4096];
    match stream.read(&mut readbuf) {
        Ok(n) => eprintln!("Read {} bytes: {:?}", n, &readbuf[..n.min(80)]),
        Err(e) => { eprintln!("Read error: {}", e); }
    }

    // Write response
    let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nAcceptEx OK!\n";
    match stream.write_all(resp) {
        Ok(_) => eprintln!("Write OK"),
        Err(e) => eprintln!("Write error: {}", e),
    }
    drop(stream);
    unsafe { WSACloseEvent(evt) };
    eprintln!("Done");
}
