mod fetch;
mod global;
mod index;
mod init;
mod install;
mod lock;
mod manifest;
pub(crate) mod name;
mod pool;
mod publish;
mod remote;
mod resolve;
pub(crate) mod store;
mod tree;
mod version;

pub use init::init;
pub use publish::publish;

use index::Index;
use install::Outcome;
use lock::{Lock, Locked};
use manifest::{Dep, Manifest};
use resolve::Resolver;
use std::path::{Path, PathBuf};
use std::time::Instant;

struct Env {
    cache: PathBuf,
    index: Index,
}

impl Env {
    fn standard() -> Self {
        Self { cache: global::default_root(), index: Index::default_index() }
    }

    fn resolver(&self) -> Resolver {
        Resolver::new(&self.cache, self.index.clone())
    }
}

pub fn add(spec: &str) -> Result<String, String> {
    timed(|| add_in(&Env::standard(), &current_dir()?, spec))
}

pub fn sync() -> Result<String, String> {
    timed(|| sync_in(&Env::standard(), &current_dir()?))
}

pub fn update(package: Option<&str>) -> Result<String, String> {
    timed(|| update_in(&Env::standard(), &current_dir()?, package))
}

pub fn remove(package: &str) -> Result<String, String> {
    timed(|| remove_in(&Env::standard(), &current_dir()?, package))
}

fn timed(run: impl FnOnce() -> Result<String, String>) -> Result<String, String> {
    let start = Instant::now();
    let message = run()?;
    Ok(format!("{} in {} ms", message, start.elapsed().as_millis()))
}

fn current_dir() -> Result<PathBuf, String> {
    std::env::current_dir().map_err(|e| format!("cannot read the current directory: {}", e))
}

fn split_range(spec: &str) -> (&str, Option<&str>) {
    match spec.rsplit_once('@') {
        Some((base, range)) if !range.contains('/') && !range.contains(':') && !base.is_empty() => (base, Some(range)),
        _ => (spec, None),
    }
}

fn add_in(env: &Env, root: &Path, spec: &str) -> Result<String, String> {
    let (base, range) = split_range(spec);
    let (package, url) = name::from_spec(base)?;
    if crate::module::BUILTIN_MODULES.contains(&package.as_str()) {
        return Err(format!("package name '{}' is a std module, so `use ... from \"{}\"` could never reach it", package, package));
    }
    let mut manifest = Manifest::load(root)?;
    if manifest.contains(&package) {
        return Err(format!("package '{}' is already a dependency (use `yidi update {}` to change its version)", package, package));
    }
    let mut resolver = env.resolver();
    let req = match range {
        Some(r) => version::parse_req(r).map(|_| r.to_string())?,
        None => latest_range(&mut resolver, &package, &url)?,
    };
    manifest.insert(&package, &Dep { url, req });
    let lock = apply(&mut resolver, &env.cache, root, &manifest, &previous(root)?)?;
    manifest.save(root)?;
    let added = lock.packages.iter().find(|p| p.name == package).map(describe).unwrap_or_default();
    Ok(format!("Added {} ({})", added, summary(&lock, resolver.downloaded)))
}

fn latest_range(resolver: &mut Resolver, package: &str, url: &Option<String>) -> Result<String, String> {
    let located = resolver.locate(package, &Dep { url: url.clone(), req: "*".into() })?;
    let remote = resolver.remote(&located)?;
    Ok(remote.tags.iter().map(|t| &t.version).max().map(version::caret).unwrap_or_else(|| "*".to_string()))
}

fn sync_in(env: &Env, root: &Path) -> Result<String, String> {
    let manifest = Manifest::load(root)?;
    let deps = manifest.dependencies()?;
    if let Some(lock) = Lock::load(root)?.filter(|l| l.inputs == lock::inputs_of(&deps)) {
        return match install::install(&env.cache, root, &lock)? {
            Outcome::UpToDate => Ok(format!("Already up to date ({} packages)", lock.packages.len())),
            Outcome::Installed { downloaded } => Ok(format!("Installed {}", summary(&lock, downloaded))),
        };
    }
    let mut resolver = env.resolver();
    let lock = apply(&mut resolver, &env.cache, root, &manifest, &previous(root)?)?;
    Ok(format!("Resolved and installed {}", summary(&lock, resolver.downloaded)))
}

fn update_in(env: &Env, root: &Path, package: Option<&str>) -> Result<String, String> {
    let manifest = Manifest::load(root)?;
    let before = previous(root)?;
    let kept: Vec<Locked> = match package {
        Some(p) => before.iter().filter(|l| l.name != name::validate(p).unwrap_or(p)).cloned().collect(),
        None => Vec::new(),
    };
    let mut resolver = env.resolver();
    let lock = apply(&mut resolver, &env.cache, root, &manifest, &kept)?;
    let changes = changes(&before, &lock.packages);
    let detail = if changes.is_empty() { "no version changed".to_string() } else { changes.join(", ") };
    Ok(format!("Updated: {} ({})", detail, summary(&lock, resolver.downloaded)))
}

