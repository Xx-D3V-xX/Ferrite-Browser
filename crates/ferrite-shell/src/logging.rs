//! A log file for runs that have no terminal.
//!
//! Launched from Finder (or a Start menu, or a launcher) the app's standard
//! error goes nowhere, so a crash or a page that stops responding leaves
//! nothing to look at. `init` points standard error at a file the person can
//! send: `~/Library/Logs/Ferrite/ferrite.log` on macOS (the folder Console.app
//! shows under *Log Reports*), with the previous run kept beside it as
//! `ferrite.previous.log`. A run from a terminal keeps printing there.
//!
//! Panics always carry a backtrace, since the engine's own threads panic
//! quietly and the page just stops answering.

use std::fs::{self, File, OpenOptions};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;

/// Where the log lives on this platform, or `None` if no home can be found.
pub fn log_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library").join("Logs").join("Ferrite"))
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Ferrite").join("logs"))
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".local").join("state")))
            .map(|d| d.join("ferrite"))
    }
}

/// Starts logging to a file when standard error is not a terminal. Returns the
/// log's path when it did. Never fails the app: no log is better than no app.
pub fn init() -> Option<PathBuf> {
    // SAFETY: called first thing in `main`, before any thread exists.
    unsafe {
        if std::env::var_os("RUST_BACKTRACE").is_none() {
            std::env::set_var("RUST_BACKTRACE", "1");
        }
    }
    if std::io::stderr().is_terminal() {
        return None;
    }
    let dir = log_dir()?;
    fs::create_dir_all(&dir).ok()?;
    let current = dir.join("ferrite.log");
    let previous = dir.join("ferrite.previous.log");
    if current.exists() {
        let _ = fs::rename(&current, &previous);
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&current)
        .ok()?;
    redirect_stderr(&file)?;
    // Keep the descriptor open for the life of the process.
    std::mem::forget(file);
    banner(&current);
    Some(current)
}

fn banner(path: &std::path::Path) {
    let mut err = std::io::stderr();
    let _ = writeln!(
        err,
        "[ferrite] {} {} on {} {} (pid {}), logging to {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::process::id(),
        path.display()
    );
}

#[cfg(unix)]
fn redirect_stderr(file: &File) -> Option<()> {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn dup2(old: i32, new: i32) -> i32;
    }
    // SAFETY: `file` is open; descriptor 2 is standard error.
    (unsafe { dup2(file.as_raw_fd(), 2) } >= 0).then_some(())
}

#[cfg(not(unix))]
fn redirect_stderr(_file: &File) -> Option<()> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_folder_is_the_one_each_platform_looks_in() {
        if let Some(dir) = log_dir() {
            let s = dir.to_string_lossy().into_owned();
            if cfg!(target_os = "macos") {
                assert!(s.ends_with("Library/Logs/Ferrite"), "{s}");
            } else if !cfg!(windows) {
                assert!(s.ends_with("ferrite"), "{s}");
            }
        }
    }
}
