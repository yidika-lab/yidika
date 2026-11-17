use crate::input::LineSource;
use crate::string::{as_str, into_header, write_into, YkString};

/// # Safety
/// Both arguments must be null or valid `yk_string` pointers.
#[no_mangle]
pub unsafe extern "C" fn yk_re_match(pattern: *const YkString, text: *const YkString) -> i64 {
    match (as_str(pattern), as_str(text)) {
        (Some(p), Some(t)) => crate::regex::is_match(p, t).unwrap_or(false) as i64,
        _ => 0,
    }
}

/// # Safety
/// Every argument must be null or a valid `yk_string` pointer.
#[no_mangle]
pub unsafe extern "C" fn yk_re_replace(pattern: *const YkString, text: *const YkString, replacement: *const YkString) -> *mut YkString {
    let replaced = match (as_str(pattern), as_str(text), as_str(replacement)) {
        (Some(p), Some(t), Some(r)) => crate::regex::replace_all(p, t, r).unwrap_or_default(),
        _ => String::new(),
    };
    into_header(&replaced)
}

/// # Safety
/// `out` must be null or point to writable memory for one `yk_string`.
#[no_mangle]
pub unsafe extern "C" fn yk_read_line_into(out: *mut YkString) {
    let line = crate::input::Stdin.next_line().unwrap_or_default();
    write_into(out, &line);
}
