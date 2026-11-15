use super::remote::{self, Remote};
use super::version::{self, Tag};
use std::path::PathBuf;

pub const REGISTRY_REPO: &str = "yidika-lab/registry";
const DEFAULT_BASE: &str = "https://raw.githubusercontent.com/yidika-lab/registry/main";
const MAX_ENTRY_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub commit: String,
    pub hash: String,
    pub yanked: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub source: String,
    pub releases: Vec<Release>,
}

#[derive(Clone, Debug)]
pub enum Index {
    Http(String),
    Dir(PathBuf),
}

impl Index {
    pub fn default_index() -> Self {
        Self::from_setting(std::env::var("YIDI_REGISTRY").ok().as_deref())
    }

    fn from_setting(setting: Option<&str>) -> Self {
        match setting.map(str::trim).filter(|s| !s.is_empty()) {
            None => Index::Http(DEFAULT_BASE.to_string()),
            Some(url) if url.starts_with("https://") || url.starts_with("http://") => Index::Http(url.trim_end_matches('/').to_string()),
            Some(dir) => Index::Dir(PathBuf::from(dir)),
        }
    }

    pub fn entry(&self, name: &str) -> Result<Option<Entry>, String> {
        let text = match self {
            Index::Dir(dir) => match std::fs::read_to_string(dir.join(entry_path(name))) {
                Ok(t) => t,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(format!("cannot read the registry entry of '{}': {}", name, e)),
            },
            Index::Http(base) => match http_get(&format!("{}/{}", base, entry_path(name)))? {
                Some(t) => t,
                None => return Ok(None),
            },
        };
        parse(&text).map(Some).map_err(|e| format!("registry entry of '{}': {}", name, e))
    }
}

pub fn entry_path(name: &str) -> String {
    format!("packages/{}.toml", name)
}

fn http_get(url: &str) -> Result<Option<String>, String> {
    match ureq::get(url).header("User-Agent", "yidi").call() {
        Ok(mut response) => response.body_mut().with_config().limit(MAX_ENTRY_BYTES).read_to_string().map(Some).map_err(|e| format!("cannot read {}: {}", url, e)),
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(e) => Err(format!("cannot reach the registry ({}): {}", url, e)),
    }
}

pub fn parse(text: &str) -> Result<Entry, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("{}", e))?;
    let source = table.get("source").and_then(|v| v.as_str()).ok_or("missing source")?.to_string();
    let mut releases = Vec::new();
    for item in table.get("release").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        let t = item.as_table().ok_or("release must be a table")?;
        let text = |k: &str| t.get(k).and_then(|v| v.as_str()).map(str::to_string).ok_or(format!("release without {}", k));
        let release = Release { version: text("version")?, commit: text("commit")?, hash: text("hash")?, yanked: t.get("yanked").and_then(|v| v.as_bool()).unwrap_or(false) };
        if version::parse_tag(&release.version).is_none() || !remote::is_commit(&release.commit) || !release.hash.starts_with("sha256-") {
            return Err(format!("invalid release {}", release.version));
        }
        releases.push(release);
    }
    Ok(Entry { source, releases })
}

pub fn render(entry: &Entry) -> String {
    let mut table = toml::Table::new();
    table.insert("source".into(), entry.source.clone().into());
    let releases: Vec<toml::Value> = entry
        .releases
        .iter()
        .map(|r| {
            let mut t = toml::Table::new();
            t.insert("version".into(), r.version.clone().into());
            t.insert("commit".into(), r.commit.clone().into());
            t.insert("hash".into(), r.hash.clone().into());
            if r.yanked {
                t.insert("yanked".into(), true.into());
            }
            t.into()
        })
        .collect();
    table.insert("release".into(), releases.into());
    toml::to_string(&table).unwrap_or_default()
}

impl Entry {
    pub fn remote(&self) -> Remote {
        let tags = self
            .releases
            .iter()
            .filter(|r| !r.yanked)
            .filter_map(|r| Some(Tag { version: version::parse_tag(&r.version)?, commit: r.commit.clone() }))
            .collect();
        Remote { head: None, tags }
    }

    pub fn with_release(existing: Option<Entry>, source: &str, release: Release) -> Result<Entry, String> {
        let mut entry = existing.unwrap_or(Entry { source: source.to_string(), releases: Vec::new() });
        if same_repo(&entry.source) != same_repo(source) {
            return Err(format!("this name is already registered for {}, not {}", entry.source, source));
        }
        if entry.releases.iter().any(|r| r.version == release.version) {
            return Err(format!("version {} is already published; bump the version in project.toml", release.version));
        }
        entry.releases.push(release);
        Ok(entry)
    }
}

fn same_repo(url: &str) -> String {
    url.trim_end_matches('/').trim_end_matches(".git").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_file, TempDir};

    fn release(v: &str) -> Release {
        Release { version: v.into(), commit: "a".repeat(40), hash: "sha256-x".into(), yanked: false }
    }

    #[test]
    fn an_entry_survives_a_round_trip() {
        let mut yanked = release("1.0.0");
        yanked.yanked = true;
        let entry = Entry { source: "https://github.com/u/greet.git".into(), releases: vec![yanked, release("1.1.0")] };
        assert_eq!(parse(&render(&entry)).unwrap(), entry);
    }

    #[test]
    fn yanked_releases_are_not_offered() {
        let mut yanked = release("1.1.0");
        yanked.yanked = true;
        let entry = Entry { source: "u".into(), releases: vec![release("1.0.0"), yanked] };
        let versions: Vec<String> = entry.remote().tags.iter().map(|t| t.version.to_string()).collect();
        assert_eq!(versions, vec!["1.0.0"]);
    }

    #[test]
    fn publishing_refuses_a_duplicate_version_or_another_repository() {
        let src = "https://github.com/u/greet.git";
        let entry = Entry::with_release(None, src, release("1.0.0")).unwrap();
        assert!(Entry::with_release(Some(entry.clone()), "https://github.com/u/greet", release("1.1.0")).is_ok());
        assert!(Entry::with_release(Some(entry.clone()), src, release("1.0.0")).unwrap_err().contains("already published"));
        assert!(Entry::with_release(Some(entry), "https://github.com/evil/greet.git", release("1.1.0")).unwrap_err().contains("already registered"));
    }

    #[test]
    fn yidi_registry_selects_a_mirror_or_a_local_folder() {
        assert!(matches!(Index::from_setting(None), Index::Http(u) if u == DEFAULT_BASE));
        assert!(matches!(Index::from_setting(Some(" ")), Index::Http(u) if u == DEFAULT_BASE));
        assert!(matches!(Index::from_setting(Some("https://mirror.example/idx/")), Index::Http(u) if u == "https://mirror.example/idx"));
        assert!(matches!(Index::from_setting(Some("/srv/registry")), Index::Dir(d) if d == PathBuf::from("/srv/registry")));
    }

    #[test]
    fn invalid_releases_are_rejected() {
        assert!(parse("source = \"u\"\n[[release]]\nversion = \"one\"\ncommit = \"x\"\nhash = \"y\"\n").is_err());
    }

    #[test]
    fn a_directory_index_reads_entries_and_misses_cleanly() {
        let dir = TempDir::new();
        let entry = Entry { source: "u".into(), releases: vec![release("1.0.0")] };
        write_file(dir.path(), &entry_path("greet"), &render(&entry));
        let index = Index::Dir(dir.path().to_path_buf());
        assert_eq!(index.entry("greet").unwrap(), Some(entry));
        assert_eq!(index.entry("other").unwrap(), None);
    }
}
