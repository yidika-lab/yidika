use std::collections::HashMap;
use crate::syntax::ast::{TypeExpr, Param};

#[derive(Debug, Clone)]
pub struct FnSig {
    pub params: Vec<TypeExpr>,
    pub ret_type: TypeExpr,
    pub self_is_ref: bool,
}

#[derive(Debug, Clone)]
pub struct InterfaceDef {
    pub methods: HashMap<String, FnSig>,
}

#[derive(Debug, Clone)]
pub struct InterfaceMethodSig {
    pub name: String,
    pub sig: FnSig,
}

#[derive(Debug, Clone)]
pub struct MethodDef {
    pub params: Vec<Param>,
    pub ret_type: TypeExpr,
    pub generics: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ClassDef {
    pub fields: Vec<Param>,
    pub methods: HashMap<String, MethodDef>,
    pub generics: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub fields: Vec<Param>,
    pub generics: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Env {
    pub(crate) types: HashMap<String, TypeExpr>,
    pub(crate) fns: HashMap<String, FnSig>,
    pub(crate) interfaces: HashMap<String, InterfaceDef>,
    pub(crate) class_interfaces: HashMap<String, Vec<String>>,
    pub(crate) classes: HashMap<String, ClassDef>,
    pub(crate) structs: HashMap<String, StructDef>,
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
        Self { types, fns: HashMap::new(), interfaces: HashMap::new(), class_interfaces: HashMap::new(), classes: HashMap::new(), structs: HashMap::new() }
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

    pub fn add_interface(&mut self, name: String, def: InterfaceDef) {
        self.interfaces.insert(name, def);
    }

    pub fn get_interface(&self, name: &str) -> Option<&InterfaceDef> {
        self.interfaces.get(name)
    }

    pub fn add_class_interface(&mut self, cls_name: String, iface_name: String) {
        self.class_interfaces.entry(cls_name).or_default().push(iface_name);
    }

    pub fn get_class_interfaces(&self, cls_name: &str) -> Option<&Vec<String>> {
        self.class_interfaces.get(cls_name)
    }

    pub fn add_class(&mut self, name: String, def: ClassDef) {
        self.classes.insert(name, def);
    }

    pub fn get_class(&self, name: &str) -> Option<&ClassDef> {
        self.classes.get(name)
    }

    pub fn add_struct(&mut self, name: String, def: StructDef) {
        self.structs.insert(name, def);
    }

    pub fn get_struct(&self, name: &str) -> Option<&StructDef> {
        self.structs.get(name)
    }
}
