use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::syntax::ast::*;
use crate::syntax::lexer;
use crate::syntax::token::Token;

pub struct Parser {
    tokens: Vec<(Token, Span)>,
    pos: usize,
    source: String,
}

impl Parser {
    pub fn parse(source: &str) -> Result<Module> {
        let mut tokens = lexer::lex(source);
        tokens.push((Token::Eof, Span::new(source.len(), source.len())));
        let mut p = Parser { tokens, pos: 0, source: source.to_string() };
        p.module()
    }

    fn peek(&self) -> &(Token, Span) { &self.tokens[self.pos] }
    fn tok(&self) -> &Token { &self.peek().0 }
    fn span(&self) -> Span { self.peek().1 }

    fn advance(&mut self) -> (Token, Span) {
        let t = self.peek().clone();
        self.pos += 1;
        t
    }

    fn eat(&mut self, expected: Token) -> Result<Span> {
        let (ref t, s) = self.peek().clone();
        if tokens_match(t, &expected) { self.pos += 1; Ok(s) }
        else { Err(error::err(ErrorKind::Syntax, s, format!("Expected {:?}, found {:?}", expected, t))) }
    }

    fn ident(&mut self) -> Result<String> {
        match self.tok() {
            Token::Ident(s) => { let s = s.clone(); self.pos += 1; Ok(s) }
            t => Err(error::err(ErrorKind::Syntax, self.span(), format!("Expected identifier, found {:?}", t)))
        }
    }

    fn field_ident(&mut self) -> Result<String> {
        let (tok, span) = self.peek().clone();
        let s = match &tok {
            Token::Ident(s) => s.clone(),
            Token::Match => "match".into(),
            Token::Fn => "fn".into(),
            Token::If => "if".into(),
            Token::Else => "else".into(),
            Token::For => "for".into(),
            Token::In => "in".into(),
            Token::While => "while".into(),
            Token::Loop => "loop".into(),
            Token::Return => "return".into(),
            Token::Struct => "struct".into(),
            Token::Class => "class".into(),
            Token::Interface => "interface".into(),
            Token::Union => "union".into(),
            Token::Type => "type".into(),
            Token::Use => "use".into(),
            Token::Export => "export".into(),
            Token::As => "as".into(),
            Token::From => "from".into(),
            Token::Async => "async".into(),
            Token::Await => "await".into(),
            Token::Spawn => "spawn".into(),
            Token::Super => "super".into(),
            Token::Mut => "mut".into(),
            Token::Ref => "ref".into(),
            Token::True => "true".into(),
            Token::False => "false".into(),
            Token::None | Token::Null => "none".into(),
            Token::OkKw => "Ok".into(),
            Token::ErrorKw => "Error".into(),
            Token::Const => "const".into(),
            Token::IntLit(s) => s.clone(),
            _ => return Err(error::err(ErrorKind::Syntax, span, format!("Expected identifier, found {:?}", tok))),
        };
        self.pos += 1;
        Ok(s)
    }

    fn str_lit(&mut self) -> Result<String> {
        match self.tok() {
            Token::StrLit(s) => { let v = s[1..s.len()-1].to_string(); self.pos += 1; Ok(v) }
            t => Err(error::err(ErrorKind::Syntax, self.span(), format!("Expected string, found {:?}", t)))
        }
    }

    // ─── Module ───────────────────────────────────────

    fn module(&mut self) -> Result<Module> {
        let mut imports = Vec::new();
        let mut exports = Vec::new();
        let mut items = Vec::new();
        loop {
            match self.tok() {
                Token::Eof => break,
                Token::Use => imports.push(self.import()?),
                Token::Export => {
                    self.advance();
                    let mut item = self.item()?;
                    let name = Self::item_name(&item.value);
                    if let Some(n) = name { exports.push(n); }
                    item.exported = true;
                    items.push(item);
                }
                _ => items.push(self.item()?),
            }
        }
        Ok(Module { span: Span::new(0, self.source.len()), imports, exports, items })
    }

fn item_name(kind: &ItemKind) -> Option<String> {
    match kind {
        ItemKind::Fn { name, .. } | ItemKind::Struct { name, .. }
        | ItemKind::Class { name, .. } | ItemKind::Interface { name, .. }
        | ItemKind::Union { name, .. } | ItemKind::TypeAlias { name, .. }
        | ItemKind::Const { name, .. } => Some(name.clone()),
    }
}

