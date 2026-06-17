use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Real(f64),
    Bool(bool),
    Str(String),
    Char(char),
    Range(i64, i64, bool), // start, end, is_char
    List(Vec<Value>),
    Struct(String, HashMap<String, Value>),
    Tuple(Vec<Value>),
    Dict(Vec<(Value, Value)>),
    Set(Vec<Value>),
    Map(HashMap<String, Value>),
    Complex(f64, f64),
    Instance(String, Vec<Value>),
    Variant(String, Vec<Value>),
    Fn(crate::interpret::class::FnDef),
    Result(bool, Box<Value>),
    Null,
    None_,
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Real(a), Value::Real(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Char(a), Value::Char(b)) => a == b,
            (Value::Range(a1, a2, _), Value::Range(b1, b2, _)) => a1 == b1 && a2 == b2,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Struct(a, b), Value::Struct(c, d)) => a == c && b == d,
            (Value::Tuple(a), Value::Tuple(b)) => a == b,
            (Value::Dict(a), Value::Dict(b)) => a == b,
            (Value::Set(a), Value::Set(b)) => a.len() == b.len() && a.iter().all(|x| b.contains(x)),
            (Value::Map(a), Value::Map(b)) => a == b,
            (Value::Complex(a1, a2), Value::Complex(b1, b2)) => a1 == b1 && a2 == b2,
            (Value::Instance(a, b), Value::Instance(c, d)) => a == c && b == d,
            (Value::Variant(a, b), Value::Variant(c, d)) => a == c && b == d,
            (Value::Fn(_), Value::Fn(_)) => false, // fn values never compare equal
            (Value::Result(a1, a2), Value::Result(b1, b2)) => a1 == b1 && a2 == b2,
            (Value::Null, Value::Null) => true,
            (Value::None_, Value::None_) => true,
            _ => false,
        }
    }
}

use std::collections::HashMap;

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Real(r) => write!(f, "{}", r),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Str(s) => write!(f, "{}", s),
            Value::Char(c) => write!(f, "{}", c),
            Value::Range(a, b, _) => write!(f, "{}..{}", a, b),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Value::Struct(name, fields) => {
                write!(f, "{} {{ ", name)?;
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, " }}")
            }
            Value::Tuple(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                write!(f, ")")
            }
            Value::Dict(pairs) => {
                write!(f, "{{")?;
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::Set(items) => {
                write!(f, "set{{")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                write!(f, "}}")
            }
            Value::Map(m) => {
                write!(f, "{{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "\"{}\": {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::None_ => write!(f, "none"),
            Value::Null => write!(f, "null"),
            Value::Instance(name, fields) => {
                write!(f, "{} {{ ", name)?;
                for (i, v) in fields.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", v)?;
                }
                write!(f, " }}")
            }
            Value::Complex(r, i) => {
                if *i < 0.0 { write!(f, "{} - {}i", r, -i) }
                else { write!(f, "{} + {}i", r, i) }
            }
            Value::Result(true, v) => write!(f, "Ok({})", v),
            Value::Result(false, v) => write!(f, "Error({})", v),
            Value::Variant(name, fields) => {
                write!(f, "{}", name)?;
                if !fields.is_empty() {
                    write!(f, "(")?;
                    for (i, v) in fields.iter().enumerate() {
                        if i > 0 { write!(f, ", ")?; }
                        write!(f, "{}", v)?;
                    }
                    write!(f, ")")?;
                }
                Ok(())
            }
            Value::Fn(_) => write!(f, "<fn>"),
        }
    }
}

#[derive(Debug)]
pub enum EvalResult {
    Value(Value),
    Return(Value),
}

pub fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Int(i) => *i != 0,
        Value::Real(f) => *f != 0.0,
        Value::None_ => false,
        Value::Null => false,
        Value::Fn(_) => true,
        _ => true,
    }
}

pub fn is_copy_value(v: &Value) -> bool {
    match v {
        Value::Int(_) | Value::Real(_) | Value::Bool(_) | Value::Char(_) | Value::None_ | Value::Null | Value::Complex(_, _) => true,
        Value::Result(_, inner) => is_copy_value(inner),
        _ => false,
    }
}

pub fn cmp_binop<F: Fn(f64, f64) -> bool>(a: &Value, b: &Value, cmp: F, span: Span) -> Result<Value> {
    let to_f64 = |v: &Value| -> Result<f64> {
        match v {
            Value::Int(i) => Ok(*i as f64),
            Value::Real(r) => Ok(*r),
            _ => Err(error::err(ErrorKind::TypeError, span, "Cannot compare")),
        }
    };
    let av = to_f64(a)?;
    let bv = to_f64(b)?;
    Ok(Value::Bool(cmp(av, bv)))
}
