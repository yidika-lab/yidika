use std::collections::HashMap;
use std::sync::Mutex;
use std::ffi::CString;
use crate::codegen::llvm_api::{LlvmApi, LLVMExecutionEngineRef, LLVMModuleRef};
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;

/// A simple MCJIT-based JIT engine.
///
/// Uses LLVM's legacy MCJIT execution engine to compile IR modules
/// and extract function pointers. This is simpler than ORC and works
/// on LLVM versions where ORC may have issues.
///
/// Each compiled handler is cached by name. The underlying LLVM
/// execution engine keeps all modules alive for the lifetime of
/// this engine.
pub struct McJit {
    api: LlvmApi,
    inner: Mutex<McJitInner>,
}

struct McJitInner {
    engine: LLVMExecutionEngineRef,
    cache: HashMap<String, unsafe extern "C" fn(*mut std::ffi::c_void)>,
}

unsafe impl Send for McJit {}
unsafe impl Sync for McJit {}

impl McJit {
    pub fn new(api: LlvmApi) -> Result<Self> {
        unsafe {
            // Initialize targets. On Windows LLVM-C.dll this is required
            // for any codegen or JIT operations.
            if let Some(f) = api.LLVMInitializeX86TargetInfo { f(); }
            if let Some(f) = api.LLVMInitializeX86Target { f(); }
            if let Some(f) = api.LLVMInitializeX86TargetMC { f(); }
            if let Some(f) = api.LLVMInitializeX86AsmPrinter { f(); }
            if let Some(f) = api.LLVMInitializeX86AsmParser { f(); }

            let create_ee = api.LLVMCreateExecutionEngineForModule.as_ref()
                .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "MCJIT: LLVMCreateExecutionEngineForModule not found"))?;

            // Create a minimal empty module to bootstrap the engine.
            let ctx = (api.LLVMContextCreate)();
            let empty_mod_name = CString::new("yk_jit_bootstrap").unwrap();
            let module = (api.LLVMModuleCreateWithNameInContext)(empty_mod_name.as_ptr(), ctx);

            let mut engine: LLVMExecutionEngineRef = std::ptr::null_mut();
            let mut err: *mut i8 = std::ptr::null_mut();
            let rc = create_ee(&mut engine, module, &mut err);

            if rc != 0 || engine.is_null() {
                let err_str = if !err.is_null() {
                    api.get_error(err)
                } else {
                    "unknown error".to_string()
                };
                (api.LLVMDisposeModule)(module);
                (api.LLVMContextDispose)(ctx);
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    format!("MCJIT: engine creation failed: {}", err_str)));
            }

            Ok(McJit {
                api,
                inner: Mutex::new(McJitInner {
                    engine,
                    cache: HashMap::new(),
                }),
            })
        }
    }

    /// Compile LLVM IR and make a function available for lookup.
    ///
    /// The IR must contain a function definition. The module is added
    /// to the execution engine. The engine keeps the compiled code alive.
    pub fn add_module(&self, ir: &str, module_name: &str) -> Result<()> {
        unsafe {
            let add_module = self.api.LLVMAddModule.as_ref()
                .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "MCJIT: LLVMAddModule not found"))?;

            let ctx = (self.api.LLVMContextCreate)();
            let ir_cstr = CString::new(ir)
                .map_err(|_| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "MCJIT: IR contains null byte"))?;
            let membuf_name = CString::new(module_name)
                .unwrap_or_else(|_| CString::new("yk_ir").unwrap());
            let membuf = (self.api.LLVMCreateMemoryBufferWithMemoryRange)(
                ir_cstr.as_ptr(), ir.len(), membuf_name.as_ptr(), 1);

            if membuf.is_null() {
                (self.api.LLVMContextDispose)(ctx);
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    "MCJIT: failed to create memory buffer"));
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
                    format!("MCJIT: IR parse failed for '{}': {}", module_name, err_str)));
            }
            // NOTE: Do NOT dispose membuf here! LLVMParseIRInContext
            // takes ownership of the memory buffer on success. On some
            // LLVM builds, disposing it after parse causes a double-free
            // access violation. The buffer is a tiny wrapper and leaking
            // it is acceptable.

            // Set target triple on the module
            let triple_c = CString::new("x86_64-pc-windows-msvc").unwrap();
            (self.api.LLVMSetTarget)(module, triple_c.as_ptr());

            let inner = self.inner.lock().unwrap();
            add_module(inner.engine, module);
            // module is now owned by the execution engine

            Ok(())
        }
    }

    /// Look up a compiled function by name.
    ///
    /// Returns a cached pointer if already compiled, otherwise
    /// queries the execution engine.
    pub fn lookup(&self, name: &str) -> Result<*mut std::ffi::c_void> {
        unsafe {
            let get_addr = self.api.LLVMGetFunctionAddress.as_ref()
                .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "MCJIT: LLVMGetFunctionAddress not found"))?;

            let name_cstr = CString::new(name)
                .map_err(|_| error::err(ErrorKind::Internal, Span::new(0, 0),
                    "MCJIT: function name contains null byte"))?;

            let inner = self.inner.lock().unwrap();

            // Check cache first
            if let Some(&f) = inner.cache.get(name) {
                return Ok(f as *mut std::ffi::c_void);
            }

            let addr = get_addr(inner.engine, name_cstr.as_ptr());
            if addr == 0 {
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    format!("MCJIT: symbol '{}' not found", name)));
            }

            let ptr = addr as *mut std::ffi::c_void;
            drop(inner);
            Ok(ptr)
        }
    }
}

impl Drop for McJit {
    fn drop(&mut self) {
        unsafe {
            if let Ok(mut inner) = self.inner.lock() {
                if !inner.engine.is_null() {
                    if let Some(dispose) = self.api.LLVMDisposeExecutionEngine {
                        dispose(inner.engine);
                    }
                    inner.engine = std::ptr::null_mut();
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
    fn test_mcjit_new() {
        let api = match create_test_api() {
            Some(a) => a,
            None => { eprintln!("SKIP: LLVM-C not available"); return; }
        };
        let jit = McJit::new(api);
        assert!(jit.is_ok(), "McJit::new() failed: {:?}", jit.err());
    }

    #[test]
    fn test_mcjit_compile_and_lookup() {
        let api = match create_test_api() {
            Some(a) => a,
            None => { eprintln!("SKIP: LLVM-C not available"); return; }
        };
        let jit = McJit::new(api).expect("McJit::new() failed");

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
        // "Hello from JIT!" is 15 characters
        assert_eq!(resp.body_len, 15, "Body length should be 15 (Hello from JIT!)");
        assert_eq!(resp.status_code, 200, "Status code should be 200");

        let body_slice = unsafe { std::slice::from_raw_parts(resp.body, 15) };
        let body_str = std::str::from_utf8(body_slice).expect("Body should be valid UTF-8");
        assert_eq!(body_str, "Hello from JIT!", "Body should match");
    }
}
