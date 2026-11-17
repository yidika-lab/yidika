use std::ffi::c_void;

#[repr(C)]
pub struct YkString {
    data: *mut u8,
    len: i64,
    cap: i64,
}

extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn calloc(count: usize, size: usize) -> *mut c_void;
}

/// # Safety
/// `s` must be null or point to a valid `yk_string` whose `data` holds `len` readable bytes.
pub unsafe fn as_str<'a>(s: *const YkString) -> Option<&'a str> {
    let s = s.as_ref()?;
    if s.data.is_null() || s.len <= 0 {
        return Some("");
    }
    std::str::from_utf8(std::slice::from_raw_parts(s.data, s.len as usize)).ok()
}

pub fn into_header(text: &str) -> *mut YkString {
    let header = unsafe { calloc(1, std::mem::size_of::<YkString>()) } as *mut YkString;
    if header.is_null() {
        return header;
    }
    let size = text.len() + 1;
    let data = unsafe { malloc(size) } as *mut u8;
    if data.is_null() {
        return header;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), data, text.len());
        *data.add(text.len()) = 0;
        *header = YkString { data, len: text.len() as i64, cap: size as i64 };
    }
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "C" {
        fn free(ptr: *mut c_void);
    }

    fn release(s: *mut YkString) {
        unsafe {
            free((*s).data as *mut c_void);
            free(s as *mut c_void);
        }
    }

    #[test]
    fn a_header_round_trips_and_owns_its_buffer() {
        let s = into_header("hello");
        assert_eq!(unsafe { as_str(s) }, Some("hello"));
        assert_eq!(unsafe { (*s).cap }, 6);
        release(s);
    }

    #[test]
    fn null_and_empty_strings_read_as_empty_or_none() {
        assert_eq!(unsafe { as_str(std::ptr::null()) }, None);
        let empty = YkString { data: std::ptr::null_mut(), len: 0, cap: 0 };
        assert_eq!(unsafe { as_str(&empty) }, Some(""));
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let mut bytes = [0xffu8, 0xfe];
        let bad = YkString { data: bytes.as_mut_ptr(), len: 2, cap: 0 };
        assert_eq!(unsafe { as_str(&bad) }, None);
    }
}