fn remove_in(env: &Env, root: &Path, package: &str) -> Result<String, String> {
    name::validate(package)?;
    let mut manifest = Manifest::load(root)?;
    if !manifest.remove(package) {
        return Err(format!("package '{}' is not a dependency", package));
    }
    let mut resolver = env.resolver();
    let lock = apply(&mut resolver, &env.cache, root, &manifest, &previous(root)?)?;
    manifest.save(root)?;
    Ok(format!("Removed {} ({} left)", package, lock.packages.len()))
}

fn previous(root: &Path) -> Result<Vec<Locked>, String> {
    Ok(Lock::load(root)?.map(|l| l.packages).unwrap_or_default())
}

fn apply(resolver: &mut Resolver, cache: &Path, root: &Path, manifest: &Manifest, preferred: &[Locked]) -> Result<Lock, String> {
    let deps = manifest.dependencies()?;
    let packages = resolver.resolve(&deps, preferred)?;
    let lock = Lock { inputs: lock::inputs_of(&deps), packages };
    if let Outcome::Installed { downloaded } = install::install(cache, root, &lock)? {
        resolver.downloaded = resolver.downloaded.max(downloaded);
    }
    lock.save(root)?;
    Ok(lock)
}

fn describe(p: &Locked) -> String {
    match &p.version {
        Some(v) => format!("{} {}", p.name, v),
        None => format!("{} {}", p.name, &p.commit[..12]),
    }
}

fn summary(lock: &Lock, downloaded: usize) -> String {
    format!("{} packages, {} downloaded, {} from cache", lock.packages.len(), downloaded, lock.packages.len().saturating_sub(downloaded))
}

