use std::fmt;
use crate::diagnostics::span::Span;

#[derive(Debug, Clone)]
pub struct YkError {
    pub kind: ErrorKind,
    pub span: Span,
    pub msg: String,
    pub source: Option<String>,
    pub file: Option<String>,
}

impl fmt::Display for YkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref src) = self.source {
            let start = self.span.start.min(src.len());
            let line_start = src[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let line_end = src[start..].find('\n')
                .map(|i| i + start).unwrap_or(src.len());
            let line_num = src[..line_start].matches('\n').count() + 1;
            let line = &src[line_start..line_end];
            let col = start.saturating_sub(line_start);
            let caret_len = (self.span.end.saturating_sub(self.span.start)).max(1);

            if let Some(ref file) = self.file {
                write!(f, "error[{:?}]: {}\n {}:{}:{}\n {:>4} | {}\n      | {:>col$}{}",
                    self.kind, self.msg, file, line_num, col + 1,
                    line_num, line, "", "^".repeat(caret_len), col = col,
                )
            } else {
                write!(f, "error[{:?}]: {}\n {:>4} | {}\n      | {:>col$}{}",
                    self.kind, self.msg, line_num, line, "", "^".repeat(caret_len), col = col,
                )
            }
        } else {
            write!(f, "error[{:?}] at {}: {}", self.kind, self.span, self.msg)
        }
    }
}

impl std::error::Error for YkError {}

impl YkError {
    pub fn with_source(mut self, source: &str) -> Self {
        self.source = Some(source.to_string());
        self
    }

    pub fn with_file(mut self, file: &str) -> Self {
        self.file = Some(file.to_string());
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorKind {
    Syntax,
    TypeError,
    NameError,
    Io,
    Internal,
    Runtime,
}

pub type Result<T> = std::result::Result<T, YkError>;

pub fn err(kind: ErrorKind, span: Span, msg: impl Into<String>) -> YkError {
    YkError { kind, span, msg: msg.into(), source: None, file: None }
}
