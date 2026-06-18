
use yidi::syntax::lexer;
use std::fs;

fn main() {
    let content = fs::read_to_string("examples/basics/main.yk").unwrap();
    let tokens = lexer::lex(&content);
    for (i, (tok, span)) in tokens.iter().enumerate() {
        println!("Token {}: {:?} at span {:?}", i, tok, span);
    }
}