    fn import(&mut self) -> Result<Import> {
        let span = self.advance().1;
        let mut names = Vec::new();
        if self.tok() == &Token::LBrace {
            self.advance();
            loop {
                let name = self.ident()?;
                let alias = if self.tok() == &Token::As { self.advance(); Some(self.ident()?) } else { None };
                names.push((name, alias));
                if self.tok() == &Token::Comma { self.advance(); } else { break; }
            }
            self.eat(Token::RBrace)?;
        } else {
            let name = self.ident()?;
            let alias = if self.tok() == &Token::As { self.advance(); Some(self.ident()?) } else { None };
            names.push((name, alias));
        }
        self.eat(Token::From)?;
        let source = self.str_lit()?;
        let lang = source.find(':').map(|i| source[..i].to_string());
        let source = lang.as_ref().map(|l| source[l.len()+1..].to_string()).unwrap_or(source);
        self.eat(Token::Semicolon)?;
        Ok(Import { span, names, source, lang })
    }

    // ─── Items ────────────────────────────────────────

    fn item(&mut self) -> Result<ItemNode> {
        match self.tok() {
            Token::Fn => self.fn_item(false),
            Token::Async => { self.advance(); self.fn_item(true) }
            Token::Struct => { self.advance(); let n=self.ident()?; let g=self.generics()?; self.eat(Token::LBrace)?; let f=self.params()?; self.eat(Token::RBrace)?; Ok(ItemNode::new(fresh_id(),Span::new(0,0),ItemKind::Struct{name:n,fields:f,generics:g})) }
            Token::Class => { self.advance(); let n=self.ident()?; let g=self.generics()?; let e=if self.tok()==&Token::Colon{self.advance();Some(self.ident()?)}else{None}; self.block_stmts()?; Ok(ItemNode::new(fresh_id(),Span::new(0,0),ItemKind::Class{name:n,extends:e,implements:vec![],fields:vec![],methods:vec![],generics:g})) }
            Token::Interface => { self.advance(); let n=self.ident()?; self.eat(Token::LBrace)?; let mut m=Vec::new(); while self.tok()!=&Token::RBrace{m.push(self.param()?);self.eat(Token::Semicolon)?;} self.eat(Token::RBrace)?; Ok(ItemNode::new(fresh_id(),Span::new(0,0),ItemKind::Interface{name:n,methods:m})) }
            Token::Union => { self.advance(); let n=self.ident()?; self.eat(Token::LBrace)?; let v=self.params()?; self.eat(Token::RBrace)?; Ok(ItemNode::new(fresh_id(),Span::new(0,0),ItemKind::Union{name:n,variants:v})) }
            Token::Type => { self.advance(); let n=self.ident()?; self.eat(Token::Eq)?; let t=self.type_()?; self.eat(Token::Semicolon)?; Ok(ItemNode::new(fresh_id(),Span::new(0,0),ItemKind::TypeAlias{name:n,type_expr:t})) }
            Token::Const => { self.advance(); let n=self.ident()?; self.eat(Token::Colon)?; let t=self.type_()?; self.eat(Token::Eq)?; let v=self.expr(0)?; self.eat(Token::Semicolon)?; Ok(ItemNode::new(fresh_id(),Span::new(0,0),ItemKind::Const{name:n,type_expr:t,value:v})) }
            _ => Err(error::err(ErrorKind::Syntax, self.span(), format!("Expected item, found {:?}", self.tok()))),
        }
    }

    fn fn_item(&mut self, is_async: bool) -> Result<ItemNode> {
        self.eat(Token::Fn)?;
        let name = self.ident()?;
        let generics = self.generics()?;
        self.eat(Token::LParen)?;
        let params = self.params()?;
        self.eat(Token::RParen)?;
        let ret_type = if self.tok() == &Token::Arrow { self.advance(); Some(self.type_()?) } else { None };
        let body = self.block_stmts()?;
        Ok(ItemNode::new(fresh_id(), Span::new(0,0), ItemKind::Fn { name, params, ret_type, body, is_async, generics }))
    }

