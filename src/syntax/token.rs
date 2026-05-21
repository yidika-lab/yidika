#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Fn, Const, If, Else, For, In, While, Loop, Infiny, Return,
    Struct, Class, Interface, Union, Type, Use, Export, As, From,
    Async, Await, Spawn, True, False, Null, None, Mut, Ref, Match, Super,
    OkKw, ErrorKw,
    TInt, TRint, TReal, TComplex, TBool, TStr, TSymbol, TVector, TMatrix, TMap,
    IntLit(String), HexLit(String), RealLit(String),
    ImagInt(String), ImagReal(String),
    StrLit(String), SymbolLit(String), CharLit(char),
    BacktickStr(String), FStrLit(String),
    Ident(String),
    Plus, Minus, Star, Slash, Eq, EqEq, NotEq,
    Lt, Gt, LtEq, GtEq, Bang, And, Or, Pipe, Inc, Dec,
    Question, ColonEq, Arrow, FatArrow,
    LParen, RParen, LBrace, RBrace, LBracket, RBracket,
    Colon, Semicolon, Comma, Dot, DotDot, DotDotDot, At, Hash,
    Error(String), Eof,
}
