const MAX_LEN: usize = 64;
const SCHEMES: [&str; 5] = ["https", "http", "ssh", "git", "file"];

pub fn validate(name: &str) -> Result<&str, String> {
    let allowed = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    let valid = !name.is_empty() && name.len() <= MAX_LEN && name.bytes().all(allowed) && !name.starts_with('-');
    if valid {
        Ok(name)
    } else {
        Err(format!("invalid package name '{}': use letters, digits, '_' and '-' (at most {} characters, not starting with '-')", name, MAX_LEN))
    }
}

pub fn from_spec(spec: &str) -> Result<(String, Option<String>), String> {
    if !spec.contains('/') && !spec.contains(':') {
        return Ok((validate(spec)?.to_string(), None));
    }
    let url = url_of(spec)?;
    let last = url.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or("");
    let name = validate(last.strip_suffix(".git").unwrap_or(last))?.to_string();
    Ok((name, Some(url)))
}

fn url_of(spec: &str) -> Result<String, String> {
    if let Some((scheme, _)) = spec.split_once("://") {
        return if SCHEMES.contains(&scheme) {
            Ok(spec.to_string())
        } else {
            Err(format!("unsupported URL scheme '{}': use one of {}", scheme, SCHEMES.join(", ")))
        };
    }
    let shorthand = spec.strip_prefix("github:").unwrap_or(spec);
    match shorthand.split_once('/') {
        Some((user, repo)) if validate(user).is_ok() && validate(repo.trim_end_matches(".git")).is_ok() => {
            Ok(format!("https://github.com/{}/{}.git", user, repo.trim_end_matches(".git")))
        }
        _ => Err(format!("invalid package '{}': use github:user/repo, user/repo or a git URL", spec)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_are_accepted() {
        for name in ["greet", "my_pkg", "http-client", "v2"] {
            assert!(validate(name).is_ok(), "{}", name);
        }
    }

    #[test]
    fn names_that_could_leave_lib_are_rejected() {
        for name in ["", ".", "..", "a/b", "a\\b", "-rf", "a b", "caf\u{e9}", &"x".repeat(65)] {
            assert!(validate(name).is_err(), "{}", name);
        }
    }

    #[test]
    fn specs_give_a_name_and_a_url() {
        let gh = ("greet".to_string(), Some("https://github.com/user/greet.git".to_string()));
        assert_eq!(from_spec("github:user/greet").unwrap(), gh);
        assert_eq!(from_spec("user/greet").unwrap(), gh);
        assert_eq!(from_spec("user/greet.git").unwrap(), gh);
        assert_eq!(from_spec("https://gitlab.com/a/greet.git").unwrap().0, "greet");
        assert_eq!(from_spec("file:///srv/repos/greet/").unwrap().0, "greet");
    }

    #[test]
    fn a_bare_name_is_a_registry_package() {
        assert_eq!(from_spec("greet").unwrap(), ("greet".to_string(), None));
    }

    #[test]
    fn specs_that_name_no_safe_directory_are_rejected() {
        for spec in ["file:///tmp/../..", "https://host/a/.", "..", "github:user", "ext::sh -c x://y", "user/../x", "-u/x"] {
            assert!(from_spec(spec).is_err(), "{}", spec);
        }
    }
}
