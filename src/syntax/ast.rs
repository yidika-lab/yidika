use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::diagnostics::span::Span;

pub type AstId = usize;
pub type TypeNode = Node<TypeExpr>;
pub type ExprNode = Node<Expr>;
pub type StmtNode = Node<Stmt>;
pub type ItemNode = Node<ItemKind>;

#[derive(Debug, Clone)]
pub struct Node<T> {
    pub id: AstId,
    pub span: Span,
    pub value: T,
    pub decorators: Vec<String>,
}

impl<T> Node<T> {
    pub fn new(id: AstId, span: Span, value: T) -> Self {
        Node { id, span, value, decorators: vec![] }
    }
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

pub fn fresh_id() -> AstId {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

pub fn reset_ids() {
    NEXT_ID.store(0, Ordering::Relaxed);
}

// ─── Types ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Named(String),
    Int(u8),
    Rint(u8),
    Real(u8),
    Bool, Str, Symbol,
    Complex(Box<TypeExpr>, Box<TypeExpr>),
    Vector(Box<TypeExpr>),
    Matrix(Box<TypeExpr>),
    List(Box<TypeExpr>),
    Set(Box<TypeExpr>),
    Map(Box<TypeExpr>, Box<TypeExpr>),
    Union(Vec<TypeExpr>),
    Tuple(Vec<TypeExpr>),
    Fn(Vec<TypeExpr>, Box<TypeExpr>),
    Generic(String, Vec<TypeExpr>),
    Nullable(Box<TypeExpr>),
    Const(Box<TypeExpr>),
    Null, None_, Infer,
}

impl fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeExpr::Named(s) => write!(f, "{}", s),
            TypeExpr::Int(w) => write!(f, "int({})", w),
            TypeExpr::Rint(w) => write!(f, "rint({})", w),
            TypeExpr::Real(w) => write!(f, "real({})", w),
            TypeExpr::Bool => write!(f, "bool"),
            TypeExpr::Str => write!(f, "str"),
            TypeExpr::Symbol => write!(f, "symbol"),
            TypeExpr::Complex(r, i) => write!(f, "complex[{}, {}]", r, i),
            TypeExpr::Vector(t) => write!(f, "vector<{}>", t),
            TypeExpr::Matrix(t) => write!(f, "matrix<{}>", t),
            TypeExpr::List(t) => write!(f, "list<{}>", t),
            TypeExpr::Set(t) => write!(f, "set<{}>", t),
            TypeExpr::Map(k, v) => write!(f, "map<{}, {}>", k, v),
            TypeExpr::Union(ts) => {
                let strs: Vec<String> = ts.iter().map(|t| t.to_string()).collect();
                write!(f, "union({})", strs.join(", "))
            }
            TypeExpr::Tuple(ts) => {
                let strs: Vec<String> = ts.iter().map(|t| t.to_string()).collect();
                write!(f, "({})", strs.join(", "))
            }
            TypeExpr::Fn(params, ret) => {
                let strs: Vec<String> = params.iter().map(|t| t.to_string()).collect();
                write!(f, "fn({}) -> {}", strs.join(", "), ret)
            }
            TypeExpr::Generic(name, args) => {
                let strs: Vec<String> = args.iter().map(|t| t.to_string()).collect();
                write!(f, "{}<{}>", name, strs.join(", "))
            }
            TypeExpr::Null => write!(f, "null"),
            TypeExpr::None_ => write!(f, "null"),
            TypeExpr::Infer => write!(f, "auto"),
            TypeExpr::Nullable(inner) => write!(f, "{}?", inner),
            TypeExpr::Const(inner) => write!(f, "const<{}>", inner),
        }
    }
}

