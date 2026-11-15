use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "project.toml";
const DEPENDENCIES: &str = "dependencies";

pub struct Manifest {
    table: toml::Table,
}

impl Manifest {
    pub fn path(root: &Path) -> PathBuf {
        root.join(FILE)
    }

    pub fn load(root: &Path) -> Result<Self, String> {
        let path = Self::path(root);
        if !path.exists() {
            return Ok(Self { table: toml::Table::new() });
        }
        let content = fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        let table = content.parse().map_err(|e| format!("cannot parse {}: {}", path.display(), e))?;
        Ok(Self { table })
    }

    pub fn for_new_project(name: &str) -> Self {
        let mut package = toml::Table::new();
        package.insert("name".into(), name.into());
        package.insert("version".into(), "0.1.0".into());
        let mut table = toml::Table::new();
        table.insert("package".into(), package.into());
        table.insert(DEPENDENCIES.into(), toml::Table::new().into());
        Self { table }
    }

    pub fn dependencies(&self) -> Result<Vec<(String, String)>, String> {
        let Some(deps) = self.table.get(DEPENDENCIES) else { return Ok(Vec::new()) };
        let deps = deps.as_table().ok_or_else(|| format!("[{}] in {} must be a table", DEPENDENCIES, FILE))?;
        deps.iter()
            .map(|(name, url)| match url.as_str() {
                Some(url) => Ok((name.clone(), url.to_string())),
                None => Err(format!("dependency '{}' in {} must be a git URL string", name, FILE)),
            })
            .collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.deps().is_some_and(|d| d.contains_key(name))
    }

    pub fn insert(&mut self, name: &str, url: &str) {
        let deps = self.table.entry(DEPENDENCIES).or_insert_with(|| toml::Table::new().into());
        if let Some(deps) = deps.as_table_mut() {
            deps.insert(name.into(), url.into());
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.table.get_mut(DEPENDENCIES).and_then(|d| d.as_table_mut()).is_some_and(|d| d.remove(name).is_some())
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        let path = Self::path(root);
        let content = toml::to_string(&self.table).map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
        fs::write(&path, content).map_err(|e| format!("cannot write {}: {}", path.display(), e))
    }

    fn deps(&self) -> Option<&toml::Table> {
        self.table.get(DEPENDENCIES).and_then(|d| d.as_table())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_file, TempDir};

    #[test]
    fn a_missing_manifest_has_no_dependencies() {
        let dir = TempDir::new();
        assert!(Manifest::load(dir.path()).unwrap().dependencies().unwrap().is_empty());
    }

    #[test]
    fn saving_keeps_the_other_sections() {
        let dir = TempDir::new();
        write_file(dir.path(), FILE, "[package]\nname = \"app\"\n\n[tools]\nfmt = true\n");
        let mut m = Manifest::load(dir.path()).unwrap();
        m.insert("greet", "https://x/greet.git");
        m.save(dir.path()).unwrap();
        let saved = fs::read_to_string(Manifest::path(dir.path())).unwrap();
        assert!(saved.contains("name = \"app\"") && saved.contains("fmt = true") && saved.contains("greet = \"https://x/greet.git\""), "{}", saved);
        assert!(saved.find("[package]").unwrap() < saved.find("[dependencies]").unwrap(), "{}", saved);
    }

    #[test]
    fn removing_reports_whether_the_dependency_existed() {
        let mut m = Manifest::for_new_project("app");
        m.insert("greet", "u");
        assert!(m.contains("greet"));
        assert!(m.remove("greet"));
        assert!(!m.remove("greet"));
        assert!(!m.contains("greet"));
    }

    #[test]
    fn a_dependency_that_is_not_a_string_is_an_error() {
        let dir = TempDir::new();
        write_file(dir.path(), FILE, "[dependencies]\ngreet = 3\n");
        assert!(Manifest::load(dir.path()).unwrap().dependencies().is_err());
    }
}
