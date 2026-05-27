pub mod arena;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MemoryRegion {
    Static,
    Stack,
    Arena,
    Device,
}

impl std::fmt::Display for MemoryRegion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryRegion::Static => write!(f, "static"),
            MemoryRegion::Stack => write!(f, "stack"),
            MemoryRegion::Arena => write!(f, "arena"),
            MemoryRegion::Device => write!(f, "device"),
        }
    }
}

impl MemoryRegion {
    pub fn from_annotation(name: &str) -> Option<Self> {
        match name {
            "static" => Some(MemoryRegion::Static),
            "arena" => Some(MemoryRegion::Arena),
            "device" => Some(MemoryRegion::Device),
            _ => None,
        }
    }
}

/// Memory usage statistics
#[derive(Debug, Clone)]
pub struct MemoryStats {
    pub stack_bytes: usize,
    pub arena_blocks: usize,
    pub arena_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_region_from_annotation() {
        assert_eq!(MemoryRegion::from_annotation("static"), Some(MemoryRegion::Static));
        assert_eq!(MemoryRegion::from_annotation("arena"), Some(MemoryRegion::Arena));
        assert_eq!(MemoryRegion::from_annotation("device"), Some(MemoryRegion::Device));
        assert_eq!(MemoryRegion::from_annotation("unknown"), None);
    }

    #[test]
    fn test_arena_integration() {
        let mut ar = arena::Arena::new();
        ar.alloc(128);
        assert!(ar.total_allocated() >= 128);
        assert!(ar.block_count() >= 1);
    }
}
