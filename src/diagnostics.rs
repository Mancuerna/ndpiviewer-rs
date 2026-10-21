//! Local, bounded diagnostics. The API deliberately accepts no user-controlled text.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: u64 = 256 * 1024;
const MAX_FILES: usize = 5;
static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

#[derive(Clone, Copy, Debug)]
pub(crate) enum Event {
    Started,
    Closed,
    StartupFailed,
    Panic,
    SlideOpenFailed,
    TileReadFailed,
    AutoContrastFailed,
    ReaderStopped,
    AnnotationLoadFailed,
    AnnotationSaveFailed,
    SettingsLoadFailed,
    SettingsSaveFailed,
    TrackingLoadFailed,
    TrackingSaveFailed,
    RegionExportFailed,
    ViewExportFailed,
    ReportExportFailed,
    AssociatedImageFailed,
    LibraryReadFailed,
}

struct Log {
    file: File,
    bytes: u64,
}

impl Log {
    fn write(&mut self, event: Event) -> io::Result<()> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let line = format!(
            "{timestamp} version={} event={event:?}\n",
            env!("CARGO_PKG_VERSION")
        );
        if self.bytes + line.len() as u64 <= MAX_BYTES {
            self.file.write_all(line.as_bytes())?;
            self.file.flush()?;
            self.bytes += line.len() as u64;
        }
        Ok(())
    }
}

pub(crate) fn directory() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|root| PathBuf::from(root).join("ndpiviewer-rs").join("logs"))
}

fn create_log(directory: &std::path::Path) -> io::Result<Log> {
    fs::create_dir_all(directory)?;
    let mut old: Vec<_> = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("session-")
                && name.ends_with(".log")
                && entry.file_type().is_ok_and(|kind| kind.is_file())
        })
        .collect();
    old.sort_by_key(|entry| entry.file_name());
    let excess = old.len().saturating_sub(MAX_FILES - 1);
    for entry in old.into_iter().take(excess) {
        // Windows may keep another running instance's log open; never force it closed.
        let _ = fs::remove_file(entry.path());
    }
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = directory.join(format!("session-{time:030}-{}.log", std::process::id()));
    Ok(Log {
        file: OpenOptions::new().write(true).create_new(true).open(path)?,
        bytes: 0,
    })
}

pub(crate) fn init() {
    if let Some(directory) = directory() {
        if let Ok(log) = create_log(&directory) {
            let _ = LOG.set(Mutex::new(log));
        }
    }
    // Do not delegate to the default hook: payloads and backtraces can expose paths
    // and slide metadata. This hook covers panics on UI and worker threads alike.
    std::panic::set_hook(Box::new(|_| record(Event::Panic)));
    record(Event::Started);
}

pub(crate) fn record(event: Event) {
    if let Some(log) = LOG.get() {
        // A panic while logging must never deadlock the panic hook.
        if let Ok(mut log) = log.try_lock() {
            let _ = log.write(event);
        }
    }
}

pub(crate) fn available() -> bool {
    LOG.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_probe() {
        if std::env::var_os("NDPI_TEST_PANIC").is_some() {
            init();
            panic!("Private patient name and C:\\private\\patient.ndpi");
        }
    }

    #[test]
    fn panic_hook_records_event_without_payload_or_paths() {
        let dir = std::env::temp_dir().join(format!("ndpi-panic-test-{}", std::process::id()));
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "diagnostics::tests::panic_probe", "--nocapture"])
            .env("NDPI_TEST_PANIC", "1")
            .env("LOCALAPPDATA", &dir)
            .output()
            .unwrap();
        assert!(!result.status.success());
        let logs = dir.join("ndpiviewer-rs/logs");
        let log = fs::read_dir(logs).unwrap().next().unwrap().unwrap();
        let text = fs::read_to_string(log.path()).unwrap();
        assert!(text.contains("event=Started") && text.contains("event=Panic"));
        assert!(!text.contains("patient") && !text.contains("private") && !text.contains("\\"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bounded_logs_rotate_without_touching_other_files() {
        let dir = std::env::temp_dir().join(format!("ndpi-log-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("keep.txt"), "untouched").unwrap();
        for _ in 0..8 {
            let mut log = create_log(&dir).unwrap();
            for _ in 0..6000 {
                log.write(Event::SlideOpenFailed).unwrap();
            }
        }
        let files: Vec<_> = fs::read_dir(&dir).unwrap().map(Result::unwrap).collect();
        assert_eq!(files.len(), MAX_FILES + 1);
        for entry in files {
            if entry.path().extension().is_some_and(|e| e == "log") {
                let data = fs::read_to_string(entry.path()).unwrap();
                assert!(data.len() as u64 <= MAX_BYTES);
                assert!(data
                    .lines()
                    .all(|line| line.ends_with("event=SlideOpenFailed")));
                assert!(!data.contains("\\") && !data.contains(".ndpi"));
            }
        }
        assert_eq!(
            fs::read_to_string(dir.join("keep.txt")).unwrap(),
            "untouched"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
