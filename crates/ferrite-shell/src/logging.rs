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
    install_panic_hook();
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

/// Hands every panic to the DevTools view (`ferrite_servo::diag`) before the
/// default hook prints it with its backtrace.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_string());
        let location = info.location().map_or_else(String::new, |l| {
            format!("{}:{}:{}", l.file(), l.line(), l.column())
        });
        ferrite_servo::diag::record_panic(thread.name().unwrap_or("unnamed"), &message, &location);
        default(info);
    }));
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

/// Windows has no `dup2` on handles: standard error is a per-process handle,
/// which Rust's `stderr` looks up on every write, so pointing it at the file
/// catches the app's own messages and panics. The engine's C libraries
/// (SpiderMonkey, GStreamer) write to the C runtime's descriptor 2 instead,
/// which is pointed at the same file where the runtime allows it.
#[cfg(windows)]
fn redirect_stderr(file: &File) -> Option<()> {
    use std::os::windows::io::AsRawHandle;
    unsafe extern "system" {
        fn SetStdHandle(which: u32, handle: *mut std::ffi::c_void) -> i32;
    }
    unsafe extern "C" {
        fn _open_osfhandle(handle: isize, flags: i32) -> i32;
        fn _dup2(from: i32, to: i32) -> i32;
    }
    const STD_ERROR_HANDLE: u32 = -12_i32 as u32;
    let handle = file.as_raw_handle();
    // SAFETY: `handle` is the open file's, and it stays open for the process.
    if unsafe { SetStdHandle(STD_ERROR_HANDLE, handle) } == 0 {
        return None;
    }
    // SAFETY: as above; the descriptor is never closed, so the handle it
    // wraps is never closed under the file.
    let fd = unsafe { _open_osfhandle(handle as isize, 0) };
    if fd >= 0 {
        unsafe { _dup2(fd, 2) };
    }
    Some(())
}

#[cfg(not(any(unix, windows)))]
fn redirect_stderr(_file: &File) -> Option<()> {
    None
}

/// Windows: the app is a GUI program, so a double-click opens no console window beside
/// it, and its standard error, which then goes nowhere, is sent to `ferrite.log` by
/// `init`. A command started from a console (`ferrite.exe smoke`) should still print
/// there: this attaches to the parent's console and points standard output and error at
/// it, unless they already lead somewhere (a pipe or a file, as on CI). Call it before
/// `init`. Returns whether it attached.
#[cfg(windows)]
pub fn attach_parent_console() -> bool {
    use std::ffi::c_void;
    unsafe extern "system" {
        fn AttachConsole(process: u32) -> i32;
        fn GetStdHandle(which: u32) -> *mut c_void;
        fn SetStdHandle(which: u32, handle: *mut c_void) -> i32;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: *mut c_void,
        ) -> *mut c_void;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_OUTPUT_HANDLE: u32 = -11_i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12_i32 as u32;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    const OPEN_EXISTING: u32 = 3;
    let unset = |which| {
        // SAFETY: reading this process's own standard handle.
        let handle = unsafe { GetStdHandle(which) };
        handle.is_null() || handle as isize == -1
    };
    if !unset(STD_OUTPUT_HANDLE) && !unset(STD_ERROR_HANDLE) {
        return false;
    }
    // SAFETY: plain Win32 calls with valid arguments; the console handle is kept open
    // for the life of the process.
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return false;
        }
        let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        let console = CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if console.is_null() || console as isize == -1 {
            return false;
        }
        for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            if unset(which) {
                SetStdHandle(which, console);
            }
        }
    }
    true
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

    /// Windows only (the CI Windows job runs it): what is written to standard
    /// error after the redirect lands in the file. Standard error is put back
    /// afterwards so the test harness keeps its own.
    #[cfg(windows)]
    #[test]
    fn standard_error_reaches_the_log_file_on_windows() {
        unsafe extern "system" {
            fn GetStdHandle(which: u32) -> *mut std::ffi::c_void;
            fn SetStdHandle(which: u32, handle: *mut std::ffi::c_void) -> i32;
        }
        const STD_ERROR_HANDLE: u32 = -12_i32 as u32;
        let path = std::env::temp_dir().join(format!("ferrite-log-{}.log", std::process::id()));
        let file = File::create(&path).unwrap();
        // SAFETY: reading and restoring this process's own standard handle.
        let saved = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
        assert!(redirect_stderr(&file).is_some());
        // `io::stderr()` directly: the test harness captures `eprintln!`.
        std::io::stderr()
            .write_all(b"ferrite-log-marker\n")
            .unwrap();
        unsafe { SetStdHandle(STD_ERROR_HANDLE, saved) };
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("ferrite-log-marker"), "{written:?}");
        std::mem::forget(file);
    }
}
