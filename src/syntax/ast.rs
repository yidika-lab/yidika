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
    pub exported: bool,
}

impl<T> Node<T> {
    pub fn new(id: AstId, span: Span, value: T) -> Self {
        Self { id, span, value, exported: false }
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
    Bool, Str, Symbol, Complex,
    Vector(Box<TypeExpr>),
    Matrix(Box<TypeExpr>),
    List(Box<TypeExpr>),
    Set(Box<TypeExpr>),
    Map(Box<TypeExpr>, Box<TypeExpr>),
    Union(Vec<TypeExpr>),
    Tuple(Vec<TypeExpr>),
    Fn(Vec<TypeExpr>, Box<TypeExpr>),
    Generic(String, Vec<TypeExpr>),
    Null, None_, Infer,
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
    FnLit(Vec<Param>, Option<TypeNode>, Box<ExprNode>),
    Await(Box<ExprNode>),
    Spawn(Box<ExprNode>),
    ResultOk(Box<ExprNode>),
    ResultErr(Box<ExprNode>),
    AsConst(Box<ExprNode>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp { Add, Sub, Mul, Div, Eq, Ne, Lt, Gt, Le, Ge, And, Or, Assign }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp { Neg, Not }

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
    For(String, ExprNode, Vec<StmtNode>),
    While(ExprNode, Vec<StmtNode>),
    Loop(Vec<StmtNode>),
    If(ExprNode, Vec<StmtNode>, Option<Vec<StmtNode>>),
    Assign(String, ExprNode),
    Destruct(Pattern, ExprNode),
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Ident(String),
    Destruct(Vec<(String, Pattern)>),
    ListDestruct(Vec<Pattern>),
    Ignore,
}

// ─── Parameters ──────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub type_expr: TypeNode,
}

// ─── Items (top-level) ───────────────────────────────────

#[derive(Debug, Clone)]
pub enum ItemKind {
    Fn { name: String, params: Vec<Param>, ret_type: Option<TypeNode>, body: Vec<StmtNode>, is_async: bool, generics: Vec<String> },
    Struct { name: String, fields: Vec<Param>, generics: Vec<String> },
    Class { name: String, extends: Option<String>, implements: Vec<String>, fields: Vec<Param>, methods: Vec<ItemKind>, generics: Vec<String> },
    Interface { name: String, methods: Vec<Param> },
    Union { name: String, variants: Vec<Param> },
    TypeAlias { name: String, type_expr: TypeNode },
    Const { name: String, type_expr: TypeNode, value: ExprNode },
}

// ─── Top-level ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Import {
    pub span: Span,
    pub names: Vec<(String, Option<String>)>,
    pub source: String,
    pub lang: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Module {
    pub span: Span,
    pub imports: Vec<Import>,
    pub exports: Vec<String>,
    pub items: Vec<ItemNode>,
}