    fn generics(&mut self) -> Result<Vec<String>> {
        if self.tok() == &Token::Lt {
            self.advance();
            let mut v = Vec::new();
            loop { v.push(self.ident()?); if self.tok() == &Token::Comma { self.advance(); } else { break; } }
            self.eat(Token::Gt)?;
            Ok(v)
        } else { Ok(Vec::new()) }
    }

    fn params(&mut self) -> Result<Vec<Param>> {
        let mut v = Vec::new();
        loop {
            match self.tok() {
                Token::RBrace | Token::RParen | Token::Eof => break,
                _ => {
                    v.push(self.param()?);
                    if self.tok() == &Token::Comma || self.tok() == &Token::Semicolon {
                        self.advance();
                    } else { break; }
                }
            }
        }
        Ok(v)
    }

    fn param(&mut self) -> Result<Param> {
        let name = self.ident()?;
        self.eat(Token::Colon)?;
        let type_expr = self.type_()?;
        Ok(Param { name, type_expr })
    }

    fn type_(&mut self) -> Result<TypeNode> {
        let (tok, span) = self.advance();
        let first = match tok {
            Token::TInt => {
                if let Token::IntLit(s) = self.tok() { let v=s.parse::<u8>().unwrap_or(0); let s2=self.advance().1; TypeNode::new(fresh_id(),span.merge(s2),TypeExpr::Int(v)) }
                else { TypeNode::new(fresh_id(),span,TypeExpr::Int(0)) }
            }
            Token::TRint => {
                if let Token::IntLit(s) = self.tok() { let v=s.parse::<u8>().unwrap_or(0); let s2=self.advance().1; TypeNode::new(fresh_id(),span.merge(s2),TypeExpr::Rint(v)) }
                else { TypeNode::new(fresh_id(),span,TypeExpr::Rint(0)) }
            }
            Token::TReal => {
                if let Token::IntLit(s) = self.tok() { let v=s.parse::<u8>().unwrap_or(0); let s2=self.advance().1; TypeNode::new(fresh_id(),span.merge(s2),TypeExpr::Real(v)) }
                else { TypeNode::new(fresh_id(),span,TypeExpr::Real(0)) }
            }
            Token::TComplex => TypeNode::new(fresh_id(),span,TypeExpr::Complex),
            Token::TBool => TypeNode::new(fresh_id(),span,TypeExpr::Bool),
            Token::TStr => TypeNode::new(fresh_id(),span,TypeExpr::Str),
            Token::TSymbol => TypeNode::new(fresh_id(),span,TypeExpr::Symbol),
            Token::TVector => TypeNode::new(fresh_id(),span,TypeExpr::Vector(Box::new(TypeExpr::Infer))),
            Token::TMatrix => TypeNode::new(fresh_id(),span,TypeExpr::Matrix(Box::new(TypeExpr::Infer))),
            Token::LBracket => { let inner = self.type_()?; self.eat(Token::RBracket)?; TypeNode::new(fresh_id(),span,TypeExpr::List(Box::new(inner.value))) }
            Token::Ident(s) => TypeNode::new(fresh_id(),span,TypeExpr::Named(s)),
            _ => return Err(error::err(ErrorKind::Syntax, span, format!("Expected type, found {:?}", tok))),
        };
        let mut variants = vec![first.value.clone()];
        let mut merged = first.span;
        while self.tok() == &Token::Pipe {
            self.advance();
            let next = self.type_()?;
            variants.push(next.value);
            merged = merged.merge(next.span);
        }
        if variants.len() > 1 {
            Ok(TypeNode::new(fresh_id(), merged, TypeExpr::Union(variants)))
        } else {
            Ok(first)
        }
    }

