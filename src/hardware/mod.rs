pub mod cpu;
pub mod memory;
pub mod gpu;

use std::fmt;
use std::sync::OnceLock;

pub use cpu::{CpuInfo, SimdCapabilities};
pub use memory::MemoryInfo;
pub use gpu::GpuInfo;

#[derive(Debug, Clone)]
pub struct OsInfo {
    pub name: String,
    pub triple: String,
}

#[derive(Debug, Clone)]
pub struct HardwareInfo {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub gpus: Vec<GpuInfo>,
    pub os: OsInfo,
}

impl fmt::Display for HardwareInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "═══ Yidika Hardware Info ═══")?;
        writeln!(f, "OS:      {} ({})", self.os.name, self.os.triple)?;
        writeln!(f, "CPU:     {} [{}]", self.cpu.name, self.cpu.vendor)?;
        writeln!(f, "Cores:   {} physical, {} logical (arch: {})",
            self.cpu.physical_cores, self.cpu.logical_cores, self.cpu.arch)?;
        writeln!(f, "Cache:   L1={}, L2={}, L3={}",
            fmt_cache(self.cpu.cache_l1),
            fmt_cache(self.cpu.cache_l2),
            fmt_cache(self.cpu.cache_l3))?;
        writeln!(f, "SIMD:    {} ({:?})",
            self.cpu.simd.best_feature_string(),
            self.cpu.simd.to_llvm_features())?;
        writeln!(f, "RAM:     {:.1} GB total, {:.1} GB available",
            memory::total_gb(&self.memory),
            memory::available_gb(&self.memory))?;
        for (i, gpu) in self.gpus.iter().enumerate() {
            let mem = if gpu.dedicated_memory > 0 {
                format!("{:.1} GB", gpu.dedicated_memory as f64 / (1024.0 * 1024.0 * 1024.0))
            } else {
                "shared".into()
            };
            writeln!(f, "GPU[{}]: {} ({}, {})", i, gpu.name, gpu.vendor, mem)?;
        }
        let opt_level = if memory::is_low_memory(&self.memory) { "O2 (low RAM)" }
            else if memory::is_high_memory(&self.memory) { "O3 (high RAM)" }
            else { "O3" };
        writeln!(f, "Opt:     {} recommended", opt_level)?;
        let parallel = if self.cpu.logical_cores > 1 { "multi-core" } else { "single-core" };
        writeln!(f, "Mode:    {}", parallel)?;
        write!(f, "═══════════════════════════════")
    }
}

fn fmt_cache(v: Option<u64>) -> String {
    match v {
        Some(n) if n >= 1024 * 1024 => format!("{:.1} MB", n as f64 / (1024.0 * 1024.0)),
        Some(n) if n >= 1024 => format!("{} KB", n / 1024),
        Some(n) => format!("{} B", n),
        None => "unknown".into(),
    }
}

pub fn detect() -> HardwareInfo {
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_list(sysinfo::CpuRefreshKind::everything());

    let mut cpu_info = cpu::detect();
    let phys = cpu::detect_physical_from_sysinfo(&sys);
    if phys > 0 {
        cpu_info.physical_cores = phys;
    }

    HardwareInfo {
        cpu: cpu_info,
        memory: memory::detect(),
        gpus: gpu::detect(),
        os: OsInfo {
            name: std::env::consts::OS.to_string(),
            triple: format!("{}-pc-{}",
                std::env::consts::ARCH,
                std::env::consts::FAMILY),
        },
    }
}

pub fn recommended_opt_level(info: &HardwareInfo) -> &str {
    if memory::is_low_memory(&info.memory) { "O2" }
    else { "O3" }
}

pub fn recommended_parallelism(info: &HardwareInfo) -> u32 {
    info.cpu.logical_cores.max(1)
}

pub fn cached_hardware_info() -> &'static HardwareInfo {
    static CACHED: OnceLock<HardwareInfo> = OnceLock::new();
    CACHED.get_or_init(|| detect())
}

impl HardwareInfo {
    pub fn cached() -> Option<&'static HardwareInfo> {
        static CACHED: OnceLock<HardwareInfo> = OnceLock::new();
        Some(CACHED.get_or_init(|| detect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hardware_detect() {
        let hw = detect();
        assert!(hw.cpu.logical_cores > 0);
        assert!(hw.memory.total_bytes > 0);
        assert!(!hw.os.name.is_empty());
    }

    #[test]
    fn test_display_no_panic() {
        let hw = detect();
        let s = hw.to_string();
        assert!(s.contains("Yidika Hardware Info"));
    }

    #[test]
    fn test_opt_level() {
        let hw = detect();
        let level = recommended_opt_level(&hw);
        assert!(level == "O2" || level == "O3");
    }

    #[test]
    fn test_recommended_parallelism() {
        let hw = detect();
        let n = recommended_parallelism(&hw);
        assert!(n >= 1);
    }
}
