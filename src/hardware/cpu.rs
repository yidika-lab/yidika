use std::sync::LazyLock;
use std::sync::Mutex;

static LLVM_CPU_NAME: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));

pub fn set_llvm_cpu_name(name: &str) {
    if let Ok(mut guard) = LLVM_CPU_NAME.lock() {
        *guard = Some(name.to_string());
    }
}

pub fn llvm_cpu_name() -> Option<String> {
    LLVM_CPU_NAME.lock().ok().and_then(|g| g.clone())
}

#[derive(Debug, Clone)]
pub struct SimdCapabilities {
    pub sse: bool,
    pub sse2: bool,
    pub sse3: bool,
    pub ssse3: bool,
    pub sse4_1: bool,
    pub sse4_2: bool,
    pub avx: bool,
    pub avx2: bool,
    pub avx512: bool,
    pub neon: bool,
    pub sve: bool,
    pub amx: bool,
}

impl SimdCapabilities {
    pub fn best_feature_string(&self) -> &str {
        if self.avx512 { "avx512" }
        else if self.avx2 { "avx2" }
        else if self.avx { "avx" }
        else if self.sse4_2 { "sse4.2" }
        else if self.neon { "neon" }
        else { "baseline" }
    }

    pub fn to_llvm_features(&self) -> Vec<&str> {
        let mut f = Vec::new();
        if self.sse { f.push("+sse"); }
        if self.sse2 { f.push("+sse2"); }
        if self.sse3 { f.push("+sse3"); }
        if self.ssse3 { f.push("+ssse3"); }
        if self.sse4_1 { f.push("+sse4.1"); }
        if self.sse4_2 { f.push("+sse4.2"); }
        if self.avx { f.push("+avx"); }
        if self.avx2 { f.push("+avx2"); }
        if self.avx512 { f.push("+avx512f"); }
        if self.neon { f.push("+neon"); }
        if self.sve { f.push("+sve"); }
        if f.is_empty() { f.push("+sse2"); }
        f
    }
}

#[derive(Debug, Clone)]
pub struct CpuInfo {
    pub name: String,
    pub vendor: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub cache_l1: Option<u64>,
    pub cache_l2: Option<u64>,
    pub cache_l3: Option<u64>,
    pub simd: SimdCapabilities,
    pub arch: String,
}

fn detect_simd() -> SimdCapabilities {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        let eax1 = std::arch::x86_64::__cpuid(1);
        let ecx1 = eax1.ecx;
        let edx1 = eax1.edx;

        let mut simd = SimdCapabilities {
            sse: (edx1 & (1 << 25)) != 0,
            sse2: (edx1 & (1 << 26)) != 0,
            sse3: (ecx1 & (1 << 0)) != 0,
            ssse3: (ecx1 & (1 << 9)) != 0,
            sse4_1: (ecx1 & (1 << 19)) != 0,
            sse4_2: (ecx1 & (1 << 20)) != 0,
            avx: (ecx1 & (1 << 28)) != 0,
            avx2: false,
            avx512: false,
            neon: false,
            sve: false,
            amx: false,
        };

        if simd.avx {
            let eax7 = std::arch::x86_64::__cpuid(7);
            simd.avx2 = (eax7.ebx & (1 << 5)) != 0;
            simd.avx512 = (eax7.ebx & (1 << 16)) != 0;
        }
        return simd;
    }

    #[cfg(target_arch = "aarch64")]
    {
        let mut simd = SimdCapabilities {
            sse: false, sse2: false, sse3: false, ssse3: false,
            sse4_1: false, sse4_2: false,
            avx: false, avx2: false, avx512: false,
            neon: true,
            sve: false,
            amx: false,
        };
        #[cfg(target_feature = "sve")]
        { simd.sve = true; }
        return simd;
    }

    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
    {
        SimdCapabilities {
            sse: false, sse2: false, sse3: false, ssse3: false,
            sse4_1: false, sse4_2: false,
            avx: false, avx2: false, avx512: false,
            neon: false, sve: false, amx: false,
        }
    }
}

fn arch_name() -> String {
    std::env::consts::ARCH.to_string()
}

fn vendor_name() -> String {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        let leaf = std::arch::x86_64::__cpuid(0);
        let b = leaf.ebx.to_le_bytes();
        let c = leaf.ecx.to_le_bytes();
        let d = leaf.edx.to_le_bytes();
        let vendor = String::from_utf8_lossy(&[b, d, c].concat()).to_string();
        if vendor.contains("GenuineIntel") { return "GenuineIntel".into(); }
        if vendor.contains("AuthenticAMD") { return "AuthenticAMD".into(); }
        return vendor;
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    { "generic".into() }
}

fn cache_info() -> (Option<u64>, Option<u64>, Option<u64>) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        let leaf_8000_0005 = std::arch::x86_64::__cpuid(0x80000005);
        let l1 = if leaf_8000_0005.eax != 0 {
            Some(((leaf_8000_0005.ecx >> 24) & 0xFF) as u64 * 1024)
        } else {
            None
        };

        let leaf_8000_0006 = std::arch::x86_64::__cpuid(0x80000006);
        let l2 = Some(((leaf_8000_0006.ecx >> 16) & 0xFFFF) as u64 * 1024);

        let edx = leaf_8000_0006.edx;
        let l3_size = ((edx >> 18) & 0xFFFF) as u64 * 512 * 1024;
        let l3 = if l3_size > 0 { Some(l3_size) } else { None };

        (l1, l2, l3)
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    { (None, None, None) }
}

pub fn detect() -> CpuInfo {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);

    let cpu_name = llvm_cpu_name()
        .unwrap_or_else(|| arch_name());

    let simd = detect_simd();
    let cache = cache_info();

    CpuInfo {
        name: cpu_name,
        vendor: vendor_name(),
        physical_cores: logical,
        logical_cores: logical,
        cache_l1: cache.0,
        cache_l2: cache.1,
        cache_l3: cache.2,
        simd,
        arch: arch_name(),
    }
}

pub fn detect_physical_from_sysinfo(sys: &sysinfo::System) -> u32 {
    sys.physical_core_count().unwrap_or(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_detect_no_panic() {
        let cpu = detect();
        assert!(cpu.logical_cores > 0, "at least 1 core");
        assert!(!cpu.arch.is_empty(), "arch non-empty");
    }

    #[test]
    fn test_simd_capabilities() {
        let simd = detect_simd();
        let features = simd.to_llvm_features();
        assert!(!features.is_empty(), "at least baseline features");
        let best = simd.best_feature_string();
        assert!(!best.is_empty());
    }

    #[test]
    fn test_simd_features_valid_format() {
        let simd = detect_simd();
        for f in simd.to_llvm_features() {
            assert!(f.starts_with('+'), "feature {} should start with +", f);
        }
    }
}
