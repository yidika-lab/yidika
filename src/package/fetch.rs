use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

pub fn fetch(url: &str, commit: &str, dest: &Path) -> Result<(), String> {
    crate::ui::note(&format!("Fetching {}", url.trim_start_matches("https://")));
    match github_repo(url) {
        Some((owner, repo)) => fetch_archive(&format!("https://codeload.github.com/{}/{}/tar.gz/{}", owner, repo, commit), dest)
            .or_else(|_| fetch_git(url, commit, dest)),
        None => fetch_git(url, commit, dest),
    }
}

fn github_repo(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("https://github.com/")?;
    let (owner, repo) = rest.trim_end_matches('/').split_once('/')?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    let simple = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) && s != "." && s != "..";
    (simple(owner) && simple(repo)).then(|| (owner.to_string(), repo.to_string()))
}

fn fetch_archive(url: &str, dest: &Path) -> Result<(), String> {
    let bytes = crate::download::get(url, "yidi", MAX_ARCHIVE_BYTES, url.trim_start_matches("https://codeload.github.com/"))?;
    let _ = fs::remove_dir_all(dest);
    unpack(&bytes, dest).inspect_err(|_| {
        let _ = fs::remove_dir_all(dest);
    })
}

pub fn unpack(gzip: &[u8], dest: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(gzip));
    let entries = archive.entries().map_err(|e| format!("unreadable archive: {}", e))?;
    let mut unpacked = 0u64;
    for (count, entry) in entries.enumerate() {
        if count >= MAX_ENTRIES {
            return Err(format!("archive has more than {} entries", MAX_ENTRIES));
        }
        let mut entry = entry.map_err(|e| format!("unreadable archive: {}", e))?;
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            continue;
        }
        let path = entry.path().map_err(|e| format!("unreadable archive path: {}", e))?.into_owned();
        let Some(rel) = without_top_dir(&path) else { continue };
        let target = dest.join(&rel);
        if kind.is_dir() {
            fs::create_dir_all(&target).map_err(|e| format!("cannot create {}: {}", target.display(), e))?;
            continue;
        }
        unpacked += entry.header().size().unwrap_or(0);
        if unpacked > MAX_UNPACKED_BYTES {
            return Err(format!("archive unpacks to more than {} bytes", MAX_UNPACKED_BYTES));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {}", parent.display(), e))?;
        }
        let mut bytes = Vec::new();
        entry.by_ref().take(MAX_UNPACKED_BYTES).read_to_end(&mut bytes).map_err(|e| format!("unreadable archive entry: {}", e))?;
        fs::write(&target, bytes).map_err(|e| format!("cannot write {}: {}", target.display(), e))?;
    }
    fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {}", dest.display(), e))
}

fn without_top_dir(path: &Path) -> Option<PathBuf> {
    let mut parts = path.components();
    match parts.next() {
        Some(Component::Normal(_)) => {}
        _ => return None,
    }
    let rest: Vec<Component> = parts.collect();
    if rest.is_empty() || !rest.iter().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(rest.iter().collect())
}

fn fetch_git(url: &str, commit: &str, dest: &Path) -> Result<(), String> {
    let repo = dest.with_extension("git");
    let _ = fs::remove_dir_all(&repo);
    fs::create_dir_all(&repo).map_err(|e| format!("cannot create {}: {}", repo.display(), e))?;
    let result = git(&repo, &["init", "-q", "--bare"])
        .and_then(|_| git(&repo, &["fetch", "-q", "--depth", "1", "--", url, commit]))
        .and_then(|_| archive(&repo, "FETCH_HEAD"))
        .and_then(|bytes| unpack(&bytes, dest))
        .map_err(|e| format!("cannot fetch {} at {}: {}", url, commit, e));
    let _ = fs::remove_dir_all(&repo);
    if result.is_err() {
        let _ = fs::remove_dir_all(dest);
    }
    result
}

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git").arg("-C").arg(repo).args(args).output().map_err(|e| format!("cannot run git ({}): is git installed?", e))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

pub fn archive(repo: &Path, revision: &str) -> Result<Vec<u8>, String> {
    git(repo, &["-c", "core.autocrlf=false", "-c", "core.eol=lf", "archive", "--format=tar.gz", "--prefix=package/", revision])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (path, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn github_urls_map_to_owner_and_repo() {
        assert_eq!(github_repo("https://github.com/user/greet.git"), Some(("user".into(), "greet".into())));
        assert_eq!(github_repo("https://github.com/user/greet"), Some(("user".into(), "greet".into())));
        assert_eq!(github_repo("https://gitlab.com/user/greet.git"), None);
        assert_eq!(github_repo("https://github.com/../x"), None);
    }

    #[test]
    fn archives_lose_their_top_folder() {
        let dir = TempDir::new();
        unpack(&tarball(&[("greet-abc/greet.yk", b"x"), ("greet-abc/text/upper.yk", b"y")]), &dir.path().join("out")).unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("out/greet.yk")).unwrap(), "x");
        assert_eq!(fs::read_to_string(dir.path().join("out/text/upper.yk")).unwrap(), "y");
    }

    #[test]
    fn archive_entries_cannot_escape_the_destination() {
        let dir = TempDir::new();
        unpack(&tarball(&[("top/../../evil.yk", b"x"), ("/abs.yk", b"x"), ("top/ok.yk", b"y")]), &dir.path().join("out")).unwrap();
        assert!(!dir.path().join("evil.yk").exists());
        assert!(!dir.path().join("abs.yk").exists());
        assert!(dir.path().join("out/ok.yk").exists());
    }
}
