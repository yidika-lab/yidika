use super::global;
use super::lock::{Lock, Locked};
use super::pool::parallel;
use super::store;
use super::tree;
use std::fs;
use std::path::Path;

const STATE: &str = ".yidi-state";

pub enum Outcome {
    UpToDate,
    Installed { downloaded: usize },
}

pub fn install(cache: &Path, root: &Path, lock: &Lock) -> Result<Outcome, String> {
    let lib = store::lib_dir(root);
    let state = lib.join(STATE);
    let digest = lock.digest();
    let present = lock.packages.iter().all(|p| store::package_dir(root, &p.name).is_ok_and(|d| d.is_dir()));
    if present && fs::read_to_string(&state).is_ok_and(|s| s == digest) {
        return Ok(Outcome::UpToDate);
    }
    let fetched = parallel(&lock.packages, |p| verified(cache, p));
    fs::create_dir_all(&lib).map_err(|e| format!("cannot create {}: {}", lib.display(), e))?;
    let mut downloaded = 0;
    for (p, result) in lock.packages.iter().zip(fetched) {
        let (dir, was_downloaded) = result?;
        downloaded += was_downloaded as usize;
        place(root, p, &dir)?;
    }
    prune(root, lock)?;
    fs::write(&state, digest).map_err(|e| format!("cannot write {}: {}", state.display(), e))?;
    Ok(Outcome::Installed { downloaded })
}

fn verified(cache: &Path, p: &Locked) -> Result<(std::path::PathBuf, bool), String> {
    let first = global::ensure(cache, &p.source, &p.commit)?;
    if tree::hash(&first.dir)? == p.hash {
        return Ok((first.dir, first.downloaded));
    }
    global::evict(cache, &p.commit);
    let again = global::ensure(cache, &p.source, &p.commit)?;
    if tree::hash(&again.dir)? == p.hash {
        return Ok((again.dir, true));
    }
    Err(format!("integrity check failed for '{}' at {}: its contents do not match {} in {}", p.name, p.commit, p.hash, super::lock::FILE))
}

fn place(root: &Path, p: &Locked, from: &Path) -> Result<(), String> {
    let dir = store::package_dir(root, &p.name)?;
    if fs::read_to_string(dir.join(tree::MARKER)).is_ok_and(|h| h == p.hash) {
        return Ok(());
    }
    store::remove_package_dir(root, &p.name)?;
    tree::link(from, &dir)?;
    fs::write(dir.join(tree::MARKER), &p.hash).map_err(|e| format!("cannot write {}: {}", dir.display(), e))
}

fn prune(root: &Path, lock: &Lock) -> Result<(), String> {
    let Ok(entries) = fs::read_dir(store::lib_dir(root)) else { return Ok(()) };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let wanted = lock.packages.iter().any(|p| p.name == name);
        if !wanted && super::name::validate(&name).is_ok() {
            store::remove_package_dir(root, &name)?;
        }
    }
    Ok(())
}
