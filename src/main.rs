fn main() {
    let cmd = yidi::cli::parse_args();
    match yidi::cli::execute(cmd) {
        Ok(msg) => { if !msg.is_empty() { println!("{}", msg); } }
        Err(e) => { eprintln!("{}", e); std::process::exit(1); }
    }
}
