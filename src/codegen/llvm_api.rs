use std::ffi::CStr;
use std::path::Path;
use std::sync::Arc;
use libloading::Library;
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::hardware::HardwareInfo;

pub(crate) type LLVMContextRef = *mut std::ffi::c_void;
pub(crate) type LLVMModuleRef = *mut std::ffi::c_void;
type LLVMMemoryBufferRef = *mut std::ffi::c_void;
type LLVMPassManagerRef = *mut std::ffi::c_void;
type LLVMTargetRef = *mut std::ffi::c_void;
type LLVMTargetMachineRef = *mut std::ffi::c_void;
type LLVMTargetDataRef = *mut std::ffi::c_void;
// ORC JIT opaque types
pub(crate) type LLVMOrcLLJITRef = *mut std::ffi::c_void;
pub(crate) type LLVMOrcLLJITBuilderRef = *mut std::ffi::c_void;
pub(crate) type LLVMOrcJITDylibRef = *mut std::ffi::c_void;
pub(crate) type LLVMOrcThreadSafeContextRef = *mut std::ffi::c_void;
pub(crate) type LLVMOrcThreadSafeModuleRef = *mut std::ffi::c_void;
#[allow(dead_code)]
pub(crate) type LLVMErrorRef = *mut std::ffi::c_void;
pub(crate) type LLVMOrcJITTargetAddress = u64;
// Legacy MCJIT types
pub(crate) type LLVMExecutionEngineRef = *mut std::ffi::c_void;

