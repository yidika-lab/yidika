use std::collections::HashMap;
use crate::syntax::ast::TypeExpr;

#[derive(Debug, Clone)]
pub struct FnSig {
    pub params: Vec<TypeExpr>,
    pub ret_type: TypeExpr,
}

#[derive(Debug, Clone)]
pub struct Env {
    pub(crate) types: HashMap<String, TypeExpr>,
    pub(crate) fns: HashMap<String, FnSig>,
}

impl Env {
    pub fn new() -> Self {
        let mut types = HashMap::new();
        types.insert("int".into(), TypeExpr::Int(0));
        types.insert("rint".into(), TypeExpr::Rint(0));
        types.insert("real".into(), TypeExpr::Real(0));
        types.insert("complex".into(), TypeExpr::Complex(Box::new(TypeExpr::Real(0)), Box::new(TypeExpr::Real(0))));
        types.insert("bool".into(), TypeExpr::Bool);
        types.insert("str".into(), TypeExpr::Str);
        types.insert("symbol".into(), TypeExpr::Symbol);
        Self { types, fns: HashMap::new() }
    }

    pub fn add_type(&mut self, name: String, ty: TypeExpr) {
        self.types.insert(name, ty);
    }

    pub fn add_fn(&mut self, name: String, sig: FnSig) {
        self.fns.insert(name, sig);
    }

    pub fn get_type(&self, name: &str) -> Option<&TypeExpr> {
        self.types.get(name)
    }

    pub fn get_fn(&self, name: &str) -> Option<&FnSig> {
        self.fns.get(name)
    }
}
