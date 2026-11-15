use super::manifest::{self, Manifest};
use super::{name, store};
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

pub fn init(dir_name: Option<&str>) -> Result<String, String> {
    let answer = match dir_name {
        Some(n) => n.to_string(),
        None => ask_project_name(&mut std::io::stdin().lock(), &mut std::io::stderr())?,
    };
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read the current directory: {}", e))?;
    let (dir, package) = init_target(&cwd, &answer)?;
    init_in(&dir, &package)
}

fn ask_project_name(input: &mut dyn BufRead, output: &mut dyn Write) -> Result<String, String> {
    let _ = write!(output, "Project name (. for the current folder) [.]: ");
    let _ = output.flush();
    let mut line = String::new();
    let read = input.read_line(&mut line).map_err(|e| format!("cannot read the project name: {}", e))?;
    if read == 0 {
        return Err("no project name given: run `yidi init <name>`, or `yidi init .` for the current folder".into());
    }
    Ok(line.trim().to_string())
}

fn init_target(cwd: &Path, answer: &str) -> Result<(PathBuf, String), String> {
    if answer.is_empty() || answer == "." {
        let folder = cwd.file_name().and_then(|f| f.to_str()).unwrap_or("").to_string();
        name::validate(&folder).map_err(|e| format!("{}; give a name to create the project in a new folder", e))?;
        return Ok((cwd.to_path_buf(), folder));
    }
    Ok((cwd.join(name::validate(answer)?), answer.to_string()))
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
    let ignored = format!("{}/", store::PACKAGES_DIR);
    if existing.lines().any(|l| l.trim() == ignored) {
        return Ok(());
    }
    let separator = if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" };
    fs::write(&path, format!("{}{}{}\n", existing, separator, ignored)).map_err(|e| format!("cannot write {}: {}", path.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_file, TempDir};

    fn read(dir: &Path, file: &str) -> String {
        fs::read_to_string(dir.join(file)).unwrap()
    }

    #[test]
    fn init_asks_for_the_name_and_dot_means_here() {
        let mut out = Vec::new();
        assert_eq!(ask_project_name(&mut "demo\n".as_bytes(), &mut out).unwrap(), "demo");
        assert!(String::from_utf8(out).unwrap().contains("Project name"));
        assert!(ask_project_name(&mut "".as_bytes(), &mut Vec::new()).is_err());
        let cwd = Path::new("/work/my-app");
        assert_eq!(init_target(cwd, ".").unwrap(), (cwd.to_path_buf(), "my-app".to_string()));
        assert_eq!(init_target(cwd, "").unwrap(), (cwd.to_path_buf(), "my-app".to_string()));
        assert_eq!(init_target(cwd, "demo").unwrap(), (cwd.join("demo"), "demo".to_string()));
        assert!(init_target(cwd, "../up").is_err());
        assert!(init_target(Path::new("/work/My App"), ".").is_err());
    }

    #[test]
    fn init_creates_a_runnable_project() {
        let dir = TempDir::new();
        init_in(dir.path(), "app").unwrap();
        assert!(read(dir.path(), "project.toml").contains("name = \"app\""));
        assert!(read(dir.path(), "main.yk").contains("fn main()"));
        assert_eq!(read(dir.path(), ".gitignore"), "@yidi/\n");
    }

    #[test]
    fn init_keeps_existing_files_and_refuses_a_second_run() {
        let dir = TempDir::new();
        write_file(dir.path(), "main.yk", "mine");
        write_file(dir.path(), ".gitignore", "target");
        init_in(dir.path(), "app").unwrap();
        assert_eq!(read(dir.path(), "main.yk"), "mine");
        assert_eq!(read(dir.path(), ".gitignore"), "target\n@yidi/\n");
        assert!(init_in(dir.path(), "app").is_err());
    }
}
