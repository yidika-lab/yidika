use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

pub mod virtual_task;

pub use virtual_task::{VirtualScheduler, VirtualTask, global_virtual_scheduler, global_virtual_scheduler_with_hw};

pub type TaskResult = Result<(), String>;

pub enum TaskMessage {
    Execute(Box<dyn FnOnce() -> TaskResult + Send + 'static>),
    Shutdown,
}

pub struct Task {
    pub id: u64,
    receiver: Receiver<TaskResult>,
    handle: Option<JoinHandle<()>>,
}

const TASK_STACK: usize = 64 * 1024;

impl Task {
    pub fn new<F>(id: u64, f: F) -> Self
    where
        F: FnOnce() -> TaskResult + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name(format!("yk-task-{}", id))
            .stack_size(TASK_STACK)
            .spawn(move || {
                let result = f();
                let _ = tx.send(result);
            })
            .ok();

        Task { id, receiver: rx, handle }
    }

    pub fn join(self) -> TaskResult {
        if let Some(h) = self.handle {
            let _ = h.join();
        }
        self.receiver.recv().unwrap_or(Ok(()))
    }
}

pub struct Scheduler {
    workers: Vec<JoinHandle<()>>,
    sender: Sender<TaskMessage>,
    next_id: AtomicU64,
}

impl Scheduler {
    pub fn new(num_workers: usize) -> Self {
        let (tx, rx) = mpsc::channel();
        let rx = Arc::new(std::sync::Mutex::new(rx));
        let mut workers = Vec::new();

        for _ in 0..num_workers {
            let rx = rx.clone();
            let handle = thread::Builder::new()
                .name("yk-worker".into())
                .stack_size(TASK_STACK)
                .spawn(move || loop {
                    let msg = { rx.lock().unwrap().recv() };
                    match msg {
                        Ok(TaskMessage::Execute(task)) => { let _ = task(); }
                        Ok(TaskMessage::Shutdown) | Err(_) => break,
                    }
                })
                .unwrap();
            workers.push(handle);
        }

        Scheduler { workers, sender: tx, next_id: AtomicU64::new(0) }
    }

    pub fn spawn<F>(&self, f: F) -> Task
    where
        F: FnOnce() -> TaskResult + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let handle = thread::Builder::new()
            .name(format!("yk-task-{}", id))
            .stack_size(TASK_STACK)
            .spawn(move || {
                let result = f();
                let _ = tx.send(result);
            })
            .ok();

        Task { id, receiver: rx, handle }
    }

    pub fn spawn_on_scheduler<F>(&self, f: F)
    where
        F: FnOnce() -> TaskResult + Send + 'static,
    {
        let _ = self.sender.send(TaskMessage::Execute(Box::new(f)));
    }

    pub fn current_id(&self) -> u64 {
        self.next_id.load(Ordering::Relaxed)
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        for _ in &self.workers {
            let _ = self.sender.send(TaskMessage::Shutdown);
        }
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

pub fn current_thread_pool_size() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn test_scheduler_spawn() {
        let sched = Scheduler::new(2);
        let ran = Arc::new(AtomicBool::new(false));
        let r = ran.clone();
        sched.spawn_on_scheduler(Box::new(move || {
            r.store(true, Ordering::Relaxed);
            Ok(())
        }));
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(ran.load(Ordering::SeqCst));
    }

    #[test]
    fn test_task_spawn_and_join() {
        let task = Task::new(1, || {
            let _ = 2 + 2;
            Ok(())
        });
        assert!(task.join().is_ok());
    }

    #[test]
    fn test_scheduler_default_pool() {
        let n = current_thread_pool_size();
        assert!(n >= 1);
    }

    #[test]
    fn test_scheduler_drop_no_panic() {
        let sched = Scheduler::new(1);
        drop(sched);
    }

    #[test]
    fn test_virtual_scheduler_global() {
        let vs = global_virtual_scheduler();
        let rx = vs.spawn(|| Ok(()));
        assert!(rx.recv().unwrap().is_ok());
    }
}