#[derive(Clone)]
#[allow(non_snake_case)]
pub struct LlvmApi {
    _lib: Arc<Library>,
    pub LLVMContextCreate: unsafe extern "C" fn() -> LLVMContextRef,
    pub LLVMContextDispose: unsafe extern "C" fn(LLVMContextRef),
    pub LLVMModuleCreateWithNameInContext: unsafe extern "C" fn(*const i8, LLVMContextRef) -> LLVMModuleRef,
    pub LLVMDisposeModule: unsafe extern "C" fn(LLVMModuleRef),
    pub LLVMCreateMemoryBufferWithMemoryRange: unsafe extern "C" fn(*const i8, usize, *const i8, i32) -> LLVMMemoryBufferRef,
    pub LLVMDisposeMemoryBuffer: unsafe extern "C" fn(LLVMMemoryBufferRef),
    pub LLVMParseIRInContext: unsafe extern "C" fn(LLVMContextRef, LLVMMemoryBufferRef, *mut LLVMModuleRef, *mut *mut i8) -> i32,
    pub LLVMParseBitcodeInContext2: unsafe extern "C" fn(LLVMContextRef, LLVMMemoryBufferRef, *mut LLVMModuleRef) -> i32,
    pub LLVMLinkModules2: unsafe extern "C" fn(LLVMModuleRef, LLVMModuleRef) -> i32,
    pub LLVMCreatePassManager: unsafe extern "C" fn() -> LLVMPassManagerRef,
    pub LLVMDisposePassManager: unsafe extern "C" fn(LLVMPassManagerRef),
    pub LLVMRunPassManager: unsafe extern "C" fn(LLVMPassManagerRef, LLVMModuleRef) -> i32,
    // Optional: removed in LLVM 15+ (new pass manager)
    pub LLVMAddConstantPropagationPass: Option<unsafe extern "C" fn(LLVMPassManagerRef)>,
    pub LLVMAddInstructionCombiningPass: Option<unsafe extern "C" fn(LLVMPassManagerRef)>,
    pub LLVMAddGVNPass: Option<unsafe extern "C" fn(LLVMPassManagerRef)>,
    pub LLVMAddAggressiveDCEPass: Option<unsafe extern "C" fn(LLVMPassManagerRef)>,
    // Target initialization (per-architecture, needed on Windows DLL builds)
    pub LLVMInitializeX86TargetInfo: Option<unsafe extern "C" fn()>,
    pub LLVMInitializeX86Target: Option<unsafe extern "C" fn()>,
    pub LLVMInitializeX86TargetMC: Option<unsafe extern "C" fn()>,
    pub LLVMInitializeX86AsmPrinter: Option<unsafe extern "C" fn()>,
    pub LLVMInitializeX86AsmParser: Option<unsafe extern "C" fn()>,
    pub LLVMGetDefaultTargetTriple: unsafe extern "C" fn() -> *mut i8,
    pub LLVMGetHostCPUName: unsafe extern "C" fn() -> *mut i8,
    pub LLVMGetHostCPUFeatures: unsafe extern "C" fn() -> *mut i8,
    pub LLVMDisposeMessage: unsafe extern "C" fn(*mut i8),
    pub LLVMGetFirstTarget: unsafe extern "C" fn() -> LLVMTargetRef,
    pub LLVMGetTargetFromTriple: unsafe extern "C" fn(*const i8, *mut LLVMTargetRef, *mut *mut i8) -> i32,
    pub LLVMCreateTargetMachine: unsafe extern "C" fn(LLVMTargetRef, *const i8, *const i8, *const i8, i32, i32, i32) -> LLVMTargetMachineRef,
    pub LLVMDisposeTargetMachine: unsafe extern "C" fn(LLVMTargetMachineRef),
    pub LLVMCreateTargetDataLayout: unsafe extern "C" fn(LLVMTargetMachineRef) -> LLVMTargetDataRef,
    pub LLVMSetModuleDataLayout: unsafe extern "C" fn(LLVMModuleRef, LLVMTargetDataRef),
    pub LLVMSetTarget: unsafe extern "C" fn(LLVMModuleRef, *const i8),
    pub LLVMTargetMachineEmitToFile: unsafe extern "C" fn(LLVMTargetMachineRef, LLVMModuleRef, *mut i8, i32, *mut *mut i8) -> i32,
    pub LLVMTargetMachineEmitToMemoryBuffer: unsafe extern "C" fn(LLVMTargetMachineRef, LLVMModuleRef, i32, *mut *mut i8, *mut LLVMMemoryBufferRef) -> i32,
    pub LLVMGetBufferStart: unsafe extern "C" fn(LLVMMemoryBufferRef) -> *const i8,
    pub LLVMGetBufferSize: unsafe extern "C" fn(LLVMMemoryBufferRef) -> usize,
    // ORC JIT functions (optional, LLVM 11+)
    pub LLVMOrcCreateLLJIT: Option<unsafe extern "C" fn(*mut LLVMOrcLLJITRef, LLVMOrcLLJITBuilderRef) -> LLVMErrorRef>,
    pub LLVMOrcDisposeLLJIT: Option<unsafe extern "C" fn(LLVMOrcLLJITRef)>,
    pub LLVMOrcCreateLLJITBuilder: Option<unsafe extern "C" fn() -> LLVMOrcLLJITBuilderRef>,
    pub LLVMOrcDisposeLLJITBuilder: Option<unsafe extern "C" fn(LLVMOrcLLJITBuilderRef)>,
    pub LLVMOrcLLJITGetMainJITDylib: Option<unsafe extern "C" fn(LLVMOrcLLJITRef) -> LLVMOrcJITDylibRef>,
    pub LLVMOrcCreateNewThreadSafeContextFromLLVMContext: Option<unsafe extern "C" fn(LLVMContextRef) -> LLVMOrcThreadSafeContextRef>,
    pub LLVMOrcDisposeThreadSafeContext: Option<unsafe extern "C" fn(LLVMOrcThreadSafeContextRef)>,
    pub LLVMOrcCreateNewThreadSafeModule: Option<unsafe extern "C" fn(LLVMModuleRef, LLVMOrcThreadSafeContextRef) -> LLVMOrcThreadSafeModuleRef>,
    pub LLVMOrcDisposeThreadSafeModule: Option<unsafe extern "C" fn(LLVMOrcThreadSafeModuleRef)>,
    pub LLVMOrcLLJITAddLLVMIRModule: Option<unsafe extern "C" fn(LLVMOrcLLJITRef, LLVMOrcJITDylibRef, LLVMOrcThreadSafeModuleRef) -> LLVMErrorRef>,
    pub LLVMOrcLLJITLookup: Option<unsafe extern "C" fn(LLVMOrcLLJITRef, *mut LLVMOrcJITTargetAddress, *const i8) -> LLVMErrorRef>,
    pub LLVMConsumeError: Option<unsafe extern "C" fn(LLVMErrorRef)>,
    pub LLVMGetErrorMessage: Option<unsafe extern "C" fn(LLVMErrorRef) -> *mut i8>,
    pub LLVMDisposeErrorMessage: Option<unsafe extern "C" fn(*mut i8)>,
    // Legacy MCJIT functions (optional)
    pub LLVMCreateExecutionEngineForModule: Option<unsafe extern "C" fn(*mut LLVMExecutionEngineRef, LLVMModuleRef, *mut *mut i8) -> i32>,
    pub LLVMDisposeExecutionEngine: Option<unsafe extern "C" fn(LLVMExecutionEngineRef)>,
    pub LLVMGetFunctionAddress: Option<unsafe extern "C" fn(LLVMExecutionEngineRef, *const i8) -> u64>,
    pub LLVMAddModule: Option<unsafe extern "C" fn(LLVMExecutionEngineRef, LLVMModuleRef)>,
    pub LLVMRemoveModule: Option<unsafe extern "C" fn(LLVMExecutionEngineRef, LLVMModuleRef, *mut LLVMModuleRef, *mut *mut i8) -> i32>,
}

