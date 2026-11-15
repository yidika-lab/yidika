use std::path::PathBuf;

pub fn cache_dir() -> PathBuf {
    if let Some(dir) = non_empty_env("XDG_CACHE_HOME") {
        return PathBuf::from(dir).join("yidi");
    }
    if cfg!(windows) {
        let base = non_empty_env("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        return base.join("yidi").join("cache");
    }
    let home = non_empty_env("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    if cfg!(target_os = "macos") {
        home.join("Library/Caches/yidi")
    } else {
        home.join(".cache/yidi")
    }
}

fn non_empty_env(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}
