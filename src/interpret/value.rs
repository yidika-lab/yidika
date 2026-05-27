use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Real(f64),
    Bool(bool),
    Str(String),
    Char(char),
    Range(i64, i64),
    List(Vec<Value>),
    Struct(String, HashMap<String, Value>),
    Tuple(Vec<Value>),
    Dict(Vec<(Value, Value)>),
    Set(Vec<Value>),
    Map(HashMap<String, Value>),
    Complex(f64, f64),
    Instance(String, Vec<Value>),
    Variant(String, Vec<Value>),
    Result(bool, Box<Value>),
    Null,
    None_,
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
            Value::Range(a, b) => write!(f, "{}..{}", a, b),
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
