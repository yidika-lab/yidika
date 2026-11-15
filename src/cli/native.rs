use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::codegen::backend::CodegenBackend;
use crate::hardware;
use crate::syntax::ast::Module;

pub(super) struct Source<'a> {
    pub path: &'a Path,
    pub text: &'a str,
    pub module: &'a Module,
}

pub(super) fn executable(sources: &[Source], backend: &dyn CodegenBackend) -> Option<PathBuf> {
    if !crate::codegen::llvm::toolchain_available() {
        return None;
    }
    executable_under(&crate::cache::cache_dir().join("run"), sources, backend)
}

fn executable_under(root: &Path, sources: &[Source], backend: &dyn CodegenBackend) -> Option<PathBuf> {
    let modules: Vec<&Module> = sources.iter().map(|s| s.module).collect();
    if crate::codegen::backend::needs_side_artifacts(&modules) {
        return None;
    }
    cached_executable(root, sources, &modules, backend)
}

fn cached_executable(root: &Path, sources: &[Source], modules: &[&Module], backend: &dyn CodegenBackend) -> Option<PathBuf> {
    let key = format!("{:016x}", program_key(sources));
    let exe = executable_in(&root.join(&key));
    if exe.is_file() {
        return Some(exe);
    }
    let unsupported = root.join(format!("{}.interp", key));
    if unsupported.is_file() {
        return None;
    }
    let staging = root.join(format!("{}.tmp-{}", key, std::process::id()));
    std::fs::create_dir_all(&staging).ok()?;
    let paths: Vec<String> = sources.iter().map(|s| s.path.to_string_lossy().to_string()).collect();
    let built = backend.compile_with_paths(modules, &paths, &staging.join("main"), &hardware::detect());
    if built.is_err() || !executable_in(&staging).is_file() {
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::write(&unsupported, b"");
        return None;
    }
    if std::fs::rename(&staging, root.join(&key)).is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    exe.is_file().then_some(exe)
}

fn executable_in(dir: &Path) -> PathBuf {
    dir.join(format!("main{}", std::env::consts::EXE_SUFFIX))
}

fn compiler_identity() -> Option<(PathBuf, u64, std::time::SystemTime)> {
    let exe = std::env::current_exe().ok()?;
    let meta = std::fs::metadata(&exe).ok()?;
    Some((exe, meta.len(), meta.modified().ok()?))
}

fn program_key(sources: &[Source]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    env!("CARGO_PKG_VERSION").hash(&mut h);
    compiler_identity().hash(&mut h);
    crate::codegen::target::host_triple().hash(&mut h);
    for s in sources {
        s.path.hash(&mut h);
        s.text.hash(&mut h);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::backend::MockBackend;

    fn test_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yidi_native_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn module(src: &str) -> Module {
        crate::syntax::parser::Parser::parse(src).unwrap()
    }

    #[test]
    fn key_changes_when_the_source_changes() {
        let m = module("fn main() {}");
        let a = [Source { path: Path::new("a.yk"), text: "fn main() {}", module: &m }];
        let b = [Source { path: Path::new("a.yk"), text: "fn main() { print(1); }", module: &m }];
        assert_ne!(program_key(&a), program_key(&b));
        assert_eq!(program_key(&a), program_key(&a));
    }

    #[test]
    fn falls_back_when_the_backend_produces_no_executable() {
        let m = module("fn main() { print(\"mock\"); }");
        let text = format!("fn main() {{ print(\"mock {}\"); }}", std::process::id());
        let sources = [Source { path: Path::new("mock.yk"), text: &text, module: &m }];
        let backend = MockBackend::new();
        let root = test_root("fallback");
        assert_eq!(executable_under(&root, &sources, &backend), None);
        assert_eq!(backend.output_paths.lock().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_build_is_not_retried() {
        let m = module("fn main() {}");
        let text = format!("fn main() {{ print(\"retry {}\"); }}", std::process::id());
        let sources = [Source { path: Path::new("retry.yk"), text: &text, module: &m }];
        let backend = MockBackend::new();
        let root = test_root("retry");
        assert_eq!(executable_under(&root, &sources, &backend), None);
        assert_eq!(executable_under(&root, &sources, &backend), None);
        assert_eq!(backend.output_paths.lock().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ffi_programs_stay_on_the_interpreter() {
        let m = module("use { add } from \"c++:math.h\"; fn main() {}");
        let sources = [Source { path: Path::new("ffi.yk"), text: "x", module: &m }];
        let backend = MockBackend::new();
        assert_eq!(executable_under(&test_root("ffi"), &sources, &backend), None);
        assert_eq!(backend.output_paths.lock().unwrap().len(), 0);
    }
}
