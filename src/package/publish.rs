use super::index::{self, Entry, Index, Release};
use super::manifest::Manifest;
use super::{fetch, tree};
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub struct Prepared {
    pub name: String,
    pub version: String,
    pub commit: String,
    pub entry: Entry,
}

pub fn publish(dry_run: bool) -> Result<String, String> {
    let root = std::env::current_dir().map_err(|e| format!("cannot read the current directory: {}", e))?;
    let task = crate::ui::Task::start("Preparing", "the release", 1);
    let prepared = prepare(&root, &Index::default_index())?;
    drop(task);
    let what = format!("{} {} ({})", prepared.name, prepared.version, &prepared.commit[..12]);
    if dry_run {
        return Ok(format!("Checked {}: ready to publish. Registry entry:\n\n{}", what, index::render(&prepared.entry)));
    }
    let task = crate::ui::Task::start("Publishing", &prepared.name, 1);
    push_tag(&root, &prepared)?;
    let submitted = match submit_with_gh(&prepared) {
        Ok(url) => format!("registry pull request: {}", url),
        Err(why) => format!("could not open the registry pull request automatically ({}).\nOpen it in the browser:\n  {}", why, manual_link(&prepared)),
    };
    drop(task);
    Ok(format!("Published {}: tag v{} pushed; {}", what, prepared.version, submitted))
}

pub fn prepare(root: &Path, registry: &Index) -> Result<Prepared, String> {
    let (name, version) = Manifest::load(root)?.package()?;
    if crate::module::BUILTIN_MODULES.contains(&name.as_str()) {
        return Err(format!("'{}' is a std module name; rename the package", name));
    }
    if !git(root, &["status", "--porcelain", "--untracked-files=no"])?.trim().is_empty() {
        return Err("the working tree has uncommitted changes; commit them first so the release matches what you tested".into());
    }
    let commit = git(root, &["rev-parse", "HEAD"])?.trim().to_string();
    let source = https_source(git(root, &["remote", "get-url", "origin"])?.trim())?;
    check_tag(root, &version, &commit)?;
    check_entry_file(root, &name)?;
    let release = Release { version: version.clone(), commit: commit.clone(), hash: committed_hash(root)?, yanked: false };
    let entry = Entry::with_release(registry.entry(&name)?, &source, release)?;
    Ok(Prepared { name, version, commit, entry })
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().map_err(|e| format!("cannot run git ({}): is git installed?", e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim()))
    }
}

fn https_source(origin: &str) -> Result<String, String> {
    if let Some(path) = origin.strip_prefix("git@github.com:") {
        return Ok(format!("https://github.com/{}", path));
    }
    if origin.starts_with("https://") || origin.starts_with("file://") {
        return Ok(origin.to_string());
    }
    Err(format!("the origin remote '{}' is not an https URL that others can download from", origin))
}

fn check_tag(root: &Path, version: &str, commit: &str) -> Result<(), String> {
    match git(root, &["rev-parse", "--verify", "--quiet", &format!("refs/tags/v{}^{{commit}}", version)]) {
        Ok(tagged) if tagged.trim() != commit => Err(format!("tag v{} already points to another commit ({})", version, tagged.trim())),
        _ => Ok(()),
    }
}

fn check_entry_file(root: &Path, name: &str) -> Result<(), String> {
    let entry = [format!("{}.yk", name), "main.yk".to_string()].into_iter().map(|f| root.join(f)).find(|p| p.is_file());
    let entry = entry.ok_or_else(|| format!("a package needs {}.yk or main.yk at its root, so `use ... from \"{}\"` finds it", name, name))?;
    crate::cli::check_project(&entry)
}

