use semver::{Version, VersionReq};

#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub version: Version,
    pub commit: String,
}

pub fn parse_tag(name: &str) -> Option<Version> {
    Version::parse(name.strip_prefix('v').unwrap_or(name)).ok()
}

pub fn parse_req(text: &str) -> Result<VersionReq, String> {
    VersionReq::parse(text).map_err(|e| format!("invalid version range '{}': {}", text, e))
}

pub fn best<'a>(tags: &'a [Tag], reqs: &[VersionReq]) -> Option<&'a Tag> {
    tags.iter().filter(|t| reqs.iter().all(|r| r.matches(&t.version))).max_by(|a, b| a.version.cmp(&b.version))
}

pub fn caret(version: &Version) -> String {
    format!("^{}", version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(list: &[&str]) -> Vec<Tag> {
        list.iter().map(|v| Tag { version: Version::parse(v).unwrap(), commit: format!("c{}", v) }).collect()
    }

    #[test]
    fn tags_with_and_without_v_are_versions() {
        assert_eq!(parse_tag("v1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_tag("1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_tag("release"), None);
    }

    #[test]
    fn the_highest_version_matching_every_range_wins() {
        let all = tags(&["1.0.0", "1.4.2", "1.9.0", "2.0.0"]);
        let pick = |rs: &[&str]| best(&all, &rs.iter().map(|r| parse_req(r).unwrap()).collect::<Vec<_>>()).map(|t| t.version.to_string());
        assert_eq!(pick(&["^1.2"]).as_deref(), Some("1.9.0"));
        assert_eq!(pick(&["^1.2", "<1.5"]).as_deref(), Some("1.4.2"));
        assert_eq!(pick(&["*"]).as_deref(), Some("2.0.0"));
        assert_eq!(pick(&["^3"]), None);
    }

    #[test]
    fn prereleases_are_only_chosen_when_asked_for() {
        let all = tags(&["1.0.0", "1.1.0-beta.1"]);
        assert_eq!(best(&all, &[parse_req("^1").unwrap()]).unwrap().version.to_string(), "1.0.0");
        assert_eq!(best(&all, &[parse_req("=1.1.0-beta.1").unwrap()]).unwrap().version.to_string(), "1.1.0-beta.1");
    }
}
