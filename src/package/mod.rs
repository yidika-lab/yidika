use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

const MANIFEST: &str = "project.toml";

type Deps = BTreeMap<String, String>;

fn load_manifest() -> Result<Deps, String> {
    if !Path::new(MANIFEST).exists() {
        return Ok(BTreeMap::new());
    }
    let content = fs::read_to_string(MANIFEST)
        .map_err(|e| format!("Failed to read {}: {}", MANIFEST, e))?;
    let table: toml::Value = content.parse()
        .map_err(|e| format!("Failed to parse {}: {}", MANIFEST, e))?;
    let deps = table.get("dependencies")
        .and_then(|v| v.as_table())
        .map(|t| {
            t.iter().map(|(k, v)| {
                (k.clone(), v.as_str().unwrap_or("").to_string())
            }).collect::<Deps>()
        })
        .unwrap_or_default();
    Ok(deps)
}

fn save_manifest(deps: &Deps) -> Result<(), String> {
    let mut dep_table = toml::value::Table::new();
    for (k, v) in deps {
        dep_table.insert(k.clone(), toml::Value::String(v.clone()));
    }
    let mut root = toml::value::Table::new();
    root.insert("dependencies".into(), toml::Value::Table(dep_table));
    let content = toml::to_string(&toml::Value::Table(root))
        .map_err(|e| format!("Failed to serialize {}: {}", MANIFEST, e))?;
    fs::write(MANIFEST, content)
        .map_err(|e| format!("Failed to write {}: {}", MANIFEST, e))
}

fn parse_spec(spec: &str) -> Result<(String, String), String> {
    if spec.contains("://") {
        let name = spec.trim_end_matches(".git")
            .rsplit('/')
            .next()
            .unwrap_or(spec)
            .to_string();
        Ok((name, spec.to_string()))
    } else if let Some(shorthand) = spec.strip_prefix("github:") {
        let name = shorthand.rsplit('/').next().unwrap_or(shorthand);
        let url = format!("https://github.com/{}.git", shorthand);
        Ok((name.to_string(), url))
    } else if spec.contains('/') && !spec.contains('\\') {
        let name = spec.rsplit('/').next().unwrap_or(spec);
        let url = format!("https://github.com/{}.git", spec);
        Ok((name.to_string(), url))
    } else {
        Err(format!(
            "Invalid package spec '{}'. Use 'github:user/repo' or a full git URL.",
            spec
        ))
    }
}

fn lib_dir() -> &'static Path {
    Path::new("lib")
}

fn clone_package(url: &str, target: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .args(["clone", "--depth", "1", url])
        .arg(target)
        .output()
        .map_err(|e| format!("Failed to run git: {}. Is git installed?", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git clone failed: {}", stderr.trim()));
    }
    Ok(())
}

fn update_package(target: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .args(["-C", &target.to_string_lossy(), "pull", "--ff-only"])
        .output()
        .map_err(|e| format!("Failed to run git pull: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git pull failed: {}", stderr.trim()));
    }
    Ok(())
}

pub fn add(spec: &str) -> Result<String, String> {
    let (name, url) = parse_spec(spec)?;
    let mut deps = load_manifest()?;

    if deps.contains_key(&name) {
        return Err(format!("Package '{}' is already a dependency", name));
    }

    deps.insert(name.clone(), url.clone());
    save_manifest(&deps)?;

    let lib = lib_dir();
    fs::create_dir_all(lib)
        .map_err(|e| format!("Failed to create lib/ directory: {}", e))?;

    let target = lib.join(&name);
    if target.exists() {
        fs::remove_dir_all(&target)
            .map_err(|e| format!("Failed to remove existing '{}': {}", target.display(), e))?;
    }

    clone_package(&url, &target)?;

    Ok(format!("Added package '{}' from {}", name, url))
}

pub fn sync() -> Result<String, String> {
    let deps = load_manifest()?;
    if deps.is_empty() {
        return Ok("No dependencies to sync.".into());
    }

    let lib = lib_dir();
    fs::create_dir_all(lib)
        .map_err(|e| format!("Failed to create lib/ directory: {}", e))?;

    let mut count = 0usize;
    for (name, url) in &deps {
        let target = lib.join(name);
        if target.exists() {
            update_package(&target)?;
        } else {
            clone_package(url, &target)?;
        }
        count += 1;
    }

    Ok(format!("Synced {} package(s)", count))
}
