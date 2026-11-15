use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub const MARKER: &str = ".yidi-hash";
const SKIPPED: [&str; 2] = [".git", MARKER];

pub fn files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    collect(dir, Path::new(""), &mut found)?;
    found.sort();
    Ok(found)
}

fn collect(base: &Path, rel: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    let dir = base.join(rel);
    let entries = fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {}", dir.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {}", dir.display(), e))?;
        let name = entry.file_name();
        if SKIPPED.iter().any(|s| name == *s) {
            continue;
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let child = rel.join(&name);
        if kind.is_dir() {
            collect(base, &child, found)?;
        } else if kind.is_file() {
            found.push(child);
        }
    }
    Ok(())
}

pub fn hash(dir: &Path) -> Result<String, String> {
    let mut hasher = Sha256::new();
    for rel in files(dir)? {
        let bytes = fs::read(dir.join(&rel)).map_err(|e| format!("cannot read {}: {}", rel.display(), e))?;
        let path = rel.to_string_lossy().replace('\\', "/");
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(format!("sha256-{}", hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>()))
}

pub fn link(src: &Path, dst: &Path) -> Result<(), String> {
    for rel in files(src)? {
        let to = dst.join(&rel);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {}", parent.display(), e))?;
        }
        let from = src.join(&rel);
        if fs::hard_link(&from, &to).is_err() {
            fs::copy(&from, &to).map_err(|e| format!("cannot copy {} to {}: {}", from.display(), to.display(), e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_file, TempDir};

    #[test]
    fn the_hash_depends_on_paths_and_contents_only() {
        let a = TempDir::new();
        let b = TempDir::new();
        for dir in [a.path(), b.path()] {
            write_file(dir, "greet.yk", "x");
            write_file(dir, "text/upper.yk", "y");
        }
        write_file(b.path(), ".git/HEAD", "ignored");
        write_file(b.path(), MARKER, "ignored");
        assert_eq!(hash(a.path()).unwrap(), hash(b.path()).unwrap());
        write_file(b.path(), "text/upper.yk", "z");
        assert_ne!(hash(a.path()).unwrap(), hash(b.path()).unwrap());
    }

    #[test]
    fn moving_bytes_between_files_changes_the_hash() {
        let a = TempDir::new();
        let b = TempDir::new();
        write_file(a.path(), "a", "xy");
        write_file(a.path(), "b", "");
        write_file(b.path(), "a", "x");
        write_file(b.path(), "b", "y");
        assert_ne!(hash(a.path()).unwrap(), hash(b.path()).unwrap());
    }

    #[test]
    fn linking_reproduces_the_tree_without_git_data() {
        let src = TempDir::new();
        let dst = TempDir::new();
        write_file(src.path(), "greet.yk", "x");
        write_file(src.path(), "text/upper.yk", "y");
        write_file(src.path(), ".git/HEAD", "z");
        link(src.path(), &dst.path().join("greet")).unwrap();
        assert_eq!(hash(src.path()).unwrap(), hash(&dst.path().join("greet")).unwrap());
        assert!(!dst.path().join("greet/.git").exists());
    }
}
