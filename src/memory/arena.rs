use std::alloc::{alloc, dealloc, Layout};
use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::{Mutex, OnceLock};

use crate::interpret::Value;

const ARENA_BLOCK_SIZE: usize = 64 * 1024;
pub const POOL_MAX_LISTS: usize = 256;
pub const POOL_MAX_MAPS: usize = 128;

struct ArenaBlock {
    data: NonNull<u8>,
    capacity: usize,
    offset: usize,
}

impl ArenaBlock {
    fn new(capacity: usize) -> Self {
        let layout = Layout::from_size_align(capacity, 16).unwrap();
        let data = NonNull::new(unsafe { alloc(layout) })
            .unwrap_or_else(|| std::process::abort());
        ArenaBlock { data, capacity, offset: 0 }
    }

    fn can_fit(&self, size: usize) -> bool {
        self.offset + size <= self.capacity
    }

    fn alloc(&mut self, size: usize) -> *mut u8 {
        let ptr = unsafe { self.data.as_ptr().add(self.offset) };
        self.offset += size;
        ptr
    }

    fn reset(&mut self) {
        self.offset = 0;
    }
}

impl Drop for ArenaBlock {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.capacity, 16).unwrap();
        unsafe { dealloc(self.data.as_ptr(), layout); }
    }
}

pub struct Arena {
    blocks: Vec<ArenaBlock>,
    current_block: usize,
    total_allocated: usize,
}

impl Arena {
    pub fn new() -> Self {
        Arena {
            blocks: vec![ArenaBlock::new(ARENA_BLOCK_SIZE)],
            current_block: 0,
            total_allocated: 0,
        }
    }

    pub fn with_block_size(block_size: usize) -> Self {
        let size = block_size.max(4096);
        Arena {
            blocks: vec![ArenaBlock::new(size)],
            current_block: 0,
            total_allocated: 0,
        }
    }

    pub fn with_hardware_info(cache_l1: Option<u64>, cache_l2: Option<u64>) -> Self {
        let size = cache_l1
            .or(cache_l2)
            .map(|l2| (l2 as usize / 2).max(ARENA_BLOCK_SIZE))
            .unwrap_or(ARENA_BLOCK_SIZE);
        Arena {
            blocks: vec![ArenaBlock::new(size)],
            current_block: 0,
            total_allocated: 0,
        }
    }

    pub fn alloc(&mut self, size: usize) -> *mut u8 {
        self.total_allocated += size;
        if self.blocks[self.current_block].can_fit(size) {
            return self.blocks[self.current_block].alloc(size);
        }
        let block_size = size.max(ARENA_BLOCK_SIZE);
        let mut new_block = ArenaBlock::new(block_size);
        let ptr = new_block.alloc(size);
        self.blocks.push(new_block);
        self.current_block = self.blocks.len() - 1;
        ptr
    }

    pub fn alloc_zeroed(&mut self, size: usize) -> *mut u8 {
        let ptr = self.alloc(size);
        unsafe { std::ptr::write_bytes(ptr, 0, size); }
        ptr
    }

    pub fn alloc_values(&mut self, values: &[Value]) -> &mut [Value] {
        let size = values.len() * std::mem::size_of::<Value>();
        let ptr = self.alloc(size) as *mut Value;
        unsafe {
            std::ptr::copy_nonoverlapping(values.as_ptr(), ptr, values.len());
            std::slice::from_raw_parts_mut(ptr, values.len())
        }
    }

    pub fn alloc_value_slice(&mut self, n: usize) -> &mut [Value] {
        let size = n * std::mem::size_of::<Value>();
        let ptr = self.alloc_zeroed(size) as *mut Value;
        unsafe { std::slice::from_raw_parts_mut(ptr, n) }
    }

    pub fn reset(&mut self) {
        for block in &mut self.blocks {
            block.reset();
        }
        self.current_block = 0;
        self.total_allocated = 0;
    }

    pub fn total_allocated(&self) -> usize {
        self.total_allocated
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }
}