    fn block_stmts(&mut self) -> Result<Vec<StmtNode>> {
        self.eat(Token::LBrace)?;
        let mut v = Vec::new();
        loop {
            match self.tok() {
                Token::RBrace | Token::Eof => break,
                _ => v.push(self.stmt()?),
            }
        }
        self.eat(Token::RBrace)?;
        Ok(v)
    }

    // ─── Statements ───────────────────────────────────

    fn stmt(&mut self) -> Result<StmtNode> {
        match self.tok() {
            Token::If => self.if_stmt(),
            Token::For => self.for_stmt(),
            Token::While => { self.advance(); let c=self.expr(0)?; let b=self.block_stmts()?; Ok(StmtNode::new(fresh_id(),Span::new(0,0),Stmt::While(c,b))) }
            Token::Loop => { self.advance(); let b=self.block_stmts()?; Ok(StmtNode::new(fresh_id(),Span::new(0,0),Stmt::Loop(b))) }
            Token::Return => {
                self.advance();
                if self.tok() == &Token::Semicolon { self.advance(); Ok(StmtNode::new(fresh_id(),Span::new(0,0),Stmt::Return(None))) }
                else { let e=self.expr(0)?; self.eat(Token::Semicolon)?; Ok(StmtNode::new(fresh_id(),Span::new(0,0),Stmt::Return(Some(e)))) }
            }
            Token::LBrace => { let b=self.block_stmts()?; Ok(StmtNode::new(fresh_id(),Span::new(0,0),Stmt::Expr(ExprNode::new(fresh_id(),Span::new(0,0),Expr::Block(b))))) }
            // Declaration: ident ':' (const | type) ['=' expr ['as' 'const']] ';'
            Token::Ident(_) => {
                let saved = self.pos;
                let name = self.ident()?;
                if self.tok() == &Token::Colon {
                    self.advance();
                    return self.decl_stmt(name);
                }
                if self.tok() == &Token::Eq {
                    self.advance();
                    let e = self.expr(0)?;
                    self.eat(Token::Semicolon)?;
                    return Ok(StmtNode::new(fresh_id(), Span::new(0,0), Stmt::Assign(name, e)));
                }
                self.pos = saved;
                let e = self.expr(0)?;
                self.eat(Token::Semicolon)?;
                Ok(StmtNode::new(fresh_id(), Span::new(0,0), Stmt::Expr(e)))
            }
            _ => { let e=self.expr(0)?; self.eat(Token::Semicolon)?; Ok(StmtNode::new(fresh_id(),Span::new(0,0),Stmt::Expr(e))) }
        }
    }

    fn decl_stmt(&mut self, name: String) -> Result<StmtNode> {
        let is_const: bool;
        let type_expr: Option<TypeNode>;

        if self.tok() == &Token::Const {
            is_const = true;
            type_expr = None;
            self.advance();
        } else {
            is_const = false;
            type_expr = Some(self.type_()?);
        }

        if self.tok() == &Token::Eq || self.tok() == &Token::ColonEq {
            self.advance();
            let mut value = self.expr(0)?;

            if self.tok() == &Token::As {
                self.advance();
                self.eat(Token::Const)?;
                value = ExprNode::new(fresh_id(), value.span, Expr::AsConst(Box::new(value)));
            }

            self.eat(Token::Semicolon)?;
            Ok(StmtNode::new(fresh_id(), Span::new(0,0), Stmt::Decl { name, type_expr, value, is_const }))
        } else {
            self.eat(Token::Semicolon)?;
            let null = ExprNode::new(fresh_id(), Span::new(0,0), Expr::LitNull);
            Ok(StmtNode::new(fresh_id(), Span::new(0,0), Stmt::Decl { name, type_expr, value: null, is_const }))
        }
    }

    fn if_stmt(&mut self) -> Result<StmtNode> {
        self.advance();
        let cond = self.expr(0)?;
        let then = self.block_stmts()?;
        let else_ = if self.tok() == &Token::Else {
            self.advance();
            if self.tok() == &Token::If { let e=self.if_stmt()?; Some(vec![e]) }
            else { Some(self.block_stmts()?) }
        } else { None };
        Ok(StmtNode::new(fresh_id(),Span::new(0,0),Stmt::If(cond,then,else_)))
    }

