use std::path::PathBuf;
use std::process::{Command, Output};

fn yidi(dir: &PathBuf, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yidi"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("yidi runs")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yidi_cli_output_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

fn has_no_terminal_codes(output: &str) {
    assert!(!output.contains('\u{1b}'), "escape codes in piped output: {:?}", output);
    assert!(!output.contains('\r'), "progress redraws in piped output: {:?}", output);
}

#[test]
fn a_status_goes_to_stderr_as_plain_text_when_piped() {
    let dir = scratch("init");
    let out = yidi(&dir, &["init", "app"]);
    assert!(out.status.success());
    assert_eq!(text(&out.stdout), "");
    let err = text(&out.stderr);
    assert!(err.starts_with("Created project 'app'"), "{}", err);
    has_no_terminal_codes(&err);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_error_is_prefixed_and_plain_when_piped() {
    let dir = scratch("error");
    std::fs::write(dir.join("bad.yk"), "fn main() { x: int = \"no\"; }").unwrap();
    let out = yidi(&dir, &["run", "bad.yk", "--interp"]);
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(err.starts_with("error"), "{}", err);
    has_no_terminal_codes(&err);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_program_prints_only_its_own_output_on_stdout() {
    let dir = scratch("run");
    std::fs::write(dir.join("hello.yk"), "print(\"hello\");").unwrap();
    let out = yidi(&dir, &["run", "hello.yk", "--interp"]);
    assert!(out.status.success());
    assert_eq!(text(&out.stdout), "hello\n");
    assert_eq!(text(&out.stderr), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn version_stays_on_stdout() {
    let dir = scratch("version");
    let out = yidi(&dir, &["--version"]);
    assert_eq!(text(&out.stdout), format!("yidi {}\n", env!("CARGO_PKG_VERSION")));
    let _ = std::fs::remove_dir_all(&dir);
}
