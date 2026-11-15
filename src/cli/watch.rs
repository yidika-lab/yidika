use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::{Event, EventKind, RecursiveMode, Watcher};

use super::Engine;
use crate::codegen::backend::CodegenBackend;

const SETTLE: Duration = Duration::from_millis(15);
const SOURCE_EXTENSION: &str = "yk";

pub(super) struct Changes {
    watcher: notify::RecommendedWatcher,
    events: Receiver<notify::Result<Event>>,
    watched: Vec<PathBuf>,
}

impl Changes {
    pub fn new() -> Result<Self, String> {
        let (tx, events) = channel();
        let watcher = notify::recommended_watcher(tx).map_err(|e| format!("Cannot watch files: {}", e))?;
        Ok(Self { watcher, events, watched: Vec::new() })
    }

    pub fn follow(&mut self, files: &[PathBuf]) {
        for dir in files.iter().filter_map(|f| f.parent()) {
            if self.watched.iter().any(|w| w == dir) {
                continue;
            }
            if self.watcher.watch(dir, RecursiveMode::NonRecursive).is_ok() {
                self.watched.push(dir.to_path_buf());
            }
        }
    }

    pub fn wait(&self) -> bool {
        loop {
            match self.events.recv() {
                Ok(Ok(event)) if is_source_change(&event) => break,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
        while self.events.recv_timeout(SETTLE).is_ok() {}
        true
    }
}

fn is_source_change(event: &Event) -> bool {
    matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_))
        && event.paths.iter().any(|p| p.extension().is_some_and(|e| e == SOURCE_EXTENSION))
}

fn entry_and(files: Vec<PathBuf>, entry: &Path) -> Vec<PathBuf> {
    let mut all = files;
    all.push(std::fs::canonicalize(entry).unwrap_or_else(|_| entry.to_path_buf()));
    all
}

type Running = Arc<Mutex<Option<Child>>>;

const INTERRUPTED_EXIT_CODE: i32 = 130;

pub(super) fn run(path: &PathBuf, engine: Engine) -> Result<String, String> {
    let mut changes = Changes::new()?;
    let running: Running = Arc::default();
    stop_child_on_exit(running.clone());
    loop {
        let child = start(path, engine, &mut changes);
        *running.lock().unwrap_or_else(|e| e.into_inner()) = child;
        let changed = changes.wait();
        stop(&running);
        if !changed {
            return Ok(String::new());
        }
    }
}

fn start(path: &PathBuf, engine: Engine, changes: &mut Changes) -> Option<Child> {
    match super::prepare_launch(path, engine) {
        Ok(mut launch) => {
            changes.follow(&entry_and(launch.sources, path));
            launch.command.spawn().map_err(|e| eprintln!("{}", e)).ok()
        }
        Err(e) => {
            eprintln!("{}", e);
            changes.follow(&entry_and(Vec::new(), path));
            None
        }
    }
}

fn stop(running: &Running) {
    if let Some(mut child) = running.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn stop_child_on_exit(running: Running) {
    let _ = ctrlc::set_handler(move || {
        stop(&running);
        std::process::exit(INTERRUPTED_EXIT_CODE);
    });
}

pub(super) fn rebuild(path: &PathBuf, backend: &dyn CodegenBackend, show_info: bool) -> Result<String, String> {
    let mut changes = Changes::new()?;
    changes.follow(&entry_and(Vec::new(), path));
    while changes.wait() {
        if let Err(e) = super::build_program(path, false, backend, show_info) {
            eprintln!("{}", e);
        }
    }
    Ok(String::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yidi_watch_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(dir).unwrap()
    }

    #[test]
    fn wakes_up_when_a_source_file_changes() {
        let dir = temp_dir("source");
        let file = dir.join("main.yk");
        std::fs::write(&file, "fn main() {}").unwrap();
        let mut changes = Changes::new().unwrap();
        changes.follow(std::slice::from_ref(&file));
        let writer = file.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            std::fs::write(writer, "fn main() { print(1); }").unwrap();
        });
        let start = Instant::now();
        assert!(changes.wait());
        assert!(start.elapsed() < Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignores_files_that_are_not_sources() {
        let event = |name: &str| Event::new(EventKind::Modify(notify::event::ModifyKind::Any)).add_path(PathBuf::from(name));
        assert!(is_source_change(&event("/p/main.yk")));
        assert!(!is_source_change(&event("/p/main.ll")));
        assert!(!is_source_change(&event("/p/main")));
        assert!(!is_source_change(&Event::new(EventKind::Access(notify::event::AccessKind::Any)).add_path(PathBuf::from("/p/main.yk"))));
    }

    #[test]
    fn follows_each_directory_once() {
        let dir = temp_dir("dirs");
        let mut changes = Changes::new().unwrap();
        changes.follow(&[dir.join("a.yk"), dir.join("b.yk")]);
        changes.follow(&[dir.join("c.yk")]);
        assert_eq!(changes.watched, vec![dir.clone()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
