use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const RELEASES: &str = "https://github.com/yidika-lab/yidika/releases";
const LATEST_API: &str = "https://api.github.com/repos/yidika-lab/yidika/releases/latest";
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const NOT_INSTALLED: [&str; 2] = ["LICENSE", "README.md"];

pub fn upgrade(check_only: bool) -> Result<String, String> {
    let current = env!("CARGO_PKG_VERSION");
    let checking = crate::ui::Task::start("Checking", "for a newer yidi", 1);
    let latest = latest_version()?;
    drop(checking);
    if !is_newer(&latest, current) {
        return Ok(format!("Checked yidi {}: already the latest version", current));
    }
    if check_only {
        return Ok(format!("Found yidi {} (installed: {}). Run `yidi upgrade` to install it.", latest, current));
    }
    let target = target_triple(std::env::consts::OS, std::env::consts::ARCH)
        .ok_or_else(|| format!("no prebuilt yidi for {}-{}", std::env::consts::ARCH, std::env::consts::OS))?;
    install_release(&latest, &target)?;
    Ok(format!("Upgraded yidi from {} to {}", current, latest))
}

fn latest_version() -> Result<String, String> {
    let body = download(LATEST_API, MAX_METADATA_BYTES)?;
    let json: serde_json::Value = serde_json::from_slice(&body).map_err(|e| format!("unreadable release metadata: {}", e))?;
    let tag = json.get("tag_name").and_then(|t| t.as_str()).ok_or("release metadata has no tag_name")?;
    parse_tag(tag).ok_or_else(|| format!("unexpected release tag '{}'", tag))
}

fn parse_tag(tag: &str) -> Option<String> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<&str> = version.split('.').collect();
    let valid = parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()));
    valid.then(|| version.to_string())
}

fn version_numbers(version: &str) -> Vec<u64> {
    version.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

fn is_newer(candidate: &str, current: &str) -> bool {
    version_numbers(candidate) > version_numbers(current)
}

fn target_triple(os: &str, arch: &str) -> Option<String> {
    match (os, arch) {
        ("linux", "x86_64" | "aarch64") => Some(format!("{}-unknown-linux-gnu", arch)),
        ("macos", "x86_64" | "aarch64") => Some(format!("{}-apple-darwin", arch)),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc".to_string()),
        _ => None,
    }
}

fn asset_name(target: &str) -> String {
    let extension = if target.contains("windows") { "zip" } else { "tar.gz" };
    format!("yidi-{}.{}", target, extension)
}

fn download(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let label = url.rsplit('/').next().unwrap_or(url);
    crate::download::get(url, "yidi-upgrade", limit, label)
}

fn expected_digest(checksum_file: &str) -> Option<String> {
    let digest = checksum_file.split_whitespace().next()?.to_ascii_lowercase();
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())).then_some(digest)
}

fn verify_digest(bytes: &[u8], expected: &str) -> Result<(), String> {
    let actual: String = Sha256::digest(bytes).iter().map(|b| format!("{:02x}", b)).collect();
    if actual == expected {
        Ok(())
    } else {
        Err(format!("checksum mismatch: expected {}, got {}", expected, actual))
    }
}

fn install_release(version: &str, target: &str) -> Result<(), String> {
    let asset = asset_name(target);
    let base = format!("{}/download/v{}/{}", RELEASES, version, asset);
    let checksum = String::from_utf8(download(&format!("{}.sha256", base), MAX_METADATA_BYTES)?).map_err(|_| "checksum file is not text")?;
    let expected = expected_digest(&checksum).ok_or("checksum file has no SHA-256 digest")?;
    let archive = download(&base, MAX_ARCHIVE_BYTES)?;
    verify_digest(&archive, &expected)?;
    let work = std::env::temp_dir().join(format!("yidi-upgrade-{}", std::process::id()));
    let result = unpack_and_install(&archive, &asset, &work, target);
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn unpack_and_install(archive: &[u8], asset: &str, work: &Path, target: &str) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(work).map_err(|e| format!("cannot create {}: {}", work.display(), e))?;
    let archive_path = work.join(asset);
    std::fs::write(&archive_path, archive).map_err(|e| format!("cannot write {}: {}", archive_path.display(), e))?;
    extract(&archive_path, work)?;
    let unpacked = work.join(format!("yidi-{}", target));
    let install_dir = installed_dir()?;
    for file in files_to_install(&unpacked)? {
        let name = file.file_name().ok_or("archive entry has no name")?;
        replace_file(&file, &install_dir.join(name))?;
    }
    Ok(())
}

