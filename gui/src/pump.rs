use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::tree::{AUTO_EXIT_MS, QUIT};

pub fn deadline() -> Option<Instant> {
    let ms = AUTO_EXIT_MS.load(Ordering::SeqCst);
    if ms < 0 {
        None
    } else {
        Some(Instant::now() + Duration::from_millis(ms as u64))
    }
}

pub fn expired(end: Option<Instant>) -> bool {
    end.is_some_and(|e| Instant::now() >= e)
}

pub fn run_pump() -> i64 {
    let end = deadline();
    loop {
        if QUIT.load(Ordering::SeqCst) != 0 {
            break;
        }
        if expired(end) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    QUIT.store(0, Ordering::SeqCst);
    0
}
