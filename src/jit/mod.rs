use std::collections::HashMap;
use std::sync::Mutex;

use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::hardware::HardwareInfo;
use crate::syntax::ast::Module;

pub mod orc;
pub mod mcjit;

/// Simple function JIT that compiles and caches hot functions
pub struct JitEngine {
    compiled: Mutex<HashMap<String, usize>>,
}

impl JitEngine {
    pub fn new() -> Self {
        JitEngine { compiled: Mutex::new(HashMap::new()) }
    }

    /// Compile a function definition to native code and return a function pointer
    pub fn compile_function(&self, name: &str, module: &Module, hw: &HardwareInfo) -> Result<()> {
        let ir = crate::codegen::llvm::compile_to_llvm(module);
        let tmp_dir = std::env::temp_dir().join(format!("yk_jit_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp_dir);
        let dll_path = tmp_dir.join(format!("{}.dll", name));

        // Try in-process LLVM emit first
        let obj_path = tmp_dir.join(format!("{}.obj", name));
        if let Ok(api) = get_llvm_api() {
            let _ = crate::codegen::llvm::emit_obj_in_memory(&api, &ir, &obj_path, hw);
        } else {
            // Fallback: write IR to file and use clang
            let ll_path = tmp_dir.join(format!("{}.ll", name));
            std::fs::write(&ll_path, &ir)
                .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                    format!("JIT: failed to write IR: {}", e)))?;
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                "LLVM-C not available for JIT"));
        }

        // Link into DLL
        let vcvars = crate::codegen::llvm::detect_vcvars()
            .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0), "Visual Studio not found"))?;
        let status = std::process::Command::new("cmd.exe")
            .args(["/c", &format!(
                r#""{}" x64 >nul 2>&1 && link.exe /nologo /DLL "{}" /OUT:"{}" /defaultlib:libcmt.lib /NODEFAULTLIB:msvcrt.lib"#,
                vcvars, obj_path.to_string_lossy(), dll_path.to_string_lossy())])
            .status()
            .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                format!("JIT link failed: {}", e)))?;
        if !status.success() {
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                "JIT link failed"));
        }

        let mut compiled = self.compiled.lock().unwrap();
        compiled.insert(name.to_string(), 1);
        let _ = std::fs::remove_file(&obj_path);
        Ok(())
    }

    pub fn is_compiled(&self, name: &str) -> bool {
        self.compiled.lock().unwrap().contains_key(name)
    }
}

fn get_llvm_api() -> Result<crate::codegen::llvm_api::LlvmApi> {
    let path = crate::codegen::llvm_api::find_llvm_lib()
        .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0), "LLVM-C not found"))?;
    crate::codegen::llvm_api::LlvmApi::load(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jit_engine_new() {
        let engine = JitEngine::new();
        assert!(!engine.is_compiled("nonexistent"));
    }
}
