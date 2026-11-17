use std::io::Read;

pub fn get(url: &str, user_agent: &str, limit: u64, label: &str) -> Result<Vec<u8>, String> {
    let fail = |e: &dyn std::fmt::Display| format!("download of {} failed: {}", url, e);
    let mut response = ureq::get(url).header("User-Agent", user_agent).call().map_err(|e| fail(&e))?;
    let total = response.body().content_length();
    if total.is_some_and(|n| n > limit) {
        return Err(fail(&format!("larger than {} bytes", limit)));
    }
    let progress = crate::ui::Download::start(label, total);
    let mut reader = response.body_mut().with_config().limit(limit).reader();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut chunk).map_err(|e| fail(&e))?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..read]);
        progress.advance(read as u64);
    }
}
