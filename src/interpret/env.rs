use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::syntax::ast::*;
use super::value::{Value, EvalResult, is_copy_value};
use super::class::{FnDef, ClassDef, ObjectDef, Frame, INLINE_CACHE_SIZE};

pub struct SubInterpreterPool {
    pool: Mutex<Vec<Interpreter>>,
    max_pool: usize,
}

impl SubInterpreterPool {
    pub fn new(max_pool: usize) -> Self {
        SubInterpreterPool { pool: Mutex::new(Vec::with_capacity(max_pool)), max_pool }
    }

    pub fn take(&self, base: &Interpreter) -> Interpreter {
        let mut guard = self.pool.lock().unwrap();
        if let Some(mut interp) = guard.pop() {
            interp.reset_with(base);
            interp
        } else {
            Interpreter::sub_from(base)
        }
    }

    pub fn return_interp(&self, mut interp: Interpreter) {
        interp.clear_state();
        let mut guard = self.pool.lock().unwrap();
        if guard.len() < self.max_pool {
            guard.push(interp);
        }
    }
}

pub struct Interpreter {
    pub globals: HashMap<String, Value>,
    pub const_vars: HashSet<String>,
    pub json_files: HashMap<String, std::path::PathBuf>,
    pub struct_defs: Arc<HashMap<String, Vec<String>>>,
    pub classes: Arc<HashMap<String, ClassDef>>,
    pub objects: Arc<HashMap<String, ObjectDef>>,
    pub functions: Arc<HashMap<String, FnDef>>,
    pub builtin_modules: Arc<HashMap<String, String>>,
    pub builtin_funcs: Arc<HashMap<String, String>>,
    pub std_imported: bool,
    pub frames: Vec<Frame>,
    pub frame_pool: Vec<Frame>,
    pub moved_frames: Vec<HashSet<String>>,
    pub global_moved: HashSet<String>,
    pub output: String,
    pub tui_mode: bool,
    pub ffi_libs: HashMap<String, Arc<libloading::Library>>,
    pub next_task_id: u64,
    pub task_rxs: HashMap<u64, std::sync::mpsc::Receiver<Value>>,
    pub value_pool: crate::memory::arena::ValuePool,
    pub source_dir: std::path::PathBuf,
}

impl Interpreter {
    pub fn new() -> Self {
        Self {
            globals: HashMap::new(),
            const_vars: HashSet::new(),
            json_files: HashMap::new(),
            struct_defs: Arc::new(HashMap::new()),
            classes: Arc::new(HashMap::new()),
            objects: Arc::new(HashMap::new()),
            functions: Arc::new(HashMap::new()),
            builtin_modules: Arc::new(HashMap::new()),
            builtin_funcs: Arc::new(HashMap::new()),
            std_imported: false,
            frames: vec![Frame::new()],
            frame_pool: Vec::new(),
            moved_frames: vec![HashSet::new()],
            global_moved: HashSet::new(),
            output: String::new(),
            tui_mode: false,
            ffi_libs: HashMap::new(),
            next_task_id: 0,
            task_rxs: HashMap::new(),
            value_pool: crate::memory::arena::ValuePool::new(),
            source_dir: std::path::PathBuf::from("."),
        }
    }

    pub fn sub_from(other: &Interpreter) -> Self {
        Self {
            globals: HashMap::new(),
            const_vars: HashSet::new(),
            json_files: other.json_files.clone(),
            struct_defs: other.struct_defs.clone(),
            classes: other.classes.clone(),
            objects: other.objects.clone(),
            functions: other.functions.clone(),
            builtin_modules: other.builtin_modules.clone(),
            builtin_funcs: other.builtin_funcs.clone(),
            std_imported: other.std_imported,
            frames: vec![Frame::new()],
            frame_pool: Vec::new(),
            moved_frames: vec![HashSet::new()],
            global_moved: HashSet::new(),
            output: String::new(),
            tui_mode: false,
            ffi_libs: other.ffi_libs.clone(),
            next_task_id: 0,
            task_rxs: HashMap::new(),
            value_pool: crate::memory::arena::ValuePool::new(),
            source_dir: other.source_dir.clone(),
        }
    }

    pub fn reset_with(&mut self, base: &Interpreter) {
        self.globals.clear();
        self.const_vars.clear();
        self.struct_defs = base.struct_defs.clone();
        self.classes = base.classes.clone();
        self.objects = base.objects.clone();
        self.functions = base.functions.clone();
        self.builtin_modules = base.builtin_modules.clone();
        self.builtin_funcs = base.builtin_funcs.clone();
        self.std_imported = base.std_imported;
        self.ffi_libs = base.ffi_libs.clone();
        self.source_dir = base.source_dir.clone();
        self.clear_state();
    }

