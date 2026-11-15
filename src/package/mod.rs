mod git;
mod manifest;
mod name;
mod store;

use manifest::Manifest;
use std::fs;
use std::path::Path;

const IGNORED: &str = "lib/";

pub fn init(dir_name: Option<&str>) -> Result<String, String> {
    let cwd = current_dir()?;
    match dir_name {
        Some(n) => init_in(&cwd.join(name::validate(n)?), n),
        None => {
            let n = cwd.file_name().and_then(|f| f.to_str()).unwrap_or("").to_string();
            name::validate(&n).map_err(|e| format!("{}; run `yidi init <name>` to create a project in a new directory", e))?;
            init_in(&cwd, &n)
        }
    }
}

pub fn add(spec: &str) -> Result<String, String> {
    add_in(&current_dir()?, spec)
}

pub fn sync() -> Result<String, String> {
    sync_in(&current_dir()?)
}

pub fn remove(package: &str) -> Result<String, String> {
    remove_in(&current_dir()?, package)
}

fn current_dir() -> Result<std::path::PathBuf, String> {
    std::env::current_dir().map_err(|e| format!("cannot read the current directory: {}", e))
}

fn init_in(dir: &Path, package: &str) -> Result<String, String> {
    if Manifest::path(dir).exists() {
        return Err(format!("{} already exists in {}", manifest::FILE, dir.display()));
    }
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
    Manifest::for_new_project(package).save(dir)?;
    write_if_missing(&dir.join("main.yk"), &format!("fn main() {{\n    print(\"Hello from {}\");\n}}\n", package))?;
    ignore_lib(dir)?;
    Ok(format!("Created project '{}' in {}", package, dir.display()))
}

fn write_if_missing(path: &Path, content: &str) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    fs::write(path, content).map_err(|e| format!("cannot write {}: {}", path.display(), e))
}

fn ignore_lib(dir: &Path) -> Result<(), String> {
    let path = dir.join(".gitignore");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == IGNORED) {
        return Ok(());
    }
    let separator = if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" };
    fs::write(&path, format!("{}{}{}\n", existing, separator, IGNORED)).map_err(|e| format!("cannot write {}: {}", path.display(), e))
}

fn add_in(root: &Path, spec: &str) -> Result<String, String> {
    let (package, url) = name::from_spec(spec)?;
    let mut manifest = Manifest::load(root)?;
    if manifest.contains(&package) {
        return Err(format!("package '{}' is already a dependency", package));
    }
    install(root, &package, &url)?;
    manifest.insert(&package, &url);
    manifest.save(root)?;
    Ok(format!("Added package '{}' from {}", package, url))
}

fn install(root: &Path, package: &str, url: &str) -> Result<(), String> {
    store::remove_package_dir(root, package)?;
    let lib = store::lib_dir(root);
    fs::create_dir_all(&lib).map_err(|e| format!("cannot create {}: {}", lib.display(), e))?;
    git::clone(url, &store::package_dir(root, package)?)
}

fn sync_in(root: &Path) -> Result<String, String> {
    let deps = Manifest::load(root)?.dependencies()?;
    if deps.is_empty() {
        return Ok("No dependencies to sync.".into());
    }
    for (package, url) in &deps {
        let dir = store::package_dir(root, package)?;
        if dir.is_dir() {
            git::update(&dir)?;
        } else {
            install(root, package, url)?;
        }
    }
    Ok(format!("Synced {} package(s)", deps.len()))
}

