use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crate::hardware::HardwareInfo;

pub type TaskResult = Result<(), String>;
type TaskClosure = Box<dyn FnOnce() -> TaskResult + Send + 'static>;

pub struct VirtualTask {
    pub id: u64,
    closure: Option<TaskClosure>,
    result_sender: Option<Sender<TaskResult>>,
}

impl VirtualTask {
    pub fn new(id: u64, f: TaskClosure) -> (Self, Receiver<TaskResult>) {
        let (tx, rx) = mpsc::channel();
        (
            VirtualTask {
                id,
                closure: Some(f),
                result_sender: Some(tx),
            },
            rx,
        )
    }

    pub fn execute(&mut self) {
        if let Some(f) = self.closure.take() {
            let result = f();
            if let Some(ref tx) = self.result_sender {
                let _ = tx.send(result);
            }
        }
    }
}

unsafe impl Send for VirtualTask {}

#[cfg(target_os = "windows")]
extern "system" {
    fn GetCurrentThread() -> isize;
    fn SetThreadAffinityMask(hThread: isize, dwThreadAffinityMask: usize) -> usize;
}

fn pin_thread_to_core(core_id: usize) {
    #[cfg(target_os = "windows")]
    unsafe {
        let handle = GetCurrentThread();
        SetThreadAffinityMask(handle, 1usize << core_id);
    }
}

const WORKER_STACK_SIZE: usize = 512 * 1024;

pub struct VirtualScheduler {
    workers: Vec<JoinHandle<()>>,
    worker_threads: Vec<thread::Thread>,
    shared: Arc<crossbeam_deque::Injector<VirtualTask>>,
    shutdown: Arc<AtomicBool>,
    next_id: AtomicU64,
    next_worker: AtomicU64,
}

impl VirtualScheduler {
    pub fn new(num_workers: usize) -> Self {
        let shared: Arc<crossbeam_deque::Injector<VirtualTask>> = Arc::new(crossbeam_deque::Injector::new());
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_threads = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut workers = Vec::new();

        for core in 0..num_workers {
            let local: crossbeam_deque::Worker<VirtualTask> = crossbeam_deque::Worker::new_fifo();
            let shared = shared.clone();
            let shutdown = shutdown.clone();
            let wt = worker_threads.clone();

            let handle = thread::Builder::new()
                .name(format!("yk-vworker-{}", core))
                .stack_size(WORKER_STACK_SIZE)
                .spawn(move || {
                    pin_thread_to_core(core);
                    wt.lock().unwrap().push(thread::current());
                    drop(wt);
                    loop {
                        if shutdown.load(Ordering::Relaxed) {
                            break;
                        }
                        if let Some(mut task) = local.pop() {
                            task.execute();
                            continue;
                        }
                        match shared.steal_batch_and_pop(&local) {
                            crossbeam_deque::Steal::Success(mut task) => {
                                task.execute();
                                continue;
                            }
                            crossbeam_deque::Steal::Retry => {
                                continue;
                            }
                            crossbeam_deque::Steal::Empty => {}
                        }
                        thread::park();
                    }
                })
                .unwrap();
            workers.push(handle);
        }

        // Wait for all workers to register their thread handles.
        // This happens immediately after they start (before entering the park loop).
        let threads: Vec<thread::Thread>;
        loop {
            let len = worker_threads.lock().unwrap().len();
            if len >= num_workers {
                threads = worker_threads.lock().unwrap().drain(..).collect();
                break;
            }
            std::thread::yield_now();
        }

        VirtualScheduler {
            workers,
            worker_threads: threads,
            shared,
            shutdown,
            next_id: AtomicU64::new(0),
            next_worker: AtomicU64::new(0),
        }
    }

    pub fn with_hardware_info(hw: &HardwareInfo) -> Self {
        let n = hw.cpu.logical_cores.max(1) as usize;
        VirtualScheduler::new(n)
    }

    pub fn spawn<F>(&self, f: F) -> Receiver<TaskResult>
    where
        F: FnOnce() -> TaskResult + Send + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (task, rx) = VirtualTask::new(id, Box::new(f));
        self.shared.push(task);
        let n = self.worker_threads.len();
        if n > 0 {
            let idx = (self.next_worker.fetch_add(1, Ordering::Relaxed) % n as u64) as usize;
            self.worker_threads[idx].unpark();
        }
        rx
    }

    pub fn current_id(&self) -> u64 {
        self.next_id.load(Ordering::Relaxed)
    }
}

impl Drop for VirtualScheduler {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        for t in &self.worker_threads {
            t.unpark();
        }
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

pub fn global_virtual_scheduler() -> &'static VirtualScheduler {
    static VSCHED: std::sync::OnceLock<VirtualScheduler> = std::sync::OnceLock::new();
    VSCHED.get_or_init(|| {
        let n = crate::runtime::current_thread_pool_size();
        VirtualScheduler::new(n)
    })
}

pub fn global_virtual_scheduler_with_hw(hw: &HardwareInfo) -> &'static VirtualScheduler {
    static VSCHED_HW: std::sync::OnceLock<VirtualScheduler> = std::sync::OnceLock::new();
    VSCHED_HW.get_or_init(|| {
        let n = hw.cpu.logical_cores.max(1) as usize;
        VirtualScheduler::new(n)
    })
}
