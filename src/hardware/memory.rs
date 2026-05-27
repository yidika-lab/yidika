use sysinfo::System;

#[derive(Debug, Clone)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

pub fn detect() -> MemoryInfo {
    let mut sys = System::new();
    sys.refresh_memory();

    MemoryInfo {
        total_bytes: sys.total_memory(),
        available_bytes: sys.available_memory(),
    }
}

pub fn total_gb(info: &MemoryInfo) -> f64 {
    info.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

pub fn available_gb(info: &MemoryInfo) -> f64 {
    info.available_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

pub fn is_low_memory(info: &MemoryInfo) -> bool {
    info.total_bytes < 4_000_000_000
}

pub fn is_high_memory(info: &MemoryInfo) -> bool {
    info.total_bytes >= 16_000_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_detect() {
        let mem = detect();
        assert!(mem.total_bytes > 0, "total memory > 0");
    }

    #[test]
    fn test_memory_gb() {
        let mem = detect();
        let gb = total_gb(&mem);
        assert!(gb > 0.0);
    }
}
