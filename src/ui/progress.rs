use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(80);

static CURRENT: Mutex<Option<ProgressBar>> = Mutex::new(None);

pub struct Task {
    bar: ProgressBar,
    subject: String,
    start: Instant,
}

impl Task {
    pub fn start(action: &str, subject: &str, steps: u64) -> Self {
        Self::on(ProgressDrawTarget::stderr(), action, subject, steps)
    }

    fn on(target: ProgressDrawTarget, action: &str, subject: &str, steps: u64) -> Self {
        let bar = ProgressBar::with_draw_target(Some(steps), target);
        bar.set_style(task_style(steps));
        bar.set_prefix(format!("{} {}", action, subject).trim_end().to_string());
        bar.enable_steady_tick(TICK);
        set_current(Some(bar.clone()));
        Self { bar, subject: subject.to_string(), start: Instant::now() }
    }

    pub fn step(&self, label: &str) {
        if !self.bar.message().is_empty() {
            self.bar.inc(1);
        }
        self.bar.set_message(label.to_string());
    }

    pub fn finish(self, done: &str) {
        let line = self.summary(done);
        drop(self);
        anstream::eprintln!("{}", super::status_line(&line));
    }

    fn summary(&self, done: &str) -> String {
        let subject = if self.subject.is_empty() { String::new() } else { format!(" {}", self.subject) };
        format!("{}{} in {}", done, subject, super::elapsed(self.start.elapsed()))
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.bar.finish_and_clear();
        set_current(None);
    }
}

pub struct Download {
    bar: ProgressBar,
}

impl Download {
    pub fn start(label: &str, total: Option<u64>) -> Self {
        Self::on(ProgressDrawTarget::stderr(), label, total)
    }

    fn on(target: ProgressDrawTarget, label: &str, total: Option<u64>) -> Self {
        let bar = ProgressBar::with_draw_target(total, target);
        bar.set_style(download_style(total.is_some()));
        bar.set_message(label.to_string());
        bar.enable_steady_tick(TICK);
        Self { bar }
    }

    pub fn advance(&self, bytes: u64) {
        self.bar.inc(bytes);
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.bar.finish_and_clear();
    }
}

pub fn note(message: &str) {
    if let Some(bar) = CURRENT.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        bar.set_message(message.to_string());
    }
}

fn set_current(bar: Option<ProgressBar>) {
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = bar;
}

fn task_style(steps: u64) -> ProgressStyle {
    let template = if steps > 1 {
        "{spinner:.white} {prefix:.bold} {bar:30.green/black.dim} {pos}/{len} {msg:.dim}"
    } else {
        "{spinner:.white} {prefix:.bold} {msg:.dim}"
    };
    style(template)
}

fn download_style(sized: bool) -> ProgressStyle {
    let template = if sized {
        "{msg:.dim} {bar:30.green/black.dim} {binary_bytes:>7}/{binary_total_bytes:7}"
    } else {
        "{spinner:.white} {msg:.dim} {binary_bytes:>7}"
    };
    style(template)
}

fn style(template: &str) -> ProgressStyle {
    ProgressStyle::with_template(template).unwrap_or_else(|_| ProgressStyle::default_spinner()).progress_chars("--")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_step_advances_the_bar_by_one() {
        let task = Task::on(ProgressDrawTarget::hidden(), "Building", "fib.yk", 3);
        task.step("Parsing");
        assert_eq!(task.bar.position(), 0);
        task.step("Checking types");
        task.step("Compiling");
        assert_eq!(task.bar.position(), 2);
        assert_eq!(task.bar.message(), "Compiling");
        assert_eq!(task.bar.prefix(), "Building fib.yk");
    }

    #[test]
    fn the_summary_names_the_subject_and_the_time() {
        let task = Task::on(ProgressDrawTarget::hidden(), "Building", "fib.yk", 3);
        assert!(task.summary("Built").starts_with("Built fib.yk in "));
        let spinner = Task::on(ProgressDrawTarget::hidden(), "Resolving", "", 1);
        assert!(spinner.summary("Resolved").starts_with("Resolved in "));
        assert_eq!(spinner.bar.prefix(), "Resolving");
    }

    #[test]
    fn a_note_updates_the_running_task() {
        let task = Task::on(ProgressDrawTarget::hidden(), "Adding", "greet", 1);
        note("Fetching github.com/ada/greet");
        assert_eq!(task.bar.message(), "Fetching github.com/ada/greet");
        drop(task);
        note("ignored when nothing runs");
    }

    #[test]
    fn a_download_counts_its_bytes() {
        let download = Download::on(ProgressDrawTarget::hidden(), "yidi-x86_64.tar.gz", Some(100));
        download.advance(40);
        download.advance(60);
        assert_eq!(download.bar.position(), 100);
        assert_eq!(download.bar.length(), Some(100));
    }
}
