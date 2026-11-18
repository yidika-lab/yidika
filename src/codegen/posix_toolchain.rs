use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;

const COMPILERS: &[&str] = &["clang++", "clang++-21", "clang++-20", "clang++-19", "clang++-18", "clang++-17"];
const RUNTIME_FLAGS: &[&str] = &["-x", "c++", "-std=c++17", "-O2", "-w", "-fno-exceptions", "-fno-rtti", "-ffunction-sections", "-fdata-sections"];

pub fn compile_to_exe(llvm_ir: &str, runtime_c: &str, output_path: &Path, extra_libs: &[String]) -> Result<()> {
    let compiler = find_compiler()?;
    let ll_path = output_path.with_extension("ll");
    write_file(&ll_path, llvm_ir)?;
    let runtime_obj = cached_runtime_object(&compiler, runtime_c)?;
    let mut cmd = Command::new(&compiler);
    cmd.args(["-O3", "-Wno-override-module"])
        .args(crate::codegen::target::CODE_LAYOUT_FLAGS)
        .arg(native_cpu_flag())
        .args(fast_linker_flag(&search_path()))
        .arg(dead_code_flag())
        .args(strip_debug_flag())
        .args(sanitizer_flags())
        .arg(&ll_path)
        .arg(&runtime_obj)
        .args(extra_libs)
        .arg("-o")
        .arg(output_path)
        .args(["-pthread", "-lm"]);
    run(cmd, "link")?;
    if std::env::var_os("YIDI_KEEP_IR").is_none() {
        let _ = std::fs::remove_file(&ll_path);
    }
    Ok(())
}

const SANITIZERS: [&str; 2] = ["address", "undefined"];

fn sanitizer_flags() -> Vec<String> {
    sanitizer_flags_for(std::env::var("YIDI_SANITIZE").ok().as_deref())
}

fn sanitizer_flags_for(requested: Option<&str>) -> Vec<String> {
    let kinds: Vec<&str> = requested.unwrap_or("").split(',').map(str::trim).filter(|k| SANITIZERS.contains(k)).collect();
    if kinds.is_empty() {
        return Vec::new();
    }
    vec!["-g".into(), format!("-fsanitize={}", kinds.join(",")), "-fno-sanitize-recover=all".into()]
}

const FAST_LINKERS: &[(&str, &str)] = &[("mold", "-fuse-ld=mold"), ("ld.lld", "-fuse-ld=lld")];

fn dead_code_flag() -> &'static str {
    if cfg!(target_os = "macos") { "-Wl,-dead_strip" } else { "-Wl,--gc-sections" }
}

fn strip_debug_flag() -> Option<&'static str> {
    if !sanitizer_flags().is_empty() {
        return None;
    }
    Some(if cfg!(target_os = "macos") { "-Wl,-S" } else { "-Wl,--strip-debug" })
}

fn fast_linker_flag(path: &std::ffi::OsStr) -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        return None;
    }
    FAST_LINKERS.iter().find(|(name, _)| find_in(path, name).is_some()).map(|(_, flag)| *flag)
}

fn search_path() -> std::ffi::OsString {
    std::env::var_os("PATH").unwrap_or_default()
}

fn find_in(path: &std::ffi::OsStr, name: &str) -> Option<PathBuf> {
    std::env::split_paths(path).map(|dir| dir.join(name)).find(|p| p.is_file())
}

fn native_cpu_flag() -> &'static str {
    if cfg!(target_arch = "aarch64") { "-mcpu=native" } else { "-march=native" }
}

pub fn find_compiler() -> Result<PathBuf> {
    let path = search_path();
    COMPILERS
        .iter()
        .find_map(|name| find_in(&path, name))
        .ok_or_else(|| fail(ErrorKind::Internal, missing_compiler_help()))
}

fn missing_compiler_help() -> String {
    let install = if cfg!(target_os = "macos") { "xcode-select --install" } else { "sudo apt install clang (or your distribution's clang package)" };
    format!("clang++ not found on PATH. Install it with: {}", install)
}