// ─── Expressions ─────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Expr {
    LitInt(i64), LitHex(i64), LitReal(f64),
    LitStr(String), LitChar(char), LitBool(bool),
    LitSymbol(String), LitNull, LitNone,
    Ident(String),
    BinOp(Box<ExprNode>, BinOp, Box<ExprNode>),
    UnOp(UnOp, Box<ExprNode>),
    Call(Box<ExprNode>, Vec<ExprNode>),
    Index(Box<ExprNode>, Box<ExprNode>),
    Field(Box<ExprNode>, String),
    Block(Vec<StmtNode>),
    If(Box<ExprNode>, Box<ExprNode>, Option<Box<ExprNode>>),
    ForIn(String, Box<ExprNode>, Box<ExprNode>),
    While(Box<ExprNode>, Box<ExprNode>),
    Loop(Box<ExprNode>),
    Range(Box<ExprNode>, Box<ExprNode>),
    Match(Box<ExprNode>, Vec<MatchArm>),
    StructLit(String, Vec<(String, ExprNode)>),
    ListLit(Vec<ExprNode>),
    SetLit(Vec<ExprNode>),
    MapLit(Vec<(ExprNode, ExprNode)>),
    TupleLit(Vec<ExprNode>),
    VectorLit(Vec<ExprNode>),
    MatrixLit(Vec<Vec<ExprNode>>),
    FnLit(Vec<Param>, Option<TypeNode>, Box<ExprNode>),
    Closure(Vec<Param>, Box<ExprNode>),
    LitComplex(Box<ExprNode>, Box<ExprNode>),
    PostInc(Box<ExprNode>),
    PostDec(Box<ExprNode>),
    Await(Box<ExprNode>),
    Spawn(Box<ExprNode>),
    ResultOk(Box<ExprNode>),
    ResultErr(Box<ExprNode>),
    Try(Box<ExprNode>),
    TryCatch(Vec<StmtNode>, String, Vec<StmtNode>),
    Variant(String, String, Vec<ExprNode>),
    SafeCall(Box<ExprNode>, String),
    Elvis(Box<ExprNode>, Box<ExprNode>),
    AsConst(Box<ExprNode>),
    As(Box<ExprNode>, TypeNode),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp { Add, Sub, Mul, Div, Mod, Pow, BitAnd, BitOr, BitXor, Shl, Shr, Eq, Ne, Lt, Gt, Le, Ge, And, Or, Assign }

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinOp::Add => write!(f, "+"),
            BinOp::Sub => write!(f, "-"),
            BinOp::Mul => write!(f, "*"),
            BinOp::Div => write!(f, "/"),
            BinOp::Mod => write!(f, "%"),
            BinOp::Pow => write!(f, "**"),
            BinOp::BitAnd => write!(f, "&"),
            BinOp::BitOr => write!(f, "|"),
            BinOp::BitXor => write!(f, "^"),
            BinOp::Shl => write!(f, "<<"),
            BinOp::Shr => write!(f, ">>"),
            BinOp::Eq => write!(f, "=="),
            BinOp::Ne => write!(f, "!="),
            BinOp::Lt => write!(f, "<"),
            BinOp::Gt => write!(f, ">"),
            BinOp::Le => write!(f, "<="),
            BinOp::Ge => write!(f, ">="),
            BinOp::And => write!(f, "&&"),
            BinOp::Or => write!(f, "||"),
            BinOp::Assign => write!(f, "="),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp { Neg, Not, BitNot }

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<ExprNode>,
    pub body: ExprNode,
}

// ─── Statements ──────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Stmt {
    Decl { name: String, type_expr: Option<TypeNode>, value: ExprNode, is_const: bool },
    Expr(ExprNode),
    Return(Option<ExprNode>),
    For(String, ExprNode, Vec<StmtNode>, bool), // bool = true for for-of (colon), false for for-in (in)
    While(ExprNode, Vec<StmtNode>),
    Loop(Vec<StmtNode>),
    If(ExprNode, Vec<StmtNode>, Option<Vec<StmtNode>>),
    Assign(String, ExprNode),
    Destruct(Pattern, ExprNode),
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Ident(String),
    Rest(String),
    Destruct(Vec<(String, Pattern)>),
    ListDestruct(Vec<Pattern>),
    LitInt(i64),
    LitReal(f64),
    LitStr(String),
    LitBool(bool),
    Variant(String, Vec<Pattern>),
    Ignore,
}

// ─── Parameters ──────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub type_expr: TypeNode,
    pub is_ref: bool,
}

// ─── Items (top-level) ───────────────────────────────────

#[derive(Debug, Clone)]
pub struct InterfaceMethod {
    pub name: String,
    pub params: Vec<Param>,
    pub ret_type: Option<TypeNode>,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<Param>,
}

#[derive(Debug, Clone)]
pub enum ItemKind {
    Fn { name: String, params: Vec<Param>, ret_type: Option<TypeNode>, body: Vec<StmtNode>, is_async: bool, generics: Vec<String>, is_open: bool, is_override: bool, is_final: bool, is_abstract_method: bool },
    Struct { name: String, fields: Vec<Param>, generics: Vec<String> },
    Class { name: String, extends: Option<String>, super_args: Vec<ExprNode>, implements: Vec<String>, fields: Vec<Param>, methods: Vec<ItemKind>, generics: Vec<String>, constructor: Vec<Param>, init_body: Vec<StmtNode>, is_open: bool, is_abstract: bool, is_data: bool },
    Interface { name: String, methods: Vec<InterfaceMethod> },
    Enum { name: String, variants: Vec<EnumVariant> },
    Object { name: String, fields: Vec<Param>, methods: Vec<ItemKind>, init_body: Vec<StmtNode> },
    Union { name: String, variants: Vec<Param> },
    TypeAlias { name: String, type_expr: TypeNode },
    Const { name: String, type_expr: TypeNode, value: ExprNode },
}

