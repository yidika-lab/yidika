pub use regex::Error;
use regex::Regex;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

const CACHE_LIMIT: usize = 64;

static CACHE: LazyLock<Mutex<HashMap<String, Regex>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn compile(pattern: &str) -> Result<Regex, regex::Error> {
    let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(re) = cache.get(pattern) {
        return Ok(re.clone());
    }
    let re = Regex::new(pattern)?;
    if cache.len() >= CACHE_LIMIT {
        cache.clear();
    }
    cache.insert(pattern.to_string(), re.clone());
    Ok(re)
}

pub fn is_match(pattern: &str, text: &str) -> Result<bool, regex::Error> {
    Ok(compile(pattern)?.is_match(text))
}

pub fn replace_all(pattern: &str, text: &str, replacement: &str) -> Result<String, regex::Error> {
    Ok(compile(pattern)?.replace_all(text, replacement).into_owned())
}

pub fn find_all(pattern: &str, text: &str) -> Result<Vec<String>, regex::Error> {
    Ok(compile(pattern)?.find_iter(text).map(|m| m.as_str().to_string()).collect())
}

pub fn split(pattern: &str, text: &str) -> Result<Vec<String>, regex::Error> {
    Ok(compile(pattern)?.split(text).map(str::to_string).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pattern_matches_anywhere_in_the_text() {
        assert!(is_match(r"\d+", "order 42").unwrap());
        assert!(!is_match(r"^\d+$", "order 42").unwrap());
    }

    #[test]
    fn replace_rewrites_every_match_with_groups() {
        assert_eq!(replace_all(r"(\w+)@(\w+)", "a@b c@d", "$2.$1").unwrap(), "b.a d.c");
    }

    #[test]
    fn find_and_split_return_every_piece() {
        assert_eq!(find_all(r"\d", "a1b2").unwrap(), vec!["1", "2"]);
        assert_eq!(split(r",\s*", "a, b,c").unwrap(), vec!["a", "b", "c"]);
    }

    #[test]
    fn an_invalid_pattern_is_an_error() {
        assert!(is_match("(", "x").is_err());
    }

    #[test]
    fn the_cache_stays_bounded() {
        for i in 0..(CACHE_LIMIT * 3) {
            is_match(&format!("x{}", i), "x1").unwrap();
        }
        assert!(CACHE.lock().unwrap().len() <= CACHE_LIMIT);
    }
}