fn cached_runtime_object(compiler: &Path, runtime_c: &str) -> Result<PathBuf> {
    let dir = crate::cache::cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| fail(ErrorKind::Io, format!("cannot create {}: {}", dir.display(), e)))?;
    let obj = dir.join(format!("yk_rt-{:016x}.o", runtime_key(compiler, runtime_c)));
    if obj.is_file() {
        return Ok(obj);
    }
    let src = dir.join(format!("yk_rt-{}.c", std::process::id()));
    let tmp = dir.join(format!("yk_rt-{}.o.tmp", std::process::id()));
    write_file(&src, runtime_c)?;
    let mut cmd = Command::new(compiler);
    cmd.args(RUNTIME_FLAGS).arg(native_cpu_flag()).arg("-c").arg(&src).arg("-o").arg(&tmp);
    let built = run(cmd, "runtime compile");
    let _ = std::fs::remove_file(&src);
    built?;
    std::fs::rename(&tmp, &obj).map_err(|e| fail(ErrorKind::Io, format!("cannot store {}: {}", obj.display(), e)))?;
    Ok(obj)
}

fn runtime_key(compiler: &Path, runtime_c: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    runtime_c.hash(&mut h);
    compiler.hash(&mut h);
    RUNTIME_FLAGS.hash(&mut h);
    native_cpu_flag().hash(&mut h);
    h.finish()
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    std::fs::write(path, content).map_err(|e| fail(ErrorKind::Io, format!("Failed to write {}: {}", path.display(), e)))
}

fn run(mut cmd: Command, step: &str) -> Result<()> {
    let out = cmd.output().map_err(|e| fail(ErrorKind::Io, format!("{} failed to start: {}", step, e)))?;
    if out.status.success() {
        return Ok(());
    }
    Err(fail(ErrorKind::Internal, format!("{} failed:\n{}", step, String::from_utf8_lossy(&out.stderr))))
}

fn fail(kind: ErrorKind, msg: String) -> error::YkError {
    error::err(kind, Span::new(0, 0), msg)
}

#[cfg(test)]
mod tests {

    #[test]
    fn sanitizers_are_parsed_from_a_comma_separated_list() {
        assert_eq!(sanitizer_flags_for(Some("address,undefined")), vec!["-g", "-fsanitize=address,undefined", "-fno-sanitize-recover=all"]);
        assert_eq!(sanitizer_flags_for(Some("address")), vec!["-g", "-fsanitize=address", "-fno-sanitize-recover=all"]);
    }

    #[test]
    fn unknown_or_missing_sanitizers_add_nothing() {
        assert!(sanitizer_flags_for(None).is_empty());
        assert!(sanitizer_flags_for(Some("thread;rm -rf /")).is_empty());
    }

    use super::*;

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn prefers_a_fast_linker_found_on_the_path() {
        let dir = std::env::temp_dir().join(format!("yidi_linker_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(fast_linker_flag(dir.as_os_str()), None);
        std::fs::write(dir.join("ld.lld"), b"").unwrap();
        assert_eq!(fast_linker_flag(dir.as_os_str()), Some("-fuse-ld=lld"));
        std::fs::write(dir.join("mold"), b"").unwrap();
        assert_eq!(fast_linker_flag(dir.as_os_str()), Some("-fuse-ld=mold"));
        let _ = std::fs::remove_dir_all(&dir);
    }


    #[test]
    fn runtime_key_changes_with_the_source() {
        let cc = Path::new("/usr/bin/clang++");
        assert_ne!(runtime_key(cc, "int a;"), runtime_key(cc, "int b;"));
        assert_eq!(runtime_key(cc, "int a;"), runtime_key(cc, "int a;"));
    }

    #[test]
    fn runtime_key_changes_with_the_compiler() {
        assert_ne!(runtime_key(Path::new("/a/clang++"), "x"), runtime_key(Path::new("/b/clang++"), "x"));
    }
}
