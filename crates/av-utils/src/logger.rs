use std::io::Write;
use std::sync::RwLock;

use crate::datetime::{Date, DateTime};
use crate::error::{Error, Result};
use log::Log;

// ----------------------------------------------------------------------------
/// How long a run's log is kept: a start removes the older ones (`prune`).
const KEEP_DAYS: i32 = 30;

// ----------------------------------------------------------------------------
struct FileLogger {
    file: RwLock<std::fs::File>,
}

// ----------------------------------------------------------------------------
impl FileLogger {
    fn init(path: &std::path::Path, level: log::LevelFilter) -> Result<()> {
        let date_time = DateTime::now().as_timestamp();
        let file_name = path.join(format!("{date_time}.log"));
        let logger = FileLogger {
            file: RwLock::new(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(file_name)
                    .map_err(|_| Error::Logging)?,
            ),
        };
        log::set_max_level(level);
        log::set_boxed_logger(Box::new(logger)).map_err(|_| Error::Logging)?;
        Ok(())
    }
}

// ----------------------------------------------------------------------------
impl Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            if let Ok(mut file) = self.file.write() {
                let _ = writeln!(
                    &mut file,
                    "{} [{:5}] {}",
                    DateTime::now().as_timestamp(),
                    record.level(),
                    record.args()
                );
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.write() {
            let _ = file.flush();
        }
    }
}

// ----------------------------------------------------------------------------
/// Log to a new file in `dir` (created if needed), named after the time the
/// run started; the logs older than `KEEP_DAYS` are removed first.
pub fn init_logger(dir: &std::path::Path) -> Result<()> {
    let level = if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    std::fs::create_dir_all(dir)?;
    prune(dir, Date::today());
    FileLogger::init(dir, level)?;
    log_panics();
    Ok(())
}

// ----------------------------------------------------------------------------
/// Remove the runs' logs in `dir` from more than `KEEP_DAYS` before `today`,
/// by the date their name starts with; anything else in `dir` stays.
fn prune(dir: &std::path::Path, today: Date) {
    let oldest = DateTime {
        date: Date::new(today.days() - KEEP_DAYS),
        time: None,
    }
    .as_timestamp();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if is_run_log(name) && name[..oldest.len()] < *oldest {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

// ----------------------------------------------------------------------------
/// Whether `name` is a run's log, `YYYYMMDD_HHMMSS.log` (`FileLogger::init`).
fn is_run_log(name: &str) -> bool {
    let digits =
        |part: &str, len: usize| part.len() == len && part.bytes().all(|b| b.is_ascii_digit());
    let stamp = name
        .strip_suffix(".log")
        .and_then(|stamp| stamp.split_once('_'));
    stamp.is_some_and(|(date, time)| digits(date, 8) && digits(time, 6))
}

// ----------------------------------------------------------------------------
/// Log every panic, then report it as before.
fn log_panics() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        let message = info.payload_as_str().unwrap_or("(no message)");
        let location = info
            .location()
            .map_or(String::new(), |at| format!(" at {at}"));
        log::error!("Panic in thread {name}: {message}{location}");
        log::logger().flush();
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn a_start_removes_the_logs_of_a_month_ago() {
        let dir = std::env::temp_dir().join(format!("av-logs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let names = [
            "20261019_235959.log",
            "20261020_000000.log",
            "20261119_101500.log",
            "20250101_000000.log.bak",
            "2025.log",
            "notes.txt",
        ];
        for name in names {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        prune(&dir, Date::from_ymd(2026, 11, 19).unwrap());
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        let mut kept = names[1..].to_vec();
        kept.sort();
        assert_eq!(left, kept);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