// ─── Top-level ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Import {
    pub span: Span,
    pub names: Vec<(String, Option<String>)>,
    pub is_const: Vec<bool>,
    pub source: String,
    pub lang: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub span: Span,
    pub imports: Vec<Import>,
    pub exports: Vec<String>,
    pub items: Vec<ItemNode>,
}

pub fn substitute_type(te: &TypeExpr, type_args: &HashMap<String, TypeExpr>) -> TypeExpr {
    match te {
        TypeExpr::Named(name) => {
            type_args.get(name).cloned().unwrap_or_else(|| te.clone())
        }
        TypeExpr::Generic(name, args) => {
            TypeExpr::Generic(name.clone(), args.iter().map(|a| substitute_type(a, type_args)).collect())
        }
        TypeExpr::List(inner) => TypeExpr::List(Box::new(substitute_type(inner, type_args))),
        TypeExpr::Set(inner) => TypeExpr::Set(Box::new(substitute_type(inner, type_args))),
        TypeExpr::Vector(inner) => TypeExpr::Vector(Box::new(substitute_type(inner, type_args))),
        TypeExpr::Matrix(inner) => TypeExpr::Matrix(Box::new(substitute_type(inner, type_args))),
        TypeExpr::Complex(r, i) => TypeExpr::Complex(
            Box::new(substitute_type(r, type_args)),
            Box::new(substitute_type(i, type_args)),
        ),
        TypeExpr::Map(k, v) => TypeExpr::Map(
            Box::new(substitute_type(k, type_args)),
            Box::new(substitute_type(v, type_args)),
        ),
        TypeExpr::Union(variants) => TypeExpr::Union(
            variants.iter().map(|v| substitute_type(v, type_args)).collect()
        ),
        TypeExpr::Tuple(elems) => TypeExpr::Tuple(
            elems.iter().map(|e| substitute_type(e, type_args)).collect()
        ),
        TypeExpr::Fn(params, ret) => TypeExpr::Fn(
            params.iter().map(|p| substitute_type(p, type_args)).collect(),
            Box::new(substitute_type(ret, type_args)),
        ),
        TypeExpr::Nullable(inner) => TypeExpr::Nullable(Box::new(substitute_type(inner, type_args))),
        _ => te.clone(),
    }
}

pub fn mangle_struct_name(name: &str, args: &[TypeExpr]) -> String {
    let mut s = name.to_string();
    for a in args {
        let mangled_arg = match a {
            TypeExpr::Int(0) => "int".into(),
            TypeExpr::Int(w) => format!("int{}", w),
            TypeExpr::Rint(0) => "rint".into(),
            TypeExpr::Rint(w) => format!("rint{}", w),
            TypeExpr::Real(0) => "real".into(),
            TypeExpr::Real(w) => format!("real{}", w),
            TypeExpr::Bool => "bool".into(),
            TypeExpr::Str => "str".into(),
            TypeExpr::Symbol => "sym".into(),
            TypeExpr::None_ => "none".into(),
            TypeExpr::Null => "null".into(),
            TypeExpr::Named(n) => n.clone(),
            TypeExpr::Generic(inner_name, inner_args) => mangle_struct_name(inner_name, inner_args),
            TypeExpr::List(inner) => format!("List-{}", mangle_struct_name("", &[inner.as_ref().clone()]).trim_start_matches('-')),
            TypeExpr::Set(inner) => format!("Set-{}", mangle_struct_name("", &[inner.as_ref().clone()]).trim_start_matches('-')),
            TypeExpr::Map(k, v) => format!("Map-{}-{}", mangle_struct_name("", &[k.as_ref().clone()]).trim_start_matches('-'), mangle_struct_name("", &[v.as_ref().clone()]).trim_start_matches('-')),
            TypeExpr::Vector(inner) => format!("Vec-{}", mangle_struct_name("", &[inner.as_ref().clone()]).trim_start_matches('-')),
            TypeExpr::Matrix(inner) => format!("Mat-{}", mangle_struct_name("", &[inner.as_ref().clone()]).trim_start_matches('-')),
            TypeExpr::Nullable(inner) => format!("Opt-{}", mangle_struct_name("", &[inner.as_ref().clone()]).trim_start_matches('-')),
            _ => {
                let s = a.to_string();
                s.chars().filter(|c| c.is_alphanumeric()).collect::<String>()
            }
        };
        s.push_str(&format!("-{}", mangled_arg));
    }
    s
}