impl Drop for Arena {
    fn drop(&mut self) {}
}

unsafe impl Send for Arena {}
unsafe impl Sync for Arena {}

pub struct ValuePool {
    lists: Vec<Vec<Value>>,
    maps: Vec<HashMap<String, Value>>,
}

impl ValuePool {
    pub fn new() -> Self {
        ValuePool { lists: Vec::new(), maps: Vec::new() }
    }

    pub fn take_list(&mut self) -> Vec<Value> {
        self.lists.pop().unwrap_or_default()
    }

    pub fn take_list_with_capacity(&mut self, cap: usize) -> Vec<Value> {
        self.lists.pop().map(|mut v| { v.reserve(cap.saturating_sub(v.len())); v }).unwrap_or_else(|| Vec::with_capacity(cap))
    }

    pub fn return_list(&mut self, mut list: Vec<Value>) {
        list.clear();
        if self.lists.len() < POOL_MAX_LISTS {
            self.lists.push(list);
        }
    }

    pub fn take_map(&mut self) -> HashMap<String, Value> {
        self.maps.pop().unwrap_or_default()
    }

    pub fn return_map(&mut self, mut map: HashMap<String, Value>) {
        map.clear();
        if self.maps.len() < POOL_MAX_MAPS {
            self.maps.push(map);
        }
    }

    pub fn reset(&mut self) {
        self.lists.clear();
        self.maps.clear();
    }
}

pub fn global_value_pool() -> &'static Mutex<ValuePool> {
    static POOL: OnceLock<Mutex<ValuePool>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(ValuePool::new()))
}

pub fn recommended_arena_block_size(cache_l1: Option<u64>, cache_l2: Option<u64>) -> usize {
    cache_l1
        .or(cache_l2)
        .map(|l2| (l2 as usize / 2).max(ARENA_BLOCK_SIZE))
        .unwrap_or(ARENA_BLOCK_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arena_alloc() {
        let mut arena = Arena::new();
        let ptr = arena.alloc(64);
        assert!(!ptr.is_null());
    }

    #[test]
    fn test_arena_with_hardware_info() {
        let arena = Arena::with_hardware_info(Some(32768), Some(262144));
        assert!(arena.block_count() >= 1);
    }

    #[test]
    fn test_recommended_block_size() {
        let size = recommended_arena_block_size(Some(32768), Some(262144));
        assert!(size >= ARENA_BLOCK_SIZE);
    }

    #[test]
    fn test_arena_alloc_zeroed() {
        let mut arena = Arena::new();
        let ptr = arena.alloc_zeroed(32);
        unsafe {
            for i in 0..32 {
                assert_eq!(*ptr.add(i), 0);
            }
        }
    }

    #[test]
    fn test_arena_multiple_blocks() {
        let mut arena = Arena::with_block_size(128);
        for _ in 0..100 {
            let ptr = arena.alloc(64);
            assert!(!ptr.is_null());
        }
        assert!(arena.total_allocated >= 100 * 64);
    }

    #[test]
    fn test_arena_reset() {
        let mut arena = Arena::new();
        arena.alloc(100);
        assert!(arena.total_allocated >= 100);
        arena.reset();
        assert_eq!(arena.total_allocated, 0);
        let ptr = arena.alloc(100);
        assert!(!ptr.is_null());
    }

    #[test]
    fn test_arena_alloc_values() {
        let mut arena = Arena::new();
        let vals = arena.alloc_values(&[Value::Int(1), Value::Int(2)]);
        assert_eq!(vals.len(), 2);
    }

    #[test]
    fn test_value_pool() {
        let mut pool = ValuePool::new();
        let list = pool.take_list();
        pool.return_list(list);
        let list2 = pool.take_list();
        assert!(list2.is_empty());
    }

    #[test]
    fn test_global_value_pool() {
        let p1 = global_value_pool();
        let p2 = global_value_pool();
        assert_eq!(p1 as *const _, p2 as *const _);
    }
}