fn committed_hash(root: &Path) -> Result<String, String> {
    let bytes = fetch::archive(root, "HEAD")?;
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let staging = std::env::temp_dir().join(format!("yidi-publish-{}-{}", std::process::id(), run));
    let result = fetch::unpack(&bytes, &staging).and_then(|_| tree::hash(&staging));
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn push_tag(root: &Path, p: &Prepared) -> Result<(), String> {
    let tag = format!("v{}", p.version);
    if git(root, &["rev-parse", "--verify", "--quiet", &format!("refs/tags/{}", tag)]).is_err() {
        git(root, &["tag", "-a", &tag, "-m", &format!("{} {}", p.name, p.version), &p.commit])?;
    }
    git(root, &["push", "origin", &format!("refs/tags/{}", tag)]).map(|_| ())
}

fn gh(args: &[&str]) -> Result<String, String> {
    let output = Command::new("gh").args(args).output().map_err(|e| format!("gh is not available: {}", e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!("gh {} failed: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&output.stderr).trim()))
    }
}

fn submit_with_gh(p: &Prepared) -> Result<String, String> {
    use base64::Engine;
    let login = gh(&["api", "user", "--jq", ".login"])?;
    let fork = format!("{}/registry", login);
    let _ = gh(&["repo", "fork", index::REGISTRY_REPO, "--clone=false"]);
    let _ = gh(&["api", "-X", "POST", &format!("repos/{}/merge-upstream", fork), "-f", "branch=main"]);
    let base = gh(&["api", &format!("repos/{}/git/ref/heads/main", fork), "--jq", ".object.sha"])?;
    let branch = format!("publish/{}-{}", p.name, p.version);
    gh(&["api", "-X", "POST", &format!("repos/{}/git/refs", fork), "-f", &format!("ref=refs/heads/{}", branch), "-f", &format!("sha={}", base)])?;
    let path = index::entry_path(&p.name);
    let content = base64::engine::general_purpose::STANDARD.encode(index::render(&p.entry));
    let message = format!("publish {} {}", p.name, p.version);
    let mut put = vec!["api".to_string(), "-X".into(), "PUT".into(), format!("repos/{}/contents/{}", fork, path), "-f".into(), format!("message={}", message), "-f".into(), format!("content={}", content), "-f".into(), format!("branch={}", branch)];
    if let Ok(sha) = gh(&["api", &format!("repos/{}/contents/{}?ref={}", fork, path, branch), "--jq", ".sha"]) {
        put.extend(["-f".into(), format!("sha={}", sha)]);
    }
    gh(&put.iter().map(String::as_str).collect::<Vec<_>>())?;
    let body = format!("Publishes {} {} from {} at {}.", p.name, p.version, p.entry.source, p.commit);
    gh(&["pr", "create", "--repo", index::REGISTRY_REPO, "--head", &format!("{}:{}", login, branch), "--title", &message, "--body", &body])
}

fn manual_link(p: &Prepared) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    let value = utf8_percent_encode(&index::render(&p.entry), NON_ALPHANUMERIC).to_string();
    format!("https://github.com/{}/new/main?filename={}&value={}", index::REGISTRY_REPO, index::entry_path(&p.name), value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_file, TempDir};

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args).output().unwrap().status.success();
        assert!(ok, "git {:?}", args);
    }

    fn package(version: &str) -> TempDir {
        let dir = TempDir::new();
        write_file(dir.path(), "project.toml", &format!("[package]\nname = \"greet\"\nversion = \"{}\"\n", version));
        write_file(dir.path(), "greet.yk", "export fn hello() -> str { return \"hi\"; }\n");
        run(dir.path(), &["init", "-q"]);
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-qm", "release"]);
        run(dir.path(), &["remote", "add", "origin", "https://github.com/someone/greet.git"]);
        dir
    }

    #[test]
    fn a_clean_package_prepares_an_entry_whose_hash_matches_a_download() {
        let pkg = package("1.2.0");
        let index = TempDir::new();
        let p = prepare(pkg.path(), &Index::Dir(index.path().to_path_buf())).unwrap();
        assert_eq!((p.name.as_str(), p.version.as_str()), ("greet", "1.2.0"));
        assert_eq!(p.entry.source, "https://github.com/someone/greet.git");
        let copy = TempDir::new();
        let url = format!("file://{}", pkg.path().display().to_string().replace('\\', "/"));
        fetch::fetch(&url, &p.commit, &copy.path().join("c")).unwrap();
        assert_eq!(p.entry.releases[0].hash, tree::hash(&copy.path().join("c")).unwrap());
    }

    #[test]
    fn the_hash_ignores_the_line_ending_settings_of_the_machine() {
        let pkg = package("1.0.0");
        let index = TempDir::new();
        let plain = prepare(pkg.path(), &Index::Dir(index.path().to_path_buf())).unwrap();
        run(pkg.path(), &["config", "core.autocrlf", "true"]);
        let converted = prepare(pkg.path(), &Index::Dir(index.path().to_path_buf())).unwrap();
        assert_eq!(plain.entry.releases[0].hash, converted.entry.releases[0].hash);
    }

    #[test]
    fn uncommitted_changes_block_publishing() {
        let pkg = package("1.0.0");
        write_file(pkg.path(), "greet.yk", "changed");
        let index = TempDir::new();
        assert!(prepare(pkg.path(), &Index::Dir(index.path().to_path_buf())).unwrap_err().contains("uncommitted"));
    }

    #[test]
    fn a_published_version_cannot_be_published_again() {
        let pkg = package("1.0.0");
        let index = TempDir::new();
        let first = prepare(pkg.path(), &Index::Dir(index.path().to_path_buf())).unwrap();
        write_file(index.path(), &index::entry_path("greet"), &index::render(&first.entry));
        assert!(prepare(pkg.path(), &Index::Dir(index.path().to_path_buf())).unwrap_err().contains("already published"));
    }

    #[test]
    fn a_package_that_does_not_type_check_is_refused() {
        let pkg = package("1.0.0");
        write_file(pkg.path(), "greet.yk", "export fn hello() -> int { return \"no\"; }\n");
        run(pkg.path(), &["commit", "-qam", "broken"]);
        let index = TempDir::new();
        assert!(prepare(pkg.path(), &Index::Dir(index.path().to_path_buf())).is_err());
    }

    #[test]
    fn ssh_origins_become_https_and_others_are_refused() {
        assert_eq!(https_source("git@github.com:u/greet.git").unwrap(), "https://github.com/u/greet.git");
        assert!(https_source("/home/me/greet").is_err());
    }

    #[test]
    fn the_manual_link_prefills_the_entry() {
        let pkg = package("1.0.0");
        let index = TempDir::new();
        let p = prepare(pkg.path(), &Index::Dir(index.path().to_path_buf())).unwrap();
        let link = manual_link(&p);
        assert!(link.starts_with("https://github.com/yidika-lab/registry/new/main?filename=packages/greet.toml&value=source"), "{}", link);
    }
}
