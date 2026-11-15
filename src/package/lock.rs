use super::manifest::Dep;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "yidi.lock";
const FORMAT: i64 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct Locked {
    pub name: String,
    pub source: String,
    pub version: Option<String>,
    pub commit: String,
    pub hash: String,
    pub dependencies: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lock {
    pub inputs: String,
    pub packages: Vec<Locked>,
}

pub fn inputs_of(deps: &[(String, Dep)]) -> String {
    let mut hasher = Sha256::new();
    for (name, dep) in deps {
        hasher.update(format!("{}\0{}\0{}\n", name, dep.url.as_deref().unwrap_or("registry"), dep.req));
    }
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

impl Lock {
    pub fn path(root: &Path) -> PathBuf {
        root.join(FILE)
    }

    pub fn load(root: &Path) -> Result<Option<Self>, String> {
        let path = Self::path(root);
        let Ok(text) = fs::read_to_string(&path) else { return Ok(None) };
        let table: toml::Table = text.parse().map_err(|e| format!("cannot parse {}: {}", path.display(), e))?;
        if table.get("version").and_then(|v| v.as_integer()) != Some(FORMAT) {
            return Ok(None);
        }
        let inputs = table.get("inputs").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let packages = table.get("package").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(locked_of).collect()).unwrap_or_default();
        Ok(Some(Self { inputs, packages }))
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        let path = Self::path(root);
        fs::write(&path, self.render()).map_err(|e| format!("cannot write {}: {}", path.display(), e))
    }

    pub fn digest(&self) -> String {
        hex(&Sha256::digest(self.render().as_bytes()))
    }

    fn render(&self) -> String {
        let mut table = toml::Table::new();
        table.insert("version".into(), FORMAT.into());
        table.insert("inputs".into(), self.inputs.clone().into());
        let packages: Vec<toml::Value> = self.packages.iter().map(|p| table_of(p).into()).collect();
        table.insert("package".into(), packages.into());
        format!("# Written by yidi. Do not edit by hand.\n{}", toml::to_string(&table).unwrap_or_default())
    }
}

fn table_of(p: &Locked) -> toml::Table {
    let mut t = toml::Table::new();
    t.insert("name".into(), p.name.clone().into());
    t.insert("source".into(), p.source.clone().into());
    if let Some(v) = &p.version {
        t.insert("version".into(), v.clone().into());
    }
    t.insert("commit".into(), p.commit.clone().into());
    t.insert("hash".into(), p.hash.clone().into());
    t.insert("dependencies".into(), p.dependencies.clone().into());
    t
}

fn locked_of(value: &toml::Value) -> Option<Locked> {
    let t = value.as_table()?;
    let text = |k: &str| t.get(k).and_then(|v| v.as_str()).map(str::to_string);
    Some(Locked {
        name: text("name")?,
        source: text("source")?,
        version: text("version"),
        commit: text("commit")?,
        hash: text("hash")?,
        dependencies: t.get("dependencies").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|d| d.as_str().map(str::to_string)).collect()).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn a_lock_survives_a_round_trip() {
        let dir = TempDir::new();
        let lock = Lock {
            inputs: inputs_of(&[("greet".into(), Dep::any("u"))]),
            packages: vec![Locked { name: "greet".into(), source: "u".into(), version: Some("1.2.0".into()), commit: "a".repeat(40), hash: "sha256-x".into(), dependencies: vec!["text".into()] }],
        };
        lock.save(dir.path()).unwrap();
        assert_eq!(Lock::load(dir.path()).unwrap(), Some(lock));
    }

    #[test]
    fn inputs_change_with_any_range() {
        let a = inputs_of(&[("greet".into(), Dep { url: Some("u".into()), req: "^1".into() })]);
        let b = inputs_of(&[("greet".into(), Dep { url: Some("u".into()), req: "^2".into() })]);
        assert_ne!(a, b);
    }
}
