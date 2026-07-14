//! File logging. A packaged (MSIX) build has no console, so `env_logger`'s
//! stderr output is invisible — mirror every line into a log file the user can
//! open from the tray (`tray::open_logs`) to diagnose an installed build.

use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Mutex;

/// Hard cap on the log file. Truncated to empty on each launch anyway (so it
/// never accumulates across runs); this only bounds a single long-running
/// session — past it the file restarts from the top.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// The log file, written and opened at the same absolute path so both agree.
/// A packaged (MSIX) process's writes to `%LOCALAPPDATA%\Capcove` are
/// redirected into its package LocalCache, and a plain-path open wouldn't find
/// them — so for a packaged build resolve that real cache location directly.
pub fn log_file_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
        if let Some(pfn) = crate::win_util::package_family_name() {
            return Some(
                local.join("Packages").join(pfn)
                    .join("LocalCache").join("Local").join("Capcove").join("capcove.log"),
            );
        }
        Some(local.join("Capcove").join("capcove.log"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/Capcove/capcove.log"))
    }
}

/// Writes each log line to both stderr (dev terminal) and the log file,
/// restarting the file from the top once it passes `MAX_LOG_BYTES`.
struct Tee(Mutex<(std::fs::File, u64)>);

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        if let Ok(mut guard) = self.0.lock() {
            let (file, written) = &mut *guard;
            if *written + buf.len() as u64 > MAX_LOG_BYTES && file.set_len(0).is_ok() {
                let _ = file.seek(SeekFrom::Start(0));
                *written = 0;
            }
            if file.write_all(buf).is_ok() {
                *written += buf.len() as u64;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        if let Ok(mut guard) = self.0.lock() {
            let _ = guard.0.flush();
        }
        Ok(())
    }
}

/// Installs the global logger. Defaults to `info` so recording/ffmpeg
/// diagnostics show up without `RUST_LOG` set (still overridable). Falls back
/// to plain stderr if the log file can't be opened.
pub fn init() {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));

    if let Some(path) = log_file_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Truncate per launch so the file always shows the current session.
        if let Ok(file) = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&path) {
            builder.target(env_logger::Target::Pipe(Box::new(Tee(Mutex::new((file, 0))))));
        }
    }

    let _ = builder.try_init();
    log::info!("Capcove {} starting", env!("CARGO_PKG_VERSION"));
}