pub fn substitute_type_node(tn: &TypeNode, type_args: &HashMap<String, TypeExpr>) -> TypeNode {
    Node { id: tn.id, span: tn.span, value: substitute_type(&tn.value, type_args), decorators: tn.decorators.clone() }
}

pub fn substitute_in_expr(expr: &ExprNode, type_args: &HashMap<String, TypeExpr>) -> ExprNode {
    let new_value = match &expr.value {
        Expr::As(inner, target_type) => Expr::As(
            Box::new(substitute_in_expr(inner, type_args)),
            substitute_type_node(target_type, type_args),
        ),
        Expr::AsConst(inner) => Expr::AsConst(Box::new(substitute_in_expr(inner, type_args))),
        Expr::BinOp(l, op, r) => Expr::BinOp(
            Box::new(substitute_in_expr(l, type_args)),
            *op,
            Box::new(substitute_in_expr(r, type_args)),
        ),
        Expr::UnOp(op, i) => Expr::UnOp(*op, Box::new(substitute_in_expr(i, type_args))),
        Expr::Call(callee, args) => Expr::Call(
            Box::new(substitute_in_expr(callee, type_args)),
            args.iter().map(|a| substitute_in_expr(a, type_args)).collect(),
        ),
        Expr::Block(stmts) => Expr::Block(
            stmts.iter().map(|s| substitute_in_stmt(s, type_args)).collect(),
        ),
        Expr::If(c, t, e) => Expr::If(
            Box::new(substitute_in_expr(c, type_args)),
            Box::new(substitute_in_expr(t, type_args)),
            e.as_ref().map(|x| Box::new(substitute_in_expr(x, type_args))),
        ),
        Expr::ForIn(name, iter, body) => Expr::ForIn(
            name.clone(),
            Box::new(substitute_in_expr(iter, type_args)),
            Box::new(substitute_in_expr(body, type_args)),
        ),
        Expr::While(cond, body) => Expr::While(
            Box::new(substitute_in_expr(cond, type_args)),
            Box::new(substitute_in_expr(body, type_args)),
        ),
        Expr::Loop(body) => Expr::Loop(Box::new(substitute_in_expr(body, type_args))),
        Expr::Range(l, r) => Expr::Range(
            Box::new(substitute_in_expr(l, type_args)),
            Box::new(substitute_in_expr(r, type_args)),
        ),
        Expr::Index(container, index) => Expr::Index(
            Box::new(substitute_in_expr(container, type_args)),
            Box::new(substitute_in_expr(index, type_args)),
        ),
        Expr::Field(obj, name) => Expr::Field(Box::new(substitute_in_expr(obj, type_args)), name.clone()),
        Expr::SafeCall(obj, name) => Expr::SafeCall(Box::new(substitute_in_expr(obj, type_args)), name.clone()),
        Expr::Elvis(a, b) => Expr::Elvis(
            Box::new(substitute_in_expr(a, type_args)),
            Box::new(substitute_in_expr(b, type_args)),
        ),
        Expr::StructLit(name, fields) => Expr::StructLit(
            name.clone(),
            fields.iter().map(|(n, e)| (n.clone(), substitute_in_expr(e, type_args))).collect(),
        ),
        Expr::ListLit(items) => Expr::ListLit(
            items.iter().map(|i| substitute_in_expr(i, type_args)).collect(),
        ),
        Expr::SetLit(items) => Expr::SetLit(
            items.iter().map(|i| substitute_in_expr(i, type_args)).collect(),
        ),
        Expr::MapLit(pairs) => Expr::MapLit(
            pairs.iter().map(|(k, v)| (substitute_in_expr(k, type_args), substitute_in_expr(v, type_args))).collect(),
        ),
        Expr::TupleLit(items) => Expr::TupleLit(
            items.iter().map(|i| substitute_in_expr(i, type_args)).collect(),
        ),
        Expr::VectorLit(items) => Expr::VectorLit(
            items.iter().map(|i| substitute_in_expr(i, type_args)).collect(),
        ),
        Expr::MatrixLit(rows) => Expr::MatrixLit(
            rows.iter().map(|r| r.iter().map(|i| substitute_in_expr(i, type_args)).collect()).collect(),
        ),
        Expr::FnLit(params, ret_type, body) => Expr::FnLit(
            params.iter().map(|p| Param {
                name: p.name.clone(),
                type_expr: substitute_type_node(&p.type_expr, type_args),
                is_ref: p.is_ref,
            }).collect(),
            ret_type.as_ref().map(|rt| substitute_type_node(rt, type_args)),
            Box::new(substitute_in_expr(body, type_args)),
        ),
        Expr::Closure(params, body) => Expr::Closure(
            params.iter().map(|p| Param {
                name: p.name.clone(),
                type_expr: substitute_type_node(&p.type_expr, type_args),
                is_ref: p.is_ref,
            }).collect(),
            Box::new(substitute_in_expr(body, type_args)),
        ),
        Expr::LitComplex(r, im) => Expr::LitComplex(
            Box::new(substitute_in_expr(r, type_args)),
            Box::new(substitute_in_expr(im, type_args)),
        ),
        Expr::PostInc(i) => Expr::PostInc(Box::new(substitute_in_expr(i, type_args))),
        Expr::PostDec(i) => Expr::PostDec(Box::new(substitute_in_expr(i, type_args))),
        Expr::Await(i) => Expr::Await(Box::new(substitute_in_expr(i, type_args))),
        Expr::Spawn(i) => Expr::Spawn(Box::new(substitute_in_expr(i, type_args))),
        Expr::Try(i) => Expr::Try(Box::new(substitute_in_expr(i, type_args))),
        Expr::TryCatch(try_body, var, catch_body) => Expr::TryCatch(
            try_body.iter().map(|s| substitute_in_stmt(s, type_args)).collect(),
            var.clone(),
            catch_body.iter().map(|s| substitute_in_stmt(s, type_args)).collect(),
        ),
        Expr::ResultOk(i) => Expr::ResultOk(Box::new(substitute_in_expr(i, type_args))),
        Expr::ResultErr(i) => Expr::ResultErr(Box::new(substitute_in_expr(i, type_args))),
        Expr::Match(expr, arms) => Expr::Match(
            Box::new(substitute_in_expr(expr, type_args)),
            arms.iter().map(|arm| MatchArm {
                pattern: arm.pattern.clone(),
                guard: arm.guard.as_ref().map(|g| substitute_in_expr(g, type_args)),
                body: substitute_in_expr(&arm.body, type_args),
            }).collect(),
        ),
        Expr::Variant(ename, vname, args) => Expr::Variant(
            ename.clone(),
            vname.clone(),
            args.iter().map(|a| substitute_in_expr(a, type_args)).collect(),
        ),
        _ => expr.value.clone(),
    };
    Node { id: expr.id, span: expr.span, value: new_value, decorators: expr.decorators.clone() }
}