    fn for_stmt(&mut self) -> Result<StmtNode> {
        self.advance();
        self.eat(Token::LParen)?;
        let name = self.ident()?;
        self.eat(Token::In)?;
        let iterable = self.expr(0)?;
        self.eat(Token::RParen)?;
        let body = self.block_stmts()?;
        Ok(StmtNode::new(fresh_id(),Span::new(0,0),Stmt::For(name,iterable,body)))
    }

    // ─── Expressions (Pratt) ──────────────────────────

    fn expr(&mut self, min_prec: u8) -> Result<ExprNode> {
        let mut lhs = self.prefix()?;
        loop {
            let op = match self.tok() {
                Token::Plus => BinOp::Add, Token::Minus => BinOp::Sub,
                Token::Star => BinOp::Mul, Token::Slash => BinOp::Div,
                Token::EqEq => BinOp::Eq, Token::NotEq => BinOp::Ne,
                Token::Lt => BinOp::Lt, Token::Gt => BinOp::Gt,
                Token::LtEq => BinOp::Le, Token::GtEq => BinOp::Ge,
                Token::And => BinOp::And, Token::Or => BinOp::Or,
                Token::Eq | Token::ColonEq => BinOp::Assign,
                _ => break,
            };
            let prec = op_prec(op);
            if prec < min_prec { break; }
            self.advance();
            let rhs = self.expr(prec + 1)?;
            lhs = ExprNode::new(fresh_id(), lhs.span, Expr::BinOp(Box::new(lhs), op, Box::new(rhs)));
        }
        // Postfix range '..'
        if self.tok() == &Token::DotDot {
            self.advance();
            let rhs = self.expr(min_prec)?;
            lhs = ExprNode::new(fresh_id(), lhs.span, Expr::Range(Box::new(lhs), Box::new(rhs)));
        }
        // Postfix index [i], field .name, and call ()
        loop {
            if self.tok() == &Token::LBracket {
                self.advance();
                let index = self.expr(0)?;
                self.eat(Token::RBracket)?;
                lhs = ExprNode::new(fresh_id(), lhs.span, Expr::Index(Box::new(lhs), Box::new(index)));
            } else if self.tok() == &Token::Dot {
                self.advance();
                let field = self.field_ident()?;
                lhs = ExprNode::new(fresh_id(), lhs.span, Expr::Field(Box::new(lhs), field));
            } else if self.tok() == &Token::LParen {
                self.advance();
                let mut args = Vec::new();
                loop {
                    match self.tok() {
                        Token::RParen => break,
                        _ => { args.push(self.expr(0)?); if self.tok()==&Token::Comma{self.advance();}else{break;} }
                    }
                }
                self.eat(Token::RParen)?;
                lhs = ExprNode::new(fresh_id(), lhs.span, Expr::Call(Box::new(lhs), args));
            } else { break; }
        }
        // Postfix 'as const'
        if self.tok() == &Token::As {
            self.advance();
            if self.tok() == &Token::Const {
                self.advance();
                lhs = ExprNode::new(fresh_id(), lhs.span, Expr::AsConst(Box::new(lhs)));
            } else {
                // Type conversion: `expr as Type`
                let _target_type = self.type_()?;
                // For now, just keep lhs. Type conversion handled later.
            }
        }
        Ok(lhs)
    }

