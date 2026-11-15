use super::{fetch, remote, tree};
use std::fs;
use std::path::{Path, PathBuf};

pub fn default_root() -> PathBuf {
    crate::cache::cache_dir().join("packages")
}

pub struct Cached {
    pub dir: PathBuf,
    pub downloaded: bool,
}

pub fn ensure(root: &Path, url: &str, commit: &str) -> Result<Cached, String> {
    if !remote::is_commit(commit) {
        return Err(format!("invalid commit '{}'", commit));
    }
    let dir = root.join(commit);
    if dir.join(tree::MARKER).is_file() {
        return Ok(Cached { dir, downloaded: false });
    }
    fs::create_dir_all(root).map_err(|e| format!("cannot create {}: {}", root.display(), e))?;
    let staging = root.join(format!(".tmp-{}-{}", commit, std::process::id()));
    fetch::fetch(url, commit, &staging)?;
    let hash = tree::hash(&staging)?;
    fs::write(staging.join(tree::MARKER), &hash).map_err(|e| format!("cannot write {}: {}", staging.display(), e))?;
    if fs::rename(&staging, &dir).is_err() {
        let _ = fs::remove_dir_all(&staging);
        if !dir.join(tree::MARKER).is_file() {
            return Err(format!("cannot store {} in {}", commit, root.display()));
        }
    }
    Ok(Cached { dir, downloaded: true })
}

pub fn evict(root: &Path, commit: &str) {
    if remote::is_commit(commit) {
        let _ = fs::remove_dir_all(root.join(commit));
    }
}
