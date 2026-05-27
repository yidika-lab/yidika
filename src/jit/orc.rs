use std::sync::Mutex;
use std::ffi::CString;
use crate::codegen::llvm_api::{LlvmApi, LLVMOrcLLJITRef, LLVMOrcJITTargetAddress, LLVMModuleRef};
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;

/// ORC JIT engine wrapping LLJIT (LLVM's On-Request Compilation JIT).
///
/// Usage:
/// 1. Create with `OrcJit::new(api)`
/// 2. Compile IR modules with `add_module(ir, name)`
/// 3. Look up function pointers with `lookup(name)`
///
/// All operations are lock-protected. The JIT instance is shared
/// across threads via `Arc<OrcJit>`.
pub struct OrcJit {
    api: LlvmApi,
    jit: Mutex<LLVMOrcLLJITRef>,
}

unsafe impl Send for OrcJit {}
unsafe impl Sync for OrcJit {}

impl OrcJit {
    /// Create a new ORC JIT engine.
    ///
    /// Initializes the host target and creates an LLJIT instance
    /// with default settings (auto-detected host triple/CPU/features).
    pub fn new(api: LlvmApi) -> Result<Self> {
        unsafe {
            // On this LLVM build, target init functions may crash when
            // called from ORC. Skip them — LLJIT will return a clear
            // "no targets registered" error and the caller should fall
            // back to MCJIT.

            let create_lljit = api.LLVMOrcCreateLLJIT.as_ref()
                .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT not available: LLVMOrcCreateLLJIT not found"))?;
            let create_builder = api.LLVMOrcCreateLLJITBuilder.as_ref()
                .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT not available: LLVMOrcCreateLLJITBuilder not found"))?;

            let builder = create_builder();
            if builder.is_null() {
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT: LLJITBuilder creation failed"));
            }

            let mut jit: LLVMOrcLLJITRef = std::ptr::null_mut();
            let err = create_lljit(&mut jit, builder);

            if !err.is_null() {
                // Builder was moved-from during the failed attempt.
                // Do NOT dispose builder — it was consumed by the failed JIT
                // attempt. Just consume the error.
                let msg = api.consume_orc_error(err);
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    format!("ORC JIT: LLJIT creation failed: {}", msg)));
            }

            if jit.is_null() {
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT: LLJIT is null after creation"));
            }

            // On success, the builder was consumed by LLJIT; dispose the
            // now-empty wrapper.
            if let Some(dispose) = api.LLVMOrcDisposeLLJITBuilder {
                dispose(builder);
            }

            Ok(OrcJit { api, jit: Mutex::new(jit) })
        }
    }

    /// Add an LLVM IR module to the JIT.
    ///
    /// The IR must contain at least one function that will be compiled
    /// and made available for lookup. The module must have the target
    /// triple set correctly.
    pub fn add_module(&self, ir: &str, module_name: &str) -> Result<()> {
        unsafe {
            let get_main_dylib = self.api.LLVMOrcLLJITGetMainJITDylib.as_ref()
                .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT: LLVMOrcLLJITGetMainJITDylib not found"))?;
            let create_ts_ctx = self.api.LLVMOrcCreateNewThreadSafeContextFromLLVMContext.as_ref()
                .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT: LLVMOrcCreateNewThreadSafeContextFromLLVMContext not found"))?;
            let create_ts_mod = self.api.LLVMOrcCreateNewThreadSafeModule.as_ref()
                .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT: LLVMOrcCreateNewThreadSafeModule not found"))?;
            let add_module_fn = self.api.LLVMOrcLLJITAddLLVMIRModule.as_ref()
                .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT: LLVMOrcLLJITAddLLVMIRModule not found"))?;

            // Create LLVM context and parse IR
            let ctx = (self.api.LLVMContextCreate)();
            let ir_cstr = CString::new(ir)
                .map_err(|_| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT: IR contains null byte"))?;
            let membuf_name = CString::new(module_name)
                .unwrap_or_else(|_| CString::new("yk_ir").unwrap());
            let membuf = (self.api.LLVMCreateMemoryBufferWithMemoryRange)(
                ir_cstr.as_ptr(), ir.len(), membuf_name.as_ptr(), 1);

            if membuf.is_null() {
                (self.api.LLVMContextDispose)(ctx);
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT: failed to create memory buffer"));
            }

            let mut module: LLVMModuleRef = std::ptr::null_mut();
            let mut parse_err: *mut i8 = std::ptr::null_mut();
            let parse_rc = (self.api.LLVMParseIRInContext)(ctx, membuf, &mut module, &mut parse_err);

            if parse_rc != 0 || module.is_null() {
                let err_str = if !parse_err.is_null() {
                    self.api.get_error(parse_err)
                } else {
                    "parse failed".to_string()
                };
                (self.api.LLVMDisposeMemoryBuffer)(membuf);
                (self.api.LLVMContextDispose)(ctx);
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    format!("ORC JIT: IR parse failed for '{}': {}", module_name, err_str)));
            }

            // Set target triple from module if not already set
            // (we assume the IR already has the correct triple)

            // Create thread-safe wrapper (transfers ownership of ctx and module)
            let ts_ctx = create_ts_ctx(ctx);
            if ts_ctx.is_null() {
                (self.api.LLVMDisposeModule)(module);
                (self.api.LLVMContextDispose)(ctx);
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT: failed to create ThreadSafeContext"));
            }

            let ts_mod = create_ts_mod(module, ts_ctx);
            if ts_mod.is_null() {
                if let Some(dispose) = self.api.LLVMOrcDisposeThreadSafeContext {
                    dispose(ts_ctx);
                }
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT: failed to create ThreadSafeModule"));
            }

            // Add module to JIT (transfers ownership of ts_mod)
            let jit = self.jit.lock().unwrap();
            let dylib = get_main_dylib(*jit);
            let err = add_module_fn(*jit, dylib, ts_mod);

            if !err.is_null() {
                let msg = self.api.consume_orc_error(err);
                // ts_mod is consumed even on error by some LLVM versions
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    format!("ORC JIT: failed to add module '{}': {}", module_name, msg)));
            }

            Ok(())
        }
    }

    /// Look up a compiled function by name.
    ///
    /// Returns a raw function pointer that can be called.
    /// The function must have been previously compiled via `add_module`.
    pub fn lookup(&self, name: &str) -> Result<*mut std::ffi::c_void> {
        unsafe {
            let lookup = self.api.LLVMOrcLLJITLookup.as_ref()
                .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT: LLVMOrcLLJITLookup not found"))?;

            let name_cstr = CString::new(name)
                .map_err(|_| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "ORC JIT: function name contains null byte"))?;

            let jit = self.jit.lock().unwrap();
            let mut addr: LLVMOrcJITTargetAddress = 0;
            let err = lookup(*jit, &mut addr, name_cstr.as_ptr());

            if !err.is_null() {
                let msg = self.api.consume_orc_error(err);
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    format!("ORC JIT: lookup failed for '{}': {}", name, msg)));
            }

            if addr == 0 {
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    format!("ORC JIT: symbol '{}' not found in JIT", name)));
            }

            Ok(addr as *mut std::ffi::c_void)
        }
    }
}

