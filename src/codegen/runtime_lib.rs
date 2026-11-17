use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = if cfg!(windows) { "yk_runtime.lib" } else { "libyk_runtime.a" };

const LINUX_LIBS: [&str; 8] = ["-Wl,--as-needed", "-lgcc_s", "-lutil", "-lrt", "-lpthread", "-lm", "-ldl", "-lc"];
const MACOS_LIBS: [&str; 3] = ["-lSystem", "-lc", "-lm"];
const WINDOWS_LIBS: [&str; 6] = ["kernel32.lib", "ntdll.lib", "userenv.lib", "ws2_32.lib", "dbghelp.lib", "synchronization.lib"];

pub fn link_args() -> Vec<String> {
    locate().map(|lib| args_for(&lib, std::env::consts::OS)).unwrap_or_default()
}

pub fn locate() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    search_dirs(&exe).into_iter().map(|dir| dir.join(FILE_NAME)).find(|path| path.is_file())
}

fn search_dirs(exe: &Path) -> Vec<PathBuf> {
    let beside = exe.parent().map(Path::to_path_buf);
    let profile = exe.ancestors().skip(2).find(|dir| is_cargo_profile(dir)).map(Path::to_path_buf);
    beside.into_iter().chain(profile).collect()
}

fn is_cargo_profile(dir: &Path) -> bool {
    matches!(dir.file_name().and_then(|n| n.to_str()), Some("debug" | "release"))
}

fn args_for(lib: &Path, os: &str) -> Vec<String> {
    let system: &[&str] = match os {
        "linux" => &LINUX_LIBS,
        "macos" => &MACOS_LIBS,
        "windows" => &WINDOWS_LIBS,
        _ => &[],
    };
    std::iter::once(lib.to_string_lossy().to_string()).chain(system.iter().map(|s| s.to_string())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_is_looked_up_beside_yidi_then_in_the_cargo_profile() {
        let installed = Path::new("/home/u/.yidi/bin/yidi");
        assert_eq!(search_dirs(installed), vec![PathBuf::from("/home/u/.yidi/bin")]);
        let test = Path::new("/p/target/debug/build/yidi/0123/out/yidi-0123");
        assert_eq!(search_dirs(test), vec![PathBuf::from("/p/target/debug/build/yidi/0123/out"), PathBuf::from("/p/target/debug")]);
        let built = Path::new("/p/target/release/yidi");
        assert_eq!(search_dirs(built), vec![PathBuf::from("/p/target/release")]);
    }

    #[test]
    fn the_library_comes_before_the_system_libraries_it_needs() {
        let lib = Path::new("rt.a");
        assert_eq!(args_for(lib, "macos"), vec!["rt.a", "-lSystem", "-lc", "-lm"]);
        assert_eq!(args_for(lib, "linux")[..2], ["rt.a".to_string(), "-Wl,--as-needed".to_string()]);
        assert!(args_for(lib, "windows").contains(&"ws2_32.lib".to_string()));
        assert_eq!(args_for(lib, "plan9"), vec!["rt.a"]);
    }

    #[test]
    fn a_native_program_can_call_the_runtime_library() {
        if !crate::codegen::llvm::toolchain_available() {
            return;
        }
        assert!(locate().is_some(), "{} is missing: run `cargo build -p yk_runtime` first", FILE_NAME);
        let dir = std::env::temp_dir().join(format!("yidi_runtime_lib_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("abi");
        let ir = "declare i64 @yk_runtime_abi()\ndefine i32 @main() {\n  %v = call i64 @yk_runtime_abi()\n  %r = trunc i64 %v to i32\n  ret i32 %r\n}\n";
        crate::codegen::llvm::compile_to_exe(ir, &exe, &crate::hardware::detect()).unwrap();
        let status = std::process::Command::new(exe.with_extension(std::env::consts::EXE_EXTENSION)).status().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(status.code(), Some(1));
    }
}
