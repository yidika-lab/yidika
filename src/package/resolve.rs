use super::global;
use super::index::Index;
use super::lock::Locked;
use super::manifest::{Dep, Manifest};
use super::pool::parallel;
use super::remote::{self, Remote};
use super::tree;
use super::version;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

const MAX_ROUNDS: usize = 64;
const PROJECT: &str = "project.toml";

struct Need {
    url: String,
    reqs: Vec<String>,
    by: Vec<String>,
}

struct Contents {
    hash: String,
    deps: Vec<(String, Dep)>,
}

pub struct Resolver {
    store: PathBuf,
    index: Index,
    remotes: HashMap<String, Remote>,
    sources: HashMap<String, String>,
    expected: HashMap<String, String>,
    contents: HashMap<String, Contents>,
    pub downloaded: usize,
}

impl Resolver {
    pub fn new(store: &Path, index: Index) -> Self {
        Self { store: store.to_path_buf(), index, remotes: HashMap::new(), sources: HashMap::new(), expected: HashMap::new(), contents: HashMap::new(), downloaded: 0 }
    }

    pub fn locate(&mut self, name: &str, dep: &Dep) -> Result<String, String> {
        if let Some(url) = &dep.url {
            return Ok(url.clone());
        }
        if let Some(url) = self.sources.get(name) {
            return Ok(url.clone());
        }
        let entry = self.index.entry(name)?.ok_or_else(|| format!("package '{}' is not in the registry; add it by URL (github:user/{}) or publish it with `yidi publish`", name, name))?;
        for r in &entry.releases {
            self.expected.insert(r.commit.clone(), r.hash.clone());
        }
        self.remotes.insert(entry.source.clone(), entry.remote());
        self.sources.insert(name.to_string(), entry.source.clone());
        Ok(entry.source)
    }

    pub fn remote(&mut self, url: &str) -> Result<&Remote, String> {
        if !self.remotes.contains_key(url) {
            let found = remote::query(url)?;
            self.remotes.insert(url.to_string(), found);
        }
        Ok(&self.remotes[url])
    }

    pub fn resolve(&mut self, roots: &[(String, Dep)], preferred: &[Locked]) -> Result<Vec<Locked>, String> {
        let mut chosen: BTreeMap<String, Locked> = BTreeMap::new();
        for _ in 0..MAX_ROUNDS {
            let needs = self.needs(roots, &chosen)?;
            self.query_remotes(&needs, preferred)?;
            let mut next = BTreeMap::new();
            for (name, need) in &needs {
                next.insert(name.clone(), self.pick(name, need, preferred)?);
            }
            self.load_contents(&next)?;
            for p in next.values_mut() {
                let contents = &self.contents[&p.commit];
                p.hash = contents.hash.clone();
                p.dependencies = contents.deps.iter().map(|(n, _)| n.clone()).collect();
            }
            if same_commits(&chosen, &next) {
                return Ok(next.into_values().collect());
            }
            chosen = next;
        }
        Err("dependency versions did not settle; check for packages that require each other with conflicting ranges".into())
    }

    fn needs(&mut self, roots: &[(String, Dep)], chosen: &BTreeMap<String, Locked>) -> Result<BTreeMap<String, Need>, String> {
        let mut wanted: Vec<(String, Dep, String)> = roots.iter().map(|(n, d)| (n.clone(), d.clone(), PROJECT.to_string())).collect();
        for p in chosen.values() {
            wanted.extend(self.contents[&p.commit].deps.iter().map(|(n, d)| (n.clone(), d.clone(), p.name.clone())));
        }
        let mut needs: BTreeMap<String, Need> = BTreeMap::new();
        for (name, dep, by) in wanted {
            let url = self.locate(&name, &dep)?;
            let need = needs.entry(name.clone()).or_insert_with(|| Need { url: url.clone(), reqs: Vec::new(), by: Vec::new() });
            if need.url != url {
                return Err(format!("'{}' is required from two sources: {} (by {}) and {} (by {})", name, need.url, need.by.join(", "), url, by));
            }
            need.reqs.push(dep.req);
            need.by.push(by);
        }
        Ok(needs)
    }

