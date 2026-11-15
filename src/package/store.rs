use super::name;
use std::fs;
use std::path::{Path, PathBuf};

pub fn lib_dir(root: &Path) -> PathBuf {
    root.join("lib")
}

pub fn package_dir(root: &Path, package: &str) -> Result<PathBuf, String> {
    Ok(lib_dir(root).join(name::validate(package)?))
}

pub fn remove_package_dir(root: &Path, package: &str) -> Result<bool, String> {
    let dir = package_dir(root, package)?;
    let meta = match fs::symlink_metadata(&dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("cannot inspect {}: {}", dir.display(), e)),
    };
    if !meta.is_dir() {
        fs::remove_file(&dir).map_err(|e| format!("cannot remove {}: {}", dir.display(), e))?;
        return Ok(true);
    }
    ensure_inside_lib(root, &dir)?;
    fs::remove_dir_all(&dir).map_err(|e| format!("cannot remove {}: {}", dir.display(), e))?;
    Ok(true)
}

fn ensure_inside_lib(root: &Path, dir: &Path) -> Result<(), String> {
    let lib = lib_dir(root).canonicalize().map_err(|e| format!("cannot resolve lib/: {}", e))?;
    let parent = dir.canonicalize().ok().and_then(|d| d.parent().map(Path::to_path_buf));
    if parent.as_deref() == Some(lib.as_path()) {
        Ok(())
    } else {
        Err(format!("refusing to remove {}: it is not a package directory inside lib/", dir.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_file, TempDir};

    #[test]
    fn removes_a_package_and_nothing_else() {
        let dir = TempDir::new();
        write_file(dir.path(), "main.yk", "x");
        write_file(dir.path(), "lib/greet/main.yk", "x");
        write_file(dir.path(), "lib/other/main.yk", "x");
        assert!(remove_package_dir(dir.path(), "greet").unwrap());
        assert!(!dir.path().join("lib/greet").exists());
        assert!(dir.path().join("lib/other/main.yk").exists() && dir.path().join("main.yk").exists());
        assert!(!remove_package_dir(dir.path(), "greet").unwrap());
    }

    #[test]
    fn traversal_names_never_reach_the_filesystem() {
        let dir = TempDir::new();
        write_file(dir.path(), "main.yk", "x");
        write_file(dir.path(), "lib/greet/main.yk", "x");
        for bad in ["..", ".", "../..", ""] {
            assert!(remove_package_dir(dir.path(), bad).is_err(), "{}", bad);
        }
        assert!(dir.path().join("main.yk").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_package_is_unlinked_not_followed() {
        let dir = TempDir::new();
        let outside = TempDir::new();
        write_file(outside.path(), "keep.yk", "x");
        fs::create_dir_all(lib_dir(dir.path())).unwrap();
        std::os::unix::fs::symlink(outside.path(), lib_dir(dir.path()).join("greet")).unwrap();
        assert!(remove_package_dir(dir.path(), "greet").unwrap());
        assert!(outside.path().join("keep.yk").exists());
    }
}
