use super::index::{self, Entry, Release};
use super::{fetch, name, remote, tree, version};
use std::path::Path;

const HOST: &str = "https://github.com/";

pub fn check(entry_file: &Path, previous_file: Option<&Path>) -> Result<String, String> {
    let package = entry_file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    check_name(package)?;
    let entry = read(entry_file)?;
    let previous = previous_file.map(read).transpose()?;
    check_source(&entry.source)?;
    let added = new_releases(previous.as_ref(), &entry)?;
    for release in &added {
        check_release(&entry.source, release)?;
    }
    let versions: Vec<&str> = added.iter().map(|r| r.version.as_str()).collect();
    Ok(format!("{}: {} release(s) verified ({})", package, added.len(), if versions.is_empty() { "yank flags only".to_string() } else { versions.join(", ") }))
}

fn read(file: &Path) -> Result<Entry, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("cannot read {}: {}", file.display(), e))?;
    index::parse(&text).map_err(|e| format!("{}: {}", file.display(), e))
}

fn check_name(package: &str) -> Result<(), String> {
    name::validate(package)?;
    if crate::module::BUILTIN_MODULES.contains(&package) {
        return Err(format!("'{}' is a std module name", package));
    }
    Ok(())
}

fn check_source(source: &str) -> Result<(), String> {
    let path = source.strip_prefix(HOST).ok_or_else(|| format!("source must be a repository on {}", HOST))?;
    let parts: Vec<&str> = path.trim_end_matches(".git").split('/').collect();
    if parts.len() == 2 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) && *p != "." && *p != "..") {
        Ok(())
    } else {
        Err(format!("source '{}' is not https://github.com/<owner>/<repository>", source))
    }
}

fn new_releases(previous: Option<&Entry>, entry: &Entry) -> Result<Vec<Release>, String> {
    let mut seen: Vec<&str> = Vec::new();
    for r in &entry.releases {
        if seen.contains(&r.version.as_str()) {
            return Err(format!("version {} is listed twice", r.version));
        }
        seen.push(&r.version);
    }
    let Some(old) = previous else {
        return if entry.releases.is_empty() { Err("a new package needs at least one release".into()) } else { Ok(entry.releases.clone()) };
    };
    if old.source != entry.source {
        return Err(format!("the source of a package cannot change ({} to {})", old.source, entry.source));
    }
    for o in &old.releases {
        let kept = entry.releases.iter().find(|r| r.version == o.version).ok_or_else(|| format!("release {} was removed; mark it yanked instead", o.version))?;
        if kept.commit != o.commit || kept.hash != o.hash {
            return Err(format!("release {} is published and cannot change", o.version));
        }
    }
    Ok(entry.releases.iter().filter(|r| !old.releases.iter().any(|o| o.version == r.version)).cloned().collect())
}

fn check_release(source: &str, release: &Release) -> Result<(), String> {
    let wanted = version::parse_tag(&release.version).ok_or_else(|| format!("'{}' is not a version", release.version))?;
    let listing = remote::query(source)?;
    let tag = listing.tags.iter().find(|t| t.version == wanted).ok_or_else(|| format!("{} has no tag v{}", source, release.version))?;
    if tag.commit != release.commit {
        return Err(format!("tag v{} of {} is commit {}, not {}", release.version, source, tag.commit, release.commit));
    }
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let staging = std::env::temp_dir().join(format!("yidi-verify-{}-{}", std::process::id(), RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let found = fetch::fetch(source, &release.commit, &staging).and_then(|_| tree::hash(&staging));
    let _ = std::fs::remove_dir_all(&staging);
    let found = found?;
    if found == release.hash {
        Ok(())
    } else {
        Err(format!("release {} has hash {}, but its contents hash to {}", release.version, release.hash, found))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_file, TempDir};
    use std::process::Command;

    fn release(v: &str, commit: &str, hash: &str) -> Release {
        Release { version: v.into(), commit: commit.repeat(40), hash: format!("sha256-{}", hash), yanked: false }
    }

    fn entry(source: &str, releases: Vec<Release>) -> Entry {
        Entry { source: source.into(), releases }
    }

    #[test]
    fn only_github_repositories_are_accepted_as_sources() {
        assert!(check_source("https://github.com/user/greet.git").is_ok());
        assert!(check_source("https://github.com/user/greet").is_ok());
        for bad in ["https://gitlab.com/user/greet.git", "https://github.com/user", "https://github.com/user/../x", "https://github.com/a/b/c"] {
            assert!(check_source(bad).is_err(), "{}", bad);
        }
    }

    #[test]
    fn names_follow_the_package_rules() {
        assert!(check_name("greet").is_ok());
        assert!(check_name("math").is_err());
        assert!(check_name("..").is_err());
    }

    #[test]
    fn history_is_append_only() {
        let old = entry("s", vec![release("1.0.0", "a", "x")]);
        let grown = entry("s", vec![release("1.0.0", "a", "x"), release("1.1.0", "b", "y")]);
        assert_eq!(new_releases(Some(&old), &grown).unwrap(), vec![release("1.1.0", "b", "y")]);
        let mut yanked = old.clone();
        yanked.releases[0].yanked = true;
        assert!(new_releases(Some(&old), &yanked).unwrap().is_empty());
        assert!(new_releases(Some(&old), &entry("s", vec![])).unwrap_err().contains("removed"));
        assert!(new_releases(Some(&old), &entry("s", vec![release("1.0.0", "c", "x")])).unwrap_err().contains("cannot change"));
        assert!(new_releases(Some(&old), &entry("other", vec![release("1.0.0", "a", "x")])).unwrap_err().contains("source"));
        assert!(new_releases(None, &entry("s", vec![])).is_err());
        assert!(new_releases(None, &entry("s", vec![release("1.0.0", "a", "x"), release("1.0.0", "b", "y")])).unwrap_err().contains("twice"));
    }

    #[test]
    fn a_release_must_match_its_tag_and_its_contents() {
        let dir = TempDir::new();
        let repo = dir.path().join("greet");
        write_file(&repo, "greet.yk", "export fn hello() -> str { return \"hi\"; }\n");
        let git = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(&repo).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args).output().unwrap();
            assert!(out.status.success(), "git {:?}", args);
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        git(&["init", "-q"]);
        git(&["add", "."]);
        git(&["commit", "-qm", "v1"]);
        git(&["tag", "v1.0.0"]);
        let commit = git(&["rev-parse", "HEAD"]);
        let p = repo.display().to_string().replace('\\', "/");
        let url = format!("file://{}{}", if p.starts_with('/') { "" } else { "/" }, p);
        let copy = TempDir::new();
        fetch::fetch(&url, &commit, &copy.path().join("c")).unwrap();
        let good = Release { version: "1.0.0".into(), commit: commit.clone(), hash: tree::hash(&copy.path().join("c")).unwrap(), yanked: false };
        assert!(check_release(&url, &good).is_ok());
        let forged = Release { hash: "sha256-forged".into(), ..good.clone() };
        assert!(check_release(&url, &forged).unwrap_err().contains("hash to"));
        let moved = Release { commit: "b".repeat(40), ..good.clone() };
        assert!(check_release(&url, &moved).unwrap_err().contains("is commit"));
        let untagged = Release { version: "2.0.0".into(), ..good };
        assert!(check_release(&url, &untagged).unwrap_err().contains("no tag v2.0.0"));
    }
}
