use std::collections::HashMap;
use std::sync::Arc;
use crate::syntax::ast::*;
use super::value::Value;

pub type MethodCache = std::sync::Mutex<HashMap<(String, String), Option<FnDef>>>;

pub const INLINE_CACHE_SIZE: usize = 4;
pub type InlineCache = std::sync::Mutex<[(String, String, Option<FnDef>); INLINE_CACHE_SIZE]>;

#[derive(Debug, Clone)]
pub struct FnDef {
    pub params: Arc<Vec<Param>>,
    pub body: Arc<Vec<StmtNode>>,
}

impl FnDef {
    pub fn new(params: Vec<Param>, body: Vec<StmtNode>) -> Self {
        FnDef { params: Arc::new(params), body: Arc::new(body) }
    }
}

#[derive(Debug)]
pub struct ClassDef {
    pub fields: Vec<String>,
    pub methods: HashMap<String, FnDef>,
    pub extends: Option<String>,
    pub constructor: Arc<Vec<Param>>,
    pub init_body: Arc<Vec<StmtNode>>,
    pub is_data: bool,
    pub method_cache: MethodCache,
    pub inline_cache: InlineCache,
}

impl Clone for ClassDef {
    fn clone(&self) -> Self {
        ClassDef {
            fields: self.fields.clone(),
            methods: self.methods.clone(),
            extends: self.extends.clone(),
            constructor: self.constructor.clone(),
            init_body: self.init_body.clone(),
            is_data: self.is_data,
            method_cache: std::sync::Mutex::new(
                self.method_cache.lock().unwrap().clone()
            ),
            inline_cache: std::sync::Mutex::new(
                self.inline_cache.lock().unwrap().clone()
            ),
        }
    }
}

impl ClassDef {
}

#[derive(Debug, Clone)]
pub struct ObjectDef {
    pub fields: Vec<String>,
    pub methods: HashMap<String, FnDef>,
    #[allow(dead_code)]
    pub init_body: Arc<Vec<StmtNode>>,
}

pub struct Frame {
    pub vars: HashMap<String, Value>,
}

impl Frame {
    pub fn new() -> Self {
        Frame { vars: HashMap::new() }
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.vars.get(name)
    }

    pub fn insert(&mut self, name: String, val: Value) {
        self.vars.insert(name, val);
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.vars.contains_key(name)
    }
}
