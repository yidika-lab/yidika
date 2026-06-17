use std::fmt;
use crate::diagnostics::span::Span;

#[derive(Debug, Clone)]
pub struct YkError {
    pub kind: ErrorKind,
    pub span: Span,
    pub msg: String,
    pub source: Option<String>,
    pub file: Option<String>,
    pub help: Option<String>,
}

impl fmt::Display for YkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Error header with bold red
        writeln!(f, "\x1b[1;31merror[{}]\x1b[0m: {}", self.kind, self.msg)?;

        // Help message if available
        if let Some(ref help) = self.help {
            writeln!(f, "  \x1b[1;36mhelp\x1b[0m: {}", help)?;
        }

        // Source code preview
        if let Some(ref src) = self.source {
            let start = self.span.start.min(src.len());
            // Find start and end of the surrounding block/function for context
            let mut block_start = start;
            let mut brace_count = 0;
            // Look backwards to find the start of a block or function
            while block_start > 0 {
                let c = src.chars().nth(block_start - 1).unwrap_or('\0');
                if c == '}' {
                    brace_count += 1;
                } else if c == '{' {
                    if brace_count == 0 {
                        break;
                    }
                    brace_count -= 1;
                }
                block_start -= 1;
            }
            // Now look forward to find the end of the block
            let mut block_end = self.span.end.min(src.len());
            let mut brace_count_forward = 0;
            let mut found_opening = false;
            while block_end < src.len() {
                let c = src.chars().nth(block_end).unwrap_or('\0');
                if c == '{' {
                    found_opening = true;
                    brace_count_forward += 1;
                } else if c == '}' {
                    brace_count_forward -= 1;
                    if found_opening && brace_count_forward <= 0 {
                        block_end += 1;
                        break;
                    }
                }
                block_end += 1;
            }
            
            // Now process lines in this block
            let block_src = &src[block_start..block_end];
            let lines: Vec<&str> = block_src.split('\n').collect();
            // Calculate absolute line number for first line of block
            let abs_line_num = src[..block_start].matches('\n').count() + 1;
            
            // Now find which line contains our error
            let mut error_line_idx = 0;
            let mut current_pos = block_start;
            for (i, line) in lines.iter().enumerate() {
                let line_len = line.len() + 1; // +1 for the newline character
                if self.span.start < current_pos + line_len {
                    error_line_idx = i;
                    break;
                }
                current_pos += line_len;
            }

            // Display file name and location
            let line_start = src[..self.span.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let col = self.span.start.saturating_sub(line_start);
            if let Some(ref file) = self.file {
                writeln!(f, "  \x1b[1;34mfile\x1b[0m: {}", file)?;
                writeln!(f, "  \x1b[1;34m-->\x1b[0m {}:{}:{}", file, abs_line_num + error_line_idx, col + 1)?;
            }
            writeln!(f, "   \x1b[1;34m|\x1b[0m")?;

            // Calculate error range within the specific error line
            let line_start = src[..self.span.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let line_end = src[self.span.start..].find('\n')
                .map(|i| i + self.span.start).unwrap_or(src.len());
            let error_start_col = self.span.start.saturating_sub(line_start);
            let error_end_col = std::cmp::min(self.span.end.saturating_sub(line_start), line_end.saturating_sub(line_start));
            let error_len = if error_end_col > error_start_col {
                error_end_col - error_start_col
            } else {
                1
            };

            // Show only relevant lines (surrounding block)
            for (i, line) in lines.iter().enumerate() {
                writeln!(f, " {:>4} \x1b[1;34m|\x1b[0m {}", abs_line_num + i, line)?;
                if i == error_line_idx {
                    // Show the highlight
                    writeln!(f, "   \x1b[1;34m|\x1b[0m {}{}",
                        " ".repeat(error_start_col),
                        "\x1b[1;31m".to_string() + &"^".repeat(error_len) + "\x1b[0m"
                    )?;
                }
            }

            // Add closing vertical line for style
            writeln!(f, "   \x1b[1;34m|\x1b[0m")?;
        }

        Ok(())
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

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Ord, Eq)]
pub enum ErrorKind {
    Syntax,
    UnexpectedToken,
    MissingToken,
    InvalidLiteral,
    TypeError,
    MismatchedType,
    IncompatibleTypes,
    CannotInferType,
    NameError,
    UndefinedName,
    NameAlreadyDefined,
    DuplicateDefinition,
    Io,
    FileNotFound,
    PermissionDenied,
    Internal,
    CompilerBug,
    JitError,
    Runtime,
    DivisionByZero,
    IndexOutOfBounds,
    NullDereference,
    UseAfterMove,
    UseAfterFree,
    BorrowError,
    MutableBorrowWhileShared,
    SharedBorrowWhileMutable,
    MoveWhileBorrowed,
    UninitializedVariable,
    UnreachableCode,
    MissingReturn,
    InvalidOperation,
    InvalidCast,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::Syntax => write!(f, "E0001"),
            ErrorKind::UnexpectedToken => write!(f, "E0002"),
            ErrorKind::MissingToken => write!(f, "E0003"),
            ErrorKind::InvalidLiteral => write!(f, "E0004"),
            ErrorKind::TypeError => write!(f, "E0101"),
            ErrorKind::MismatchedType => write!(f, "E0102"),
            ErrorKind::IncompatibleTypes => write!(f, "E0103"),
            ErrorKind::CannotInferType => write!(f, "E0104"),
            ErrorKind::NameError => write!(f, "E0201"),
            ErrorKind::UndefinedName => write!(f, "E0202"),
            ErrorKind::NameAlreadyDefined => write!(f, "E0203"),
            ErrorKind::DuplicateDefinition => write!(f, "E0204"),
            ErrorKind::Io => write!(f, "E0301"),
            ErrorKind::FileNotFound => write!(f, "E0302"),
            ErrorKind::PermissionDenied => write!(f, "E0303"),
            ErrorKind::Internal => write!(f, "E0901"),
            ErrorKind::CompilerBug => write!(f, "E0902"),
            ErrorKind::JitError => write!(f, "E0903"),
            ErrorKind::Runtime => write!(f, "E1001"),
            ErrorKind::DivisionByZero => write!(f, "E1002"),
            ErrorKind::IndexOutOfBounds => write!(f, "E1003"),
            ErrorKind::NullDereference => write!(f, "E1004"),
            ErrorKind::UseAfterMove => write!(f, "E1005"),
            ErrorKind::UseAfterFree => write!(f, "E1006"),
            ErrorKind::BorrowError => write!(f, "E1007"),
            ErrorKind::MutableBorrowWhileShared => write!(f, "E1008"),
            ErrorKind::SharedBorrowWhileMutable => write!(f, "E1009"),
            ErrorKind::MoveWhileBorrowed => write!(f, "E1010"),
            ErrorKind::UninitializedVariable => write!(f, "E1011"),
            ErrorKind::UnreachableCode => write!(f, "E1012"),
            ErrorKind::MissingReturn => write!(f, "E1013"),
            ErrorKind::InvalidOperation => write!(f, "E1014"),
            ErrorKind::InvalidCast => write!(f, "E1015"),
        }
    }
}