pub fn find_llvm_lib() -> Option<String> {
    // Detect platform-specific LLVM-C library name
    let name = if cfg!(target_os = "windows") { "LLVM-C.dll" }
               else if cfg!(target_os = "macos") { "libLLVM-C.dylib" }
               else { "libLLVM-C.so" };

    // Try common search strategies
    find_lib_in_path(name)
        .or_else(|| find_lib_in_common_locations(name))
}

fn find_lib_in_path(name: &str) -> Option<String> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p).find_map(|d| {
            let full = d.join(name);
            if full.exists() { Some(full.to_string_lossy().to_string()) } else { None }
        })
    })
}

fn find_lib_in_common_locations(name: &str) -> Option<String> {
    let bases: &[&str] = if cfg!(target_os = "windows") {
        &[r"C:\Program Files\LLVM\bin", r"C:\Program Files (x86)\LLVM\bin"]
    } else if cfg!(target_os = "macos") {
        &["/usr/local/opt/llvm/lib", "/opt/homebrew/opt/llvm/lib"]
    } else {
        &["/usr/lib/llvm-18/lib", "/usr/lib/llvm-17/lib", "/usr/lib/llvm-16/lib",
          "/usr/lib/x86_64-linux-gnu", "/usr/lib64", "/usr/lib"]
    };
    for base in bases {
        let full = std::path::Path::new(base).join(name);
        if full.exists() {
            return Some(full.to_string_lossy().to_string());
        }
    }
    None
}

macro_rules! sym {
    ($lib:expr, $name:ident, $sig:ty) => {
        unsafe {
            let sym: libloading::Symbol<$sig> = $lib.get(concat!(stringify!($name), "\0").as_bytes())
                .map_err(|e| crate::diagnostics::error::err(
                    crate::diagnostics::error::ErrorKind::Internal,
                    crate::diagnostics::span::Span::new(0, 0),
                    format!("LLVM symbol '{}' not found: {}", stringify!($name), e)))?;
            *sym
        }
    };
}

macro_rules! opt_sym {
    ($lib:expr, $name:ident, $sig:ty) => {
        unsafe {
            $lib.get::<$sig>(concat!(stringify!($name), "\0").as_bytes())
                .ok()
                .map(|sym| *sym)
        }
    };
}

