use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::syntax::ast::{self, Module};
use crate::syntax::parser::Parser;

pub type FileId = usize;

#[derive(Clone)]
pub struct LoadedModule {
    pub id: FileId,
    pub path: PathBuf,
    pub module: Module,
    pub source: String,
}

pub struct ModuleLoader {
    files: Vec<LoadedModule>,
    loaded: HashMap<PathBuf, FileId>,
    visiting: Vec<PathBuf>,
    root: PathBuf,
}

impl ModuleLoader {
    pub fn new(root: PathBuf) -> Self {
        Self { files: Vec::new(), loaded: HashMap::new(), visiting: Vec::new(), root }
    }

    pub fn load(&mut self, path: &Path) -> Result<FileId> {
        let canonical = self.canonicalize(path)?;

        if self.visiting.contains(&canonical) {
            let cycle: Vec<String> = self.visiting.iter()
                .chain(std::iter::once(&canonical))
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            return Err(error::err(
                ErrorKind::Internal, Span::new(0, 0),
                format!("Circular import detected: {}", cycle.join(" → ")),
            ));
        }

        if let Some(&id) = self.loaded.get(&canonical) {
            return Ok(id);
        }

        let source = fs::read_to_string(&canonical)
            .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                format!("Failed to read '{}': {}", canonical.display(), e)))?;

        ast::reset_ids();
        let stem = canonical.file_stem().and_then(|s| s.to_str()).unwrap_or("yk");
        let module = Parser::parse_with_name(&source, stem)
            .map_err(|e| e.with_source(&source).with_file(&canonical.to_string_lossy()))?;

        let id = self.files.len();
        self.files.push(LoadedModule {
            id,
            path: canonical.clone(),
            module,
            source,
        });
        self.loaded.insert(canonical.clone(), id);
        self.visiting.push(canonical.clone());

        let imports = self.files[id].module.imports.clone();
        for import in &imports {
            self.resolve_import(import, &canonical)?;
        }

        self.visiting.pop();
        Ok(id)
    }

    fn resolve_import(&mut self, import: &ast::Import, current: &Path) -> Result<()> {
        if let Some(lang) = &import.lang {
            match lang.as_str() {
                "rust" => {
                    let ffi_dir = self.root.join("lib").join("ffi");
                    fs::create_dir_all(&ffi_dir)
                        .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                            format!("Failed to create ffi dir: {}", e)))?;
                    self.compile_rust_ffi(current, &import.source, &ffi_dir)?;
                }
                "c++" => {
                    let ffi_dir = self.root.join("lib").join("ffi");
                    fs::create_dir_all(&ffi_dir)
                        .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                            format!("Failed to create ffi dir: {}", e)))?;
                    self.compile_cpp_ffi(current, &import.source, &ffi_dir)?;
                }
                _ => {}
            }
            return Ok(());
        }

        // Builtin standard library modules (no file to load)
        if matches!(import.source.as_str(), "std" | "io" | "math" | "time" | "json" | "datetime" | "path" | "base64" | "regex" | "net") {
            return Ok(());
        }

        let import_path = PathBuf::from(&import.source);

        if import.source.starts_with(".") {
            let base = current.parent().unwrap_or(Path::new("."));
            let resolved = base.join(&import_path);
            self.load(&resolved)?;
            return Ok(());
        }

        Err(error::err(
            ErrorKind::Internal, Span::new(0, 0),
            format!("Unsupported import path: '{}'", import.source),
        ))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };

        if path.exists() {
            return Ok(fs::canonicalize(&path)
                .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0), e.to_string()))?);
        }

        let with_ext = path.with_extension("yk");
        if with_ext.exists() {
            return Ok(fs::canonicalize(&with_ext)
                .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0), e.to_string()))?);
        }

        Err(error::err(
            ErrorKind::Io, Span::new(0, 0),
            format!("File not found: '{}'", path.display()),
        ))
    }

    fn compile_rust_ffi(&self, current: &Path, source: &str, out_dir: &Path) -> Result<()> {
        let base = current.parent().unwrap_or(Path::new("."));
        let src_path = base.join(source);

        let dll_name = format!("yk_ffi_{}.dll", source.replace('/', "_").replace('.', ""));
        let dll_path = out_dir.join(&dll_name);

        if dll_path.exists() {
            return Ok(());
        }

        if src_path.is_dir() {
            // Cargo project — run cargo build
            let output = std::process::Command::new("cargo")
                .args(["build", "--release", "--message-format=json"])
                .current_dir(&src_path)
                .output()
                .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                    format!("Failed to run cargo for FFI: {}", e)))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    format!("cargo build failed for FFI: {}", stderr)));
            }
        } else {
            // Single .rs file — compile with rustc
            let output = std::process::Command::new("rustc")
                .args(["--crate-type", "cdylib", "-o"])
                .arg(&dll_path)
                .arg(&src_path)
                .output()
                .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                    format!("Failed to run rustc for FFI: {}", e)))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    format!("rustc failed for FFI: {}", stderr)));
            }
        }

        Ok(())
    }

    fn detect_cxx_compiler() -> Option<String> {
        // Try clang++ first
        if let Ok(out) = std::process::Command::new("clang++").arg("--version").output() {
            if out.status.success() { return Some("clang++".into()); }
        }
        // Try MSVC cl.exe
        if let Ok(out) = std::process::Command::new("cl.exe").output() {
            if out.status.success() { return Some("cl.exe".into()); }
        }
        None
    }

    fn compile_cpp_ffi(&self, current: &Path, source: &str, out_dir: &Path) -> Result<()> {
        let base = current.parent().unwrap_or(Path::new("."));
        let src_path = base.join(source);

        if !src_path.exists() {
            // Source not found; FFI will fail at runtime if the DLL is also missing
            return Ok(());
        }

        let dll_name = format!("yk_ffi_{}.dll", source.replace('/', "_").replace('.', ""));
        let dll_path = out_dir.join(&dll_name);

        if dll_path.exists() {
            return Ok(());
        }

        let compiler = match Self::detect_cxx_compiler() {
            Some(c) => c,
            None => {
                // No C++ compiler; FFI will fail at runtime
                return Ok(());
            }
        };

        let output = if compiler == "cl.exe" {
            std::process::Command::new("cl.exe")
                .args(["/LD", "/nologo", "/Fo:"])
                .arg(out_dir.join("yk_ffi_obj.obj"))
                .arg(&src_path)
                .arg("/link")
                .arg(&format!("/OUT:{}", dll_path.to_string_lossy()))
                .output()
                .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                    format!("Failed to run cl.exe for C++ FFI: {}", e)))?
        } else {
            let lib_name = format!("yk_ffi_{}.lib", source.replace('/', "_").replace('.', ""));
            let lib_path = out_dir.join(&lib_name);
            std::process::Command::new(&compiler)
                .args(["-shared", "-fPIC", "-o"])
                .arg(&dll_path)
                .arg(&src_path)
                .arg("-Xlinker")
                .arg(&format!("/IMPLIB:{}", lib_path.to_string_lossy()))
                .output()
                .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                    format!("Failed to run clang++ for C++ FFI: {}", e)))?
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                format!("C++ compiler failed for FFI: {}", stderr)));
        }

        // Clean up cl.exe's side-effect files
        if compiler == "cl.exe" {
            let _ = std::fs::remove_file(out_dir.join("yk_ffi_obj.obj"));
            let _ = std::fs::remove_file(out_dir.join("yk_ffi_obj.lib"));
            let _ = std::fs::remove_file(out_dir.join("yk_ffi_obj.exp"));
        }

        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = &LoadedModule> {
        self.files.iter()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::io::Write;

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("yk_test_{}", n));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = fs::File::create(&path).unwrap();
        write!(file, "{}", content).unwrap();
    }

    #[test]
    fn load_single_file() {
        let dir = TempDir::new();
        write_file(dir.path(), "main.yk", "fn main() { x: int = 1; }");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());
        let id = loader.load(&Path::new("main.yk")).unwrap();
        assert_eq!(loader.file_count(), 1);
        assert_eq!(id, 0);
    }

    #[test]
    fn load_with_import() {
        let dir = TempDir::new();
        write_file(dir.path(), "utils.yk", "fn greet(name: str) -> str { return \"Hello \" + name; }");
        write_file(dir.path(), "main.yk", "use {greet} from \"./utils.yk\"; fn main() { msg: str = greet(\"World\"); }");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());
        loader.load(&Path::new("main.yk")).unwrap();
        assert_eq!(loader.file_count(), 2);
    }

    #[test]
    fn load_with_transitive_import() {
        let dir = TempDir::new();
        write_file(dir.path(), "lib/math.yk", "fn add(a: int, b: int) -> int { return a + b; }");
        write_file(dir.path(), "lib/utils.yk", "use {add} from \"./math.yk\"; fn double(x: int) -> int { return add(x, x); }");
        write_file(dir.path(), "main.yk", "use {double} from \"./lib/utils.yk\"; fn main() { result: int = double(5); }");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());
        loader.load(&Path::new("main.yk")).unwrap();
        assert_eq!(loader.file_count(), 3);
    }

    #[test]
    fn error_file_not_found() {
        let dir = TempDir::new();
        write_file(dir.path(), "main.yk", "use {x} from \"./nope.yk\"; fn main() {}");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());
        let result = loader.load(&Path::new("main.yk"));
        assert!(result.is_err());
        assert!(result.unwrap_err().msg.contains("not found"));
    }

    #[test]
    fn error_circular_import() {
        let dir = TempDir::new();
        write_file(dir.path(), "a.yk", "use {b} from \"./b.yk\"; fn a() -> int { return b(); }");
        write_file(dir.path(), "b.yk", "use {a} from \"./a.yk\"; fn b() -> int { return a(); }");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());
        let result = loader.load(&Path::new("a.yk"));
        assert!(result.is_err());
        assert!(result.unwrap_err().msg.contains("Circular"));
    }

    #[test]
    fn skip_ffi_imports() {
        let dir = TempDir::new();
        write_file(dir.path(), "main.yk", "use {gpu} from \"c++:./engine.hpp\"; fn main() {}");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());
        loader.load(&Path::new("main.yk")).unwrap();
        assert_eq!(loader.file_count(), 1);
    }

    #[test]
    fn load_without_yk_extension() {
        let dir = TempDir::new();
        write_file(dir.path(), "helper.yk", "fn help() -> int { return 42; }");
        write_file(dir.path(), "main.yk", "use {help} from \"./helper\"; fn main() { x: int = help(); }");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());
        loader.load(&Path::new("main.yk")).unwrap();
        assert_eq!(loader.file_count(), 2);
    }

    #[test]
    fn load_cached() {
        let dir = TempDir::new();
        write_file(dir.path(), "lib.yk", "fn f() -> int { return 1; }");
        write_file(dir.path(), "main.yk", "use {f} from \"./lib.yk\"; fn main() { x: int = f(); }");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());
        loader.load(&Path::new("main.yk")).unwrap();
        loader.load(&Path::new("lib.yk")).unwrap();
        assert_eq!(loader.file_count(), 2);
    }
}
