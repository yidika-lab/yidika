use std::path::Path;
use crate::diagnostics::error::Result;
use crate::hardware::HardwareInfo;
use crate::syntax::ast::Module;

pub trait CodegenBackend {
    fn compile(&self, module: &Module, output_path: &Path) -> Result<()> {
        self.compile_with_info(module, output_path, &crate::hardware::detect())
    }
    fn compile_with_info(&self, module: &Module, output_path: &Path, hw: &HardwareInfo) -> Result<()>;
    fn name(&self) -> &str;
}

pub struct LlvmBackend;

impl CodegenBackend for LlvmBackend {
    fn compile_with_info(&self, module: &Module, output_path: &Path, hw: &HardwareInfo) -> Result<()> {
        let llvm_ir = super::llvm::compile_to_llvm(module);
        super::llvm::compile_to_exe(&llvm_ir, output_path, hw)
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
    fn compile_with_info(&self, _module: &Module, output_path: &Path, _hw: &HardwareInfo) -> Result<()> {
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
        backend.compile(&module, Path::new("test.exe")).unwrap();
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
        backend.compile_with_info(&module, Path::new("test.exe"), &hw).unwrap();
        assert_eq!(backend.modules.lock().unwrap().len(), 1);
    }
}