impl LlvmApi {
    pub fn load(path: &str) -> Result<Self> {
        let lib = Arc::new(unsafe {
            Library::new(path)
                .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                    format!("Failed to load LLVM-C library '{}': {}", path, e)))?
        });

        Ok(Self {
            _lib: lib.clone(),
            LLVMContextCreate: sym!(lib, LLVMContextCreate, unsafe extern "C" fn() -> LLVMContextRef),
            LLVMContextDispose: sym!(lib, LLVMContextDispose, unsafe extern "C" fn(LLVMContextRef)),
            LLVMModuleCreateWithNameInContext: sym!(lib, LLVMModuleCreateWithNameInContext, unsafe extern "C" fn(*const i8, LLVMContextRef) -> LLVMModuleRef),
            LLVMDisposeModule: sym!(lib, LLVMDisposeModule, unsafe extern "C" fn(LLVMModuleRef)),
            LLVMCreateMemoryBufferWithMemoryRange: sym!(lib, LLVMCreateMemoryBufferWithMemoryRange, unsafe extern "C" fn(*const i8, usize, *const i8, i32) -> LLVMMemoryBufferRef),
            LLVMDisposeMemoryBuffer: sym!(lib, LLVMDisposeMemoryBuffer, unsafe extern "C" fn(LLVMMemoryBufferRef)),
            LLVMParseIRInContext: sym!(lib, LLVMParseIRInContext, unsafe extern "C" fn(LLVMContextRef, LLVMMemoryBufferRef, *mut LLVMModuleRef, *mut *mut i8) -> i32),
            LLVMParseBitcodeInContext2: sym!(lib, LLVMParseBitcodeInContext2, unsafe extern "C" fn(LLVMContextRef, LLVMMemoryBufferRef, *mut LLVMModuleRef) -> i32),
            LLVMLinkModules2: sym!(lib, LLVMLinkModules2, unsafe extern "C" fn(LLVMModuleRef, LLVMModuleRef) -> i32),
            LLVMCreatePassManager: sym!(lib, LLVMCreatePassManager, unsafe extern "C" fn() -> LLVMPassManagerRef),
            LLVMDisposePassManager: sym!(lib, LLVMDisposePassManager, unsafe extern "C" fn(LLVMPassManagerRef)),
            LLVMRunPassManager: sym!(lib, LLVMRunPassManager, unsafe extern "C" fn(LLVMPassManagerRef, LLVMModuleRef) -> i32),
            LLVMAddConstantPropagationPass: opt_sym!(lib, LLVMAddConstantPropagationPass, unsafe extern "C" fn(LLVMPassManagerRef)),
            LLVMAddInstructionCombiningPass: opt_sym!(lib, LLVMAddInstructionCombiningPass, unsafe extern "C" fn(LLVMPassManagerRef)),
            LLVMAddGVNPass: opt_sym!(lib, LLVMAddGVNPass, unsafe extern "C" fn(LLVMPassManagerRef)),
            LLVMAddAggressiveDCEPass: opt_sym!(lib, LLVMAddAggressiveDCEPass, unsafe extern "C" fn(LLVMPassManagerRef)),
            LLVMInitializeX86TargetInfo: opt_sym!(lib, LLVMInitializeX86TargetInfo, unsafe extern "C" fn()),
            LLVMInitializeX86Target: opt_sym!(lib, LLVMInitializeX86Target, unsafe extern "C" fn()),
            LLVMInitializeX86TargetMC: opt_sym!(lib, LLVMInitializeX86TargetMC, unsafe extern "C" fn()),
            LLVMInitializeX86AsmPrinter: opt_sym!(lib, LLVMInitializeX86AsmPrinter, unsafe extern "C" fn()),
            LLVMInitializeX86AsmParser: opt_sym!(lib, LLVMInitializeX86AsmParser, unsafe extern "C" fn()),
            LLVMGetDefaultTargetTriple: sym!(lib, LLVMGetDefaultTargetTriple, unsafe extern "C" fn() -> *mut i8),
            LLVMGetHostCPUName: sym!(lib, LLVMGetHostCPUName, unsafe extern "C" fn() -> *mut i8),
            LLVMGetHostCPUFeatures: sym!(lib, LLVMGetHostCPUFeatures, unsafe extern "C" fn() -> *mut i8),
            LLVMDisposeMessage: sym!(lib, LLVMDisposeMessage, unsafe extern "C" fn(*mut i8)),
            LLVMGetFirstTarget: sym!(lib, LLVMGetFirstTarget, unsafe extern "C" fn() -> LLVMTargetRef),
            LLVMGetTargetFromTriple: sym!(lib, LLVMGetTargetFromTriple, unsafe extern "C" fn(*const i8, *mut LLVMTargetRef, *mut *mut i8) -> i32),
            LLVMCreateTargetMachine: sym!(lib, LLVMCreateTargetMachine, unsafe extern "C" fn(LLVMTargetRef, *const i8, *const i8, *const i8, i32, i32, i32) -> LLVMTargetMachineRef),
            LLVMDisposeTargetMachine: sym!(lib, LLVMDisposeTargetMachine, unsafe extern "C" fn(LLVMTargetMachineRef)),
            LLVMCreateTargetDataLayout: sym!(lib, LLVMCreateTargetDataLayout, unsafe extern "C" fn(LLVMTargetMachineRef) -> LLVMTargetDataRef),
            LLVMSetModuleDataLayout: sym!(lib, LLVMSetModuleDataLayout, unsafe extern "C" fn(LLVMModuleRef, LLVMTargetDataRef)),
            LLVMSetTarget: sym!(lib, LLVMSetTarget, unsafe extern "C" fn(LLVMModuleRef, *const i8)),
            LLVMTargetMachineEmitToFile: sym!(lib, LLVMTargetMachineEmitToFile, unsafe extern "C" fn(LLVMTargetMachineRef, LLVMModuleRef, *mut i8, i32, *mut *mut i8) -> i32),
            LLVMTargetMachineEmitToMemoryBuffer: sym!(lib, LLVMTargetMachineEmitToMemoryBuffer, unsafe extern "C" fn(LLVMTargetMachineRef, LLVMModuleRef, i32, *mut *mut i8, *mut LLVMMemoryBufferRef) -> i32),
            LLVMGetBufferStart: sym!(lib, LLVMGetBufferStart, unsafe extern "C" fn(LLVMMemoryBufferRef) -> *const i8),
            LLVMGetBufferSize: sym!(lib, LLVMGetBufferSize, unsafe extern "C" fn(LLVMMemoryBufferRef) -> usize),
            LLVMOrcCreateLLJIT: opt_sym!(lib, LLVMOrcCreateLLJIT, unsafe extern "C" fn(*mut LLVMOrcLLJITRef, LLVMOrcLLJITBuilderRef) -> LLVMErrorRef),
            LLVMOrcDisposeLLJIT: opt_sym!(lib, LLVMOrcDisposeLLJIT, unsafe extern "C" fn(LLVMOrcLLJITRef)),
            LLVMOrcCreateLLJITBuilder: opt_sym!(lib, LLVMOrcCreateLLJITBuilder, unsafe extern "C" fn() -> LLVMOrcLLJITBuilderRef),
            LLVMOrcDisposeLLJITBuilder: opt_sym!(lib, LLVMOrcDisposeLLJITBuilder, unsafe extern "C" fn(LLVMOrcLLJITBuilderRef)),
            LLVMOrcLLJITGetMainJITDylib: opt_sym!(lib, LLVMOrcLLJITGetMainJITDylib, unsafe extern "C" fn(LLVMOrcLLJITRef) -> LLVMOrcJITDylibRef),
            LLVMOrcCreateNewThreadSafeContextFromLLVMContext: opt_sym!(lib, LLVMOrcCreateNewThreadSafeContextFromLLVMContext, unsafe extern "C" fn(LLVMContextRef) -> LLVMOrcThreadSafeContextRef),
            LLVMOrcDisposeThreadSafeContext: opt_sym!(lib, LLVMOrcDisposeThreadSafeContext, unsafe extern "C" fn(LLVMOrcThreadSafeContextRef)),
            LLVMOrcCreateNewThreadSafeModule: opt_sym!(lib, LLVMOrcCreateNewThreadSafeModule, unsafe extern "C" fn(LLVMModuleRef, LLVMOrcThreadSafeContextRef) -> LLVMOrcThreadSafeModuleRef),
            LLVMOrcDisposeThreadSafeModule: opt_sym!(lib, LLVMOrcDisposeThreadSafeModule, unsafe extern "C" fn(LLVMOrcThreadSafeModuleRef)),
            LLVMOrcLLJITAddLLVMIRModule: opt_sym!(lib, LLVMOrcLLJITAddLLVMIRModule, unsafe extern "C" fn(LLVMOrcLLJITRef, LLVMOrcJITDylibRef, LLVMOrcThreadSafeModuleRef) -> LLVMErrorRef),
            LLVMOrcLLJITLookup: opt_sym!(lib, LLVMOrcLLJITLookup, unsafe extern "C" fn(LLVMOrcLLJITRef, *mut LLVMOrcJITTargetAddress, *const i8) -> LLVMErrorRef),
            LLVMConsumeError: opt_sym!(lib, LLVMConsumeError, unsafe extern "C" fn(LLVMErrorRef)),
            LLVMGetErrorMessage: opt_sym!(lib, LLVMGetErrorMessage, unsafe extern "C" fn(LLVMErrorRef) -> *mut i8),
            LLVMDisposeErrorMessage: opt_sym!(lib, LLVMDisposeErrorMessage, unsafe extern "C" fn(*mut i8)),
            LLVMCreateExecutionEngineForModule: opt_sym!(lib, LLVMCreateExecutionEngineForModule, unsafe extern "C" fn(*mut LLVMExecutionEngineRef, LLVMModuleRef, *mut *mut i8) -> i32),
            LLVMDisposeExecutionEngine: opt_sym!(lib, LLVMDisposeExecutionEngine, unsafe extern "C" fn(LLVMExecutionEngineRef)),
            LLVMGetFunctionAddress: opt_sym!(lib, LLVMGetFunctionAddress, unsafe extern "C" fn(LLVMExecutionEngineRef, *const i8) -> u64),
            LLVMAddModule: opt_sym!(lib, LLVMAddModule, unsafe extern "C" fn(LLVMExecutionEngineRef, LLVMModuleRef)),
            LLVMRemoveModule: opt_sym!(lib, LLVMRemoveModule, unsafe extern "C" fn(LLVMExecutionEngineRef, LLVMModuleRef, *mut LLVMModuleRef, *mut *mut i8) -> i32),
        })
    }

    pub fn get_error(&self, err_msg: *mut i8) -> String {
        if err_msg.is_null() {
            "unknown error".to_string()
        } else {
            let s = unsafe { CStr::from_ptr(err_msg) }.to_string_lossy().to_string();
            unsafe { (self.LLVMDisposeMessage)(err_msg) };
            s
        }
    }

    /// Convert an LLVMErrorRef to a Rust String. Consumes the error.
    pub fn consume_orc_error(&self, err: LLVMErrorRef) -> String {
        if err.is_null() {
            return String::new();
        }
        if let Some(get_msg) = self.LLVMGetErrorMessage {
            if let Some(dispose_msg) = self.LLVMDisposeErrorMessage {
                let msg_ptr = unsafe { get_msg(err) };
                let result = if msg_ptr.is_null() {
                    "unknown ORC error".to_string()
                } else {
                    let s = unsafe { CStr::from_ptr(msg_ptr) }.to_string_lossy().to_string();
                    unsafe { dispose_msg(msg_ptr) };
                    s
                };
                return result;
            }
        }
        // Fallback: just consume error without message
        if let Some(consume) = self.LLVMConsumeError {
            unsafe { consume(err) };
        }
        "ORC JIT error".to_string()
    }
}

