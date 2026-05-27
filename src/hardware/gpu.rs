#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub dedicated_memory: u64,
    pub api_type: String,
}

fn detect_windows_dxdiag() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    if let Ok(out) = std::process::Command::new("wmic")
        .args(["path", "Win32_VideoController", "get", "Name,AdapterRAM,AdapterCompatibility", "/format:csv"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines().skip(1) {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 3 {
                let name = parts[1].trim().to_string();
                let vendor = parts[2].trim().to_string();
                let mem = parts.get(3)
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .unwrap_or(0);
                if !name.is_empty() {
                    gpus.push(GpuInfo {
                        name,
                        vendor,
                        dedicated_memory: mem,
                        api_type: "DirectX".into(),
                    });
                }
            }
        }
    }
    gpus
}

#[cfg(target_os = "linux")]
fn detect_linux_lspci() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    if let Ok(out) = std::process::Command::new("lspci")
        .args(["-v", "-m"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut current_vendor = String::new();
        for line in stdout.lines() {
            if line.contains("VGA") || line.contains("3D") {
                let parts: Vec<&str> = line.split('"').collect();
                if parts.len() >= 3 {
                    current_vendor = parts[1].to_string();
                }
            }
            if line.contains("Memory at") && !current_vendor.is_empty() {
                gpus.push(GpuInfo {
                    name: current_vendor.clone(),
                    vendor: current_vendor.clone(),
                    dedicated_memory: 0,
                    api_type: "Vulkan".into(),
                });
                current_vendor.clear();
            }
        }
    }
    gpus
}

pub fn detect() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    if cfg!(target_os = "windows") {
        gpus = detect_windows_dxdiag();
    }
    #[cfg(target_os = "linux")]
    {
        if gpus.is_empty() {
            gpus = detect_linux_lspci();
        }
    }
    if gpus.is_empty() {
        gpus.push(GpuInfo {
            name: "Unknown (no GPU detected)".into(),
            vendor: "unknown".into(),
            dedicated_memory: 0,
            api_type: "none".into(),
        });
    }
    gpus
}

pub fn has_dedicated_gpu(gpus: &[GpuInfo]) -> bool {
    gpus.iter().any(|g| g.dedicated_memory > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_detect_no_panic() {
        let gpus = detect();
        assert!(!gpus.is_empty(), "at least unknown gpu");
    }
}
