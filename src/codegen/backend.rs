use std::path::Path;
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::hardware::HardwareInfo;
use crate::syntax::ast::Module;

pub trait CodegenBackend {
    fn compile(&self, modules: &[&Module], output_path: &Path) -> Result<()> {
        self.compile_with_info(modules, output_path, &crate::hardware::detect())
    }
    fn compile_with_info(&self, modules: &[&Module], output_path: &Path, hw: &HardwareInfo) -> Result<()> {
        self.compile_with_paths(modules, &[], output_path, hw)
    }
    fn compile_with_paths(&self, modules: &[&Module], _file_paths: &[String], output_path: &Path, hw: &HardwareInfo) -> Result<()> {
        self.compile_with_info(modules, output_path, hw)
    }
    fn name(&self) -> &str;
}

pub struct LlvmBackend;

pub fn needs_side_artifacts(modules: &[&Module]) -> bool {
    gui_imported(modules) || modules.iter().any(|m| m.imports.iter().any(|import| import.lang.is_some()))
}

fn gui_imported(modules: &[&Module]) -> bool {
    modules.iter().any(|m| {
        m.imports.iter().any(|import| {
            import.lang.is_none()
                && import.source == "std"
                && import.names.iter().any(|(local, orig)| {
                    local == "gui" || orig.as_deref() == Some("gui") || local == "std" || orig.as_deref() == Some("std")
                })
        })
    })
}

fn gui_artifacts(output_dir: Option<&Path>) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let exe = std::env::current_exe().ok();
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Some(dir) = output_dir {
        roots.push(dir.join("lib").join("ffi"));
        roots.push(dir.to_path_buf());
    }
    if let Some(e) = exe {
        if let Some(d) = e.parent() {
            roots.push(d.join("lib").join("ffi"));
            roots.push(d.to_path_buf());
            for profile in ["release", "debug"] {
                if let Some(t) = d.parent() {
                    roots.push(t.join(profile).to_path_buf());
                }
            }
        }
    }
    roots.push(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("release"));
    roots.push(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("debug"));
    roots.push(std::path::PathBuf::from("target").join("release"));
    roots.push(std::path::PathBuf::from("target").join("debug"));
    for root in roots {
        let dll = root.join("yk_ffi_gui.dll");
        for lib_name in ["yk_ffi_gui.dll.lib", "yk_ffi_gui.lib"] {
            let lib = root.join(lib_name);
            if dll.exists() && lib.exists() {
                return Some((dll, lib));
            }
        }
    }
    None
}

impl CodegenBackend for LlvmBackend {
    fn compile_with_paths(&self, modules: &[&Module], file_paths: &[String], output_path: &Path, hw: &HardwareInfo) -> Result<()> {
        let errors = super::llvm::validate_modules(modules, file_paths);
        if !errors.is_empty() {
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0), errors.join("\n")));
        }
        let llvm_ir = super::llvm::compile_to_llvm_modules(modules)
            .map_err(|refusal| error::err(ErrorKind::Internal, Span::new(0, 0), refusal))?;

        // Collect FFI import library paths for linking
        let mut ffi_libs = Vec::new();
        if let Some(root) = output_path.parent() {
            let ffi_dir = root.join("lib").join("ffi");
            for module in modules {
                for import in &module.imports {
                    if let Some(lang) = &import.lang {
                        if lang == "rust" || lang == "c++" {
                            let lib_name = format!("yk_ffi_{}.lib", import.source.replace('/', "_").replace('.', ""));
                            let lib_path = ffi_dir.join(&lib_name);
                            if lib_path.exists() {
                                ffi_libs.push(lib_path.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
        }

        let mut gui_dll: Option<std::path::PathBuf> = None;
        if gui_imported(modules) {
            match gui_artifacts(output_path.parent()) {
                Some((dll, lib)) => {
                    ffi_libs.push(lib.to_string_lossy().to_string());
                    gui_dll = Some(dll);
                }
                None => {
                    return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                        "gui module requires yk_ffi_gui.dll and its import library; build with `cargo build -p yk_ffi_gui --release`".to_string()));
                }
            }
        }

        super::llvm::compile_to_exe_with_extra_libs(&llvm_ir, output_path, hw, &ffi_libs)?;
        if let Some(dll) = gui_dll {
            let dest = output_path.with_file_name("yk_ffi_gui.dll");
            let _ = std::fs::copy(&dll, &dest);
        }
        Ok(())
    }

    fn compile_with_info(&self, modules: &[&Module], output_path: &Path, hw: &HardwareInfo) -> Result<()> {
        self.compile_with_paths(modules, &[], output_path, hw)
    }

    fn name(&self) -> &str {
        "LLVM AOT (adaptive)"
    }
}

pub struct MockBackend {
    pub modules: std::sync::Mutex<Vec<String>>,
    pub output_paths: std::sync::Mutex<Vec<std::path::PathBuf>>,
}

impl MockBackend {
    pub fn new() -> Self {
        Self {
            modules: std::sync::Mutex::new(Vec::new()),
            output_paths: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl CodegenBackend for MockBackend {
    fn compile_with_info(&self, _modules: &[&Module], output_path: &Path, _hw: &HardwareInfo) -> Result<()> {
        self.modules.lock().unwrap().push("compiled".into());
        self.output_paths.lock().unwrap().push(output_path.to_path_buf());
        Ok(())
    }

    fn name(&self) -> &str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parser::Parser;

    #[test]
    fn mock_backend_records_compile() {
        let backend = MockBackend::new();
        let module = Parser::parse("fn main() {}").unwrap();
        backend.compile(&[&module], Path::new("test.exe")).unwrap();
        assert_eq!(backend.modules.lock().unwrap().len(), 1);
        assert_eq!(backend.output_paths.lock().unwrap()[0], Path::new("test.exe"));
    }

    #[test]
    fn llvm_backend_implements_trait() {
        let backend = LlvmBackend;
        assert_eq!(backend.name(), "LLVM AOT (adaptive)");
    }

    #[test]
    fn compile_with_info_passes() {
        let backend = MockBackend::new();
        let module = Parser::parse("fn main() {}").unwrap();
        let hw = crate::hardware::detect();
        backend.compile_with_info(&[&module], Path::new("test.exe"), &hw).unwrap();
        assert_eq!(backend.modules.lock().unwrap().len(), 1);
    }
}