    fn prefix(&mut self) -> Result<ExprNode> {
        let (tok, span) = self.advance();
        match tok {
            Token::IntLit(s) => Ok(ExprNode::new(fresh_id(),span,Expr::LitInt(s.parse::<i64>().unwrap_or(0)))),
            Token::HexLit(s) => Ok(ExprNode::new(fresh_id(),span,Expr::LitHex(i64::from_str_radix(&s[2..],16).unwrap_or(0)))),
            Token::RealLit(s) => Ok(ExprNode::new(fresh_id(),span,Expr::LitReal(s.parse::<f64>().unwrap_or(0.0)))),
            Token::StrLit(s) => Ok(ExprNode::new(fresh_id(),span,Expr::LitStr(unescape_str(&s[1..s.len()-1])))),
            Token::True => Ok(ExprNode::new(fresh_id(),span,Expr::LitBool(true))),
            Token::False => Ok(ExprNode::new(fresh_id(),span,Expr::LitBool(false))),
            Token::Null => Ok(ExprNode::new(fresh_id(),span,Expr::LitNull)),
            Token::None => Ok(ExprNode::new(fresh_id(),span,Expr::LitNone)),
            Token::SymbolLit(s) => Ok(ExprNode::new(fresh_id(),span,Expr::LitSymbol(s[1..].to_string()))),
            Token::CharLit(c) => Ok(ExprNode::new(fresh_id(),span,Expr::LitChar(c))),
            Token::Ident(name) => {
                if name == "map" && self.tok() == &Token::LBrace { self.map_lit(span) }
                else if name == "set" && self.tok() == &Token::LBrace { self.set_lit(span) }
                else if self.tok() == &Token::LParen { self.call(name, span) }
                else if self.tok() == &Token::LBrace { self.struct_lit(name, span) }
                else { Ok(ExprNode::new(fresh_id(),span,Expr::Ident(name))) }
            }
            Token::Minus => { let e=self.expr(7)?; Ok(ExprNode::new(fresh_id(),span,Expr::UnOp(UnOp::Neg,Box::new(e)))) }
            Token::Bang => { let e=self.expr(7)?; Ok(ExprNode::new(fresh_id(),span,Expr::UnOp(UnOp::Not,Box::new(e)))) }
            Token::LParen => {
                let e = self.expr(0)?;
                if self.tok() == &Token::Comma {
                    let mut items = vec![e];
                    loop {
                        self.advance();
                        if self.tok() == &Token::RParen { break; }
                        items.push(self.expr(0)?);
                        if self.tok() != &Token::Comma { break; }
                    }
                    self.eat(Token::RParen)?;
                    Ok(ExprNode::new(fresh_id(),span,Expr::TupleLit(items)))
                } else {
                    self.eat(Token::RParen)?;
                    Ok(e)
                }
            }
            Token::LBrace => { self.pos-=1; let b=self.block_stmts()?; Ok(ExprNode::new(fresh_id(),span,Expr::Block(b))) }
            Token::Fn => {
                self.eat(Token::LParen)?;
                let params = self.params()?;
                self.eat(Token::RParen)?;
                let ret_type = if self.tok() == &Token::Arrow { self.advance(); Some(self.type_()?) } else { None };
                self.eat(Token::LBrace)?;
                let body = self.expr(0)?;
                self.eat(Token::RBrace)?;
                Ok(ExprNode::new(fresh_id(), span, Expr::FnLit(params, ret_type, Box::new(body))))
            }
            Token::LBracket => {
                let mut v=Vec::new();
                loop {
                    match self.tok() { Token::RBracket => { break; } _ => { v.push(self.expr(0)?); if self.tok()==&Token::Comma{self.advance();}else{break;} } }
                }
                self.eat(Token::RBracket)?;
                Ok(ExprNode::new(fresh_id(),span,Expr::ListLit(v)))
            }
            Token::OkKw => { let e=self.expr(0)?; Ok(ExprNode::new(fresh_id(),span,Expr::ResultOk(Box::new(e)))) }
            Token::ErrorKw => { let e=self.expr(0)?; Ok(ExprNode::new(fresh_id(),span,Expr::ResultErr(Box::new(e)))) }
            Token::Spawn => { let e=self.expr(0)?; Ok(ExprNode::new(fresh_id(),span,Expr::Spawn(Box::new(e)))) }
            Token::Await => { let e=self.expr(0)?; Ok(ExprNode::new(fresh_id(),span,Expr::Await(Box::new(e)))) }
            // Type name used as function: str(42)
            Token::BacktickStr(s) => Ok(ExprNode::new(fresh_id(), span, Expr::LitStr(s))),
            Token::FStrLit(raw) => self.parse_fstring(raw, span),
            Token::TStr => {
                if self.tok() == &Token::LParen { self.call("str".into(), span) }
                else { Err(error::err(ErrorKind::Syntax, span, "Expected '(' after 'str'")) }
            }
            _ => Err(error::err(ErrorKind::Syntax, span, format!("Unexpected token {:?}", tok))),
        }
    }

