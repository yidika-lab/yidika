use crate::diagnostics::span::Span;
use crate::syntax::token::Token;

pub struct Lexer {
    source: Vec<char>,
    pos: usize,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self { source: source.chars().collect(), pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.source.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.source.get(self.pos).copied();
        if c.is_some() { self.pos += 1; }
        c
    }

    fn skip(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => { self.advance(); }
                Some('/') => {
                    let saved = self.pos;
                    self.advance();
                    match self.peek() {
                        Some('/') => {
                            self.advance();
                            while let Some(c) = self.advance() {
                                if c == '\n' { break; }
                            }
                        }
                        Some('*') => {
                            self.advance();
                            loop {
                                match self.advance() {
                                    None => break,
                                    Some('*') if self.peek() == Some('/') => { self.advance(); break; }
                                    _ => {}
                                }
                            }
                        }
                        _ => { self.pos = saved; return; }
                    }
                }
                _ => return,
            }
        }
    }

    fn word(&mut self, start: usize) -> Token {
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' { self.advance(); } else { break; }
        }
        let s: String = self.source[start..self.pos].iter().collect();
        match s.as_str() {
            "fn" => Token::Fn, "const" => Token::Const,
            "if" => Token::If, "else" => Token::Else,
            "for" => Token::For, "in" => Token::In,
            "while" => Token::While, "loop" => Token::Loop,
            "return" => Token::Return, "struct" => Token::Struct,
            "class" => Token::Class, "interface" => Token::Interface,
            "union" => Token::Union, "type" => Token::Type,
            "use" => Token::Use, "export" => Token::Export,
            "as" => Token::As, "from" => Token::From,
            "async" => Token::Async, "await" => Token::Await,
            "spawn" => Token::Spawn,
            "true" => Token::True, "false" => Token::False,
            "null" => Token::Null, "None" => Token::None,
            "Ok" => Token::OkKw, "Error" => Token::ErrorKw,
            "mut" => Token::Mut, "ref" => Token::Ref,
            "match" => Token::Match, "super" => Token::Super,
            "int" => Token::TInt, "rint" => Token::TRint,
            "real" => Token::TReal, "complex" => Token::TComplex,
            "bool" => Token::TBool, "str" => Token::TStr,
            "symbol" => Token::TSymbol,
            "vector" => Token::TVector, "matrix" => Token::TMatrix,
            _ => Token::Ident(s),
        }
    }

    fn number(&mut self, start: usize) -> Token {
        let is_hex = self.peek() == Some('x') || self.peek() == Some('X');
        if is_hex { self.advance(); }
        while let Some(c) = self.peek() {
            if c.is_ascii_hexdigit() || c == '_' { self.advance(); } else { break; }
        }
        let raw: String = self.source[start..self.pos].iter().filter(|&&c| c != '_').collect();
        if is_hex { return Token::HexLit(raw); }
        if self.peek() == Some('.') && self.source.get(self.pos + 1).map_or(false, |c| c.is_ascii_digit()) {
            self.advance();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() || c == '_' { self.advance(); } else { break; }
            }
            let raw: String = self.source[start..self.pos].iter().filter(|&&c| c != '_').collect();
            return Token::RealLit(raw);
        }
        Token::IntLit(raw)
    }

    fn string(&mut self, start: usize) -> Token {
        while let Some(c) = self.advance() {
            if c == '\\' { self.advance(); continue; }
            if c == '"' { break; }
        }
        Token::StrLit(self.source[start..self.pos].iter().collect())
    }
}

impl Iterator for Lexer {
    type Item = (Token, Span);

    fn next(&mut self) -> Option<Self::Item> {
        self.skip();
        let start = self.pos;
        let tok = match self.advance() {
            None => return None,
            Some('(') => Token::LParen, Some(')') => Token::RParen,
            Some('{') => Token::LBrace, Some('}') => Token::RBrace,
            Some('[') => Token::LBracket, Some(']') => Token::RBracket,
            Some(';') => Token::Semicolon, Some(',') => Token::Comma,
            Some(':') => if self.peek() == Some('=') { self.advance(); Token::ColonEq } else { Token::Colon },
            Some('.') => if self.peek() == Some('.') { self.advance(); Token::DotDot } else { Token::Dot },
            Some('+') => if self.peek() == Some('+') { self.advance(); Token::Inc } else { Token::Plus },
            Some('-') => if self.peek() == Some('>') { self.advance(); Token::Arrow } else { Token::Minus },
            Some('*') => Token::Star,
            Some('/') => Token::Slash,
            Some('!') => if self.peek() == Some('=') { self.advance(); Token::NotEq } else { Token::Bang },
            Some('=') => if self.peek() == Some('=') { self.advance(); Token::EqEq } else { Token::Eq },
            Some('<') => if self.peek() == Some('=') { self.advance(); Token::LtEq } else { Token::Lt },
            Some('>') => if self.peek() == Some('=') { self.advance(); Token::GtEq } else { Token::Gt },
            Some('&') => if self.peek() == Some('&') { self.advance(); Token::And } else { return Some((Token::Error("expected &&".into()), Span::new(start, self.pos))); },
            Some('|') => if self.peek() == Some('|') { self.advance(); Token::Or } else { Token::Pipe },
            Some('?') => Token::Question, Some('@') => Token::At, Some('#') => Token::Hash,
            Some('"') => self.string(start),
            Some('`') => {
                let content_start = self.pos;
                loop {
                    match self.advance() {
                        Some('`') => break,
                        Some(_) => continue,
                        None => break,
                    }
                }
                let content: String = self.source[content_start..self.pos.saturating_sub(1)].iter().collect();
                Token::BacktickStr(content)
            }
            Some('f') if self.peek() == Some('"') || self.peek() == Some('\'') || self.peek() == Some('`') => {
                let quote = self.advance().unwrap(); // skip the " or '
                let content_start = self.pos;
                let mut depth = 0u32;
                while let Some(c) = self.advance() {
                    if c == quote && depth == 0 { break; }
                    if c == '{' { depth += 1; }
                    else if c == '}' && depth > 0 { depth -= 1; }
                }
                let content: String = self.source[content_start..self.pos.saturating_sub(1)].iter().collect();
                Token::FStrLit(content)
            }
            Some(c) if c.is_ascii_digit() => self.number(start),
            Some(c) if c.is_alphabetic() || c == '_' => self.word(start),
            Some('\'') => {
                while let Some(c) = self.advance() {
                    if c == '\\' { self.advance(); continue; }
                    if c == '\'' { break; }
                }
                Token::StrLit(self.source[start..self.pos].iter().collect())
            }
            Some(c) => Token::Error(format!("unexpected '{}'", c)),
        };
        Some((tok, Span::new(start, self.pos)))
    }
}

pub fn lex(source: &str) -> Vec<(Token, Span)> {
    Lexer::new(source).collect()
}
