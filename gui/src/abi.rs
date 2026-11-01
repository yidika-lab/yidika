use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;

fn s(p: *const u8, n: i64) -> String {
    if p.is_null() || n <= 0 {
        return String::new();
    }
    unsafe { String::from_utf8_lossy(slice::from_raw_parts(p, n as usize)).into_owned() }
}

fn g<F: FnOnce() -> i64>(f: F) -> i64 {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(-1)
}

#[no_mangle]
pub extern "C" fn yk_gui_window(p: *const u8, n: i64, w: i64, h: i64) -> i64 {
    g(|| crate::window(&s(p, n), w, h))
}

#[no_mangle]
pub extern "C" fn yk_gui_div(parent: i64) -> i64 {
    g(|| crate::div(parent))
}

#[no_mangle]
pub extern "C" fn yk_gui_text(parent: i64, p: *const u8, n: i64) -> i64 {
    g(|| crate::text(parent, &s(p, n)))
}

#[no_mangle]
pub extern "C" fn yk_gui_button(parent: i64, p: *const u8, n: i64) -> i64 {
    g(|| crate::button(parent, &s(p, n)))
}

#[no_mangle]
pub extern "C" fn yk_gui_style(id: i64, p: *const u8, n: i64) -> i64 {
    g(|| crate::set_style(id, &s(p, n)))
}

#[no_mangle]
pub extern "C" fn yk_gui_set_text(id: i64, p: *const u8, n: i64) -> i64 {
    g(|| crate::set_text(id, &s(p, n)))
}

#[no_mangle]
pub extern "C" fn yk_gui_append(parent: i64, child: i64) -> i64 {
    g(|| crate::append(parent, child))
}

#[no_mangle]
pub extern "C" fn yk_gui_remove(id: i64) -> i64 {
    g(|| crate::remove(id))
}

#[no_mangle]
pub extern "C" fn yk_gui_on(id: i64, p: *const u8, n: i64, addr: i64, ctx: i64) -> i64 {
    g(|| crate::on(id, &s(p, n), addr, ctx))
}

#[no_mangle]
pub extern "C" fn yk_gui_run(id: i64) -> i64 {
    g(|| crate::run(id))
}

#[no_mangle]
pub extern "C" fn yk_gui_refresh() -> i64 {
    g(crate::refresh)
}

#[no_mangle]
pub extern "C" fn yk_gui_quit() -> i64 {
    g(crate::quit)
}

#[no_mangle]
pub extern "C" fn yk_gui_auto_exit(ms: i64) -> i64 {
    g(|| crate::auto_exit(ms))
}

#[no_mangle]
pub extern "C" fn yk_gui_dump(id: i64) -> i64 {
    g(|| crate::dump(id))
}

#[no_mangle]
pub extern "C" fn yk_gui_synthetic_click(id: i64) -> i64 {
    g(|| crate::synthetic_click(id))
}

#[no_mangle]
pub extern "C" fn yk_gui_synthetic_hover(id: i64) -> i64 {
    g(|| crate::synthetic_hover(id))
}

#[no_mangle]
pub extern "C" fn yk_gui_node_x(id: i64) -> i64 {
    g(|| crate::node_x(id))
}

#[no_mangle]
pub extern "C" fn yk_gui_node_y(id: i64) -> i64 {
    g(|| crate::node_y(id))
}

#[no_mangle]
pub extern "C" fn yk_gui_node_w(id: i64) -> i64 {
    g(|| crate::node_w(id))
}

#[no_mangle]
pub extern "C" fn yk_gui_node_h(id: i64) -> i64 {
    g(|| crate::node_h(id))
}
