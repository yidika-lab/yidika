pub mod ffi;
pub mod regex;
pub mod string;

pub const ABI_VERSION: i64 = 1;

#[no_mangle]
pub extern "C" fn yk_runtime_abi() -> i64 {
    ABI_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_abi_version_is_exported() {
        assert_eq!(yk_runtime_abi(), ABI_VERSION);
    }
}
