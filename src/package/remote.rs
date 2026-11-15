use super::version::{self, Tag};
use std::process::Command;

#[derive(Clone, Debug, Default)]
pub struct Remote {
    pub head: Option<String>,
    pub tags: Vec<Tag>,
}

pub fn query(url: &str) -> Result<Remote, String> {
    let output = Command::new("git")
        .args(["ls-remote", "--", url, "HEAD", "refs/tags/*"])
        .output()
        .map_err(|e| format!("cannot run git ({}): is git installed?", e))?;
    if !output.status.success() {
        return Err(format!("cannot list versions of {}: {}", url, String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(parse_refs(&String::from_utf8_lossy(&output.stdout)))
}

pub fn is_commit(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn parse_refs(listing: &str) -> Remote {
    let mut remote = Remote::default();
    let mut peeled: Vec<(String, String)> = Vec::new();
    let mut direct: Vec<(String, String)> = Vec::new();
    for line in listing.lines() {
        let Some((commit, reference)) = line.split_once('\t') else { continue };
        if !is_commit(commit) {
            continue;
        }
        if reference == "HEAD" {
            remote.head = Some(commit.to_string());
        } else if let Some(tag) = reference.strip_prefix("refs/tags/") {
            match tag.strip_suffix("^{}") {
                Some(name) => peeled.push((name.to_string(), commit.to_string())),
                None => direct.push((tag.to_string(), commit.to_string())),
            }
        }
    }
    for (name, commit) in direct {
        let commit = peeled.iter().find(|(n, _)| *n == name).map(|(_, c)| c.clone()).unwrap_or(commit);
        if let Some(version) = version::parse_tag(&name) {
            remote.tags.push(Tag { version, commit });
        }
    }
    remote
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotated_tags_resolve_to_their_commit() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let c = "c".repeat(40);
        let listing = format!("{a}\tHEAD\n{b}\trefs/tags/v1.0.0\n{c}\trefs/tags/v1.1.0\n{b}\trefs/tags/v1.1.0^{{}}\n{a}\trefs/tags/nightly\n");
        let remote = parse_refs(&listing);
        assert_eq!(remote.head.as_deref(), Some(a.as_str()));
        let found: Vec<(String, String)> = remote.tags.iter().map(|t| (t.version.to_string(), t.commit.clone())).collect();
        assert_eq!(found, vec![("1.0.0".to_string(), b.clone()), ("1.1.0".to_string(), b)]);
    }

    #[test]
    fn lines_without_a_valid_commit_are_ignored() {
        let remote = parse_refs("../../etc\trefs/tags/v1.0.0\nnot a line\n");
        assert!(remote.head.is_none() && remote.tags.is_empty());
    }
}
