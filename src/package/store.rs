use super::name;
use std::fs;
use std::path::{Path, PathBuf};

pub const PACKAGES_DIR: &str = "@yidi";

pub fn lib_dir(root: &Path) -> PathBuf {
    root.join(PACKAGES_DIR)
}

pub fn find_import(importer_dir: &Path, spec: &str) -> Option<PathBuf> {
    let (package, sub) = match spec.split_once('/') {
        Some((p, s)) => (p, Some(s)),
        None => (spec, None),
    };
    name::validate(package).ok()?;
    if let Some(s) = sub {
        if !s.split('/').all(|part| name::validate(part.strip_suffix(".yk").unwrap_or(part)).is_ok()) {
            return None;
        }
    }
    importer_dir.ancestors().map(|dir| lib_dir(dir).join(package)).find(|d| d.is_dir()).and_then(|dir| entry_file(&dir, package, sub))
}

fn entry_file(dir: &Path, package: &str, sub: Option<&str>) -> Option<PathBuf> {
    let candidates = match sub {
        Some(s) => vec![dir.join(s).with_extension("yk")],
        None => vec![dir.join(format!("{}.yk", package)), dir.join("main.yk")],
    };
    candidates.into_iter().find(|c| c.is_file())
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
    let lib = lib_dir(root).canonicalize().map_err(|e| format!("cannot resolve {}/: {}", PACKAGES_DIR, e))?;
    let parent = dir.canonicalize().ok().and_then(|d| d.parent().map(Path::to_path_buf));
    if parent.as_deref() == Some(lib.as_path()) {
        Ok(())
    } else {
        Err(format!("refusing to remove {}: it is not a package directory inside {}/", dir.display(), PACKAGES_DIR))
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
        write_file(dir.path(), "@yidi/greet/main.yk", "x");
        write_file(dir.path(), "@yidi/other/main.yk", "x");
        assert!(remove_package_dir(dir.path(), "greet").unwrap());
        assert!(!dir.path().join("@yidi/greet").exists());
        assert!(dir.path().join("@yidi/other/main.yk").exists() && dir.path().join("main.yk").exists());
        assert!(!remove_package_dir(dir.path(), "greet").unwrap());
    }

    #[test]
    fn traversal_names_never_reach_the_filesystem() {
        let dir = TempDir::new();
        write_file(dir.path(), "main.yk", "x");
        write_file(dir.path(), "@yidi/greet/main.yk", "x");
        for bad in ["..", ".", "../..", ""] {
            assert!(remove_package_dir(dir.path(), bad).is_err(), "{}", bad);
        }
        assert!(dir.path().join("main.yk").exists());
    }

    #[test]
    fn an_import_finds_the_nearest_package_directory_upwards() {
        let dir = TempDir::new();
        write_file(dir.path(), "@yidi/greet/greet.yk", "x");
        write_file(dir.path(), "@yidi/greet/main.yk", "x");
        write_file(dir.path(), "@yidi/greet/text/upper.yk", "x");
        write_file(dir.path(), "@yidi/solo/main.yk", "x");
        let deep = dir.path().join("src/api");
        fs::create_dir_all(&deep).unwrap();
        assert_eq!(find_import(&deep, "greet"), Some(dir.path().join("@yidi/greet/greet.yk")));
        assert_eq!(find_import(&deep, "solo"), Some(dir.path().join("@yidi/solo/main.yk")));
        assert_eq!(find_import(&deep, "greet/text/upper"), Some(dir.path().join("@yidi/greet/text/upper.yk")));
        assert_eq!(find_import(&deep, "greet/text/upper.yk"), Some(dir.path().join("@yidi/greet/text/upper.yk")));
        assert_eq!(find_import(&deep, "missing"), None);
    }

    #[test]
    fn an_import_cannot_climb_out_of_a_package() {
        let dir = TempDir::new();
        write_file(dir.path(), "secret.yk", "x");
        write_file(dir.path(), "@yidi/greet/main.yk", "x");
        for spec in ["greet/../../secret", "../secret", "greet/", "greet//main", "@yidi/greet"] {
            assert_eq!(find_import(dir.path(), spec), None, "{}", spec);
        }
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