    fn query_remotes(&mut self, needs: &BTreeMap<String, Need>, preferred: &[Locked]) -> Result<(), String> {
        let mut urls: Vec<String> = needs
            .iter()
            .filter(|(name, need)| keep(name, need, preferred).is_none() && !self.remotes.contains_key(&need.url))
            .map(|(_, need)| need.url.clone())
            .collect();
        urls.dedup();
        for (url, found) in urls.iter().zip(parallel(&urls, |u| remote::query(u))) {
            self.remotes.insert(url.clone(), found?);
        }
        Ok(())
    }

    fn pick(&self, name: &str, need: &Need, preferred: &[Locked]) -> Result<Locked, String> {
        if let Some(p) = keep(name, need, preferred) {
            return Ok(p.clone());
        }
        let remote = &self.remotes[&need.url];
        let reqs = need.reqs.iter().map(|r| version::parse_req(r)).collect::<Result<Vec<_>, _>>()?;
        let unpinned = need.reqs.iter().all(|r| r == "*");
        let (commit, found) = if unpinned && remote.tags.is_empty() {
            (remote.head.clone().ok_or_else(|| format!("{} has no commits", need.url))?, None)
        } else {
            let tag = version::best(&remote.tags, &reqs).ok_or_else(|| no_match(name, need, remote))?;
            (tag.commit.clone(), Some(tag.version.to_string()))
        };
        Ok(Locked { name: name.to_string(), source: need.url.clone(), version: found, commit, hash: String::new(), dependencies: Vec::new() })
    }

    fn load_contents(&mut self, next: &BTreeMap<String, Locked>) -> Result<(), String> {
        let missing: Vec<(String, String)> = next.values().filter(|p| !self.contents.contains_key(&p.commit)).map(|p| (p.source.clone(), p.commit.clone())).collect();
        let store = self.store.clone();
        let loaded = parallel(&missing, |(url, commit)| read_contents(&store, url, commit));
        for ((url, commit), result) in missing.iter().zip(loaded) {
            let (contents, downloaded) = result?;
            if let Some(expected) = self.expected.get(commit).filter(|h| **h != contents.hash) {
                global::evict(&self.store, commit);
                return Err(format!("{} at {} does not match the registry ({} expected, {} found)", url, commit, expected, contents.hash));
            }
            self.downloaded += downloaded as usize;
            self.contents.insert(commit.clone(), contents);
        }
        Ok(())
    }
}

fn keep<'a>(name: &str, need: &Need, preferred: &'a [Locked]) -> Option<&'a Locked> {
    let p = preferred.iter().find(|p| p.name == name && p.source == need.url)?;
    let fits = match &p.version {
        Some(v) => {
            let v = semver::Version::parse(v).ok()?;
            need.reqs.iter().all(|r| version::parse_req(r).is_ok_and(|r| r.matches(&v)))
        }
        None => need.reqs.iter().all(|r| r == "*"),
    };
    fits.then_some(p)
}

fn read_contents(store: &Path, url: &str, commit: &str) -> Result<(Contents, bool), String> {
    let cached = global::ensure(store, url, commit)?;
    let deps = Manifest::load(&cached.dir)?.dependencies().map_err(|e| format!("{} at {}: {}", url, commit, e))?;
    Ok((Contents { hash: tree::hash(&cached.dir)?, deps }, cached.downloaded))
}

fn no_match(name: &str, need: &Need, remote: &Remote) -> String {
    let mut versions: Vec<String> = remote.tags.iter().map(|t| t.version.to_string()).collect();
    versions.sort();
    let available = if versions.is_empty() { "none".to_string() } else { versions.join(", ") };
    let wanted: Vec<String> = need.reqs.iter().zip(&need.by).map(|(r, b)| format!("{} (by {})", r, b)).collect();
    format!("no version of '{}' matches {}; available: {}", name, wanted.join(" and "), available)
}

fn same_commits(a: &BTreeMap<String, Locked>, b: &BTreeMap<String, Locked>) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|((na, pa), (nb, pb))| na == nb && pa.commit == pb.commit)
}