fn remove_in(root: &Path, package: &str) -> Result<String, String> {
    name::validate(package)?;
    let mut manifest = Manifest::load(root)?;
    let listed = manifest.remove(package);
    let deleted = store::remove_package_dir(root, package)?;
    if !listed && !deleted {
        return Err(format!("package '{}' is not a dependency", package));
    }
    if listed {
        manifest.save(root)?;
    }
    Ok(format!("Removed package '{}'", package))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_file, TempDir};
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args).output().unwrap().status.success();
        assert!(ok, "git {:?}", args);
    }

    fn file_url(path: &Path) -> String {
        let p = path.display().to_string().replace('\\', "/");
        format!("file://{}{}", if p.starts_with('/') { "" } else { "/" }, p)
    }

    fn upstream(content: &str) -> (TempDir, String) {
        let repo = TempDir::new();
        let dir = repo.path().join("greet");
        write_file(&dir, "main.yk", content);
        git(&dir, &["init", "-q"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "v1"]);
        let url = file_url(&dir);
        (repo, url)
    }

    fn read(dir: &Path, file: &str) -> String {
        fs::read_to_string(dir.join(file)).unwrap()
    }

    #[test]
    fn init_creates_a_runnable_project() {
        let dir = TempDir::new();
        init_in(dir.path(), "app").unwrap();
        assert!(read(dir.path(), "project.toml").contains("name = \"app\""));
        assert!(read(dir.path(), "main.yk").contains("fn main()"));
        assert_eq!(read(dir.path(), ".gitignore"), "lib/\n");
    }

    #[test]
    fn init_keeps_existing_files_and_refuses_a_second_run() {
        let dir = TempDir::new();
        write_file(dir.path(), "main.yk", "mine");
        write_file(dir.path(), ".gitignore", "target");
        init_in(dir.path(), "app").unwrap();
        assert_eq!(read(dir.path(), "main.yk"), "mine");
        assert_eq!(read(dir.path(), ".gitignore"), "target\nlib/\n");
        assert!(init_in(dir.path(), "app").is_err());
    }

    #[test]
    fn add_sync_and_remove_round_trip() {
        let (repo, url) = upstream("export fn hello() -> str { return \"v1\"; }\n");
        let app = TempDir::new();
        init_in(app.path(), "app").unwrap();
        add_in(app.path(), &url).unwrap();
        assert!(read(app.path(), "lib/greet/main.yk").contains("v1"));
        assert!(Manifest::load(app.path()).unwrap().contains("greet"));
        assert!(add_in(app.path(), &url).is_err());

        let upstream_dir = repo.path().join("greet");
        write_file(&upstream_dir, "main.yk", "export fn hello() -> str { return \"v2\"; }\n");
        git(&upstream_dir, &["commit", "-qam", "v2"]);
        sync_in(app.path()).unwrap();
        assert!(read(app.path(), "lib/greet/main.yk").contains("v2"));

        fs::remove_dir_all(app.path().join("lib")).unwrap();
        sync_in(app.path()).unwrap();
        assert!(app.path().join("lib/greet/main.yk").exists());

        remove_in(app.path(), "greet").unwrap();
        assert!(!app.path().join("lib/greet").exists());
        assert!(!Manifest::load(app.path()).unwrap().contains("greet"));
        assert!(read(app.path(), "project.toml").contains("name = \"app\""));
        assert!(remove_in(app.path(), "greet").is_err());
    }

    #[test]
    fn a_failed_clone_leaves_the_manifest_untouched() {
        let app = TempDir::new();
        init_in(app.path(), "app").unwrap();
        let before = read(app.path(), "project.toml");
        assert!(add_in(app.path(), &file_url(&app.path().join("missing"))).is_err());
        assert_eq!(read(app.path(), "project.toml"), before);
    }

    #[test]
    fn the_reported_traversal_spec_deletes_nothing() {
        let app = TempDir::new();
        write_file(app.path(), "main.yk", "fn main() {}");
        assert!(add_in(app.path(), "file:///tmp/../..").is_err());
        assert!(app.path().join("main.yk").exists());
    }

    #[test]
    fn sync_refuses_a_hand_edited_traversal_name() {
        let app = TempDir::new();
        write_file(app.path(), "main.yk", "fn main() {}");
        write_file(app.path(), "project.toml", "[dependencies]\n\"..\" = \"file:///nowhere\"\n");
        assert!(sync_in(app.path()).is_err());
        assert!(app.path().join("main.yk").exists());
    }
}