// ─────────────────────────────────────────────────────────────
// WARNING SYSTEM
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct YkWarning {
    pub kind: WarningKind,
    pub span: Span,
    pub msg: String,
    pub source: Option<String>,
    pub file: Option<String>,
    pub help: Option<String>,
}

impl fmt::Display for YkWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Warning header with bold yellow
        writeln!(f, "\x1b[1;33mwarning[{}]\x1b[0m: {}", self.kind, self.msg)?;

        // Help message if available
        if let Some(ref help) = self.help {
            writeln!(f, "  \x1b[1;36mhelp\x1b[0m: {}", help)?;
        }

        // Source code preview (same style as errors but with yellow highlight)
        if let Some(ref src) = self.source {
            let start = self.span.start.min(src.len());
            let line_start = src[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let line_end = src[start..].find('\n')
                .map(|i| i + start).unwrap_or(src.len());

            // Calculate line numbers
            let line_num = src[..line_start].matches('\n').count() + 1;

            // Display file name and location
            let col = start.saturating_sub(line_start);
            if let Some(ref file) = self.file {
                writeln!(f, "  \x1b[1;34m-->\x1b[0m {}:{}:{}", file, line_num, col + 1)?;
            }

            // Calculate the actual error range on this line
            let line_text = &src[line_start..line_end];
            let line_end_col = line_end.saturating_sub(line_start);
            let error_start_col = col;
            let error_end_col = std::cmp::min(
                self.span.end.saturating_sub(line_start),
                line_end_col
            );
            let error_len = if error_end_col > error_start_col {
                error_end_col - error_start_col
            } else {
                1
            };

            // Add some surrounding lines for better context
            writeln!(f, "   \x1b[1;34m|\x1b[0m")?;

            // Show the line with the error
            writeln!(f, " {:>4} \x1b[1;34m|\x1b[0m {}", line_num, line_text)?;

            // Show the highlight (yellow for warnings)
            writeln!(f, "   \x1b[1;34m|\x1b[0m {}{}",
                " ".repeat(error_start_col),
                "\x1b[1;33m".to_string() + &"^".repeat(error_len) + "\x1b[0m"
            )?;

            // Add closing vertical line for style
            writeln!(f, "   \x1b[1;34m|\x1b[0m")?;
        }

        Ok(())
    }
}