    fn clear_state(&mut self) {
        self.frames.clear();
        self.frames.push(Frame::new());
        self.frame_pool.clear();
        self.moved_frames.clear();
        self.moved_frames.push(HashSet::new());
        self.global_moved.clear();
        self.output.clear();
        self.next_task_id = 0;
        self.task_rxs.clear();
        self.value_pool.reset();
    }

    pub fn load_module(&mut self, module: &Module) {
        {
            let struct_defs = Arc::make_mut(&mut self.struct_defs);
            let classes = Arc::make_mut(&mut self.classes);
            let functions = Arc::make_mut(&mut self.functions);
            let builtin_modules = Arc::make_mut(&mut self.builtin_modules);
            let builtin_funcs = Arc::make_mut(&mut self.builtin_funcs);

            for import in &module.imports {
                let source = import.source.as_str();
                match source {
                    "std" => {
                        self.std_imported = true;
                        for (name, _) in &import.names {
                            if name == "std" {
                                for sub in crate::stdlib::list_submodules() {
                                    builtin_modules.insert(sub.to_string(), sub.to_string());
                                }
                            } else if crate::stdlib::list_submodules().any(|s| s == name.as_str()) {
                                builtin_modules.insert(name.clone(), name.clone());
                            }
                        }
                    }
                    "io" | "json" | "datetime" | "path" | "base64" | "regex" => {
                        for (name, _) in &import.names {
                            builtin_modules.insert(name.clone(), source.to_string());
                        }
                    }
                    "math" | "time" | "net" => {
                        for (name, _) in &import.names {
                            builtin_funcs.insert(name.clone(), source.to_string());
                        }
                    }
                    _ => {
                        if let Some(lang) = &import.lang {
                            if lang == "json" {
                                let json_path = self.source_dir.join(&import.source);
                                let json_str = match std::fs::read_to_string(&json_path) {
                                    Ok(s) => s,
                                    Err(e) => {
                                        eprintln!("Warning: failed to read JSON file '{}': {}", json_path.display(), e);
                                        continue;
                                    }
                                };
                                let json_val: serde_json::Value = match serde_json::from_str(&json_str) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        eprintln!("Warning: failed to parse JSON file '{}': {}", json_path.display(), e);
                                        continue;
                                    }
                                };
                                let val = crate::interpret::builtins::json_to_value(json_val);
                                for (i, (name, _)) in import.names.iter().enumerate() {
                                    builtin_modules.insert(name.clone(), "json".to_string());
                                    self.globals.insert(name.clone(), val.clone());
                                    if import.is_const.get(i).copied().unwrap_or(false) {
                                        self.const_vars.insert(name.clone());
                                    }
                                    self.json_files.insert(name.clone(), json_path.clone());
                                }
                            } else {
                                for (name, _) in &import.names {
                                    builtin_modules.insert(name.clone(), format!("{}:{}", lang, source));
                                }
                                let lib_name = format!("yk_ffi_{}", import.source.replace('/', "_").replace('.', ""));
                                let ext = std::env::consts::DLL_EXTENSION;
                                let lib_path = std::path::Path::new("lib").join("ffi").join(format!("{}.{}", lib_name, ext));
                                if lib_path.exists() {
                                    match unsafe { libloading::Library::new(&lib_path) } {
                                        Ok(lib) => { self.ffi_libs.insert(lib_name, Arc::new(lib)); }
                                        Err(e) => { eprintln!("Warning: failed to load FFI library '{}': {}", lib_path.display(), e); }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for item in &module.items {
                match &item.value {
                    ItemKind::Fn { name, params, body, .. } => {
                        functions.insert(name.clone(), FnDef::new(params.clone(), body.clone()));
                    }
                    ItemKind::Struct { name, fields, .. } => {
                        struct_defs.insert(name.clone(), fields.iter().map(|p| p.name.clone()).collect());
                    }
                    ItemKind::Class { name, fields, methods, extends, constructor, init_body, is_data, .. } => {
                        let mut cls_methods: HashMap<String, FnDef> = HashMap::new();
                        for m in methods {
                            if let ItemKind::Fn { name: mname, params, body, .. } = m {
                                cls_methods.insert(mname.clone(), FnDef::new(params.clone(), body.clone()));
                            }
                        }
                        let mut all_fields: Vec<String> = Vec::new();
                        if let Some(parent) = &extends {
                            if let Some(parent_cls) = classes.get(parent) {
                                all_fields.extend(parent_cls.fields.clone());
                            }
                        }
                        all_fields.extend(constructor.iter().map(|p| p.name.clone()));
                        all_fields.extend(fields.iter().map(|p| p.name.clone()));
                        classes.insert(name.clone(), ClassDef {
                            fields: all_fields,
                            methods: cls_methods,
                            extends: extends.clone(),
                            constructor: Arc::new(constructor.clone()),
                            init_body: Arc::new(init_body.clone()),
                            is_data: *is_data,
                            method_cache: std::sync::Mutex::new(HashMap::new()),
                            inline_cache: std::sync::Mutex::new(Default::default()),
                        });
                    }
                    _ => {}
                }
            }
        }

        {
            let objects = Arc::make_mut(&mut self.objects);
            for item in &module.items {
                if let ItemKind::Object { name, fields, methods, .. } = &item.value {
                    let mut obj_methods: HashMap<String, FnDef> = HashMap::new();
                    for m in methods {
                        if let ItemKind::Fn { name: mname, params, body, .. } = m {
                            obj_methods.insert(mname.clone(), FnDef::new(params.clone(), body.clone()));
                        }
                    }
                    objects.insert(name.clone(), ObjectDef {
                        fields: fields.iter().map(|p| p.name.clone()).collect(),
                        methods: obj_methods,
                        init_body: Arc::new(Vec::new()),
                    });
                    let instance_fields = vec![Value::None_; fields.len()];
                    self.globals.insert(name.clone(), Value::Instance(name.clone(), instance_fields));
                }
            }
        }
        for item in &module.items {
            if let ItemKind::Object { init_body, .. } = &item.value {
                for s in init_body {
                    if let Ok(Some(_)) = self.exec_stmt(s) { break; }
                }
            }
        }

        for item in &module.items {
            if let ItemKind::Const { name, value, .. } = &item.value {
                let val = match self.eval_expr(value).unwrap_or(EvalResult::Value(Value::None_)) {
                    EvalResult::Value(v) => v,
                    EvalResult::Return(v) => v,
                };
                self.globals.insert(name.clone(), val);
                self.const_vars.insert(name.clone());
            }
        }
    }

    pub fn run_main(&mut self) -> Result<String> {
        const STACK_SIZE: usize = 8 * 1024 * 1024;
        let builder = std::thread::Builder::new()
            .name("interpreter".into())
            .stack_size(STACK_SIZE);
        let functions = self.functions.clone();
        let globals = self.globals.clone();
        let struct_defs = self.struct_defs.clone();
        let classes = self.classes.clone();
        let const_vars = self.const_vars.clone();
        let objects = self.objects.clone();
        let builtin_modules = self.builtin_modules.clone();
        let builtin_funcs = self.builtin_funcs.clone();
        let tui_mode = self.tui_mode;
        let std_imported = self.std_imported;
        let ffi_libs = self.ffi_libs.clone();
        let source_dir = self.source_dir.clone();
        let json_files = self.json_files.clone();
        let result = builder.spawn(move || {
            let mut interp = Interpreter {
                frames: vec![Frame::new()],
                frame_pool: Vec::new(),
                globals,
                const_vars,
                json_files,
                struct_defs,
                classes,
                functions,
                objects,
                builtin_modules,
                builtin_funcs,
                output: String::new(),
                tui_mode,
                std_imported,
                ffi_libs,
                moved_frames: vec![HashSet::new()],
                global_moved: HashSet::new(),
                next_task_id: 0,
                task_rxs: HashMap::new(),
                value_pool: crate::memory::arena::ValuePool::new(),
                source_dir,
            };
            let fndef = interp.functions.get("main").cloned();
            match fndef {
                Some(fn_def) => {
                    interp.push_frame();
                    let _result = interp.run_fn_body(&fn_def)?;
                    interp.pop_frame();
                    Ok(std::mem::replace(&mut interp.output, String::new()))
                }
                None => Err(error::err(ErrorKind::NameError, Span::new(0, 0),
                    "No 'main' function found")),
            }
        }).map_err(|e| error::err(ErrorKind::Internal, Span::new(0, 0),
            format!("Failed to spawn interpreter thread: {}", e)))?;
        result.join().map_err(|_| error::err(ErrorKind::Internal, Span::new(0, 0),
            "Interpreter thread panicked"))?
    }

    pub fn find_class_method(&self, cls_name: &str, method: &str) -> Option<FnDef> {
        let cls = self.classes.get(cls_name)?;
        {
            let ic = cls.inline_cache.lock().unwrap();
            for (ckey0, ckey1, cval) in ic.iter() {
                if ckey0 == cls_name && ckey1 == method {
                    return cval.clone();
                }
            }
        }
        let result = cls.methods.get(method).cloned()
            .or_else(|| {
                cls.extends.as_ref().and_then(|parent| {
                    self.classes.get(parent).and_then(|p| p.methods.get(method).cloned())
                })
            });
        let mut ic = cls.inline_cache.lock().unwrap();
        let idx = fast_hash(cls_name, method) % INLINE_CACHE_SIZE;
        ic[idx] = (cls_name.to_string(), method.to_string(), result.clone());
        result
    }

    pub fn run_init_blocks(&mut self, cls: &ClassDef, instance: Value, span: Span) -> Result<Value> {
        let mut instance = instance;
        if let Some(parent_name) = &cls.extends {
            if let Some(parent_cls) = self.classes.get(parent_name).cloned() {
                instance = self.run_init_blocks(&parent_cls, instance.clone(), span)?;
                self.push_frame();
                self.frames.last_mut().unwrap().insert("self".into(), instance.clone());
                for s in parent_cls.init_body.iter() {
                    if let Some(r) = self.exec_stmt(s)? {
                        self.pop_frame();
                        return match r {
                            Value::Result(false, _) => Ok(r),
                            _ => Err(self.err(span, "Init block should not return a value")),
                        };
                    }
                }
                instance = self.frames.last().and_then(|f| f.get("self")).cloned()
                    .unwrap_or(instance);
                self.pop_frame();
                // Fall through to run own init_body
            }
        }
        // Run the current class's own init blocks
        self.push_frame();
        self.frames.last_mut().unwrap().insert("self".into(), instance.clone());
        for s in cls.init_body.iter() {
            if let Some(r) = self.exec_stmt(s)? {
                self.pop_frame();
                return match r {
                    Value::Result(false, _) => Ok(r),
                    _ => Err(self.err(span, "Init block should not return a value")),
                };
            }
        }
        instance = self.frames.last().and_then(|f| f.get("self")).cloned()
            .unwrap_or(instance);
        self.pop_frame();
        Ok(instance)
    }

    pub fn is_moved(&self, name: &str) -> bool {
        if let Some(current) = self.moved_frames.last() {
            if current.contains(name) { return true; }
        }
        if self.global_moved.contains(name) { return true; }
        false
    }

    pub fn mark_moved_name(&mut self, name: &str) {
        if let Some(current) = self.moved_frames.last_mut() {
            current.insert(name.to_string());
        } else {
            self.global_moved.insert(name.to_string());
        }
    }

    pub fn mark_moved_expr(&mut self, expr: &ExprNode, val: &Value) {
        if is_copy_value(val) { return; }
        match &expr.value {
            Expr::Ident(src) => { self.mark_moved_name(src); }
            Expr::Field(obj, _) => {
                if let Expr::Ident(src) = &obj.value { self.mark_moved_name(src); }
            }
            _ => {}
        }
    }

    pub fn get_var(&self, name: &str) -> Result<Value> {
        if name == "v" {
            eprintln!("DEBUG get_var: name={}, frames={}", name, self.frames.len());
            for (i, f) in self.frames.iter().enumerate() {
                eprintln!("DEBUG get_var: frame[{}] keys: {:?}", i, f.vars.keys().collect::<Vec<_>>());
            }
        }
        for frame in self.frames.iter().rev() {
            if let Some(val) = frame.get(name) {
                return Ok(val.clone());
            }
        }
        self.globals.get(name).cloned()
            .ok_or_else(|| self.err(Span::new(0, 0), format!("Variable '{}' not found", name)))
    }

    pub fn is_const_global(&self, name: &str) -> bool {
        self.const_vars.contains(name)
    }

    pub fn set_var(&mut self, name: &str, val: Value) -> Result<()> {
        if name == "v" {
            eprintln!("DEBUG set_var: name={}, val={:?}, frames={}, top_frame_keys={:?}", name, val, self.frames.len(), self.frames.last().map(|f| f.vars.keys().collect::<Vec<_>>()));
        }
        if self.const_vars.contains(name) || self.global_moved.contains(name) {
            return Err(self.err(Span::new(0, 0), format!("Cannot assign to const or moved variable '{}'", name)));
        }
        for frame in self.frames.iter_mut().rev() {
            if frame.contains_key(name) {
                frame.insert(name.to_string(), val);
                return Ok(());
            }
        }
        if self.globals.contains_key(name) {
            self.globals.insert(name.to_string(), val);
            return Ok(());
        }
        if let Some(current) = self.frames.last_mut() {
            current.insert(name.to_string(), val);
            return Ok(());
        }
        self.globals.insert(name.to_string(), val);
        Ok(())
    }

    pub fn push_frame(&mut self) {
        if let Some(mut pooled) = self.frame_pool.pop() {
            pooled.vars.clear();
            self.frames.push(pooled);
        } else {
            self.frames.push(Frame::new());
        }
        self.moved_frames.push(HashSet::new());
    }

    pub fn pop_frame(&mut self) {
        if let Some(frame) = self.frames.pop() {
            self.frame_pool.push(frame);
        }
        self.moved_frames.pop();
    }

    pub fn err(&self, span: Span, msg: impl Into<String>) -> error::YkError {
        error::err(ErrorKind::Runtime, span, msg)
    }
}

pub fn fast_hash(a: &str, b: &str) -> usize {
    let h = a.len().wrapping_mul(31).wrapping_add(b.len());
    h
}