pub fn compile_ir(api: &LlvmApi, llvm_ir: &str, runtime_bc: &[u8], output_path: &Path, hw: &HardwareInfo) -> Result<()> {
    unsafe {
        // Initialize X86 target (required on Windows DLL builds; LLVM-C.dll per-target init)
        if let Some(f) = api.LLVMInitializeX86TargetInfo { f(); }
        if let Some(f) = api.LLVMInitializeX86Target { f(); }
        if let Some(f) = api.LLVMInitializeX86TargetMC { f(); }
        if let Some(f) = api.LLVMInitializeX86AsmPrinter { f(); }
        if let Some(f) = api.LLVMInitializeX86AsmParser { f(); }

        let ctx = (api.LLVMContextCreate)();
        let cleanup = || { (api.LLVMContextDispose)(ctx); };

        // Parse LLVM IR
        let ir_str = std::ffi::CString::new(llvm_ir)
            .map_err(|_| error::err(ErrorKind::Internal, Span::new(0, 0), "LLVM IR contains null byte"))?;
        let name = std::ffi::CString::new("yk").unwrap();
        let membuf = (api.LLVMCreateMemoryBufferWithMemoryRange)(ir_str.as_ptr(), llvm_ir.len(), name.as_ptr(), 1);
        if membuf.is_null() {
            cleanup();
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0), "LLVM: failed to create memory buffer"));
        }
        let mut module: LLVMModuleRef = std::ptr::null_mut();
        let mut err: *mut i8 = std::ptr::null_mut();
        let parse_rc = (api.LLVMParseIRInContext)(ctx, membuf, &mut module, &mut err);
        if parse_rc != 0 {
            let err_str = api.get_error(err);
            (api.LLVMDisposeMemoryBuffer)(membuf);
            cleanup();
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0), format!("LLVM IR parse failed: {}", err_str)));
        }

        // Link runtime bitcode
        if !runtime_bc.is_empty() {
            let rt_name = std::ffi::CString::new("yk_rt.bc").unwrap();
            let rt_buf = (api.LLVMCreateMemoryBufferWithMemoryRange)(runtime_bc.as_ptr() as *const i8, runtime_bc.len(), rt_name.as_ptr(), 0);
            if !rt_buf.is_null() {
                let mut rt_mod: LLVMModuleRef = std::ptr::null_mut();
                if (api.LLVMParseBitcodeInContext2)(ctx, rt_buf, &mut rt_mod) == 0 && !rt_mod.is_null() {
                    let triple_c = std::ffi::CString::new(hw.os.triple.as_str()).unwrap();
                    (api.LLVMSetTarget)(rt_mod, triple_c.as_ptr());
                    (api.LLVMLinkModules2)(module, rt_mod);
                }
                (api.LLVMDisposeMemoryBuffer)(rt_buf);
            }
        }

        // Use hardware-adaptive triple and CPU info
        let triple_c = std::ffi::CString::new(hw.os.triple.as_str()).unwrap();
        let cpu_c = std::ffi::CString::new(hw.cpu.name.as_str()).unwrap();

        let llvm_features = hw.cpu.simd.to_llvm_features().join(",");
        let features_c = std::ffi::CString::new(llvm_features).unwrap();

        (api.LLVMSetTarget)(module, triple_c.as_ptr());

        // Build adaptive optimization passes
        let pm = (api.LLVMCreatePassManager)();
        if let Some(f) = api.LLVMAddConstantPropagationPass { f(pm); }
        if let Some(f) = api.LLVMAddInstructionCombiningPass { f(pm); }
        if let Some(f) = api.LLVMAddGVNPass { f(pm); }
        if let Some(f) = api.LLVMAddAggressiveDCEPass { f(pm); }
        (api.LLVMRunPassManager)(pm, module);
        (api.LLVMDisposePassManager)(pm);

        // Create target machine with adaptive optimization level
        let opt_level: i32 = if crate::hardware::memory::is_low_memory(&hw.memory) { 2 }
            else { 3 };

        let mut target_ref = (api.LLVMGetFirstTarget)();
        if target_ref.is_null() {
            let mut err_target: *mut i8 = std::ptr::null_mut();
            let rc = (api.LLVMGetTargetFromTriple)(triple_c.as_ptr(), &mut target_ref, &mut err_target);
            if rc != 0 || target_ref.is_null() {
                let err_str = if !err_target.is_null() { api.get_error(err_target) } else { "unknown".into() };
                (api.LLVMDisposeModule)(module);
                cleanup();
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    format!("LLVM: no target for triple '{}' ({})", hw.os.triple, err_str)));
            }
        }
        let tm = (api.LLVMCreateTargetMachine)(target_ref, triple_c.as_ptr(), cpu_c.as_ptr(), features_c.as_ptr(), opt_level, 2, 0);

        if tm.is_null() {
            (api.LLVMDisposeModule)(module);
            cleanup();
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                "LLVM: failed to create target machine (architecture not supported?)"));
        }

        // Set data layout
        let td = (api.LLVMCreateTargetDataLayout)(tm);
        (api.LLVMSetModuleDataLayout)(module, td);

        // Emit object file
        let out = std::ffi::CString::new(output_path.to_string_lossy().as_ref()).unwrap();
        let mut err2: *mut i8 = std::ptr::null_mut();
        let emit_result = (api.LLVMTargetMachineEmitToFile)(tm, module, out.into_raw() as *mut i8, 1, &mut err2);

        (api.LLVMDisposeTargetMachine)(tm);
        (api.LLVMDisposeModule)(module);
        cleanup();

        if emit_result != 0 {
            let err_str = api.get_error(err2);
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                format!("LLVM emit failed: {}", err_str)));
        }
        Ok(())
    }
}