impl YkWarning {
    pub fn with_source(mut self, source: &str) -> Self {
        self.source = Some(source.to_string());
        self
    }

    pub fn with_file(mut self, file: &str) -> Self {
        self.file = Some(file.to_string());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Ord, Eq)]
pub enum WarningKind {
    DeadCode,
    UnusedVariable,
    UnusedParameter,
    UnusedImport,
    UnusedFunction,
    UnusedType,
    ShadowedVariable,
    RedundantCast,
    RedundantClone,
    RedundantReturn,
    TrivialComparison,
    PossibleOverflow,
    ImplicitConversion,
    Deprecated,
    Experimental,
    Style,
    NamingConvention,
}

impl fmt::Display for WarningKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WarningKind::DeadCode => write!(f, "W0001"),
            WarningKind::UnusedVariable => write!(f, "W0002"),
            WarningKind::UnusedParameter => write!(f, "W0003"),
            WarningKind::UnusedImport => write!(f, "W0004"),
            WarningKind::UnusedFunction => write!(f, "W0005"),
            WarningKind::UnusedType => write!(f, "W0006"),
            WarningKind::ShadowedVariable => write!(f, "W0007"),
            WarningKind::RedundantCast => write!(f, "W0008"),
            WarningKind::RedundantClone => write!(f, "W0009"),
            WarningKind::RedundantReturn => write!(f, "W0010"),
            WarningKind::TrivialComparison => write!(f, "W0011"),
            WarningKind::PossibleOverflow => write!(f, "W0012"),
            WarningKind::ImplicitConversion => write!(f, "W0013"),
            WarningKind::Deprecated => write!(f, "W0014"),
            WarningKind::Experimental => write!(f, "W0015"),
            WarningKind::Style => write!(f, "W0016"),
            WarningKind::NamingConvention => write!(f, "W0017"),
        }
    }
}

// ─────────────────────────────────────────────────────────────
// DIAGNOSTICS COLLECTOR
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Diagnostics {
    errors: Vec<YkError>,
    warnings: Vec<YkWarning>,
    treat_warnings_as_errors: bool,
}

impl Diagnostics {
    pub fn new() -> Self {
        Diagnostics {
            errors: Vec::new(),
            warnings: Vec::new(),
            treat_warnings_as_errors: true, // STRICT BY DEFAULT!
        }
    }

    pub fn new_strict() -> Self {
        Self::new()
    }

    pub fn set_treat_warnings_as_errors(&mut self, value: bool) {
        self.treat_warnings_as_errors = value;
    }

    pub fn add_error(&mut self, error: YkError) {
        self.errors.push(error);
    }

    pub fn add_warning(&mut self, warning: YkWarning) {
        self.warnings.push(warning);
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty() || (self.treat_warnings_as_errors && !self.warnings.is_empty())
    }

    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    pub fn errors(&self) -> &[YkError] {
        &self.errors
    }

    pub fn warnings(&self) -> &[YkWarning] {
        &self.warnings
    }

    pub fn into_result(self) -> Result<()> {
        if self.has_errors() {
            if let Some(first_error) = self.errors.first() {
                return Err(first_error.clone());
            }
            if self.treat_warnings_as_errors {
                if let Some(first_warning) = self.warnings.first() {
                    return Err(err(
                        ErrorKind::TypeError,
                        first_warning.span,
                        format!("warning treated as error: {}", first_warning.msg)
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn print_all(&self) {
        for warning in &self.warnings {
            eprintln!("{}", warning);
        }
        for error in &self.errors {
            eprintln!("{}", error);
        }
        if !self.errors.is_empty() {
            eprintln!("\x1b[31;1merror:\x1b[0m aborting due to {} previous error(s)", self.errors.len());
        }
        if self.treat_warnings_as_errors && !self.warnings.is_empty() && self.errors.is_empty() {
            eprintln!("\x1b[31;1merror:\x1b[0m aborting due to {} warning(s) treated as errors", self.warnings.len());
        }
    }
}

pub type Result<T> = std::result::Result<T, YkError>;

pub fn err(kind: ErrorKind, span: Span, msg: impl Into<String>) -> YkError {
    YkError { kind, span, msg: msg.into(), source: None, file: None, help: None }
}

pub fn warn(kind: WarningKind, span: Span, msg: impl Into<String>) -> YkWarning {
    YkWarning { kind, span, msg: msg.into(), source: None, file: None, help: None }
}
