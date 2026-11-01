use std::cell::Cell;
use std::sync::{Mutex, OnceLock};

use crate::diagnostics::error::{err, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::interpret::{FnDef, Interpreter, Value};
use crate::syntax::ast::{Expr, ExprNode};

static HANDLER_SLOTS: OnceLock<Mutex<Vec<FnDef>>> = OnceLock::new();

fn slots() -> &'static Mutex<Vec<FnDef>> {
    HANDLER_SLOTS.get_or_init(|| Mutex::new(Vec::new()))
}

thread_local! {
    static CURRENT_INTERP: Cell<*mut Interpreter> = const { Cell::new(std::ptr::null_mut()) };
}

extern "C" fn host_cb(ev: i64, slot: i64) -> i64 {
    let fndef = slots().lock().unwrap_or_else(|e| e.into_inner()).get(slot as usize).cloned();
    let Some(fndef) = fndef else { return 0 };
    CURRENT_INTERP.with(|c| {
        let p = c.get();
        if p.is_null() {
            return 0;
        }
        let interp = unsafe { &mut *p };
        interp.push_frame();
        if let Some(param) = fndef.params.first() {
            interp.frames.last_mut().unwrap().insert(param.name.clone(), Value::Int(ev));
        }
        let _ = interp.run_fn_body(&fndef);
        interp.pop_frame();
        0
    })
}

fn v_int(v: &Value) -> i64 {
    match v {
        Value::Int(i) => *i,
        Value::Real(r) => *r as i64,
        Value::Bool(b) => *b as i64,
        _ => 0,
    }
}

fn v_str(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        _ => String::new(),
    }
}

fn arg(args: &[Value], i: usize, span: Span, func: &str) -> Result<Value> {
    args.get(i)
        .cloned()
        .ok_or_else(|| err(ErrorKind::Runtime, span, format!("gui.{} expects more arguments", func)))
}

pub fn call_gui(
    func: &str,
    args: Vec<Value>,
    raw_args: &[ExprNode],
    interp: &mut Interpreter,
    span: Span,
) -> Result<Value> {
    let prev_interp = CURRENT_INTERP.with(|c| c.get());
    CURRENT_INTERP.with(|c| c.set(interp as *mut Interpreter));
    struct Restore(Option<*mut Interpreter>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT_INTERP.with(|c| c.set(self.0.unwrap_or(std::ptr::null_mut())));
        }
    }
    let _restore = Restore(if prev_interp.is_null() { None } else { Some(prev_interp) });
    let v = match func {
        "window" => {
            yk_ffi_gui::window(&v_str(&arg(&args, 0, span, func)?), v_int(&arg(&args, 1, span, func)?), v_int(&arg(&args, 2, span, func)?))
        }
        "div" => yk_ffi_gui::div(v_int(&arg(&args, 0, span, func)?)),
        "text" => yk_ffi_gui::text(v_int(&arg(&args, 0, span, func)?), &v_str(&arg(&args, 1, span, func)?)),
        "button" => yk_ffi_gui::button(v_int(&arg(&args, 0, span, func)?), &v_str(&arg(&args, 1, span, func)?)),
        "style" => yk_ffi_gui::set_style(v_int(&arg(&args, 0, span, func)?), &v_str(&arg(&args, 1, span, func)?)),
        "set_text" => yk_ffi_gui::set_text(v_int(&arg(&args, 0, span, func)?), &v_str(&arg(&args, 1, span, func)?)),
        "append" => yk_ffi_gui::append(v_int(&arg(&args, 0, span, func)?), v_int(&arg(&args, 1, span, func)?)),
        "remove" => yk_ffi_gui::remove(v_int(&arg(&args, 0, span, func)?)),
        "refresh" => yk_ffi_gui::refresh(),
        "quit" => yk_ffi_gui::quit(),
        "auto_exit" => yk_ffi_gui::auto_exit(v_int(&arg(&args, 0, span, func)?)),
        "dump" => {
            let id = v_int(&arg(&args, 0, span, func)?);
            let text = yk_ffi_gui::dump_to_string(id);
            if interp.tui_mode {
                use std::io::Write;
                print!("{}", text);
                let _ = std::io::stdout().flush();
            } else {
                interp.output.push_str(&text);
            }
            0
        }
        "synthetic_click" => yk_ffi_gui::synthetic_click(v_int(&arg(&args, 0, span, func)?)),
        "synthetic_hover" => yk_ffi_gui::synthetic_hover(v_int(&arg(&args, 0, span, func)?)),
        "node_x" => yk_ffi_gui::node_x(v_int(&arg(&args, 0, span, func)?)),
        "node_y" => yk_ffi_gui::node_y(v_int(&arg(&args, 0, span, func)?)),
        "node_w" => yk_ffi_gui::node_w(v_int(&arg(&args, 0, span, func)?)),
        "node_h" => yk_ffi_gui::node_h(v_int(&arg(&args, 0, span, func)?)),
        "on" => {
            let id = v_int(&arg(&args, 0, span, func)?);
            let event = v_str(&arg(&args, 1, span, func)?);
            let fndef = match raw_args.get(2).map(|a| &a.value) {
                Some(Expr::Ident(name)) => interp.functions.get(name.as_str()).cloned(),
                _ => match args.get(2) {
                    Some(Value::Fn(f)) => Some(f.clone()),
                    _ => None,
                },
            };
            let Some(fndef) = fndef else {
                return Err(err(ErrorKind::Runtime, span, "gui.on expects a named function or fn value"));
            };
            let slot = {
                let mut g = slots().lock().unwrap_or_else(|e| e.into_inner());
                g.push(fndef);
                g.len() - 1
            };
            let f: extern "C" fn(i64, i64) -> i64 = host_cb;
            yk_ffi_gui::on(id, &event, f as usize as i64, slot as i64)
        }
        "run" => {
            let id = v_int(&arg(&args, 0, span, func)?);
            yk_ffi_gui::run(id)
        }
        _ => {
            return Err(err(ErrorKind::Runtime, span, format!("Unknown gui function '{}'", func)));
        }
    };
    Ok(Value::Int(v))
}