    fn call(&mut self, name: String, span: Span) -> Result<ExprNode> {
        self.eat(Token::LParen)?;
        let mut args = Vec::new();
        loop {
            match self.tok() {
                Token::RParen => break,
                _ => { args.push(self.expr(0)?); if self.tok()==&Token::Comma{self.advance();}else{break;} }
            }
        }
        self.eat(Token::RParen)?;
        let callee = ExprNode::new(fresh_id(), span, Expr::Ident(name));
        Ok(ExprNode::new(fresh_id(), span, Expr::Call(Box::new(callee), args)))
    }

    fn map_lit(&mut self, span: Span) -> Result<ExprNode> {
        self.advance();
        let mut pairs = Vec::new();
        loop {
            match self.tok() {
                Token::RBrace => { self.advance(); break; }
                _ => {
                    let key = self.expr(0)?;
                    self.eat(Token::Colon)?;
                    let val = self.expr(0)?;
                    pairs.push((key, val));
                    if self.tok() == &Token::Comma { self.advance(); }
                    else if self.tok() != &Token::RBrace {
                        return Err(error::err(ErrorKind::Syntax, self.span(), "Expected , or } in map literal"));
                    }
                }
            }
        }
        Ok(ExprNode::new(fresh_id(), span, Expr::MapLit(pairs)))
    }

    fn set_lit(&mut self, span: Span) -> Result<ExprNode> {
        self.advance();
        let mut items = Vec::new();
        loop {
            match self.tok() {
                Token::RBrace => { self.advance(); break; }
                _ => {
                    items.push(self.expr(0)?);
                    if self.tok() == &Token::Comma { self.advance(); }
                    else if self.tok() != &Token::RBrace {
                        return Err(error::err(ErrorKind::Syntax, self.span(), "Expected , or } in set literal"));
                    }
                }
            }
        }
        Ok(ExprNode::new(fresh_id(), span, Expr::SetLit(items)))
    }

    fn parse_fstring(&mut self, raw: String, span: Span) -> Result<ExprNode> {
        enum Part { Text(String), Expr(ExprNode) }
        let mut parts: Vec<Part> = Vec::new();
        let mut text = String::new();
        let mut cs = raw.chars().peekable();
        while let Some(c) = cs.next() {
            if c == '{' {
                if cs.peek() == Some(&'{') { cs.next(); text.push('{'); }
                else {
                    if !text.is_empty() { parts.push(Part::Text(std::mem::take(&mut text))); }
                    let mut depth = 1u32;
                    let mut expr_src = String::new();
                    while let Some(ec) = cs.next() {
                        if ec == '{' { depth += 1; }
                        else if ec == '}' { depth -= 1; if depth == 0 { break; } }
                        expr_src.push(ec);
                    }
                    if depth != 0 { return Err(error::err(ErrorKind::Syntax, span, "Unclosed { in f-string")); }
                    let expr_node = Parser::parse_expr(expr_src.trim())?;
                    parts.push(Part::Expr(expr_node));
                }
            } else if c == '}' {
                if cs.peek() == Some(&'}') { cs.next(); text.push('}'); }
                else { return Err(error::err(ErrorKind::Syntax, span, "Unmatched } in f-string")); }
            } else { text.push(c); }
        }
        if !text.is_empty() { parts.push(Part::Text(text)); }
        let mut result: Option<ExprNode> = None;
        for part in parts {
            let node = match part {
                Part::Text(s) => ExprNode::new(fresh_id(), span, Expr::LitStr(s)),
                Part::Expr(e) => {
                    let callee = ExprNode::new(fresh_id(), span, Expr::Ident("str".into()));
                    ExprNode::new(fresh_id(), span, Expr::Call(Box::new(callee), vec![e]))
                }
            };
            result = Some(match result {
                Some(left) => ExprNode::new(fresh_id(), span,
                    Expr::BinOp(Box::new(left), BinOp::Add, Box::new(node))),
                None => node,
            });
        }
        Ok(result.unwrap_or_else(|| ExprNode::new(fresh_id(), span, Expr::LitStr(String::new()))))
    }

