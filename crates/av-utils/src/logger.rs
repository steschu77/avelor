use std::io::Write;
use std::sync::RwLock;

use crate::datetime::DateTime;
use crate::error::{Error, Result};
use log::Log;

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
/// run started.
pub fn init_logger(dir: &std::path::Path) -> Result<()> {
    let level = if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    std::fs::create_dir_all(dir)?;
    FileLogger::init(dir, level)?;
    log_panics();
    Ok(())
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
