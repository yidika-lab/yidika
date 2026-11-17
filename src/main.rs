fn main() {
    let cmd = yidi::cli::parse_args();
    let reports_status = cmd.reports_status();
    match yidi::cli::execute(cmd) {
        Ok(msg) if msg.is_empty() => {}
        Ok(msg) if reports_status => anstream::eprintln!("{}", yidi::ui::status_line(&msg)),
        Ok(msg) => anstream::println!("{}", msg),
        Err(e) => {
            anstream::eprintln!("{}", yidi::ui::error_line(&e));
            std::process::exit(1);
        }
    }
}
