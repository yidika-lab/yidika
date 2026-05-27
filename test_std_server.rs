use std::net::TcpListener;
use std::thread;
fn main() {
    let listener = TcpListener::bind("127.0.0.1:3001").unwrap();
    listener.set_nonblocking(true).unwrap();
    println!("Listening on 3001");
    loop {
        match listener.accept() {
            Ok((stream, addr)) => {
                println!("Accepted connection from {}", addr);
                drop(stream);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => println!("Error: {}", e),
        }
    }
}
