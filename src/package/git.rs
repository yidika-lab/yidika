use std::path::Path;
use std::process::Command;

pub fn clone(url: &str, target: &Path) -> Result<(), String> {
    run(Command::new("git").args(["clone", "--quiet", "--depth", "1", "--", url]).arg(target), "git clone")
}

pub fn update(target: &Path) -> Result<(), String> {
    run(Command::new("git").arg("-C").arg(target).args(["pull", "--quiet", "--ff-only"]), "git pull")
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let output = cmd.output().map_err(|e| format!("cannot run git ({}): is git installed?", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("{} failed: {}", what, String::from_utf8_lossy(&output.stderr).trim()))
    }
}