fn changes(before: &[Locked], after: &[Locked]) -> Vec<String> {
    after
        .iter()
        .filter_map(|p| match before.iter().find(|b| b.name == p.name) {
            Some(b) if b.commit == p.commit => None,
            Some(b) => Some(format!("{} -> {}", describe(b), describe(p).split_once(' ').map(|(_, v)| v.to_string()).unwrap_or_default())),
            None => Some(format!("+{}", describe(p))),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_file, TempDir};
    use std::fs;
    use std::process::Command;

    struct Repo {
        _dir: TempDir,
        path: PathBuf,
    }

    impl Repo {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let dir = TempDir::new();
            let path = dir.path().join(name);
            fs::create_dir_all(&path).unwrap();
            let repo = Self { _dir: dir, path };
            repo.git(&["init", "-q"]);
            repo.commit(files);
            repo
        }

        fn git(&self, args: &[&str]) {
            let ok = Command::new("git").arg("-C").arg(&self.path).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args).output().unwrap().status.success();
            assert!(ok, "git {:?}", args);
        }

        fn commit(&self, files: &[(&str, &str)]) {
            for (f, c) in files {
                write_file(&self.path, f, c);
            }
            self.git(&["add", "."]);
            self.git(&["commit", "-q", "--allow-empty", "-m", "change"]);
        }

        fn release(&self, tag: &str, files: &[(&str, &str)]) {
            self.commit(files);
            self.git(&["tag", tag]);
        }

        fn rev(&self, refname: &str) -> String {
            let out = Command::new("git").arg("-C").arg(&self.path).args(["rev-parse", &format!("{}^{{commit}}", refname)]).output().unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        }

        fn url(&self) -> String {
            let p = self.path.display().to_string().replace('\\', "/");
            format!("file://{}{}", if p.starts_with('/') { "" } else { "/" }, p)
        }
    }

    fn read(dir: &Path, file: &str) -> String {
        fs::read_to_string(dir.join(file)).unwrap()
    }

    fn greet(v: &str) -> (&'static str, String) {
        ("greet.yk", format!("export fn hello() -> str {{ return \"{}\"; }}\n", v))
    }

    struct TestEnv {
        _cache: TempDir,
        index: TempDir,
        env: Env,
    }

    impl TestEnv {
        fn new() -> Self {
            let (cache, index) = (TempDir::new(), TempDir::new());
            let env = Env { cache: cache.path().to_path_buf(), index: Index::Dir(index.path().to_path_buf()) };
            Self { _cache: cache, index, env }
        }

        fn publish(&self, name: &str, repo: &Repo, versions: &[&str]) {
            let releases = versions.iter().map(|v| {
                let commit = repo.rev(&format!("v{}", v));
                let dir = TempDir::new();
                fetch::fetch(&repo.url(), &commit, &dir.path().join("t")).unwrap();
                index::Release { version: v.to_string(), commit, hash: tree::hash(&dir.path().join("t")).unwrap(), yanked: false }
            }).collect();
            let entry = index::Entry { source: repo.url(), releases };
            write_file(self.index.path(), &index::entry_path(name), &index::render(&entry));
        }
    }

    fn project() -> TempDir {
        let app = TempDir::new();
        write_file(app.path(), "project.toml", "[package]\nname = \"app\"\n");
        app
    }

    #[test]
    fn ranges_split_from_specs() {
        assert_eq!(split_range("github:u/greet@^1.2"), ("github:u/greet", Some("^1.2")));
        assert_eq!(split_range("u/greet"), ("u/greet", None));
        assert_eq!(split_range("ssh://git@host/u/greet"), ("ssh://git@host/u/greet", None));
        assert_eq!(split_range("ssh://git@host/u/greet@1"), ("ssh://git@host/u/greet", Some("1")));
    }

    #[test]
    fn add_picks_the_latest_release_and_records_a_caret_range() {
        let repo = Repo::new("greet", &[]);
        let (f, c) = greet("v1.0");
        repo.release("v1.0.0", &[(f, &c)]);
        let (f, c) = greet("v1.4");
        repo.release("v1.4.0", &[(f, &c)]);
        let (cache, app) = (TestEnv::new(), project());
        let msg = add_in(&cache.env, app.path(), &repo.url()).unwrap();
        assert!(msg.contains("greet 1.4.0"), "{}", msg);
        assert!(read(app.path(), "@yidi/greet/greet.yk").contains("v1.4"));
        assert!(read(app.path(), "project.toml").contains("version = \"^1.4.0\""));
        assert!(read(app.path(), "yidi.lock").contains("version = \"1.4.0\""));
    }

    #[test]
    fn sync_is_a_no_op_then_works_offline_from_the_cache() {
        let repo = Repo::new("greet", &[]);
        let (f, c) = greet("v1");
        repo.release("v1.0.0", &[(f, &c)]);
        let (cache, app) = (TestEnv::new(), project());
        add_in(&cache.env, app.path(), &repo.url()).unwrap();
        assert!(sync_in(&cache.env, app.path()).unwrap().starts_with("Already up to date"));
        let url = repo.url();
        drop(repo);
        fs::remove_dir_all(app.path().join("@yidi")).unwrap();
        let msg = sync_in(&cache.env, app.path()).unwrap();
        assert!(msg.contains("0 downloaded"), "{}", msg);
        assert!(read(app.path(), "@yidi/greet/greet.yk").contains("v1"));
        assert!(read(app.path(), "yidi.lock").contains(&url));
    }

    #[test]
    fn the_lock_holds_versions_until_update() {
        let repo = Repo::new("greet", &[]);
        let (f, c) = greet("v1.0");
        repo.release("v1.0.0", &[(f, &c)]);
        let (cache, app) = (TestEnv::new(), project());
        add_in(&cache.env, app.path(), &repo.url()).unwrap();
        let (f, c) = greet("v1.1");
        repo.release("v1.1.0", &[(f, &c)]);
        sync_in(&cache.env, app.path()).unwrap();
        assert!(read(app.path(), "@yidi/greet/greet.yk").contains("v1.0"));
        let msg = update_in(&cache.env, app.path(), Some("greet")).unwrap();
        assert!(msg.contains("greet 1.0.0 -> 1.1.0"), "{}", msg);
        assert!(read(app.path(), "@yidi/greet/greet.yk").contains("v1.1"));
    }

    #[test]
    fn transitive_dependencies_are_installed_and_pruned() {
        let text = Repo::new("text", &[]);
        text.release("v1.0.0", &[("text.yk", "export fn upper(s: str) -> str { return s; }\n")]);
        let greet = Repo::new("greet", &[]);
        let manifest = format!("[dependencies]\ntext = {{ git = \"{}\", version = \"^1\" }}\n", text.url());
        greet.release("v2.0.0", &[("greet.yk", "use {upper} from \"text\";\n"), ("project.toml", &manifest)]);
        let (cache, app) = (TestEnv::new(), project());
        add_in(&cache.env, app.path(), &greet.url()).unwrap();
        assert!(app.path().join("@yidi/text/text.yk").exists());
        assert!(read(app.path(), "yidi.lock").contains("dependencies = [\"text\"]"));
        remove_in(&cache.env, app.path(), "greet").unwrap();
        assert!(!app.path().join("@yidi/greet").exists() && !app.path().join("@yidi/text").exists());
        assert!(remove_in(&cache.env, app.path(), "greet").is_err());
    }

    #[test]
    fn conflicting_ranges_are_reported_with_who_asked() {
        let text = Repo::new("text", &[]);
        text.release("v1.0.0", &[("text.yk", "x")]);
        text.release("v2.0.0", &[("text.yk", "y")]);
        let greet = Repo::new("greet", &[]);
        greet.release("v1.0.0", &[("greet.yk", "x"), ("project.toml", &format!("[dependencies]\ntext = {{ git = \"{}\", version = \"^2\" }}\n", text.url()))]);
        let (cache, app) = (TestEnv::new(), project());
        add_in(&cache.env, app.path(), &format!("{}@^1", text.url())).unwrap();
        let err = add_in(&cache.env, app.path(), &greet.url()).unwrap_err();
        assert!(err.contains("no version of 'text'") && err.contains("by greet") && err.contains("by project.toml"), "{}", err);
        assert!(!read(app.path(), "project.toml").contains("greet"));
    }

    #[test]
    fn a_tampered_package_is_fetched_again() {
        let repo = Repo::new("greet", &[]);
        let (f, c) = greet("v1");
        repo.release("v1.0.0", &[(f, &c)]);
        let (cache, app) = (TestEnv::new(), project());
        add_in(&cache.env, app.path(), &repo.url()).unwrap();
        let entry = fs::read_dir(&cache.env.cache).unwrap().flatten().find(|e| !e.file_name().to_string_lossy().starts_with('.')).unwrap().path();
        fs::write(entry.join("greet.yk"), "tampered").unwrap();
        fs::remove_dir_all(app.path().join("@yidi")).unwrap();
        sync_in(&cache.env, app.path()).unwrap();
        assert!(read(app.path(), "@yidi/greet/greet.yk").contains("v1"));
    }

    #[test]
    fn a_registry_name_resolves_through_the_index_without_ls_remote() {
        let repo = Repo::new("greet", &[]);
        let (f, c) = greet("v1.0");
        repo.release("v1.0.0", &[(f, &c)]);
        let (f, c) = greet("v1.2");
        repo.release("v1.2.0", &[(f, &c)]);
        let (cache, app) = (TestEnv::new(), project());
        cache.publish("greet", &repo, &["1.0.0"]);
        let msg = add_in(&cache.env, app.path(), "greet").unwrap();
        assert!(msg.contains("greet 1.0.0"), "the index only lists 1.0.0: {}", msg);
        assert!(read(app.path(), "project.toml").contains("greet = \"^1.0.0\""));
        assert!(read(app.path(), "@yidi/greet/greet.yk").contains("v1.0"));
    }

    #[test]
    fn a_registry_hash_that_does_not_match_is_refused() {
        let repo = Repo::new("greet", &[]);
        let (f, c) = greet("v1");
        repo.release("v1.0.0", &[(f, &c)]);
        let (cache, app) = (TestEnv::new(), project());
        let entry = index::Entry { source: repo.url(), releases: vec![index::Release { version: "1.0.0".into(), commit: repo.rev("v1.0.0"), hash: "sha256-forged".into(), yanked: false }] };
        write_file(cache.index.path(), &index::entry_path("greet"), &index::render(&entry));
        assert!(add_in(&cache.env, app.path(), "greet").unwrap_err().contains("does not match the registry"));
        assert!(!app.path().join("@yidi/greet").exists());
    }

    #[test]
    fn an_unknown_registry_name_says_how_to_add_it() {
        let (cache, app) = (TestEnv::new(), project());
        assert!(add_in(&cache.env, app.path(), "nothing-here").unwrap_err().contains("not in the registry"));
    }

    #[test]
    fn a_repository_without_tags_follows_its_head() {
        let repo = Repo::new("greet", &[("greet.yk", "head")]);
        let (cache, app) = (TestEnv::new(), project());
        add_in(&cache.env, app.path(), &repo.url()).unwrap();
        assert!(read(app.path(), "@yidi/greet/greet.yk").contains("head"));
        assert!(!read(app.path(), "project.toml").contains("version"));
    }

    #[test]
    fn a_failed_add_leaves_the_project_untouched() {
        let (cache, app) = (TestEnv::new(), project());
        let before = read(app.path(), "project.toml");
        assert!(add_in(&cache.env, app.path(), &format!("file://{}/missing", app.path().display().to_string().replace('\\', "/"))).is_err());
        assert_eq!(read(app.path(), "project.toml"), before);
        assert!(!app.path().join("yidi.lock").exists());
    }

    #[test]
    fn std_names_and_traversal_specs_are_refused() {
        let (cache, app) = (TestEnv::new(), project());
        write_file(app.path(), "main.yk", "fn main() {}");
        assert!(add_in(&cache.env, app.path(), "github:someone/math").unwrap_err().contains("std module"));
        assert!(add_in(&cache.env, app.path(), "file:///tmp/../..").is_err());
        write_file(app.path(), "project.toml", "[dependencies]\n\"..\" = \"file:///nowhere\"\n");
        assert!(sync_in(&cache.env, app.path()).is_err());
        assert!(app.path().join("main.yk").exists());
    }
}