    pub fn parse_expr(source: &str) -> Result<ExprNode> {
        let mut tokens = lexer::lex(source);
        tokens.push((Token::Eof, Span::new(source.len(), source.len())));
        let mut p = Parser { tokens, pos: 0, source: source.to_string() };
        p.expr(0)
    }

    fn struct_lit(&mut self, name: String, span: Span) -> Result<ExprNode> {
        self.advance();
        let mut fields = Vec::new();
        loop {
            match self.tok() {
                Token::RBrace => { self.advance(); break; }
                _ => {
                    let fname = self.ident()?;
                    self.eat(Token::Colon)?;
                    let val = self.expr(0)?;
                    fields.push((fname, val));
                    if self.tok() == &Token::Comma { self.advance(); }
                    else if self.tok() != &Token::RBrace {
                        return Err(error::err(ErrorKind::Syntax, self.span(), "Expected , or } in struct literal"));
                    }
                }
            }
        }
        Ok(ExprNode::new(fresh_id(), span, Expr::StructLit(name, fields)))
    }
}

fn op_prec(op: BinOp) -> u8 {
    use BinOp::*;
    match op { Assign => 1, Or => 2, And => 3, Eq|Ne => 4, Lt|Gt|Le|Ge => 5, Add|Sub => 6, Mul|Div => 7 }
}

fn unescape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('0') => out.push('\0'),
                Some(other) => { out.push('\\'); out.push(other); }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn tokens_match(a: &Token, b: &Token) -> bool {
    use Token::*;
    matches!((a,b),
        (Fn,Fn)|(Const,Const)|(If,If)|(Else,Else)|(For,For)|(In,In)|(While,While)|(Loop,Loop)|(Return,Return)
        |(Struct,Struct)|(Class,Class)|(Interface,Interface)|(Union,Union)|(Type,Type)|(Use,Use)|(Export,Export)|(As,As)|(From,From)
        |(Async,Async)|(Await,Await)|(Spawn,Spawn)|(True,True)|(False,False)|(Null,Null)|(None,None)
        |(OkKw,OkKw)|(ErrorKw,ErrorKw)|(Mut,Mut)|(Ref,Ref)|(Match,Match)|(Super,Super)
        |(TInt,TInt)|(TRint,TRint)|(TReal,TReal)|(TComplex,TComplex)|(TBool,TBool)|(TStr,TStr)|(TSymbol,TSymbol)|(TVector,TVector)|(TMatrix,TMatrix)
        |(Plus,Plus)|(Minus,Minus)|(Star,Star)|(Slash,Slash)|(Eq,Eq)|(EqEq,EqEq)|(NotEq,NotEq)
        |(Lt,Lt)|(Gt,Gt)|(LtEq,LtEq)|(GtEq,GtEq)|(Bang,Bang)|(And,And)|(Or,Or)|(Pipe,Pipe)|(Inc,Inc)|(Dec,Dec)|(Question,Question)|(ColonEq,ColonEq)|(Arrow,Arrow)
        |(LParen,LParen)|(RParen,RParen)|(LBrace,LBrace)|(RBrace,RBrace)|(LBracket,LBracket)|(RBracket,RBracket)
        |(Colon,Colon)|(Semicolon,Semicolon)|(Comma,Comma)|(Dot,Dot)|(DotDot,DotDot)|(At,At)|(Hash,Hash)|(Eof,Eof)
        |(Ident(_),Ident(_))|(IntLit(_),IntLit(_))|(StrLit(_),StrLit(_))|(BacktickStr(_),BacktickStr(_))|(FStrLit(_),FStrLit(_))|(RealLit(_),RealLit(_))|(HexLit(_),HexLit(_))|(SymbolLit(_),SymbolLit(_))|(CharLit(_),CharLit(_))
        |(Error(_),Error(_))
    )
}
