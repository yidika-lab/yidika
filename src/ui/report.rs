use owo_colors::OwoColorize;
use std::time::Duration;

pub fn elapsed(duration: Duration) -> String {
    let ms = duration.as_millis();
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.2}s", duration.as_secs_f64())
    }
}

pub fn status_line(message: &str) -> String {
    match message.split_once(' ') {
        Some((verb, rest)) => format!("{} {}", verb.green().bold(), rest),
        None => message.green().bold().to_string(),
    }
}

pub fn error_line(message: &str) -> String {
    if message.starts_with('\u{1b}') || message.starts_with("error") {
        return message.to_string();
    }
    format!("{}: {}", "error".red().bold(), message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> String {
        anstream::adapter::strip_str(text).to_string()
    }

    #[test]
    fn short_durations_are_in_milliseconds_and_long_ones_in_seconds() {
        assert_eq!(elapsed(Duration::from_millis(42)), "42ms");
        assert_eq!(elapsed(Duration::from_millis(1234)), "1.23s");
    }

    #[test]
    fn a_status_line_colors_only_its_verb() {
        let line = status_line("Compiled fib.yk in 1.23s");
        assert_eq!(plain(&line), "Compiled fib.yk in 1.23s");
        assert!(line.starts_with('\u{1b}'));
        assert!(line.ends_with(" fib.yk in 1.23s"));
    }

    #[test]
    fn a_plain_error_gets_a_red_error_prefix() {
        assert_eq!(plain(&error_line("cannot read fib.yk")), "error: cannot read fib.yk");
    }

    #[test]
    fn an_error_that_already_has_its_prefix_is_kept() {
        let diagnostic = "\u{1b}[1;31merror[E0001]\u{1b}[0m: bad type";
        assert_eq!(error_line(diagnostic), diagnostic);
        assert_eq!(error_line("error: x"), "error: x");
    }
}