pub fn substitute_in_stmt(stmt: &StmtNode, type_args: &HashMap<String, TypeExpr>) -> StmtNode {
    let new_value = match &stmt.value {
        Stmt::Decl { name, type_expr, value, is_const } => Stmt::Decl {
            name: name.clone(),
            type_expr: type_expr.as_ref().map(|te| substitute_type_node(te, type_args)),
            value: substitute_in_expr(value, type_args),
            is_const: *is_const,
        },
        Stmt::Expr(e) => Stmt::Expr(substitute_in_expr(e, type_args)),
        Stmt::Return(e) => Stmt::Return(e.as_ref().map(|x| substitute_in_expr(x, type_args))),
        Stmt::For(name, iter, body, is_for_of) => Stmt::For(
            name.clone(),
            substitute_in_expr(iter, type_args),
            body.iter().map(|s| substitute_in_stmt(s, type_args)).collect(),
            *is_for_of,
        ),
        Stmt::While(cond, body) => Stmt::While(
            substitute_in_expr(cond, type_args),
            body.iter().map(|s| substitute_in_stmt(s, type_args)).collect(),
        ),
        Stmt::Loop(body) => Stmt::Loop(
            body.iter().map(|s| substitute_in_stmt(s, type_args)).collect(),
        ),
        Stmt::If(cond, then_body, else_body) => Stmt::If(
            substitute_in_expr(cond, type_args),
            then_body.iter().map(|s| substitute_in_stmt(s, type_args)).collect(),
            else_body.as_ref().map(|eb| eb.iter().map(|s| substitute_in_stmt(s, type_args)).collect()),
        ),
        Stmt::Assign(name, expr) => Stmt::Assign(name.clone(), substitute_in_expr(expr, type_args)),
        Stmt::Destruct(pattern, expr) => Stmt::Destruct(pattern.clone(), substitute_in_expr(expr, type_args)),
    };
    Node { id: stmt.id, span: stmt.span, value: new_value, decorators: stmt.decorators.clone() }
}