fn extract(archive: &Path, into: &Path) -> Result<(), String> {
    let status = std::process::Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .status()
        .map_err(|e| format!("cannot run tar: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("tar could not extract {}", archive.display()))
    }
}

fn installed_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|e| format!("cannot locate the running yidi: {}", e))?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| "the running yidi has no parent directory".to_string())
}

fn files_to_install(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("unexpected archive layout ({}): {}", dir.display(), e))?;
    let files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && !NOT_INSTALLED.iter().any(|n| p.file_name().is_some_and(|f| f == *n)))
        .collect();
    if files.is_empty() {
        return Err(format!("no files to install in {}", dir.display()));
    }
    Ok(files)
}

fn replace_file(new: &Path, dest: &Path) -> Result<(), String> {
    let mut old_name = dest.as_os_str().to_owned();
    old_name.push(".old");
    let old = PathBuf::from(old_name);
    let _ = std::fs::remove_file(&old);
    let had_previous = dest.exists();
    if had_previous {
        std::fs::rename(dest, &old).map_err(|e| permission_hint(dest, e))?;
    }
    if let Err(e) = std::fs::copy(new, dest) {
        if had_previous {
            let _ = std::fs::rename(&old, dest);
        }
        return Err(permission_hint(dest, e));
    }
    let _ = std::fs::remove_file(&old);
    Ok(())
}

fn permission_hint(path: &Path, e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        format!("cannot write {}: permission denied (rerun with the rights of the user who installed yidi)", path.display())
    } else {
        format!("cannot write {}: {}", path.display(), e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_with_or_without_v_give_the_version() {
        assert_eq!(parse_tag("v0.2.10").as_deref(), Some("0.2.10"));
        assert_eq!(parse_tag("1.0.0").as_deref(), Some("1.0.0"));
    }

    #[test]
    fn tags_that_are_not_plain_versions_are_rejected() {
        for tag in ["v1.0", "v1.0.0-rc1", "v1..0", "v1.0.0/../x", "latest", "v1234567890.0.0"] {
            assert_eq!(parse_tag(tag), None, "{}", tag);
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn targets_match_the_release_matrix() {
        assert_eq!(target_triple("linux", "x86_64").as_deref(), Some("x86_64-unknown-linux-gnu"));
        assert_eq!(target_triple("linux", "aarch64").as_deref(), Some("aarch64-unknown-linux-gnu"));
        assert_eq!(target_triple("macos", "aarch64").as_deref(), Some("aarch64-apple-darwin"));
        assert_eq!(target_triple("macos", "x86_64").as_deref(), Some("x86_64-apple-darwin"));
        assert_eq!(target_triple("windows", "x86_64").as_deref(), Some("x86_64-pc-windows-msvc"));
        assert_eq!(target_triple("windows", "aarch64"), None);
        assert_eq!(target_triple("freebsd", "x86_64"), None);
    }

    #[test]
    fn windows_ships_a_zip_and_others_a_tarball() {
        assert_eq!(asset_name("x86_64-pc-windows-msvc"), "yidi-x86_64-pc-windows-msvc.zip");
        assert_eq!(asset_name("aarch64-apple-darwin"), "yidi-aarch64-apple-darwin.tar.gz");
    }

    #[test]
    fn checksum_files_from_sha256sum_and_shasum_are_read() {
        let digest = "a".repeat(64);
        assert_eq!(expected_digest(&format!("{}  yidi.tar.gz\n", digest)), Some(digest.clone()));
        assert_eq!(expected_digest(&digest.to_uppercase()), Some(digest));
        assert_eq!(expected_digest("not-a-digest yidi.tar.gz"), None);
        assert_eq!(expected_digest(""), None);
    }

    #[test]
    fn a_wrong_digest_is_refused() {
        let hello = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify_digest(b"hello", hello).is_ok());
        assert!(verify_digest(b"hellO", hello).is_err());
    }

    #[test]
    fn replacing_a_file_swaps_its_contents_and_leaves_no_backup() {
        let dir = std::env::temp_dir().join(format!("yidi-replace-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (new, dest) = (dir.join("new"), dir.join("yidi"));
        std::fs::write(&new, "v2").unwrap();
        std::fs::write(&dest, "v1").unwrap();
        replace_file(&new, &dest).unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "v2");
        assert!(!dir.join("yidi.old").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn license_and_readme_are_not_installed() {
        let dir = std::env::temp_dir().join(format!("yidi-layout-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["LICENSE", "README.md", "yidi"] {
            std::fs::write(dir.join(name), name).unwrap();
        }
        assert_eq!(files_to_install(&dir).unwrap(), vec![dir.join("yidi")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