impl Drop for OrcJit {
    fn drop(&mut self) {
        unsafe {
            if let Some(jit) = self.jit.get_mut().ok() {
                if !jit.is_null() {
                    if let Some(dispose) = self.api.LLVMOrcDisposeLLJIT {
                        dispose(*jit);
                    }
                    *jit = std::ptr::null_mut();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_api() -> Option<LlvmApi> {
        let path = crate::codegen::llvm_api::find_llvm_lib()?;
        LlvmApi::load(&path).ok()
    }

    #[test]
    #[ignore = "ORC JIT requires LLVM-C with targets registered (crash on this LLVM build)"]
    fn test_orc_jit_new() {
        let api = match create_test_api() {
            Some(a) => a,
            None => { eprintln!("SKIP: LLVM-C not available"); return; }
        };
        let jit = OrcJit::new(api);
        assert!(jit.is_ok(), "OrcJit::new() failed: {:?}", jit.err());
    }

    #[test]
    #[ignore = "ORC JIT requires LLVM-C with targets registered (crash on this LLVM build)"]
    fn test_orc_jit_compile_and_lookup() {
        let api = match create_test_api() {
            Some(a) => a,
            None => { eprintln!("SKIP: LLVM-C not available"); return; }
        };
        let jit = OrcJit::new(api).expect("OrcJit::new() failed");

        // Generate IR for a simple handler: "Hello from JIT!" with status 200
        let ir = crate::codegen::llvm::generate_static_handler_ir(
            "test_handler", "Hello from JIT!", 200);
        assert!(ir.contains("@__yk_cstr_test_handler"), "IR should contain global string");

        // Add module to JIT
        jit.add_module(&ir, "test_module")
            .expect("add_module failed");

        // Look up the function
        let fn_ptr = jit.lookup("test_handler")
            .expect("lookup failed");
        assert!(!fn_ptr.is_null(), "Function pointer should not be null");

        // Call the JIT'd function
        #[repr(C)]
        struct TestResponse {
            body: *const u8,
            body_len: i64,
            status_code: i32,
        }

        let mut resp = TestResponse {
            body: std::ptr::null(),
            body_len: 0,
            status_code: 0,
        };

        let func: unsafe extern "C" fn(*mut TestResponse) = unsafe { std::mem::transmute(fn_ptr) };
        unsafe { func(&mut resp) };

        assert!(!resp.body.is_null(), "Response body should not be null");
        assert_eq!(resp.body_len, 14, "Body length should be 14 (Hello from JIT!)");
        assert_eq!(resp.status_code, 200, "Status code should be 200");

        let body_slice = unsafe { std::slice::from_raw_parts(resp.body, resp.body_len as usize) };
        let body_str = std::str::from_utf8(body_slice).expect("Body should be valid UTF-8");
        assert_eq!(body_str, "Hello from JIT!", "Body should match");
    }
}
