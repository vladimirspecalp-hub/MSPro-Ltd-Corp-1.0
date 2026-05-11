//! mspro-rollback-helper.exe
//!
//! Out-of-process swap utility for MSPro-Ltd Corp version rollback.
//!
//! Usage:
//!   mspro-rollback-helper --target <PATH> --source <PATH> --pid <NUM>
//!
//! Behavior:
//!   1. Wait for the parent process (`--pid`) to exit, up to 10 s.
//!   2. Retry copying `<source>` over `<target>` every 500 ms, up to 20 times
//!      (10 s total). Windows can hold a file handle on a closing `.exe`
//!      longer than expected — the retry loop is the production-grade fix
//!      for risk R3 in the Step 2 plan.
//!   3. On success — spawn the (now-restored) target exe and exit 0.
//!   4. On timeout — write a diagnostic log to
//!      `%LOCALAPPDATA%\ru.msproltd.corp\rollback-error.log` and exit 2.
//!
//! Exit codes:
//!   0 — swap + restart succeeded
//!   1 — bad CLI arguments
//!   2 — file lock could not be released within 10 s (write error log)
//!   3 — target exe spawn failed after successful swap (rare)

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::thread::sleep;
use std::time::{Duration, Instant};

const SWAP_RETRY_COUNT: u32 = 20;
const SWAP_RETRY_INTERVAL: Duration = Duration::from_millis(500);
const PID_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
struct Args {
    target: PathBuf,
    source: PathBuf,
    pid: u32,
}

fn parse_args() -> Result<Args, String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut target: Option<PathBuf> = None;
    let mut source: Option<PathBuf> = None;
    let mut pid: Option<u32> = None;
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--target" => {
                target = Some(PathBuf::from(raw.get(i + 1).ok_or("--target needs a value")?));
                i += 2;
            }
            "--source" => {
                source = Some(PathBuf::from(raw.get(i + 1).ok_or("--source needs a value")?));
                i += 2;
            }
            "--pid" => {
                pid = Some(
                    raw.get(i + 1)
                        .ok_or("--pid needs a value")?
                        .parse()
                        .map_err(|e| format!("--pid invalid: {e}"))?,
                );
                i += 2;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Args {
        target: target.ok_or("--target is required")?,
        source: source.ok_or("--source is required")?,
        pid: pid.ok_or("--pid is required")?,
    })
}

#[cfg(windows)]
fn wait_for_pid_exit(pid: u32, timeout: Duration) {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            // Process is already gone (or no permission). Proceed immediately.
            return;
        }
        let _ = WaitForSingleObject(handle, timeout.as_millis() as u32);
        let _ = CloseHandle(handle);
    }
    // Even after WaitForSingleObject returns, Windows can still hold a
    // mandatory file lock on the exe for a short tail. The retry loop in
    // try_swap() is what guarantees we eventually win.
    let _ = WAIT_OBJECT_0;
}

#[cfg(not(windows))]
fn wait_for_pid_exit(_pid: u32, timeout: Duration) {
    // Best-effort fallback — sleep the full timeout. We are Windows-only in production.
    sleep(timeout);
}

fn try_swap(source: &Path, target: &Path) -> std::io::Result<()> {
    let mut last_err: Option<std::io::Error> = None;
    let started = Instant::now();
    for attempt in 0..SWAP_RETRY_COUNT {
        match std::fs::copy(source, target) {
            Ok(_) => {
                eprintln!(
                    "[rollback-helper] swap succeeded on attempt {} ({} ms elapsed)",
                    attempt + 1,
                    started.elapsed().as_millis()
                );
                return Ok(());
            }
            Err(e) => {
                last_err = Some(e);
                sleep(SWAP_RETRY_INTERVAL);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("retry loop exhausted")))
}

fn write_error_log(message: &str) {
    let log_dir = match std::env::var("LOCALAPPDATA") {
        Ok(v) => PathBuf::from(v).join("ru.msproltd.corp"),
        Err(_) => return,
    };
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("rollback-error.log");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("[{timestamp}] {message}\n");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[rollback-helper] bad args: {e}");
            return ExitCode::from(1);
        }
    };

    eprintln!(
        "[rollback-helper] waiting for pid {} to exit (timeout {:?})…",
        args.pid, PID_WAIT_TIMEOUT
    );
    wait_for_pid_exit(args.pid, PID_WAIT_TIMEOUT);

    eprintln!(
        "[rollback-helper] swapping {} → {}",
        args.source.display(),
        args.target.display()
    );
    if let Err(e) = try_swap(&args.source, &args.target) {
        let msg = format!(
            "swap failed after {} attempts ({:?} interval): {e:?} — \
             source={}, target={}",
            SWAP_RETRY_COUNT,
            SWAP_RETRY_INTERVAL,
            args.source.display(),
            args.target.display()
        );
        eprintln!("[rollback-helper] ERROR: {msg}");
        write_error_log(&msg);
        return ExitCode::from(2);
    }

    match Command::new(&args.target).spawn() {
        Ok(_) => {
            eprintln!(
                "[rollback-helper] spawned restored exe: {}",
                args.target.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            let msg = format!("spawn restored exe failed: {e:?}");
            eprintln!("[rollback-helper] ERROR: {msg}");
            write_error_log(&msg);
            ExitCode::from(3)
        }
    }
}
