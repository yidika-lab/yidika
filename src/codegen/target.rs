pub fn host_triple() -> &'static str {
    triple_for(std::env::consts::ARCH, std::env::consts::OS)
}

fn triple_for(arch: &str, os: &str) -> &'static str {
    match (arch, os) {
        ("x86_64", "windows") => "x86_64-pc-windows-msvc",
        ("aarch64", "windows") => "aarch64-pc-windows-msvc",
        ("x86_64", "macos") => "x86_64-apple-macosx",
        ("aarch64", "macos") => "arm64-apple-macosx",
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
        (_, "linux") => "x86_64-unknown-linux-gnu",
        _ => "x86_64-pc-windows-msvc",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_hosts_map_to_llvm_triples() {
        assert_eq!(triple_for("x86_64", "windows"), "x86_64-pc-windows-msvc");
        assert_eq!(triple_for("x86_64", "linux"), "x86_64-unknown-linux-gnu");
        assert_eq!(triple_for("aarch64", "linux"), "aarch64-unknown-linux-gnu");
        assert_eq!(triple_for("aarch64", "macos"), "arm64-apple-macosx");
        assert_eq!(triple_for("x86_64", "macos"), "x86_64-apple-macosx");
    }

    #[test]
    fn host_triple_matches_this_machine() {
        assert_eq!(host_triple(), triple_for(std::env::consts::ARCH, std::env::consts::OS));
    }
}
