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

impl CodegenBackend for LlvmBackend {
    fn compile_with_paths(&self, modules: &[&Module], file_paths: &[String], output_path: &Path, hw: &HardwareInfo) -> Result<()> {
        let errors = super::llvm::validate_modules(modules, file_paths);
        if !errors.is_empty() {
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0), errors.join("\n")));
        }
        let llvm_ir = super::llvm::compile_to_llvm_modules(modules);

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

        super::llvm::compile_to_exe_with_extra_libs(&llvm_ir, output_path, hw, &ffi_libs)
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
