//! Crash reports: a panic writes what happened (with the versions of the
//! game and of SDL) to ~/.snowballz_crash.log and shows a message box.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Set while a guarded game step runs: a panic there is logged (no dialog)
/// and the game carries on.
static GUARDED: AtomicBool = AtomicBool::new(false);
static GUARDED_ERRORS: AtomicU32 = AtomicU32::new(0);

pub fn set_guarded(on: bool) {
    GUARDED.store(on, Ordering::SeqCst);
}

/// How many errors guarded steps have had so far.
pub fn guarded_errors() -> u32 {
    GUARDED_ERRORS.load(Ordering::SeqCst)
}

pub fn log_path() -> String {
    let p = crate::settings::config_path();
    match p.strip_suffix(".ini") {
        Some(base) => format!("{}_crash.log", base),
        None => format!("{}_crash.log", p),
    }
}

/// The game's version, as in release names (the VERSION file; it may have
/// four numbers, like 0.9.8.1).
pub fn version() -> &'static str {
    include_str!("../VERSION").trim()
}

pub fn versions() -> String {
    let mut s = format!(
        "snowballz {} (rust), {} {}",
        version(),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if crate::sdl::is_loaded() {
        s.push_str(&format!(
            ", SDL {}, driver {}, renderer {}",
            crate::sdl::version_string(),
            crate::sdl::video_driver(),
            crate::gfx::renderer_name()
        ));
    }
    s
}

fn now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // UTC date from the unix time (no time zone database here)
    let days = secs / 86400;
    let (h, m, s) = ((secs % 86400) / 3600, (secs % 3600) / 60, secs % 60);
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if mo <= 2 { 1 } else { 0 };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC", y, mo, d, h, m, s)
}

/// Append a report to the log; returns the log path.
pub fn write(what: &str, where_: &str, show_box: bool) -> String {
    let report = format!(
        "=== {} {}===\n{}\n{}\n",
        now(),
        if where_.is_empty() { String::new() } else { format!("({}) ", where_) },
        versions(),
        what
    );
    let path = log_path();
    let shown = match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut f) => {
            let _ = f.write_all(report.as_bytes());
            path.clone()
        }
        Err(_) => "(could not write the log file)".to_string(),
    };
    eprint!("{}", report);
    eprintln!("\nA crash report was saved to: {}", shown);
    if show_box {
        let last = what.lines().next().unwrap_or("").to_string();
        crate::sdl::message_box("Snowballz crashed", &format!("{}\n\nA report was saved to:\n{}", last, shown));
    }
    shown
}

pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown error".to_string()
        };
        let loc = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        let bt = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let main = thread == "main";
        let guarded = main && GUARDED.load(Ordering::SeqCst);
        if guarded {
            // only the first few are worth a report
            if GUARDED_ERRORS.fetch_add(1, Ordering::SeqCst) >= 5 {
                return;
            }
        }
        write(
            &format!("panic{}: {}\n{}", loc, msg, bt),
            &format!("thread {}", thread),
            main && !guarded && std::env::var("SNOWBALLZ_NO_DIALOG").is_err(),
        );
    }));
}
